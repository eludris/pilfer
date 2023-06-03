use std::{
    env,
    io::{stdin, stdout, Write},
    path::PathBuf,
};

use anyhow::{bail, Context};
use directories::ProjectDirs;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use todel::models::{InstanceInfo, SessionCreated};
use tokio::fs;

use crate::prompt;

/*
curl \
  --json '{
  "indentifier": "yendri"
  "password": "authentícame por favor"
  "platform": "linux",
  "client":"pilfer"
}' \
  https://api.eludris.gay/sessions
{
  "token": "",
  "session": {
    "indentifier": "yendri",
    "password": "authentícame por favor",
    "platform": "linux",
    "client": "pilfer"
  }
}

curl \
  --json '{
  "username": "yendri",
  "email": "yendri@llamoyendri.io",
  "password": "autentícame por favor"
}' \
  https://api.eludris.gay/users

{
  "id": 48615849987333,
  "username": "yendri",
  "social_credit": 0,
  "badges": 0,
  "permissions": 0
}

"do you already have an account? (Y/n)" for login/signup
hide password input

ask for login/signup initially
*/

// print!("What's your name? > ");
// stdout.flush().unwrap();

// let mut name = String::new();

// io::stdin().read_line(&mut name).unwrap();

// let name = name.trim();

const CLIENT_NAME: &str = "pilfer";
const PLATFORM_NAME: &str = env::consts::OS;

#[derive(Debug, Deserialize, Serialize)]
struct Config {
    session: String,
    username: String,
}

pub async fn get_token(
    info: &InstanceInfo,
    http_client: &Client,
) -> Result<(String, String), anyhow::Error> {
    let conf_dir = match env::var("PILFER_CONF") {
        Ok(dir) => Ok::<PathBuf, anyhow::Error>(PathBuf::try_from(dir).context(
            "Could not convert the `PILFER_CONF` environment variable into a valid path",
        )?),
        Err(env::VarError::NotPresent) => Ok(ProjectDirs::from("", "eludris", "pilfer")
            // According to the `directories` docs the error is raised when a home path isn't found
            // but that wouldn't make much sense for windows so we use `base` here.
            .context("Could not find a valid base directory")?
            .config_dir()
            .to_path_buf()),
        Err(env::VarError::NotUnicode(_)) => {
            bail!("The value of the `PILFER_CONF` environment variable must be a valid unicode string")
        }
    }?;

    if !conf_dir.exists() {
        fs::create_dir_all(&conf_dir)
            .await
            .context("Could not create config directory")?;
    }
    let config_path = conf_dir.join("Pilfer.toml");
    if config_path.exists() {
        // Use existing config.
        let config = fs::read_to_string(&config_path)
            .await
            .context("Could not read config file")?;
        let config: Config = toml::from_str(&config).context("Could not parse config file")?;
        return Ok((config.session, config.username));
    }

    print!("Do you already have an account? (Y/n) > ");
    stdout().flush().unwrap();
    let mut input = String::new();
    stdin().read_line(&mut input).unwrap();
    let input = input.trim();

    loop {
        match input {
            "y" | "Y" | "" => break login(info, http_client, config_path).await,
            // "n" | "N" => signup(info, http_client).await,
            _ => {
                println!("Invalid input");
            }
        }
    }
}

async fn login(
    info: &InstanceInfo,
    http_client: &Client,
    config_path: PathBuf,
) -> Result<(String, String), anyhow::Error> {
    let username = prompt!("Username > ");
    let password = rpassword::prompt_password("Password > ").unwrap();

    create_session(info, http_client, username, password, config_path).await
}

async fn create_session(
    info: &InstanceInfo,
    http_client: &Client,
    username: String,
    password: String,
    config_path: PathBuf,
) -> Result<(String, String), anyhow::Error> {
    let session = json!({
        "identifier": username,
        "password": password,
        "platform": PLATFORM_NAME,
        "client": CLIENT_NAME
    });

    let token = http_client
        .post(format!("{}/sessions", info.oprish_url))
        .json(&session)
        .send()
        .await
        .expect("Can not connect to Oprish")
        .json::<SessionCreated>()
        .await?
        .token;

    let config = Config {
        session: token.clone(),
        username: username.clone(),
    };

    fs::write(config_path, toml::to_string(&config)?)
        .await
        .context("Could not write config file")?;
    Ok((token, username))
}
