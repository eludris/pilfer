#![allow(clippy::uninlined_format_args)]
mod gateway;
mod models;
mod ui;
mod user;
mod utils;
mod version;

use anyhow::anyhow;
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
use std::{
    collections::HashMap,
    env,
    error::Error,
    io,
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
    vec,
};
use todel::models::{ErrorResponse, InstanceInfo, Message, MessageCreate};
use tokio::{sync::Mutex as AsyncMutex, task::spawn_blocking};
use tui::{
    backend::{Backend, CrosstermBackend},
    style::{Color, Style},
    Terminal,
};
use ui::ui;
use version::check_version;

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

    // pub fn check_version(info: &InstanceInfo) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Err(e) = check_version(&info) {
        println!("Error: {}", e);
        return Ok(());
    }

    let (token, name) = user::get_token(&info, &http_client).await?;

    if flag == Some("--verify".to_string()) {
        match env::args().nth(2) {
            Some(code) => {
                let res = http_client
                    .post(format!("{}/users/verify?code={}", info.oprish_url, code))
                    .header("Authorization", &token)
                    .send()
                    .await
                    .expect("Can not connect to Oprish");
                if res.status().is_success() {
                    println!("Successfully verified");
                } else {
                    match res.json::<ErrorResponse>().await? {
                        ErrorResponse::Validation {
                            value_name, info, ..
                        } => return Err(anyhow!("{}: {}", value_name, info)),
                        _ => return Err(anyhow!("Could not verify: {:?}", info)),
                    }
                }
            }
            None => {
                return Err(anyhow!("Usage: pilfer --verify <code>"));
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
    let terminal = Terminal::new(backend)?;

    let app = Arc::new(AppContext {
        input: Mutex::new(String::new()),
        name,
        messages: Mutex::new(vec![]),
        users: AsyncMutex::new(HashMap::new()),
        http_client,
        rest_url: info.oprish_url,
        gateway_url: info.pandemonium_url,
        focused: AtomicBool::new(true),
        users_list_enabled: AtomicBool::new(true),
        #[cfg(target_os = "linux")]
        notification: Mutex::new(None),
    });

    tokio::spawn(handle_gateway(Arc::clone(&app), token.clone()));

    let (mut terminal, res) = spawn_blocking(move || run_app(terminal, app, token))
        .await
        .unwrap();

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
    mut terminal: Terminal<B>,
    app: Arc<AppContext>,
    token: String,
) -> (Terminal<B>, Result<(), Box<dyn Error + Send + Sync>>) {
    let mut logic = || -> Result<bool, Box<dyn Error + Send + Sync>> {
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
                        if !app.input.lock().unwrap().is_empty() {
                            let request = app
                                .http_client
                                .post(format!("{}/messages/", app.rest_url))
                                .header("Authorization", &token)
                                .json(&MessageCreate {
                                    content: app
                                        .input
                                        .lock()
                                        .unwrap()
                                        .drain(..)
                                        .collect::<String>(),
                                    disguise: None,
                                });
                            tokio::spawn(handle_request(request, Arc::clone(&app)));
                        }
                    }
                    KeyCode::Char(c) => {
                        // Keybingings go here
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            match c {
                                'c' => return Ok(false),
                                'l' => app.messages.lock().unwrap().clear(),
                                ' ' => app.input.lock().unwrap().push('\n'),
                                'u' => {
                                    app.users_list_enabled.fetch_xor(false, Ordering::SeqCst);
                                }
                                _ => {}
                            };
                        } else {
                            app.input.lock().unwrap().push(c);
                        }
                    }
                    KeyCode::Backspace => {
                        app.input.lock().unwrap().pop();
                    }
                    _ => {}
                },
                _ => {}
            }
        };

        Ok(true)
    };

    loop {
        match logic() {
            Ok(true) => {}
            Ok(false) => break,
            Err(err) => return (terminal, Err(err)),
        }
    }

    (terminal, Ok(()))
}

async fn handle_request(request: RequestBuilder, app: Arc<AppContext>) {
    let res = request.send().await;

    match res {
        Ok(res) => match res.json::<Response<Message>>().await {
            Ok(resp) => match resp {
                Response::Error(resp) => match resp {
                    ErrorResponse::RateLimited { retry_after, .. } => {
                        app.messages.lock().unwrap().push((
                            PilferMessage::System(SystemMessage {
                                content: format!(
                                    "System: You've been ratelimited, retry in {}s",
                                    retry_after / 1000
                                ),
                            }),
                            Style::default().fg(Color::Red),
                        ))
                    }
                    _ => app.messages.lock().unwrap().push((
                        PilferMessage::System(SystemMessage {
                            content: format!("System: Couldn't send message: {:?}", resp),
                        }),
                        Style::default().fg(Color::Red),
                    )),
                },
                Response::Success(_) => {}
            },
            Err(err) => app.messages.lock().unwrap().push((
                PilferMessage::System(SystemMessage {
                    content: format!(
                        "System: Couldn't send message: got invalid response: {:?}",
                        err
                    ),
                }),
                Style::default().fg(Color::Red),
            )),
        },
        Err(err) => app.messages.lock().unwrap().push((
            PilferMessage::System(SystemMessage {
                content: format!("System: Couldn't send message: {:?}", err),
            }),
            Style::default().fg(Color::Red),
        )),
    };
}
