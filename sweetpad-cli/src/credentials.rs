//! One credential store for terminal commands and the editor. No secrets in TOML.
use std::io::{BufRead, IsTerminal, Read, Write};
use std::process::ExitCode;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

static STORE_LOCK: Mutex<()> = Mutex::new(());
const SERVICE: &str = "sweetpad.remote";
const STORE_ERROR: &str = "cannot access the OS credential store (Linux requires a running Secret Service, such as GNOME Keyring or KWallet); no password was written to a file";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    #[default]
    Ssh,
    Keychain,
}

pub(crate) fn key(host: &str, port: Option<u16>, kind: Kind) -> String {
    let prefix = match kind {
        Kind::Ssh => "sweetpad.remote.password",
        Kind::Keychain => "sweetpad.remote.keychain",
    };
    format!(
        "{prefix}:{}",
        serde_json::json!([host, port.map_or_else(String::new, |p| p.to_string())])
    )
}

fn entry(key: &str) -> Result<keyring::Entry, String> {
    if !key.starts_with("sweetpad.remote.password:")
        && !key.starts_with("sweetpad.remote.keychain:")
    {
        return Err("invalid SweetPad credential key".into());
    }
    keyring::Entry::new(SERVICE, key).map_err(|_| STORE_ERROR.into())
}

pub(crate) fn get(key: &str) -> Result<Option<Zeroizing<String>>, String> {
    Ok(lookup(key)?.0)
}

#[derive(Deserialize, Serialize)]
struct StoredCredential<T> {
    value: Option<T>,
}

#[derive(Deserialize)]
struct Request {
    action: String,
    key: String,
    value: Option<String>,
}

#[derive(Serialize)]
struct Reply<'a> {
    value: Option<&'a str>,
    known: bool,
}

fn lookup(key: &str) -> Result<(Option<Zeroizing<String>>, bool), String> {
    let _lock = STORE_LOCK.lock().map_err(|_| STORE_ERROR)?;
    match entry(key)?.get_password() {
        Ok(value) => {
            let value = Zeroizing::new(value);
            let record: StoredCredential<String> =
                serde_json::from_str(&value).map_err(|_| "invalid stored SweetPad credential")?;
            Ok((record.value.map(Zeroizing::new), true))
        }
        Err(keyring::Error::NoEntry) => Ok((None, false)),
        Err(_) => Err(STORE_ERROR.into()),
    }
}

pub(crate) fn set(key: &str, value: &str) -> Result<(), String> {
    validate(value)?;
    write(key, Some(value))
}

fn write(key: &str, value: Option<&str>) -> Result<(), String> {
    let encoded = Zeroizing::new(
        serde_json::to_string(&StoredCredential { value })
            .map_err(|_| "cannot encode credential")?,
    );
    let _lock = STORE_LOCK.lock().map_err(|_| STORE_ERROR)?;
    entry(key)?
        .set_password(&encoded)
        .map_err(|_| STORE_ERROR.into())
}

pub(crate) fn delete(key: &str) -> Result<(), String> {
    // Retain only a password-free migration marker so an old editor password
    // cannot be resurrected after the user forgets it from the CLI.
    write(key, None)
}

pub(crate) fn forget_endpoint(host: &str, port: Option<u16>) {
    for kind in [Kind::Ssh, Kind::Keychain] {
        if let Err(error) = delete(&key(host, port, kind)) {
            eprintln!(
                "SweetPad: removed the Mac, but saved passwords could not be cleared: {error}. Use 'sweetpad remote password HOST --forget' and '--keychain --forget' when the credential store is available."
            );
            break;
        }
    }
}

pub(crate) fn validate(value: &str) -> Result<(), String> {
    if value.contains(['\n', '\r', '\0']) || value.len() > 4096 {
        return Err("password must be a single line of at most 4096 bytes".into());
    }
    Ok(())
}

pub(crate) fn read_secret(input: impl Read) -> Result<Zeroizing<String>, String> {
    let mut value = Zeroizing::new(String::new());
    input
        .take(8192)
        .read_to_string(&mut value)
        .map_err(|_| "cannot read password from stdin")?;
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    validate(&value)?;
    Ok(value)
}

pub(crate) fn manage(
    host: &str,
    port: Option<u16>,
    kind: Kind,
    forget: bool,
    stdin: bool,
) -> Result<(), String> {
    let key = key(host, port, kind);
    if forget {
        return delete(&key);
    }
    let password = if stdin {
        read_secret(std::io::stdin().lock())?
    } else {
        Zeroizing::new(
            rpassword::prompt_password(format!(
                "{} password for {host} (saved in the OS credential store): ",
                if kind == Kind::Ssh {
                    "SSH login"
                } else {
                    "Login keychain"
                }
            ))
            .map_err(|_| "cannot prompt for a password; use --stdin with a private pipe")?,
        )
    };
    set(&key, &password)
}

/// Private, bounded stdio protocol for the extension. Only this helper returns
/// secrets, to a pipe owned by the extension, never a normal CLI report or TTY.
#[must_use]
pub fn run_helper(args: &[String]) -> Option<ExitCode> {
    if args.first().map(String::as_str) != Some("__credential-store") {
        return None;
    }
    if std::io::stdin().is_terminal() || std::io::stdout().is_terminal() {
        return Some(ExitCode::FAILURE);
    }
    let result = (|| -> Result<Zeroizing<String>, String> {
        let mut line = Zeroizing::new(String::new());
        std::io::stdin()
            .lock()
            .take(65536)
            .read_line(&mut line)
            .map_err(|_| "cannot read credential request")?;
        let mut request: Request =
            serde_json::from_str(&line).map_err(|_| "invalid credential request")?;
        let value = request.value.take().map(Zeroizing::new);
        let (result, known) = match request.action.as_str() {
            "get" => lookup(&request.key)?,
            "store" => {
                set(
                    &request.key,
                    value
                        .as_deref()
                        .map(String::as_str)
                        .ok_or("missing credential")?,
                )?;
                (None, true)
            }
            "delete" => {
                delete(&request.key)?;
                (None, true)
            }
            _ => return Err("invalid credential action".into()),
        };
        Ok(Zeroizing::new(
            serde_json::to_string(&Reply {
                value: result.as_deref().map(String::as_str),
                known,
            })
            .map_err(|_| "cannot encode credential reply")?,
        ))
    })();
    Some(match result {
        Ok(reply) => {
            if writeln!(std::io::stdout(), "{}", reply.as_str()).is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_and_password_purposes_are_separate() {
        assert_eq!(
            key("dev@mac", None, Kind::Ssh),
            "sweetpad.remote.password:[\"dev@mac\",\"\"]"
        );
        assert_ne!(
            key("dev@mac", None, Kind::Ssh),
            key("dev@mac", Some(22), Kind::Ssh)
        );
        assert_ne!(
            key("dev@mac", None, Kind::Ssh),
            key("dev@mac", None, Kind::Keychain)
        );
    }
    #[test]
    fn stdin_preserves_shell_characters_but_rejects_multiline_and_oversized_secrets() {
        assert_eq!(
            &**read_secret("p@ss '$ ü\r\n".as_bytes()).unwrap(),
            "p@ss '$ ü"
        );
        assert!(read_secret("first\nsecond".as_bytes()).is_err());
        assert!(read_secret("x".repeat(9000).as_bytes()).is_err());
    }
}
