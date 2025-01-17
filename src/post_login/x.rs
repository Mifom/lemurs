use rand::Rng;
use std::env;
use std::error::Error;
use std::fmt::Display;
use std::fs::remove_file;
use std::process::{Child, Command, Stdio};
use std::{thread, time};

use std::path::PathBuf;

use log::{error, info};

use crate::auth::AuthUserInfo;
use crate::env_container::EnvironmentContainer;

const XSTART_TIMEOUT_SECS: u64 = 20;
const XSTART_CHECK_INTERVAL_MILLIS: u64 = 100;

#[derive(Debug, Clone)]
pub enum XSetupError {
    DisplayEnvVar,
    VTNREnvVar,
    FillingXAuth,
    InvalidUTF8Path,
    XServerStart,
    XServerTimeout,
    XServerStatusCheck,
}

impl Display for XSetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DisplayEnvVar => f.write_str("`DISPLAY` is not set"),
            Self::VTNREnvVar => f.write_str("`XDG_VTNR` is not set"),
            Self::FillingXAuth => f.write_str("Failed to fill `.Xauthority` file"),
            Self::InvalidUTF8Path => f.write_str("Path that is given is not valid UTF8"),
            Self::XServerStart => f.write_str("Failed to start X server binary"),
            Self::XServerTimeout => f.write_str("Timeout while waiting for X server to start"),
            Self::XServerStatusCheck => f.write_str("Failed to check for X server status"),
        }
    }
}

impl Error for XSetupError {}

fn mcookie() -> String {
    // TODO: Verify that this is actually safe. Maybe just use the mcookie binary?? Is that always
    // available?
    let mut rng = rand::thread_rng();
    let cookie: u128 = rng.gen();
    format!("{cookie:032x}")
}

pub fn setup_x(
    process_env: &mut EnvironmentContainer,
    user_info: &AuthUserInfo,
) -> Result<Child, XSetupError> {
    use std::os::unix::process::CommandExt;

    info!("Start setup of X");

    let display_value = env::var("DISPLAY").map_err(|_| XSetupError::DisplayEnvVar)?;
    let vtnr_value = env::var("XDG_VTNR").map_err(|_| XSetupError::VTNREnvVar)?;

    // Setup xauth
    let xauth_dir =
        PathBuf::from(env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| user_info.dir.to_string()));
    let xauth_path = xauth_dir.join(".Xauthority");

    info!("Filling Xauthority file");

    // Make sure that we are generating a new file. This is necessary since sometimes, there may be
    // a `root` permission `.Xauthority` file there.
    let _ = remove_file(xauth_path.clone());

    Command::new(super::SYSTEM_SHELL)
        .arg("-c")
        .arg(format!(
            "/usr/bin/xauth add {} . {}",
            display_value,
            mcookie()
        ))
        .uid(user_info.uid)
        .gid(user_info.gid)
        .stdout(Stdio::null()) // TODO: Maybe this should be logged or something?
        .stderr(Stdio::null()) // TODO: Maybe this should be logged or something?
        .status()
        .map_err(|err| {
            error!("Filling xauth file failed. Reason: {}", err);
            XSetupError::FillingXAuth
        })?;

    let xauth_path = xauth_path.to_str().ok_or(XSetupError::InvalidUTF8Path)?;
    process_env.set("XAUTHORITY", xauth_path);

    let doubledigit_vtnr = if vtnr_value.len() == 1 {
        format!("0{vtnr_value}")
    } else {
        vtnr_value
    };

    info!("Run X server");
    let child = Command::new(super::SYSTEM_SHELL)
        .arg("-c")
        .arg(format!("/usr/bin/X {display_value} vt{doubledigit_vtnr}",))
        .stdout(Stdio::null()) // TODO: Maybe this should be logged or something?
        .stderr(Stdio::null()) // TODO: Maybe this should be logged or something?
        .spawn()
        .map_err(|err| {
            error!("Starting X server failed. Reason: {}", err);
            XSetupError::XServerStart
        })?;

    // Wait for XServer to boot-up
    let start_time = time::SystemTime::now();
    loop {
        // Timeout
        if match start_time.elapsed() {
            Ok(dur) => dur.as_secs() >= XSTART_TIMEOUT_SECS,
            Err(_) => {
                error!("Failed to resolve elapsed time");
                std::process::exit(1);
            }
        } {
            error!("Starting X timed out!");
            return Err(XSetupError::XServerTimeout);
        }

        match Command::new(super::SYSTEM_SHELL)
            .arg("-c")
            .arg("timeout 1s /usr/bin/xset q")
            .stdout(Stdio::null()) // TODO: Maybe this should be logged or something?
            .stderr(Stdio::null()) // TODO: Maybe this should be logged or something?
            .status()
        {
            Ok(status) => {
                if status.success() {
                    break;
                }
            }
            Err(_) => {
                error!("Failed to run `xset` to check X server status");
                return Err(XSetupError::XServerStatusCheck);
            }
        }

        thread::sleep(time::Duration::from_millis(XSTART_CHECK_INTERVAL_MILLIS));
    }

    info!("X server is running");

    Ok(child)
}
