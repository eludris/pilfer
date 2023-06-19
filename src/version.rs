use std::{cmp::Ordering, time::Duration};

use lazy_static::lazy_static;
use regex::Regex;
use todel::models::InstanceInfo;

const ERROR: &str =
    "This version of Pilfer is more recent than the instance you are connecting to.

Make sure you are using the right instance. An example of running Pilfer with a different instance is:

    $ INSTANCE_URL=https://api.eludris.gay/next pilfer

If you are using the correct instance, downgrade pilfer.";
const WARNING: &str =
    "Warning: This version of Pilfer is older than the instance you are connecting to.

Please update Pilfer to the latest version.";

pub fn check_version(info: &InstanceInfo) -> Result<(), String> {
    lazy_static! {
        static ref VERSION_REGEX: Regex = Regex::new(r"^(?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\.(?P<patch>0|[1-9]\d*)(?:-(?P<prerelease>(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+(?P<buildmetadata>[0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$").unwrap();
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

    let instance = (
        instance_major,
        instance_minor,
        instance_patch,
        instance_prerelease,
    );
    let current = (
        current_major,
        current_minor,
        current_patch,
        current_prerelease,
    );

    match instance.cmp(&current) {
        Ordering::Equal => {}
        Ordering::Less => {
            eprintln!("{}", ERROR);
            return Err("Aborted.".to_string());
        }
        Ordering::Greater => {
            eprintln!("{}", WARNING);
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    Ok(())
}
