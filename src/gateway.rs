use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use notify_rust::Notification;
use rand::{rngs::StdRng, Rng, SeedableRng};
use todel::models::{ClientPayload, Message, ServerPayload, StatusType, User};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tui::style::{Color, Style};

use crate::models::{AppContext, PilferMessage, Response, SystemMessage};

// It's either this or excessive amounts of arcs and mutexes over AppContext.
#[allow(clippy::too_many_arguments)]
pub async fn handle_gateway(app: Arc<AppContext>, token: String) {
    let rng = Arc::new(AsyncMutex::new(StdRng::from_entropy()));
    let mut wait = 0;
    loop {
        if wait > 0 {
            time::sleep(Duration::from_secs(wait)).await;
        }

        let socket = match connect_async(&app.gateway_url).await {
            Ok((socket, _)) => socket,
            Err(err) => {
                if wait < 64 {
                    wait *= 2;
                }
                app.messages.lock().unwrap().push((
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

        let (mut tx, mut rx) = socket.split();
        let ping;
        loop {
            if let Some(Ok(WsMessage::Text(msg))) = rx.next().await {
                if let Ok(msg) = serde_json::from_str(&msg) {
                    match msg {
                        ServerPayload::Hello {
                            heartbeat_interval, ..
                        } => {
                            // Authenticate
                            if let Err(err) = tx
                                .send(WsMessage::Text(
                                    serde_json::to_string(&ClientPayload::Authenticate(
                                        token.clone(),
                                    ))
                                    .unwrap(),
                                ))
                                .await
                            {
                                app.messages.lock().unwrap().push((
                                    PilferMessage::System(SystemMessage {
                                        content: format!("Could not authenticate: {:?}", err),
                                    }),
                                    Style::default().fg(Color::Red),
                                ));
                                return;
                            }

                            // Handle ping-pong loop
                            let rng = Arc::clone(&rng);
                            ping = tokio::spawn(async move {
                                time::sleep(Duration::from_millis(
                                    rng.lock().await.gen_range(0..heartbeat_interval),
                                ))
                                .await;
                                while let Ok(()) = tx
                                    .send(WsMessage::Text(
                                        serde_json::to_string(&ClientPayload::Ping).unwrap(),
                                    ))
                                    .await
                                {
                                    time::sleep(Duration::from_millis(heartbeat_interval)).await;
                                }
                            });
                            break;
                        }
                        ServerPayload::RateLimit { wait } => {
                            app.messages.lock().unwrap().push((
                                PilferMessage::System(SystemMessage {
                                    content: format!("Rate limited, waiting {}s", wait / 1000),
                                }),
                                Style::default().fg(Color::Red),
                            ));
                            time::sleep(Duration::from_millis(wait)).await;
                        }
                        _ => continue,
                    }
                }
            }
        }

        app.messages.lock().unwrap().push((
            PilferMessage::System(SystemMessage {
                content: "Connected to Pandemonium".to_string(),
            }),
            Style::default().fg(Color::Green),
        ));

        // Handle receiving pandemonium events
        while let Some(Ok(msg)) = rx.next().await {
            match msg {
                WsMessage::Text(msg) => {
                    let msg: Message = match serde_json::from_str(&msg) {
                        Ok(ServerPayload::MessageCreate(msg)) => msg,
                        Ok(ServerPayload::Authenticated {
                            user,
                            users: online_users,
                        }) => {
                            app.messages.lock().unwrap().push((
                                PilferMessage::System(SystemMessage {
                                    content: "Authenticated with Pandemonium!".to_string(),
                                }),
                                Style::default().fg(Color::Green),
                            ));
                            let mut users = app.users.lock().await;
                            users.insert(user.id, user);
                            users.extend(online_users.into_iter().map(|user| (user.id, user)));
                            continue;
                        }
                        Ok(ServerPayload::UserUpdate(user)) => {
                            if user.status.status_type != StatusType::Offline {
                                app.users.lock().await.insert(user.id, user);
                            }
                            continue;
                        }
                        Ok(ServerPayload::PresenceUpdate { user_id, status }) => {
                            let mut users = app.users.lock().await;

                            if status.status_type == StatusType::Offline {
                                users.remove(&user_id);
                                continue;
                            };

                            if let Some(user) = users.get_mut(&user_id) {
                                user.status = status;
                            } else {
                                let user = match app
                                    .http_client
                                    .get(format!("{}/users/{}", app.rest_url, user_id))
                                    .send()
                                    .await
                                    .expect("Can not connect to Oprish")
                                    .json::<Response<User>>()
                                    .await
                                    .unwrap()
                                {
                                    Response::Success(user) => user,
                                    Response::Error(err) => {
                                        app.messages.lock().unwrap().push((
                                            PilferMessage::System(SystemMessage {
                                                content: format!(
                                                    "Could not get user {}: {}",
                                                    user_id, err
                                                ),
                                            }),
                                            Style::default().fg(Color::Red),
                                        ));
                                        continue;
                                    }
                                };
                                users.insert(user.id, user);
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    if !app.focused.load(std::sync::atomic::Ordering::Relaxed) {
                        #[cfg(target_os = "linux")]
                        {
                            let mut notif = app.notification.lock().unwrap();
                            match notif.as_mut() {
                                Some(notif) => {
                                    notif
                                        .summary(&format!("New Pilfer message from {}", msg.author))
                                        .body(&msg.message.content.to_string());
                                    notif.update()
                                }
                                None => {
                                    *notif = match Notification::new()
                                        .summary(&format!("New Pilfer message from {}", msg.author))
                                        .body(&msg.message.content)
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
                            .summary(&format!("New Pilfer message from {}", msg.author))
                            .body(&msg.message.content)
                            .show()
                            .ok();
                    }
                    // Highlight the message if your name got mentioned
                    let style = if msg
                        .message
                        .content
                        .to_lowercase()
                        .contains(&app.name.to_lowercase())
                    {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default()
                    };
                    // Add to the Pifler's context
                    app.messages
                        .lock()
                        .unwrap()
                        .push((PilferMessage::Eludris(Box::new(msg)), style));
                }
                WsMessage::Close(frame) => {
                    if let Some(frame) = frame {
                        if wait < 64 {
                            wait *= 2;
                        }
                        app.messages.lock().unwrap().push((
                            PilferMessage::System(SystemMessage {
                                content: format!("{}, retrying in {}s", frame.reason, wait),
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
}
