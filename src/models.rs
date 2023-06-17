use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
use notify_rust::NotificationHandle;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use todel::models::{ErrorResponse, Message, User};
use tokio::sync::Mutex as AsyncMutex;
use tui::style::Style;

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response<T> {
    Success(T),
    Error(ErrorResponse),
}

// While in hindsight this might look like it's modeled in a bad way, you're right, it's modeled in
// a bad way.
#[derive(Debug)]
pub struct SystemMessage {
    pub content: String,
}

#[derive(Debug)]
pub enum PilferMessage {
    Eludris(Box<Message>),
    System(SystemMessage),
}

impl Display for PilferMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PilferMessage::Eludris(msg) => write!(f, "[{}]: {}", msg.author, msg.message.content),
            PilferMessage::System(msg) => write!(f, "{}", msg.content),
        }
    }
}

pub struct AppContext {
    /// Current input
    pub input: String,
    /// User name
    pub name: String,
    /// Received messages
    pub messages: Arc<Mutex<Vec<(PilferMessage, Style)>>>,
    /// Online users
    pub users: Arc<AsyncMutex<HashMap<u64, User>>>,
    /// Reqwest HttpClient
    pub http_client: Arc<Client>,
    /// Oprish URL
    pub rest_url: String,
    /// Whether the user is currently focused.
    pub focused: Arc<AtomicBool>,
    /// Whether the online users list is enabled.
    pub users_list_enabled: bool,
    /// The notification
    #[cfg(target_os = "linux")]
    pub notification: Arc<Mutex<Option<NotificationHandle>>>,
}
