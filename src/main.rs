use crossterm::{
    event::{self, DisableFocusChange, EnableFocusChange, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::{SinkExt, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    env,
    error::Error,
    fmt::Display,
    io::{self, Write},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
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
    input: String,
    name: String,
    messages: Arc<Mutex<Vec<EludrisMessage>>>,
    http_client: Client,
    rest_url: String,
    focused: Arc<AtomicBool>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    print!("What's your name? > ");
    let mut stdout = io::stdout();
    stdout.flush().unwrap();

    let mut name = String::new();

    io::stdin().read_line(&mut name).unwrap();

    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, EnableFocusChange)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let messages = Arc::new(Mutex::new(vec![]));
    let focused = Arc::new(AtomicBool::new(true));

    let app = AppContext {
        input: String::new(),
        name: name.trim().to_string(),
        messages: Arc::clone(&messages),
        http_client: Client::new(),
        rest_url: env::var("REST_URL").unwrap_or_else(|_| REST_URL.to_string()),
        focused: Arc::clone(&focused),
    };

    let gateway_url = env::var("GATEWAY_URL").unwrap_or_else(|_| GATEWAY_URL.to_string());

    let (socket, _) = connect_async(gateway_url).await.unwrap();

    let (mut tx, rx) = socket.split();

    tokio::spawn(async move {
        loop {
            tx.send(Message::Ping(vec![])).await.unwrap();
            time::sleep(Duration::from_secs(15)).await;
        }
    });

    tokio::spawn(async move {
        rx.for_each(|msg| async {
            if let Ok(Message::Text(msg)) = msg {
                let msg: EludrisMessage = serde_json::from_str(&msg).unwrap();
                if !focused.load(std::sync::atomic::Ordering::Relaxed) {
                    Command::new("notify-send")
                        .arg("-r")
                        .arg("3903492")
                        .arg("New Eludris Message")
                        .arg(msg.to_string())
                        .spawn()
                        .unwrap();
                }
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
                    Command::new("notify-send")
                        .arg("-r")
                        .arg("3903492")
                        .arg("clear")
                        .arg("-t")
                        .arg("1")
                        .spawn()
                        .unwrap();
                }

                Event::FocusLost => app.focused.store(false, Ordering::Relaxed),
                Event::Key(key) => match key.code {
                    KeyCode::Enter => {
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
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            match c {
                                'c' => break,
                                'l' => app.messages.lock().unwrap().clear(),
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

    let messages: String = app
        .messages
        .lock()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join("\n");

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

    let messages: Vec<String> = messages
        .lines()
        .rev()
        .take((chunks[0].height - 2) as usize)
        .map(ToString::to_string)
        .collect();

    let messages: String = messages
        .into_iter()
        .rev()
        .collect::<Vec<String>>()
        .join("\n");

    let messages = Paragraph::new(messages)
        // .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("Messages"));
    f.render_widget(messages, chunks[0]);

    let input: String = app
        .input
        .chars()
        .rev()
        .take((chunks[1].width - 2) as usize)
        .collect();
    let input: String = input.chars().rev().collect();

    let input =
        Paragraph::new(input.as_ref()).block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(input, chunks[1]);
    f.set_cursor(chunks[1].x + app.input.width() as u16 + 1, chunks[1].y + 1);
}
