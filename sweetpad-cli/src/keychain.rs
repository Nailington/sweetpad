//! Unlock in the build's SSH security session. The password travels only over
//! the SSH stdin stream, after the receiving terminal has disabled echo.
use std::io::{BufRead, Read, Write};
use std::process::{Child, ExitCode};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use zeroize::{Zeroize, Zeroizing};

pub(crate) struct Unlock {
    password: Zeroizing<String>,
    marker: String,
    ready: Option<Receiver<()>>,
    output: Option<JoinHandle<std::io::Result<()>>>,
    sent: bool,
    #[cfg(unix)]
    terminal: Option<crate::cli::rawmode::RawMode>,
    #[cfg(windows)]
    terminal: bool,
}

impl Unlock {
    pub(crate) fn new(password: Zeroizing<String>) -> Result<Self, String> {
        crate::credentials::validate(&password)?;
        Ok(Self {
            password,
            marker: format!(
                "sweetpad-keychain-ready-{}",
                crate::ssh_auth::random_token()?
            ),
            ready: None,
            output: None,
            sent: false,
            #[cfg(unix)]
            terminal: None,
            #[cfg(windows)]
            terminal: false,
        })
    }

    pub(crate) fn command(&self, command: &str, interactive: bool) -> String {
        let helper = crate::portable_remote::remote_invocation(&["__unlock-keychain".into()]);
        let prepare = if interactive {
            "sweetpad_tty=$(stty -g) || exit 78; stty -echo -icanon min 1 time 0 || exit 78; "
        } else {
            ""
        };
        let restore = if interactive {
            "stty \"$sweetpad_tty\" || exit 78; "
        } else {
            ""
        };
        format!(
            "{prepare}printf '%s\\n' '{}'; ({helper}) || exit 78; {restore}{command}",
            self.marker
        )
    }

    pub(crate) fn attach(&mut self, child: &mut Child) -> Result<(), String> {
        let output = child.stdout.take().ok_or("missing unlock output stream")?;
        let marker = self.marker.clone();
        let (ready, receiver) = mpsc::channel();
        self.ready = Some(receiver);
        self.output = Some(std::thread::spawn(move || {
            relay(output, &marker, &ready, std::io::stdout())
        }));
        Ok(())
    }

    pub(crate) fn tick(&mut self, child: &mut Child, interactive: bool) -> Result<(), String> {
        if !self.sent {
            match self
                .ready
                .as_ref()
                .ok_or("missing keychain handshake")?
                .try_recv()
            {
                Ok(()) => {
                    let frame = Zeroizing::new(
                        serde_json::to_string(&*self.password)
                            .map_err(|_| "cannot encode keychain credential")?,
                    );
                    let input = child.stdin.as_mut().ok_or("missing unlock input stream")?;
                    writeln!(input, "{}", frame.as_str())
                        .and_then(|()| input.flush())
                        .map_err(|_| "cannot send keychain credential")?;
                    self.password.zeroize();
                    self.sent = true;
                    if interactive {
                        #[cfg(unix)]
                        {
                            self.terminal = Some(
                                crate::cli::rawmode::RawMode::enable()
                                    .map_err(|_| "cannot enable remote session input")?,
                            );
                        }
                        #[cfg(windows)]
                        {
                            crossterm::terminal::enable_raw_mode()
                                .map_err(|_| "cannot enable remote session input")?;
                            self.terminal = true;
                        }
                    } else {
                        child.stdin.take();
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(
                        "the remote CLI did not accept keychain unlock; update the CLI on the Mac"
                            .into(),
                    );
                }
            }
        }
        if interactive {
            #[cfg(unix)]
            if let crate::cli::rawmode::Input::Key(key) = crate::cli::rawmode::poll_key() {
                let mut buffer = [0u8; 4];
                if let Some(input) = child.stdin.as_mut() {
                    input
                        .write_all(key.encode_utf8(&mut buffer).as_bytes())
                        .map_err(|_| "remote input closed")?;
                }
            }
            #[cfg(windows)]
            if crossterm::event::poll(std::time::Duration::from_millis(50))
                .map_err(|_| "cannot read remote input")?
            {
                if let crossterm::event::Event::Key(key) =
                    crossterm::event::read().map_err(|_| "cannot read remote input")?
                {
                    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
                    if key.kind != KeyEventKind::Release {
                        let text = match key.code {
                            KeyCode::Char(c)
                                if key.modifiers.contains(KeyModifiers::CONTROL)
                                    && c.is_ascii_alphabetic() =>
                            {
                                String::from(char::from(c.to_ascii_lowercase() as u8 - b'a' + 1))
                            }
                            KeyCode::Char(c) => c.to_string(),
                            KeyCode::Enter => "\r".into(),
                            KeyCode::Esc => "\x1b".into(),
                            KeyCode::Backspace => "\x7f".into(),
                            KeyCode::Up => "\x1b[A".into(),
                            KeyCode::Down => "\x1b[B".into(),
                            KeyCode::Right => "\x1b[C".into(),
                            KeyCode::Left => "\x1b[D".into(),
                            _ => String::new(),
                        };
                        if let Some(input) = child.stdin.as_mut() {
                            input
                                .write_all(text.as_bytes())
                                .map_err(|_| "remote input closed")?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl Drop for Unlock {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.terminal {
            let _ = crossterm::terminal::disable_raw_mode();
        }
        // SSH must have exited (or been killed) before this guard is dropped.
        if let Some(output) = self.output.take() {
            let _ = output.join();
        }
    }
}

fn relay(
    input: impl Read,
    marker: &str,
    ready: &mpsc::Sender<()>,
    mut output: impl Write,
) -> std::io::Result<()> {
    let mut input = std::io::BufReader::new(input);
    loop {
        let mut line = Vec::new();
        if input.by_ref().take(65536).read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line
            .strip_suffix(b"\r\n")
            .or_else(|| line.strip_suffix(b"\n"))
            .unwrap_or(&line)
            == marker.as_bytes()
        {
            let _ = ready.send(());
            break;
        }
        output.write_all(&line)?;
        output.flush()?;
    }
    let mut buffer = [0u8; 8192];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        output.write_all(&buffer[..n])?;
        output.flush()?;
    }
    Ok(())
}

/// Receiving-Mac helper; never use the `security -p PASSWORD` command, whose
/// arguments would expose the credential in process listings.
#[must_use]
pub fn run_helper(args: &[String]) -> Option<ExitCode> {
    if args.first().map(String::as_str) != Some("__unlock-keychain") {
        return None;
    }
    Some(match unlock_input(std::io::stdin().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "SweetPad keychain: {error}. Save a separate password with 'sweetpad remote password MAC --keychain' on the client."
            );
            ExitCode::from(78)
        }
    })
}

fn unlock_input(mut input: impl Read) -> Result<(), String> {
    let mut frame = Zeroizing::new(Vec::new());
    // One byte at a time so subsequent interactive input stays in the SSH pipe.
    loop {
        let mut b = [0u8; 1];
        if input
            .read(&mut b)
            .map_err(|_| "cannot read keychain credential")?
            == 0
        {
            return Err("keychain credential stream closed".into());
        }
        if b[0] == b'\n' {
            break;
        }
        if frame.len() >= 32768 {
            return Err("keychain credential is too long".into());
        }
        frame.push(b[0]);
    }
    let password = Zeroizing::new(
        serde_json::from_slice::<String>(&frame)
            .map_err(|_| "invalid keychain credential frame")?,
    );
    crate::credentials::validate(&password)?;
    unlock(&password)
}

#[cfg(target_os = "macos")]
fn unlock(password: &str) -> Result<(), String> {
    let path = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or("HOME is missing")?
        .join("Library/Keychains/login.keychain-db");
    let mut keychain = security_framework::os::macos::keychain::SecKeychain::open(&path)
        .map_err(|e| format!("cannot open login keychain (OSStatus {})", e.code()))?;
    keychain
        .unlock(Some(password))
        .map_err(|e| format!("cannot unlock login keychain (OSStatus {})", e.code()))
}

#[cfg(not(target_os = "macos"))]
fn unlock(_password: &str) -> Result<(), String> {
    Err("keychain unlock requires macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_never_contains_the_password_and_disables_echo_before_handshake() {
        let unlock = Unlock::new(Zeroizing::new("secret '$ ü".into())).unwrap();
        let command = unlock.command("exec sweetpad run", true);
        assert!(!command.contains("secret"));
        assert!(command.find("stty -echo").unwrap() < command.find("printf").unwrap());
        assert!(command.contains("__unlock-keychain"));
    }
    #[test]
    fn relay_preserves_output_and_strips_only_the_exact_handshake() {
        let (tx, rx) = mpsc::channel();
        let mut output = Vec::new();
        relay(
            "banner\nready\r\napp logs\n".as_bytes(),
            "ready",
            &tx,
            &mut output,
        )
        .unwrap();
        rx.recv().unwrap();
        assert_eq!(output, b"banner\napp logs\n");
    }
    #[test]
    fn truncated_secret_frame_is_rejected_without_including_its_contents() {
        let error = unlock_input("\"do not print this".as_bytes()).unwrap_err();
        assert!(!error.contains("do not print"));
    }
}
