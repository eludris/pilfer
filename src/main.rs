use crossterm::{
    event::{self, DisableFocusChange, EnableFocusChange, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use discord_rich_presence::{
    activity::{Activity, Assets, Button, Timestamps},
    DiscordIpc, DiscordIpcClient,
};
use futures::{SinkExt, StreamExt};
use notify_rust::Notification;
#[cfg(target_os = "linux")]
use notify_rust::NotificationHandle;
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    env,
    error::Error,
    fmt::Display,
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
    vec,
};
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Corner, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use unicode_width::UnicodeWidthStr;

const REST_URL: &str = "https://eludris.tooty.xyz/";
const GATEWAY_URL: &str = "wss://eludris.tooty.xyz/ws/";
const PILFER_APP_ID: &str = "1028728489165193247";

#[derive(Debug, Serialize, Deserialize)]
struct RatelimitResponse {
    data: RatelimitData,
}

#[derive(Debug, Serialize, Deserialize)]
struct RatelimitData {
    retry_after: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct EludrisMessage {
    author: String,
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum MessageResponse {
    Message(EludrisMessage),
    Ratelimited(RatelimitResponse),
}

// While in hindsight this might look like it's modeled in a bad way, you're right, it's modeled in
// a bad way.
#[derive(Debug)]
struct SystemMessage {
    content: String,
}

#[derive(Debug)]
enum PilferMessage {
    Eludris(EludrisMessage),
    System(SystemMessage),
}

impl Display for EludrisMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("[{}]: {}", self.author, self.content))
    }
}

impl Display for PilferMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PilferMessage::Eludris(msg) => write!(f, "{}", msg),
            PilferMessage::System(msg) => write!(f, "{}", msg.content),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct InfoResponse {
    instance_name: String,
}

struct AppContext {
    /// Current input
    input: String,
    /// User name
    name: String,
    /// Received messages
    messages: Arc<Mutex<Vec<(PilferMessage, Style)>>>,
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
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |p| {
        disable_raw_mode().unwrap();
        let terminal = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(terminal).unwrap();
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableFocusChange
        )
        .unwrap();
        hook(p);
    }));
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

    let rest_url = env::var("REST_URL").unwrap_or_else(|_| REST_URL.to_string());
    let http_client = Client::new();
    let info: InfoResponse = http_client
        .get(&rest_url)
        .send()
        .await
        .expect("Can not connect to Oprish")
        .json()
        .await
        .expect("Server returned a malformed info response");

    // Discord rich presence stuff
    let mut client = DiscordIpcClient::new(PILFER_APP_ID).unwrap();
    if client.connect().is_ok() {
        let assets = Assets::new()
            .large_image("pilfer")
            .large_text("Using Pilfer; An Eludris TUI interface");

        let timestamp = Timestamps::new().start(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs() as i64,
        );

        let buttons = vec![
            Button::new("Eludris", "https://github.com/eludris"),
            Button::new("Pilfer", "https://github.com/eludris/pilfer"),
        ];

        client
            .set_activity(
                Activity::new()
                    .details("Chatting on Eludris")
                    .state(&format!("Talking on {} as {}", info.instance_name, name))
                    .assets(assets)
                    .timestamps(timestamp)
                    .buttons(buttons),
            )
            .unwrap();
    }

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
        name: name.clone(),
        messages: Arc::clone(&messages),
        http_client,
        rest_url,
        focused: Arc::clone(&focused),
        #[cfg(target_os = "linux")]
        notification: Arc::clone(&notification),
    };

    tokio::spawn(async move {
        let mut wait = 0;
        loop {
            let gateway_url = env::var("GATEWAY_URL").unwrap_or_else(|_| GATEWAY_URL.to_string());

            if wait > 0 {
                time::sleep(Duration::from_secs(wait)).await;
            }

            let socket = match connect_async(gateway_url).await {
                Ok((socket, _)) => socket,
                Err(err) => {
                    if wait < 64 {
                        wait *= 2;
                    }
                    messages.lock().unwrap().push((
                        PilferMessage::System(SystemMessage {
                            content: format!(
                                "Could not connect: {:?}, reconnecting in {}s (press Ctrl+C to exit)",
                                err, wait
                            ),
                        }),
                        Style::default().fg(Color::Red),
                    ));
                    continue;
                }
            };
            wait = 0;
            messages.lock().unwrap().push((
                PilferMessage::System(SystemMessage {
                    content: "Connected to Pandemonium".to_string(),
                }),
                Style::default().fg(Color::Green),
            ));

            let (mut tx, mut rx) = socket.split();

            // Handle ping-pong loop
            let ping = tokio::spawn(async move {
                loop {
                    match tx.send(Message::Ping(vec![])).await {
                        Ok(()) => time::sleep(Duration::from_secs(20)).await,
                        Err(_) => break,
                    };
                }
            });

            // Handle receiving pandemonium events
            while let Some(Ok(msg)) = rx.next().await {
                match msg {
                    Message::Text(msg) => {
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
                        // Highlight the message if your name got mentioned
                        let style = if msg.content.to_lowercase().contains(&name.to_lowercase()) {
                            Style::default().fg(Color::Yellow)
                        } else {
                            Style::default()
                        };
                        // Add to the Pifler's context
                        messages
                            .lock()
                            .unwrap()
                            .push((PilferMessage::Eludris(msg), style));
                    }
                    Message::Close(frame) => {
                        if let Some(frame) = frame {
                            messages.lock().unwrap().push((
                                PilferMessage::System(SystemMessage {
                                    content: format!("{}, retrying", frame.reason),
                                }),
                                Style::default().fg(Color::Red),
                            ))
                        }
                        ping.abort();
                        continue;
                    }
                    _ => {}
                }
            }
        }
    });

    let res = run_app(&mut terminal, app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableFocusChange
    )?;

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
                            let messages = Arc::clone(&app.messages);
                            tokio::spawn(handle_request(request, messages));
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

    let messages: Vec<ListItem> = app
        .messages
        .lock()
        .unwrap()
        .iter()
        .flat_map(|m| {
            // Seperates lines which are longer than the view width with newline characters
            // since it doesn't wrap sometimes for some reason
            m.0.to_string()
                .lines()
                .map(|l| {
                    // Probably a newline
                    if l.is_empty() {
                        ListItem::new("\n")
                    } else {
                        ListItem::new(
                            {
                                l.chars().enumerate().map(|(i, x)| {
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
                            }
                            .collect::<String>(),
                        )
                        .style(m.1)
                    }
                })
                .collect::<Vec<ListItem>>()
        })
        .rev()
        .collect();

    let message_list = List::new(messages)
        .block(Block::default().borders(Borders::ALL).title("Messages"))
        .start_corner(Corner::BottomLeft);
    f.render_widget(message_list, chunks[0]);

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

async fn handle_request(
    request: RequestBuilder,
    messages: Arc<Mutex<Vec<(PilferMessage, Style)>>>,
) {
    let res = request.send().await;
    match res {
        Ok(res) => match res.json::<MessageResponse>().await {
            Ok(resp) => match resp {
                MessageResponse::Ratelimited(resp) => messages.lock().unwrap().push((
                    PilferMessage::System(SystemMessage {
                        content: format!(
                            "System: You've been ratelimited, try in {}s",
                            resp.data.retry_after / 1000
                        ),
                    }),
                    Style::default().fg(Color::Cyan),
                )),
                MessageResponse::Message(_) => {}
            },
            Err(err) => messages.lock().unwrap().push((
                PilferMessage::System(SystemMessage {
                    content: format!("System: Couldn't send message: {:?}", err),
                }),
                Style::default().fg(Color::Red),
            )),
        },
        Err(err) => messages.lock().unwrap().push((
            PilferMessage::System(SystemMessage {
                content: format!("System: Couldn't send message: {:?}", err),
            }),
            Style::default().fg(Color::Red),
        )),
    };
}
