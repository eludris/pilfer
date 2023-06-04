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
use todel::models::{ErrorResponse, InstanceInfo, SessionCreated, User};
use tokio::fs;

use crate::{models::Response, prompt};

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
            "n" | "N" => break signup(info, http_client, config_path).await,
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
    let username = prompt!("Username/Email > ");
    let password = rpassword::prompt_password("Password > ").unwrap();

    create_session(info, http_client, username, password, config_path).await
}

async fn signup(
    info: &InstanceInfo,
    http_client: &Client,
    config_path: PathBuf,
) -> Result<(String, String), anyhow::Error> {
    loop {
        let username = prompt!("Username > ");
        let email = prompt!("Email > ");
        let password = rpassword::prompt_password("Password > ").unwrap();

        let user = json!({
            "username": username,
            "email": email,
            "password": password,
        });

        let user = http_client
            .post(format!("{}/users", info.oprish_url))
            .json(&user)
            .send()
            .await
            .expect("Can not connect to Oprish")
            .json::<Response<User>>()
            .await?;

        match user {
            Response::Success(_) => {
                println!("Account created successfully!");
                if info.email_address.is_some() {
                    println!("Please check your email to verify your account.");
                }
                println!("You can now login with your username and password.");
                prompt!("Press enter to continue > ");
                break create_session(info, http_client, username, password, config_path).await;
            }
            Response::Error(ErrorResponse::Conflict { item, .. }) => {
                eprintln!("Could not create account");
                eprintln!("{} already exists", item);
            }
            Response::Error(ErrorResponse::Validation {
                value_name, info, ..
            }) => {
                eprintln!("Could not create account");
                eprintln!("{}: {}", value_name, info);
            }
            Response::Error(error) => {
                eprintln!("Could not create account");
                eprintln!("{:?}", error);
            }
        }
    }
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

    let token = match http_client
        .post(format!("{}/sessions", info.oprish_url))
        .json(&session)
        .send()
        .await
        .expect("Can not connect to Oprish")
        .json::<Response<SessionCreated>>()
        .await?
    {
        Response::Success(session) => session.token,
        Response::Error(ErrorResponse::NotFound { .. }) => {
            return Err(anyhow::anyhow!(
                "Could not create session, user {} not found",
                username
            ))
        }
        Response::Error(ErrorResponse::Unauthorized { .. }) => {
            return Err(anyhow::anyhow!(
                "Could not create session, invalid password"
            ))
        }
        Response::Error(error) => {
            return Err(anyhow::anyhow!("Could not create session: {:?}", error));
        }
    };

    let config = Config {
        session: token.clone(),
        username: username.clone(),
    };

    fs::write(config_path, toml::to_string(&config)?)
        .await
        .context("Could not write config file")?;
    Ok((token, username))
}
