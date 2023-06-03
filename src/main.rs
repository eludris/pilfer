#![allow(clippy::uninlined_format_args)]
mod gateway;
mod models;
mod ui;
mod user;
mod utils;

use crossterm::{
    cursor::{CursorShape, SetCursorShape},
    event::{self, DisableFocusChange, EnableFocusChange, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use discord_rich_presence::{
    activity::{Activity, Assets, Button, Timestamps},
    DiscordIpc, DiscordIpcClient,
};
use gateway::handle_gateway;
use models::{AppContext, PilferMessage, Response, SystemMessage};
use reqwest::{Client, RequestBuilder};
use serde_json::json;
use std::{
    env,
    error::Error,
    io,
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
    vec,
};
use todel::models::{ErrorResponse, InstanceInfo, Message};
use tui::{
    backend::{Backend, CrosstermBackend},
    style::{Color, Style},
    Terminal,
};
use ui::ui;

pub const REST_URL: &str = "https://eludris.tooty.xyz/";
pub const PILFER_APP_ID: &str = "1028728489165193247";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |p| {
        disable_raw_mode().unwrap();
        let terminal = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(terminal).unwrap();
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableFocusChange,
            SetCursorShape(CursorShape::Block),
        )
        .unwrap();
        hook(p);
    }));
    let mut stdout = io::stdout();

    let flag = env::args().nth(1);
    if let Some(ref flag) = flag {
        if flag == "-v" || flag == "--version" {
            println!("Version: {}", VERSION);
            return Ok(());
        }
    }

    let rest_url = env::var("INSTANCE_URL").unwrap_or_else(|_| REST_URL.to_string());
    let http_client = Client::new();
    let info: InstanceInfo = http_client
        .get(&rest_url)
        .send()
        .await
        .expect("Cannot connect to Oprish")
        .json()
        .await
        .expect("Server returned a malformed info response");

    let (token, name) = user::get_token(&info, &http_client).await?;

    if flag == Some("--verify".to_string()) {
        match env::args().nth(2) {
            Some(code) => {
                if !http_client
                    .post(format!("{}/users/verify?code={}", info.oprish_url, code))
                    .header("Authorization", &token)
                    .send()
                    .await
                    .unwrap()
                    .status()
                    .is_success()
                {
                    println!("Verification failed");
                    std::process::exit(1);
                }
            }
            None => {
                println!("Usage: pilfer --verify <code>");
                std::process::exit(1);
            }
        };
    };

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
            Button::new("Eludris", "https://eludris.pages.dev/"),
            Button::new("Pilfer", "https://github.com/eludris/pilfer/"),
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
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableFocusChange,
        SetCursorShape(CursorShape::Line),
    )?;
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

    tokio::spawn(handle_gateway(
        info.pandemonium_url,
        messages,
        focused,
        #[cfg(target_os = "linux")]
        notification,
        name,
        token.clone(),
    ));

    let res = run_app(&mut terminal, app, token);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableFocusChange,
        SetCursorShape(CursorShape::Block),
    )?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: AppContext,
    token: String,
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
                                .header("Authorization", &token)
                                .json(&json!({"content": app.input.drain(..).collect::<String>()}));
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

async fn handle_request(
    request: RequestBuilder,
    messages: Arc<Mutex<Vec<(PilferMessage, Style)>>>,
) {
    let res = request.send().await;
    match res {
        Ok(res) => match res.json::<Response<Message>>().await {
            Ok(resp) => match resp {
                Response::Error(resp) => match resp {
                    ErrorResponse::RateLimited { retry_after, .. } => {
                        messages.lock().unwrap().push((
                            PilferMessage::System(SystemMessage {
                                content: format!(
                                    "System: You've been ratelimited, retry in {}s",
                                    retry_after / 1000
                                ),
                            }),
                            Style::default().fg(Color::Red),
                        ))
                    }
                    _ => messages.lock().unwrap().push((
                        PilferMessage::System(SystemMessage {
                            content: format!("System: Couldn't send message: {:?}", resp),
                        }),
                        Style::default().fg(Color::Red),
                    )),
                },
                Response::Success(_) => {}
            },
            Err(_) => messages.lock().unwrap().push((
                PilferMessage::System(SystemMessage {
                    content: "System: Couldn't send message: got invalid response".to_string(),
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
