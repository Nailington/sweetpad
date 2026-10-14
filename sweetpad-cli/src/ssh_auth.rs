//! OpenSSH askpass bridge to the VS Code extension. Credentials travel over an
//! authenticated loopback socket, never through argv, environment, or files.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(not(windows), derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Auth {
    #[default]
    Ssh,
    Password,
}

impl Auth {
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::Ssh
    }
}

pub(crate) fn has_broker() -> bool {
    std::env::var_os("SWEETPAD_ASKPASS_ADDR").is_some()
        && std::env::var_os("SWEETPAD_ASKPASS_TOKEN").is_some()
}

static CONNECTION: AtomicU64 = AtomicU64::new(0);

pub(crate) fn configure(command: &mut Command, host: &str, port: Option<u16>, auth: Auth) {
    if auth == Auth::Password {
        command.args([
            "-o",
            "PreferredAuthentications=keyboard-interactive,password",
            "-o",
            "PubkeyAuthentication=no",
            "-o",
            "PasswordAuthentication=yes",
            "-o",
            "KbdInteractiveAuthentication=yes",
            "-o",
            "BatchMode=no",
        ]);
    }
    if std::env::var_os("SWEETPAD_ASKPASS_ADDR").is_some()
        && std::env::var_os("SWEETPAD_ASKPASS_TOKEN").is_some()
        && let Ok(executable) = std::env::current_exe()
    {
        command
            .env("SSH_ASKPASS", executable)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("SWEETPAD_ASKPASS_HELPER", "1")
            .env("SWEETPAD_ASKPASS_HOST", host)
            .env(
                "SWEETPAD_ASKPASS_PORT",
                port.map_or_else(String::new, |p| p.to_string()),
            )
            .env(
                "SWEETPAD_ASKPASS_CONNECTION",
                format!(
                    "{}-{}",
                    std::process::id(),
                    CONNECTION.fetch_add(1, Ordering::Relaxed)
                ),
            )
            .args(["-o", "BatchMode=no", "-o", "NumberOfPasswordPrompts=3"]);
    }
}

/// Run before argument parsing when OpenSSH launches this binary as askpass.
/// Errors deliberately contain no prompt, token, or credential.
#[must_use]
pub fn run_askpass() -> Option<ExitCode> {
    if std::env::var("SWEETPAD_ASKPASS_HELPER").ok().as_deref() != Some("1") {
        return None;
    }
    Some(match askpass() {
        Ok(answer) => {
            // OpenSSH reads a single line from the helper's stdout.
            if answer.contains(['\r', '\n', '\0'])
                || writeln!(std::io::stdout(), "{answer}").is_err()
            {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(_) => ExitCode::FAILURE,
    })
}

fn askpass() -> Result<String, Box<dyn std::error::Error>> {
    let address: SocketAddr = std::env::var("SWEETPAD_ASKPASS_ADDR")?.parse()?;
    if address.ip() != IpAddr::from([127, 0, 0, 1]) {
        return Err("askpass requires IPv4 loopback".into());
    }
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(300)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let request = serde_json::json!({
        "token": std::env::var("SWEETPAD_ASKPASS_TOKEN")?,
        "host": std::env::var("SWEETPAD_ASKPASS_HOST")?,
        "port": std::env::var("SWEETPAD_ASKPASS_PORT")?,
        "connection": std::env::var("SWEETPAD_ASKPASS_CONNECTION")?,
        "prompt": std::env::args().nth(1).unwrap_or_default(),
        "confirm": std::env::var("SSH_ASKPASS_PROMPT").ok().as_deref() == Some("confirm"),
    });
    writeln!(stream, "{request}")?;
    let mut line = String::new();
    BufReader::new(stream)
        .take(64 * 1024)
        .read_line(&mut line)?;
    let response: serde_json::Value = serde_json::from_str(&line)?;
    response["answer"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "authentication cancelled".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_auth_disables_keys_without_putting_credentials_in_arguments() {
        let mut command = Command::new("ssh");
        configure(&mut command, "dev@mac.local", None, Auth::Password);
        let args: Vec<_> = command.get_args().map(|s| s.to_string_lossy()).collect();
        assert!(args.contains(&"PreferredAuthentications=keyboard-interactive,password".into()));
        assert!(args.contains(&"PubkeyAuthentication=no".into()));
        assert!(args.contains(&"BatchMode=no".into()));
    }
}
