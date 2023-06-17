use std::io::{stdin, stdout, Write};

use lazy_static::lazy_static;
use regex::Regex;
use todel::models::InstanceInfo;

const WARNING: &str =
    "Warning: This version of Pilfer is more recent than the instance you are connecting to.

Make sure you are using the right instance. An example of running Pilfer with a different instance is:

    $ INSTANCE_URL=https://api.eludris.gay/next pilfer";
const ERROR: &str = "This version of Pilfer is older than the instance you are connecting to.

Please update Pilfer to the latest version.";

pub fn check_version(info: &InstanceInfo) -> Result<(), String> {
    lazy_static! {
        static ref VERSION_REGEX: Regex= Regex::new(r"^(?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\.(?P<patch>0|[1-9]\d*)(?:-(?P<prerelease>(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+(?P<buildmetadata>[0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$").unwrap();
    }

    let current_version = VERSION_REGEX
        .captures(env!("CARGO_PKG_VERSION"))
        .ok_or("Error: Current version is not a valid semver.")?;
    let instance_version = VERSION_REGEX
        .captures(&info.version)
        .ok_or("Error: Instance version is not a valid semver.")?;
    println!("Current version: {}", env!("CARGO_PKG_VERSION"));
    println!("Instance version: {}", info.version);

    let current_major = current_version
        .name("major")
        .unwrap()
        .as_str()
        .parse::<u16>()
        .unwrap();
    let current_minor = current_version
        .name("minor")
        .unwrap()
        .as_str()
        .parse::<u16>()
        .unwrap();
    let current_patch = current_version
        .name("patch")
        .unwrap()
        .as_str()
        .parse::<u16>()
        .unwrap();
    let current_prerelease = current_version
        .name("prerelease")
        .map(|s| s.as_str())
        .unwrap_or("");

    let instance_major = instance_version
        .name("major")
        .unwrap()
        .as_str()
        .parse::<u16>()
        .unwrap();
    let instance_minor = instance_version
        .name("minor")
        .unwrap()
        .as_str()
        .parse::<u16>()
        .unwrap();
    let instance_patch = instance_version
        .name("patch")
        .unwrap()
        .as_str()
        .parse::<u16>()
        .unwrap();
    let instance_prerelease = instance_version
        .name("prerelease")
        .map(|s| s.as_str())
        .unwrap_or("");

    if instance_major < current_major
        || instance_minor < current_minor
        || instance_patch < current_patch
        || current_prerelease != instance_prerelease
    {
        eprintln!("{}", WARNING);
        print!("Continue anyway? (Y/n) > ");
        stdout().flush().unwrap();
        let mut input = String::new();
        stdin().read_line(&mut input).unwrap();
        let input = input.trim();

        loop {
            match input {
                "y" | "Y" | "" => break,
                "n" | "N" => return Err("Aborted.".to_string()),
                _ => {
                    println!("Invalid input");
                }
            }
        }
    } else if instance_major > current_major
        || instance_minor > current_minor
        || instance_patch > current_patch
    {
        return Err(ERROR.into());
    }
    Ok(())
}
