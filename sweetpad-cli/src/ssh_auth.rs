//! OpenSSH askpass bridge to the VS Code extension. Credentials travel over an
//! authenticated loopback socket, never through argv, environment, or files.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::credentials::{self, Kind};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Answer<'a> {
    answer: Option<&'a str>,
}

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

static NATIVE_BROKER: OnceLock<Result<(String, String), String>> = OnceLock::new();

pub(crate) fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "cannot generate authentication token")?;
    Ok(bytes
        .iter()
        .fold(String::with_capacity(64), |mut result, byte| {
            let _ = write!(result, "{byte:02x}");
            result
        }))
}

fn native_broker() -> Result<&'static (String, String), String> {
    NATIVE_BROKER
        .get_or_init(|| {
            let listener = std::net::TcpListener::bind("127.0.0.1:0")
                .map_err(|_| "cannot start local credential broker")?;
            let address = listener
                .local_addr()
                .map_err(|_| "cannot read credential broker address")?
                .to_string();
            let token = random_token()?;
            let expected = token.clone();
            std::thread::spawn(move || {
                let attempts = std::sync::Arc::new(Mutex::new(HashMap::new()));
                for stream in listener.incoming().flatten() {
                    let token = expected.clone();
                    let attempts = attempts.clone();
                    std::thread::spawn(move || {
                        let _ = respond_native(stream, &token, &attempts);
                    });
                }
            });
            Ok((address, token))
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn account_password(prompt: &str) -> bool {
    let prompt = prompt.trim().to_ascii_lowercase();
    prompt.ends_with("password:")
        && !prompt.contains("passphrase")
        && !prompt.contains("one-time")
        && !prompt.contains("one time")
}

type Attempts = Mutex<HashMap<String, std::time::Instant>>;

fn respond_native(mut stream: TcpStream, token: &str, attempts: &Attempts) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(300)))
        .map_err(|_| "credential broker timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| "credential broker timeout")?;
    let mut line = zeroize::Zeroizing::new(String::new());
    BufReader::new(&stream)
        .take(64 * 1024)
        .read_line(&mut line)
        .map_err(|_| "invalid credential request")?;
    let request: serde_json::Value =
        serde_json::from_str(&line).map_err(|_| "invalid credential request")?;
    // The token is random, local, and never written to disk. Authenticate before
    // touching either the OS store or the user's terminal.
    let supplied = request["token"].as_str().unwrap_or_default();
    if supplied.len() != token.len()
        || supplied
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |different, (a, b)| different | (a ^ b))
            != 0
    {
        return Ok(());
    }
    let host = request["host"].as_str().ok_or("missing host")?;
    let port = request["port"].as_str().and_then(|s| s.parse::<u16>().ok());
    let prompt = request["prompt"].as_str().ok_or("missing prompt")?;
    let confirm = request["confirm"].as_bool().unwrap_or(false)
        || prompt.contains("yes/no")
        || prompt.to_lowercase().contains("fingerprint");
    let remember = !confirm && account_password(prompt);
    let key = credentials::key(host, port, Kind::Ssh);
    // Serialize prompts and store operations, including simultaneous sync/query
    // connections. Subsequent connections find the password just saved here.
    let mut attempts = attempts.lock().map_err(|_| "credential broker stopped")?;
    attempts.retain(|_, time| time.elapsed() < Duration::from_secs(300));
    let connection = format!("{key}:{}:{prompt}", request["connection"]);
    let retry = attempts
        .insert(connection, std::time::Instant::now())
        .is_some();
    let mut answer = if remember && !retry {
        credentials::get(&key).ok().flatten()
    } else {
        None
    };
    if remember && retry {
        let _ = credentials::delete(&key);
    }
    if answer.is_none() && request["allowPrompt"].as_bool() != Some(false) {
        let text = if confirm {
            format!("SSH host verification for {host}: {prompt} (type yes to trust): ")
        } else {
            format!("SSH authentication for {host}: {prompt}")
        };
        answer = rpassword::prompt_password(text)
            .ok()
            .map(zeroize::Zeroizing::new);
        if let Some(password) = answer.as_ref().filter(|_| remember)
            && let Err(error) = credentials::set(&key, password)
        {
            eprintln!("SweetPad: {error}. This connection will use the password once.");
        }
    }
    let reply = zeroize::Zeroizing::new(
        serde_json::to_string(&Answer {
            answer: answer.as_deref().map(String::as_str),
        })
        .map_err(|_| "cannot encode authentication reply")?,
    );
    writeln!(stream, "{}", reply.as_str())
        .map_err(|_| "cannot reply to authentication request".into())
}

static CONNECTION: AtomicU64 = AtomicU64::new(0);

pub(crate) fn configure(
    command: &mut Command,
    host: &str,
    port: Option<u16>,
    auth: Auth,
    non_interactive: bool,
) {
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
        ]);
    }
    let broker = if has_broker() {
        Some((
            std::env::var("SWEETPAD_ASKPASS_ADDR").unwrap_or_default(),
            std::env::var("SWEETPAD_ASKPASS_TOKEN").unwrap_or_default(),
        ))
    } else {
        native_broker().ok().cloned()
    };
    if let Some((address, token)) = broker
        && let Ok(executable) = std::env::current_exe()
    {
        command
            .env("SWEETPAD_ASKPASS_ADDR", address)
            .env("SWEETPAD_ASKPASS_TOKEN", token)
            .env(
                "SWEETPAD_AUTH_PROMPT",
                if non_interactive && !has_broker() {
                    "0"
                } else {
                    "1"
                },
            )
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
    } else if non_interactive {
        command.args(["-o", "BatchMode=yes"]);
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
            let answer = zeroize::Zeroizing::new(answer);
            // OpenSSH reads a single line from the helper's stdout.
            if answer.contains(['\r', '\n', '\0'])
                || writeln!(std::io::stdout(), "{}", answer.as_str()).is_err()
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
        "allowPrompt": std::env::var("SWEETPAD_AUTH_PROMPT").ok().as_deref() != Some("0"),
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

pub(crate) fn saved_keychain_password(
    host: &str,
    port: Option<u16>,
) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    if has_broker() {
        let address: SocketAddr = std::env::var("SWEETPAD_ASKPASS_ADDR")
            .map_err(|_| "missing credential broker")?
            .parse()
            .map_err(|_| "invalid credential broker")?;
        if address.ip() != IpAddr::from([127, 0, 0, 1]) {
            return Err("credential broker must use IPv4 loopback".into());
        }
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))
            .map_err(|_| "cannot contact credential broker")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(300)))
            .map_err(|_| "credential broker timeout")?;
        let request = serde_json::json!({
            "token":std::env::var("SWEETPAD_ASKPASS_TOKEN").map_err(|_| "missing credential token")?,
            "host":host,"port":port.map_or_else(String::new, |p| p.to_string()),
            "connection":"keychain","prompt":"Login keychain password", "confirm":false,
            "kind":"keychain", "savedOnly":true
        });
        writeln!(stream, "{request}").map_err(|_| "cannot request keychain credential")?;
        let mut line = zeroize::Zeroizing::new(String::new());
        BufReader::new(stream)
            .take(64 * 1024)
            .read_line(&mut line)
            .map_err(|_| "cannot read keychain credential")?;
        let mut response: serde_json::Value =
            serde_json::from_str(&line).map_err(|_| "invalid keychain credential reply")?;
        return Ok(response["answer"]
            .take()
            .as_str()
            .map(|s| zeroize::Zeroizing::new(s.to_string())));
    }
    // A key-only/headless setup can run without a local credential service.
    // Explicit password management reports unavailable/locked-store errors.
    Ok(
        credentials::get(&credentials::key(host, port, Kind::Keychain))
            .ok()
            .flatten()
            .or_else(|| {
                credentials::get(&credentials::key(host, port, Kind::Ssh))
                    .ok()
                    .flatten()
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_auth_disables_keys_without_putting_credentials_in_arguments() {
        let mut command = Command::new("ssh");
        configure(&mut command, "dev@mac.local", None, Auth::Password, false);
        let args: Vec<_> = command.get_args().map(|s| s.to_string_lossy()).collect();
        assert!(args.contains(&"PreferredAuthentications=keyboard-interactive,password".into()));
        assert!(args.contains(&"PubkeyAuthentication=no".into()));
        assert!(args.contains(&"BatchMode=no".into()));
    }

    #[test]
    fn one_time_codes_and_key_passphrases_are_not_login_passwords() {
        assert!(account_password("dev@mac's password: "));
        assert!(!account_password("Enter passphrase for key: "));
        assert!(!account_password("One-time password: "));
        assert!(!account_password("Verification code: "));
    }
}
