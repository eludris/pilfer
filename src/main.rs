use crossterm::{
    event::{self, DisableFocusChange, EnableFocusChange, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::{SinkExt, StreamExt};
use notify_rust::Notification;
#[cfg(target_os = "linux")]
use notify_rust::NotificationHandle;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    env,
    error::Error,
    fmt::Display,
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex},
    time::Duration,
    vec,
};
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Serialize, Deserialize)]
struct EludrisMessage {
    author: String,
    content: String,
}

impl Display for EludrisMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("[{}]: {}", self.author, self.content))
    }
}

const REST_URL: &str = "https://eludris.tooty.xyz/";
const GATEWAY_URL: &str = "wss://eludris.tooty.xyz/ws/";

struct AppContext {
    /// Current input
    input: String,
    /// User name
    name: String,
    /// Received messages
    messages: Arc<Mutex<Vec<EludrisMessage>>>,
    /// Reqwest HTTPClient
    http_client: Client,
    /// Oprish URL
    rest_url: String,
    /// Whether the user is currently focused.
    focused: Arc<AtomicBool>,
    /// The notification
    #[cfg(target_os = "linux")]
    notification: Arc<Mutex<Option<NotificationHandle>>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut stdout = io::stdout();

    // Get a name that complies with Eludris' 2-32 name character limit
    let name = env::var("PILFER_NAME").unwrap_or_else(|_| loop {
        print!("What's your name? > ");
        stdout.flush().unwrap();

        let mut name = String::new();

        io::stdin().read_line(&mut name).unwrap();

        let name = name.trim();

        if name.len() <= 32 && name.len() >= 2 {
            break name.to_string();
        }

        println!("Your name has to be between 2 and 32 characters long, try again!");
    });

    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, EnableFocusChange)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let messages = Arc::new(Mutex::new(vec![]));

    let focused = Arc::new(AtomicBool::new(true));
    #[cfg(target_os = "linux")]
    let notification = Arc::new(Mutex::new(None));

    let app = AppContext {
        input: String::new(),
        name,
        messages: Arc::clone(&messages),
        http_client: Client::new(),
        rest_url: env::var("REST_URL").unwrap_or_else(|_| REST_URL.to_string()),
        focused: Arc::clone(&focused),
        #[cfg(target_os = "linux")]
        notification: Arc::clone(&notification),
    };

    let gateway_url = env::var("GATEWAY_URL").unwrap_or_else(|_| GATEWAY_URL.to_string());

    let (socket, _) = connect_async(gateway_url).await.unwrap();

    let (mut tx, rx) = socket.split();

    // Handle ping-pong loop
    tokio::spawn(async move {
        loop {
            tx.send(Message::Ping(vec![])).await.unwrap();
            time::sleep(Duration::from_secs(15)).await;
        }
    });

    // Handle receiving pandemonium events
    tokio::spawn(async move {
        rx.for_each(|msg| async {
            if let Ok(Message::Text(msg)) = msg {
                let msg: EludrisMessage = serde_json::from_str(&msg).unwrap();
                if !focused.load(std::sync::atomic::Ordering::Relaxed) {
                    #[cfg(target_os = "linux")]
                    {
                        let mut notif = notification.lock().unwrap();
                        match notif.as_mut() {
                            Some(notif) => {
                                notif.body(&msg.to_string());
                                notif.update()
                            }
                            None => {
                                *notif = match Notification::new()
                                    .summary("New Pilfer Message")
                                    .body(&msg.to_string())
                                    .show()
                                {
                                    Ok(notif) => Some(notif),
                                    Err(_) => None,
                                };
                            }
                        }
                    }
                    #[cfg(not(target_os = "linux"))]
                    Notification::new()
                        .summary("New Pilfer Message")
                        .body(&msg.to_string())
                        .show()
                        .ok();
                }
                // Add to the Pifler's context
                messages.lock().unwrap().push(msg);
            }
        })
        .await;
    });

    let res = run_app(&mut terminal, app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableFocusChange
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: AppContext,
) -> Result<(), Box<dyn Error>> {
    loop {
        terminal.draw(|f| ui(f, &app))?;

        if event::poll(Duration::from_millis(100))? {
            let event = event::read()?;
            match event {
                Event::FocusGained => {
                    app.focused.store(true, Ordering::Relaxed);
                    // Kill the displayed notification if it currently exists
                    #[cfg(target_os = "linux")]
                    if let Some(notif) = app.notification.lock().unwrap().take() {
                        notif.close();
                    }
                }
                Event::FocusLost => app.focused.store(false, Ordering::Relaxed),
                Event::Key(key) => match key.code {
                    KeyCode::Enter => {
                        // Send a message
                        if !app.input.is_empty() {
                            let request = app
                                .http_client
                                .post(format!("{}/messages/", app.rest_url))
                                .json(
                                    &json!({"author": app.name, "content": app.input.drain(..).collect::<String>()})
                                );
                            tokio::spawn(async { request.send().await.unwrap() });
                        }
                    }
                    KeyCode::Char(c) => {
                        // Keybingings go here
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            match c {
                                'c' => break,
                                'l' => app.messages.lock().unwrap().clear(),
                                ' ' => app.input.push('\n'),
                                _ => {}
                            }
                        } else {
                            app.input.push(c);
                        }
                    }
                    KeyCode::Backspace => {
                        app.input.pop();
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    Ok(())
}

fn ui<B: Backend>(f: &mut Frame<B>, app: &AppContext) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)].as_ref())
        .split(f.size());

    // Handle making the messages display with mutiline support and with the paragraph preview
    // scrolling down to show the last message, this is quite painful to do thanks to tui-rs

    // Convert them to one string
    let messages: String = app
        .messages
        .lock()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join("\n");

    // Make messages longer than the view width seperate lines
    let messages = messages
        .lines()
        .map(|m| {
            m.chars()
                .enumerate()
                .map(|(i, x)| {
                    format!(
                        "{}{}",
                        x,
                        if (i + 1) % (chunks[0].width - 2) as usize == 0 {
                            "\n"
                        } else {
                            ""
                        }
                    )
                })
                .collect::<String>()
        })
        .collect::<Vec<String>>()
        .join("\n");

    // Reverse and only take enough lines to fill the view height since any more would make the
    // latest messages not visible
    let messages: Vec<String> = messages
        .lines()
        .rev()
        .take((chunks[0].height - 2) as usize)
        .map(ToString::to_string)
        .collect();

    // Reverse them again
    let messages: String = messages
        .into_iter()
        .rev()
        .collect::<Vec<String>>()
        .join("\n");

    let messages =
        Paragraph::new(messages).block(Block::default().borders(Borders::ALL).title("Messages"));
    f.render_widget(messages, chunks[0]);

    // Reverse the input to make it scroll to the right if you exceed the view width while typing
    let input_text: String = app
        .input
        .split('\n')
        .last()
        .unwrap_or("")
        .chars()
        .rev()
        .take((chunks[1].width - 2) as usize)
        .collect();
    let input_text: String = input_text.chars().rev().collect();

    let input = Paragraph::new(input_text.as_ref())
        .block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(input, chunks[1]);
    f.set_cursor(chunks[1].x + input_text.width() as u16 + 1, chunks[1].y + 1);
}
