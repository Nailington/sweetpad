//! Portable SSH/archive transport, plus the remote-only Windows frontend.
//! Xcode commands run in the ordinary SweetPad CLI on the receiving Mac.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::IsTerminal;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Mac {
    pub(crate) host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) identity_file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) port: Option<u16>,
    #[serde(skip)]
    pub(crate) batch_mode: bool,
}

impl Mac {
    fn ssh(&self) -> Command {
        let mut command = Command::new("ssh");
        command.args([
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=2",
        ]);
        if self.batch_mode {
            command.args(["-o", "BatchMode=yes"]);
        }
        if let Some(key) = &self.identity_file {
            command
                .arg("-i")
                .arg(key)
                .args(["-o", "IdentitiesOnly=yes"]);
        }
        if let Some(port) = self.port {
            command.arg("-p").arg(port.to_string());
        }
        command
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Registry {
    macs: BTreeMap<String, Mac>,
}

impl Registry {
    fn path() -> Result<PathBuf, String> {
        if cfg!(windows) {
            return std::env::var_os("APPDATA")
                .map(|dir| PathBuf::from(dir).join("sweetpad").join("remotes.toml"))
                .ok_or_else(|| "APPDATA is unset; cannot locate SweetPad's Mac registry".into());
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or_else(|| "HOME or XDG_CONFIG_HOME is needed for the Mac registry".to_string())?;
        Ok(base.join("sweetpad").join("remotes.toml"))
    }

    fn load() -> Result<Self, String> {
        let path = Self::path()?;
        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                toml::from_str(&contents).map_err(|e| format!("{}: {e}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn save(&self) -> Result<(), String> {
        let path = Self::path()?;
        let parent = path
            .parent()
            .ok_or_else(|| "invalid registry path".to_string())?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temporary = parent.join(format!(".remotes-{}.tmp", std::process::id()));
        let data = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&temporary, data).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        std::fs::rename(&temporary, &path).map_err(|e| e.to_string())
    }

    fn resolve(&self, name: &str) -> Result<Mac, String> {
        if let Some(mac) = self.macs.get(name) {
            if valid_host(&mac.host) && mac.port != Some(0) {
                return Ok(mac.clone());
            }
            return Err(format!(
                "saved Mac {name:?} has an invalid SSH target or port"
            ));
        }
        if valid_host(name) {
            return Ok(Mac {
                host: name.into(),
                identity_file: None,
                port: None,
                batch_mode: false,
            });
        }
        Err(format!(
            "no saved Mac named {name:?}; run `sweetpad remote list`"
        ))
    }
}

pub(crate) fn lock_registry(path: &Path) -> Result<File, String> {
    let parent = path.parent().ok_or("invalid registry path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut options = File::options();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path.with_extension("lock"))
        .map_err(|e| e.to_string())?;
    file.lock().map_err(|e| e.to_string())?;
    Ok(file)
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_@".contains(&b))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn remote_invocation(args: &[String]) -> String {
    let executable =
        std::env::var("SWEETPAD_REMOTE_EXECUTABLE").unwrap_or_else(|_| "sweetpad".into());
    format!(
        "PATH=/opt/homebrew/bin:/usr/local/bin:$PATH exec {} {}",
        shell_quote(&executable),
        args.iter()
            .map(|arg| shell_quote(arg))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

#[cfg(windows)]
static REMOTE_INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
unsafe extern "system" fn console_interrupt(kind: u32) -> i32 {
    if kind <= 1 {
        REMOTE_INTERRUPTED.store(true, Ordering::Release);
        1
    } else {
        0
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
    fn GetFileInformationByHandleEx(
        file: *mut std::ffi::c_void,
        class: i32,
        information: *mut std::ffi::c_void,
        size: u32,
    ) -> i32;
}

#[cfg(windows)]
fn windows_change_time(path: &Path) -> Result<i128, String> {
    use std::hash::Hasher;
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    #[derive(Default)]
    struct BasicInfo {
        created: i64,
        accessed: i64,
        written: i64,
        changed: i64,
        attributes: u32,
    }
    let file = File::options()
        .access_mode(0x0080) // FILE_READ_ATTRIBUTES, including unreadable content.
        .custom_flags(0x0220_0000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT.
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut info = BasicInfo::default();
    // FileBasicInfo = 0. The handle remains owned by File throughout this call.
    let success = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            0,
            (&raw mut info).cast(),
            u32::try_from(std::mem::size_of::<BasicInfo>()).unwrap(),
        )
    };
    if success != 0 {
        return Ok(i128::from(info.changed) * 100);
    }
    // Some non-NTFS filesystems cannot provide change time. Hash content there
    // instead of silently missing equal-size edits with a restored mtime.
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let mut input = File::open(path).map_err(|e| e.to_string())?;
    let mut buffer = [0u8; 8192];
    loop {
        let read = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hash.write(&buffer[..read]);
    }
    Ok(i128::from(hash.finish()))
}

pub(crate) fn run_session(
    mac: &Mac,
    command: &str,
    interactive: bool,
) -> Result<std::process::ExitStatus, String> {
    run_session_options(mac, command, interactive, &[])
}

fn interrupt_session(mac: &Mac, pid_file: &str) -> Result<bool, String> {
    let signal = format!(
        "test -r {} && kill -INT \"$(cat {})\"",
        shell_quote(pid_file),
        shell_quote(pid_file)
    );
    ssh_status(mac, &signal)
}

fn run_session_options(
    mac: &Mac,
    command: &str,
    interactive: bool,
    ssh_options: &[String],
) -> Result<std::process::ExitStatus, String> {
    let token = format!(
        "/tmp/sweetpad-session-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    );
    let pid_file = format!("{token}/pid");
    let inner = format!(
        "trap - INT; echo $$ > {}; {command}",
        shell_quote(&pid_file)
    );
    let wrapper = format!(
        "trap ':' INT; umask 077; mkdir {} || exit 1; sh -c {}; code=$?; rm -f {}; rmdir {}; exit $code",
        shell_quote(&token),
        shell_quote(&inner),
        shell_quote(&pid_file),
        shell_quote(&token)
    );
    let mut ssh = mac.ssh();
    ssh.args(ssh_options);
    ssh.args([if interactive { "-tt" } else { "-T" }, &mac.host, &wrapper])
        .stdin(if interactive {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    if !interactive {
        use std::os::unix::process::CommandExt;
        ssh.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Keep Ctrl-C in this client until the remote command has finalized.
        ssh.creation_flags(0x0000_0200);
        REMOTE_INTERRUPTED.store(false, Ordering::Release);
        unsafe {
            SetConsoleCtrlHandler(Some(console_interrupt), 1);
        }
    }
    #[cfg(unix)]
    crate::cli::signals::defer_interrupt(true);
    let result = (|| {
        let mut child = ssh.spawn().map_err(|e| format!("cannot start ssh: {e}"))?;
        let mut pending_interrupt = false;
        loop {
            // Background editor tasks have piped stdio, so a Ctrl-C byte is
            // not a console event. A per-process cancellation file lets them
            // request the same graceful remote signal/finalization path.
            if std::env::var_os("SWEETPAD_CANCEL_FILE")
                .is_some_and(|path| Path::new(&path).exists())
            {
                pending_interrupt = true;
                if let Some(path) = std::env::var_os("SWEETPAD_CANCEL_FILE") {
                    let _ = std::fs::remove_file(path);
                }
            }
            #[cfg(unix)]
            {
                pending_interrupt |= crate::cli::signals::take_forwarded();
            }
            #[cfg(windows)]
            {
                pending_interrupt |= REMOTE_INTERRUPTED.swap(false, Ordering::AcqRel);
            }
            if pending_interrupt {
                // The SSH stream remains open while simctl/xcodebuild handles INT.
                if interrupt_session(mac, &pid_file)? {
                    pending_interrupt = false;
                }
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                // A killed/disconnected SSH client may leave its remote command
                // running. Try a separate connection to cancel only this session;
                // preserve the original exit code if the network is still down.
                if !status.success() {
                    let _ = interrupt_session(mac, &pid_file);
                }
                return Ok(status);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })();
    #[cfg(unix)]
    crate::cli::signals::defer_interrupt(false);
    #[cfg(windows)]
    unsafe {
        SetConsoleCtrlHandler(Some(console_interrupt), 0);
    }
    result
}

pub(crate) fn remote_workspace(id: u64, files: bool) -> String {
    let base = std::env::var("SWEETPAD_REMOTE_ROOT").unwrap_or_else(|_| ".sweetpad/remote".into());
    let path = format!(
        "{}{}/{id:016x}",
        base.trim_end_matches('/'),
        if files { "/files" } else { "" }
    );
    if files {
        format!(
            "{path}/{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    } else {
        path
    }
}

struct Parsed {
    remote: Option<String>,
    chdir: Option<PathBuf>,
    json: bool,
    ndjson: bool,
    non_interactive: bool,
    forwarded: Vec<String>,
}

fn parse(argv: &[String]) -> Result<Parsed, String> {
    let mut remote = None;
    let mut chdir = None;
    let mut json_alias = false;
    let mut output = None;
    let mut non_interactive = false;
    let mut forwarded = Vec::new();
    let mut index = 0;
    while index < argv.len() {
        let arg = &argv[index];
        if arg == "--" {
            forwarded.extend_from_slice(&argv[index..]);
            break;
        }
        if arg == "--remote" || arg == "-C" {
            index += 1;
            let value = argv
                .get(index)
                .ok_or_else(|| format!("{arg} needs a value"))?;
            if arg == "--remote" {
                remote = Some(value.clone());
            } else {
                chdir = Some(PathBuf::from(value));
            }
        } else if let Some(value) = arg.strip_prefix("--remote=") {
            remote = Some(value.to_string());
        } else if let Some(value) = arg.strip_prefix("-C").filter(|value| !value.is_empty()) {
            chdir = Some(PathBuf::from(value));
        } else if arg == "--json" {
            json_alias = true;
            forwarded.push(arg.clone());
        } else if arg == "--non-interactive" {
            non_interactive = true;
            forwarded.push(arg.clone());
        } else if arg == "-o" || arg == "--output" {
            index += 1;
            let value = argv
                .get(index)
                .ok_or_else(|| format!("{arg} needs a value"))?;
            output = Some(value.clone());
            forwarded.extend([arg.clone(), value.clone()]);
        } else {
            if let Some(value) = arg
                .strip_prefix("--output=")
                .or_else(|| arg.strip_prefix("-o").filter(|value| !value.is_empty()))
            {
                output = Some(value.to_string());
            }
            forwarded.push(arg.clone());
        }
        index += 1;
    }
    if output
        .as_deref()
        .is_some_and(|value| !matches!(value, "human" | "quiet" | "json" | "ndjson"))
    {
        return Err("output must be human, quiet, json, or ndjson".into());
    }
    Ok(Parsed {
        remote,
        chdir,
        json: output
            .as_deref()
            .map_or(json_alias, |value| value == "json"),
        ndjson: output.as_deref() == Some("ndjson"),
        non_interactive,
        forwarded,
    })
}

fn print_data(json: bool, ndjson: bool, data: &serde_json::Value) {
    if ndjson {
        println!(
            "{}",
            serde_json::json!({"event":"result","ok":true,"data":data})
        );
    } else if json {
        println!("{}", serde_json::json!({"schema":1,"ok":true,"data":data}));
    } else if let Some(macs) = data.get("macs").and_then(serde_json::Value::as_array) {
        if macs.is_empty() {
            println!("no saved Macs");
        }
        for mac in macs {
            println!(
                "{}  →  {}",
                mac["name"].as_str().unwrap_or(""),
                mac["host"].as_str().unwrap_or("")
            );
        }
    }
}

fn registry_args(args: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if matches!(arg.as_str(), "-o" | "--output" | "--developer-dir") {
            iter.next();
        } else if !(matches!(
            arg.as_str(),
            "--json"
                | "--non-interactive"
                | "--no-color"
                | "-v"
                | "--verbose"
                | "-q"
                | "--quiet"
                | "--gh-annotations"
        ) || arg.starts_with("--output=")
            || arg.starts_with("--developer-dir=")
            || (arg.starts_with("-o") && arg.len() > 2))
        {
            result.push(arg.clone());
        }
    }
    result
}

#[allow(clippy::too_many_lines)] // Manage all registry actions in one dispatch.
fn manage(argv: &[String], json: bool, ndjson: bool) -> Result<(), String> {
    let action = argv.first().map_or("list", String::as_str);
    let _lock = if matches!(action, "add" | "remove") {
        Some(lock_registry(&Registry::path()?)?)
    } else {
        None
    };
    let mut registry = Registry::load()?;
    let name = argv.get(1);
    match action {
        "add" => {
            let name = name.ok_or("remote add needs a name and SSH target")?;
            let host = argv.get(2).ok_or("remote add needs an SSH target")?;
            if name.trim().is_empty()
                || name.trim() != name
                || name.chars().any(char::is_control)
                || !valid_host(host)
            {
                return Err("invalid Mac name or SSH target".into());
            }
            let mut key = None;
            let mut port = None;
            let mut replace = false;
            let mut index = 3;
            while index < argv.len() {
                match argv[index].as_str() {
                    "--identity-file" => {
                        index += 1;
                        let path = argv.get(index).ok_or("--identity-file needs a path")?;
                        let expanded =
                            if path == "~" || path.starts_with("~/") || path.starts_with("~\\") {
                                std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                                    .map(PathBuf::from)
                                    .ok_or("cannot expand ~ without a home directory")?
                                    .join(&path[2.min(path.len())..])
                            } else {
                                PathBuf::from(path)
                            };
                        let path = std::fs::canonicalize(&expanded)
                            .map_err(|e| format!("identity file {path}: {e}"))?;
                        if !path.is_file() {
                            return Err("identity path is not a file".into());
                        }
                        key = Some(path);
                    }
                    "--port" => {
                        index += 1;
                        let value = argv.get(index).ok_or("--port needs a value")?;
                        let parsed: u16 = value
                            .parse()
                            .map_err(|_| "SSH port must be between 1 and 65535")?;
                        if parsed == 0 {
                            return Err("SSH port must be greater than zero".into());
                        }
                        port = Some(parsed);
                    }
                    "--replace" => replace = true,
                    other => return Err(format!("unknown remote add option {other}")),
                }
                index += 1;
            }
            if registry.macs.contains_key(name) && !replace {
                return Err(format!("Mac {name:?} already exists (pass --replace)"));
            }
            registry.macs.insert(
                name.clone(),
                Mac {
                    host: host.clone(),
                    identity_file: key,
                    port,
                    batch_mode: false,
                },
            );
            registry.save()?;
            if !json && !ndjson {
                eprintln!("saved Mac {name:?}");
            }
        }
        "remove" => {
            let name = name.ok_or("remote remove needs a name")?;
            if registry.macs.remove(name).is_none() {
                return Err(format!("no saved Mac named {name:?}"));
            }
            registry.save()?;
            if !json && !ndjson {
                eprintln!("removed Mac {name:?}");
            }
        }
        "show" => {
            let name = name.ok_or("remote show needs a name")?;
            let mac = registry
                .macs
                .get(name)
                .ok_or_else(|| format!("no saved Mac named {name:?}"))?
                .clone();
            registry.macs.clear();
            registry.macs.insert(name.clone(), mac);
        }
        "list" => {}
        _ => return Err(format!("unknown remote action {action}")),
    }
    let macs: Vec<_> = registry
        .macs
        .iter()
        .map(|(name, mac)| {
            serde_json::json!({
                "name":name,"host":mac.host,"identityFile":mac.identity_file,"port":mac.port
            })
        })
        .collect();
    print_data(json, ndjson, &serde_json::json!({"macs":macs}));
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Workspace {
    None,
    Files,
    Snapshot,
    Live,
}

fn resource(args: &[String]) -> Option<(&str, Option<&str>)> {
    let mut iter = args.iter().map(String::as_str);
    while let Some(arg) = iter.next() {
        if matches!(arg, "-o" | "--output" | "--developer-dir") {
            iter.next();
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        return Some((arg, iter.next()));
    }
    None
}

fn workspace(args: &[String]) -> Workspace {
    let Some((name, action)) = resource(args) else {
        return Workspace::Snapshot;
    };
    match name {
        "devices" | "device" | "destination" | "doctor" | "help" | "completions"
        | "self-update" => Workspace::None,
        "simulator" | "sim" => match action {
            Some("push" | "media-add" | "record" | "screenshot") => Workspace::Files,
            _ => Workspace::None,
        },
        "open" if matches!(action, Some("sim" | "config")) => Workspace::None,
        "run"
            if args
                .iter()
                .any(|arg| matches!(arg.as_str(), "--detach" | "--no-logs")) =>
        {
            Workspace::Snapshot
        }
        "run" => Workspace::Live,
        "app"
            if matches!(action, None | Some("run"))
                && args
                    .iter()
                    .any(|arg| matches!(arg.as_str(), "--detach" | "--no-logs")) =>
        {
            Workspace::Snapshot
        }
        "app" if matches!(action, None | Some("run")) => Workspace::Live,
        "build" | "test" if args.iter().any(|arg| arg == "--watch") => Workspace::Live,
        _ => Workspace::Snapshot,
    }
}

fn find_project_root(cwd: &Path) -> PathBuf {
    for dir in cwd.ancestors() {
        if dir.join("sweetpad.toml").is_file() || dir.join("Package.swift").is_file() {
            return dir.to_path_buf();
        }
        if std::fs::read_dir(dir).is_ok_and(|entries| {
            entries.filter_map(Result::ok).any(|entry| {
                matches!(
                    entry.path().extension().and_then(std::ffi::OsStr::to_str),
                    Some("xcodeproj" | "xcworkspace")
                )
            })
        }) {
            return dir.to_path_buf();
        }
    }
    cwd.to_path_buf()
}

pub(crate) fn id(root: &Path, mac: &Mac) -> u64 {
    root.to_string_lossy()
        .bytes()
        .chain([0])
        .chain(mac.host.bytes())
        .chain([0])
        .chain(mac.port.unwrap_or(22).to_string().bytes())
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        })
}

fn local_relative(path: &Path, root: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        fn ordinary(path: &Path) -> PathBuf {
            let text = path.to_string_lossy();
            if let Some(tail) = text.strip_prefix(r"\\?\UNC\") {
                PathBuf::from(format!(r"\\{tail}"))
            } else if let Some(tail) = text.strip_prefix(r"\\?\") {
                PathBuf::from(tail)
            } else {
                path.to_path_buf()
            }
        }
        let path = ordinary(path);
        let root = ordinary(root);
        let mut parts = path.components();
        for expected in root.components() {
            let Some(actual) = parts.next() else {
                return Err("path is outside the synced workspace".into());
            };
            if !actual
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&expected.as_os_str().to_string_lossy())
            {
                return Err("path is outside the synced workspace".into());
            }
        }
        Ok(parts.collect())
    }
    #[cfg(not(windows))]
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .map_err(|_| "path is outside the synced workspace".into())
}

fn remote_relative(path: &Path, root: &Path, cwd: &Path) -> Result<String, String> {
    if !path.is_absolute() {
        return Ok(path.to_string_lossy().replace('\\', "/"));
    }
    let relative = local_relative(path, root)
        .map_err(|_| format!("{} is outside the synced workspace", path.display()))?;
    let depth = cwd
        .strip_prefix(root)
        .map_err(|_| "working directory is outside synced workspace")?
        .components()
        .count();
    let mut pieces = vec!["..".to_string(); depth];
    pieces.extend(relative.components().filter_map(|part| match part {
        Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
        _ => None,
    }));
    Ok(if pieces.is_empty() {
        ".".into()
    } else {
        pieces.join("/")
    })
}

fn forward_args(args: &[String], root: &Path, cwd: &Path) -> Result<Vec<String>, String> {
    const PATH_FLAGS: [&str; 9] = [
        "--project",
        "--workspace",
        "--hot-entitlements",
        "--hot-selfcheck",
        "--output-file",
        "--result-bundle",
        "--junit",
        "--output-dir",
        "--export-options",
    ];
    let mut forwarded = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            forwarded.push(arg.clone());
            forwarded.extend(iter.cloned());
            break;
        }
        if PATH_FLAGS.contains(&arg.as_str()) {
            let value = iter.next().ok_or_else(|| format!("{arg} needs a path"))?;
            forwarded.push(arg.clone());
            forwarded.push(remote_relative(Path::new(value), root, cwd)?);
        } else if let Some((flag, value)) = arg.split_once('=') {
            if PATH_FLAGS.contains(&flag) {
                forwarded.push(format!(
                    "{flag}={}",
                    remote_relative(Path::new(value), root, cwd)?
                ));
            } else {
                forwarded.push(arg.clone());
            }
        } else if Path::new(arg).is_absolute() && local_relative(Path::new(arg), root).is_ok() {
            forwarded.push(remote_relative(Path::new(arg), root, cwd)?);
        } else {
            forwarded.push(arg.clone());
        }
    }
    Ok(forwarded)
}

const EXCLUDED: [&str; 7] = [
    ".sweetpad-tools",
    ".build",
    "target",
    "node_modules",
    "DerivedData",
    "build",
    ".DS_Store",
];

fn is_excluded(relative: &Path, include_git: bool, include_build: bool) -> bool {
    relative.components().any(|component| match component {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            (EXCLUDED.contains(&name.as_ref()) && (!include_build || name != "build"))
                || (!include_git && name == ".git")
                || (!include_build
                    && (name == "sweetpad-shots"
                        || Path::new(name.as_ref())
                            .extension()
                            .is_some_and(|ext| ext == "xcresult")))
        }
        _ => false,
    })
}

fn inputs(args: &[String], cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let mut positional = Vec::new();
    let mut iter = args.iter().map(String::as_str);
    while let Some(arg) = iter.next() {
        if matches!(
            arg,
            "-o" | "--output" | "--developer-dir" | "--target" | "--output-file"
        ) {
            iter.next();
        } else if !arg.starts_with('-') {
            positional.push(arg);
        }
    }
    let paths = match positional.as_slice() {
        ["simulator" | "sim", "push", _, payload, ..] => vec![*payload],
        ["simulator" | "sim", "media-add", paths @ ..] => paths.to_vec(),
        _ => return Ok(Vec::new()),
    };
    paths.into_iter().map(|arg| {
        let path = Path::new(arg);
        let absolute = if path.is_absolute() { path.to_path_buf() } else { cwd.join(path) };
        let canonical = std::fs::canonicalize(&absolute).map_err(|e| format!("cannot read input file {arg}: {e}"))?;
        let relative = local_relative(&absolute, cwd)?;
        if local_relative(&canonical, cwd).is_err()
            || relative.components().any(|part| part == Component::ParentDir) {
            return Err(format!("input file {arg} is outside the working directory; use -C to select its directory"));
        }
        if !canonical.is_file() { return Err(format!("input {arg} is not a file")); }
        Ok(cwd.join(relative))
    }).collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum EntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Fingerprint {
    kind: EntryKind,
    size: u64,
    modified_ns: u64,
    target: Option<String>,
    #[serde(default)]
    mode: u32,
    #[serde(default)]
    changed_ns: i128,
}

#[derive(Default, Serialize, Deserialize)]
struct Manifest {
    files: BTreeMap<String, Fingerprint>,
}

fn relative_key(path: &Path) -> Result<String, String> {
    let value = path
        .to_str()
        .ok_or_else(|| format!("{} is not a Unicode path", path.display()))?;
    Ok(if cfg!(windows) {
        value.replace('\\', "/")
    } else {
        value.to_string()
    })
}

fn scan_project(root: &Path) -> Result<Manifest, String> {
    let mut manifest = Manifest::default();
    let mut walker = WalkDir::new(root).into_iter();
    while let Some(item) = walker.next() {
        let item = item.map_err(|e| e.to_string())?;
        if item.path() == root {
            continue;
        }
        let relative = item.path().strip_prefix(root).map_err(|e| e.to_string())?;
        if is_excluded(relative, true, false) {
            if item.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        let kind = if item.file_type().is_dir() {
            EntryKind::Directory
        } else if item.file_type().is_symlink() {
            EntryKind::Symlink
        } else if item.file_type().is_file() {
            EntryKind::File
        } else {
            continue;
        };
        let metadata = std::fs::symlink_metadata(item.path()).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        let (mode, changed_ns) = {
            use std::os::unix::fs::MetadataExt;
            (
                metadata.mode() & 0o7777,
                i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec()),
            )
        };
        #[cfg(windows)]
        let (mode, changed_ns) = (
            u32::from(metadata.permissions().readonly()),
            if kind == EntryKind::File {
                windows_change_time(item.path())?
            } else {
                0
            },
        );
        let fingerprint = Fingerprint {
            mode,
            changed_ns: if kind == EntryKind::Directory {
                0
            } else {
                changed_ns
            },
            size: if kind == EntryKind::File {
                metadata.len()
            } else {
                0
            },
            modified_ns: if kind == EntryKind::Directory {
                0
            } else {
                metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .and_then(|age| u64::try_from(age.as_nanos()).ok())
                    .unwrap_or(0)
            },
            target: if kind == EntryKind::Symlink {
                Some(
                    std::fs::read_link(item.path())
                        .map_err(|e| e.to_string())?
                        .to_string_lossy()
                        .into_owned(),
                )
            } else {
                None
            },
            kind,
        };
        manifest.files.insert(relative_key(relative)?, fingerprint);
    }
    Ok(manifest)
}

fn manifest_path(root: &Path, mac: &Mac) -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .filter(|value| !value.is_empty())
            .map_or_else(std::env::temp_dir, PathBuf::from)
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .unwrap_or_else(std::env::temp_dir)
    };
    base.join("sweetpad")
        .join("sync")
        .join(format!("{:016x}.json", id(root, mac)))
}

fn save_manifest(path: &Path, manifest: &Manifest) -> Result<(), String> {
    let parent = path.parent().ok_or("invalid sync cache path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(
        &temp,
        serde_json::to_vec(manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}

pub(crate) fn ssh_status(mac: &Mac, command: &str) -> Result<bool, String> {
    mac.ssh()
        .args(["-T", &mac.host, command])
        // Control requests must not compete with the main SSH session for a
        // Windows pseudoconsole input handle (OpenSSH can block on that read).
        .stdin(Stdio::null())
        .status()
        .map(|status| status.success())
        .map_err(|e| format!("cannot start ssh: {e}"))
}

fn valid_relative_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn changed_paths(previous: &Manifest, current: &Manifest) -> (Vec<String>, Vec<String>) {
    let changed = current
        .files
        .iter()
        .filter_map(|(path, fingerprint)| {
            (previous.files.get(path) != Some(fingerprint)).then_some(path.clone())
        })
        .collect();
    let deleted = previous
        .files
        .iter()
        .filter_map(|(path, old)| {
            let replace = current.files.get(path).is_some_and(|new| {
                new.kind != old.kind || (old.kind == EntryKind::Symlink && new != old)
            });
            (replace || !current.files.contains_key(path)).then_some(path.clone())
        })
        .collect();
    (changed, deleted)
}

/// Keep one project workspace on the Mac, sending only changed paths after the first transfer.
pub(crate) fn sync_project(root: &Path, mac: &Mac, remote_dir: &str) -> Result<(), String> {
    let path = manifest_path(root, mac);
    std::fs::create_dir_all(path.parent().ok_or("invalid sync cache path")?)
        .map_err(|e| e.to_string())?;
    // A GUI refresh and build can start together. Serialize the transaction so
    // a second first-time sync cannot clear a workspace the first just created.
    let transaction = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))
        .map_err(|e| e.to_string())?;
    transaction
        .lock()
        .map_err(|e| format!("cannot lock sync cache: {e}"))?;
    let old = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok());
    let current = scan_project(root)?;
    let marker = format!("{remote_dir}.sync-v1");
    let ready = old.is_some()
        && ssh_status(
            mac,
            &format!(
                "test -d {} && test -f {}",
                shell_quote(remote_dir),
                shell_quote(&marker)
            ),
        )?;
    if !ready {
        let prepare = format!(
            "umask 077; rm -f {} && rm -rf {} && mkdir -p {} && chmod 700 {}",
            shell_quote(&marker),
            shell_quote(remote_dir),
            shell_quote(remote_dir),
            shell_quote(remote_dir)
        );
        if !ssh_status(mac, &prepare)? {
            return Err(format!("cannot prepare workspace on {}", mac.host));
        }
        send_archive(root, mac, remote_dir, None)?;
        if !ssh_status(mac, &format!("touch {}", shell_quote(&marker)))? {
            return Err(format!("cannot mark workspace ready on {}", mac.host));
        }
    } else if let Some(old) = &old {
        let (changed, deleted) = changed_paths(old, &current);
        for batch in deleted.chunks(100) {
            let mut remove = String::from("rm -rf");
            for relative in batch {
                if !valid_relative_key(relative) {
                    return Err("invalid path in sync cache".into());
                }
                remove.push(' ');
                remove.push_str(&shell_quote(&format!("{remote_dir}/{relative}")));
            }
            if !ssh_status(mac, &remove)? {
                return Err(format!("cannot remove deleted files on {}", mac.host));
            }
        }
        if !changed.is_empty() {
            let files: Vec<_> = changed
                .into_iter()
                .map(|relative| root.join(relative))
                .collect();
            send_archive(root, mac, remote_dir, Some(&files))?;
        }
    }
    save_manifest(&path, &current)
}

pub(crate) fn send_archive(
    root: &Path,
    mac: &Mac,
    remote_dir: &str,
    files: Option<&[PathBuf]>,
) -> Result<(), String> {
    send_archive_inner(root, mac, remote_dir, files, false)
}

pub(crate) fn send_inputs(
    root: &Path,
    mac: &Mac,
    remote_dir: &str,
    files: &[PathBuf],
) -> Result<(), String> {
    send_archive_inner(root, mac, remote_dir, Some(files), true)
}

fn send_archive_inner(
    root: &Path,
    mac: &Mac,
    remote_dir: &str,
    files: Option<&[PathBuf]>,
    follow_inputs: bool,
) -> Result<(), String> {
    let command = format!("tar -xf - -C {}", shell_quote(remote_dir));
    let mut ssh = mac.ssh();
    let mut child = ssh
        .args(["-T", &mac.host, &command])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start ssh: {e}"))?;
    let result = (|| {
        let stdin = child.stdin.take().ok_or("cannot write SSH archive")?;
        let mut archive = tar::Builder::new(stdin);
        append_archive(&mut archive, root, files, follow_inputs)?;
        archive.finish().map_err(|e| e.to_string())?;
        drop(archive);
        let status = child.wait().map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("upload to {} failed ({status})", mac.host))
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn append_archive<W: std::io::Write>(
    archive: &mut tar::Builder<W>,
    root: &Path,
    files: Option<&[PathBuf]>,
    follow_inputs: bool,
) -> Result<(), String> {
    archive.follow_symlinks(follow_inputs);
    if let Some(files) = files {
        for file in files {
            let relative = file.strip_prefix(root).map_err(|_| {
                format!("input {} is outside the working directory", file.display())
            })?;
            if follow_inputs || !is_excluded(relative, true, false) {
                let metadata = std::fs::symlink_metadata(file).map_err(|e| e.to_string())?;
                if metadata.is_dir() {
                    archive
                        .append_dir(relative, file)
                        .map_err(|e| e.to_string())?;
                } else if metadata.file_type().is_symlink() {
                    archive
                        .append_path_with_name(file, relative)
                        .map_err(|e| e.to_string())?;
                } else {
                    let data = File::open(file).map_err(|e| e.to_string())?;
                    let mut header = tar::Header::new_gnu();
                    header.set_metadata(&data.metadata().map_err(|e| e.to_string())?);
                    archive
                        .append_data(&mut header, relative, data)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
    } else {
        let mut walker = WalkDir::new(root).into_iter();
        while let Some(item) = walker.next() {
            let item = item.map_err(|e| e.to_string())?;
            let path = item.path();
            if path == root {
                continue;
            }
            let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
            if is_excluded(relative, true, false) {
                if item.file_type().is_dir() {
                    walker.skip_current_dir();
                }
                continue;
            }
            if item.file_type().is_dir() {
                archive
                    .append_dir(relative, path)
                    .map_err(|e| e.to_string())?;
            } else {
                archive
                    .append_path_with_name(path, relative)
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

pub(crate) fn mark_download_baseline(mac: &Mac, remote_dir: &str) -> Result<String, String> {
    let marker = format!(
        "{remote_dir}.pull-stamp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    if ssh_status(mac, &format!("touch {}", shell_quote(&marker)))? {
        Ok(marker)
    } else {
        Err(format!("cannot mark download baseline on {}", mac.host))
    }
}

fn download_command(remote_dir: &str, baseline: &str) -> Result<String, String> {
    let base = Path::new(remote_dir)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or("invalid remote workspace path")?;
    let marker_name = Path::new(baseline)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or("invalid download marker path")?;
    let marker = format!("../{marker_name}");
    let listing = format!("../{base}.pull-list-{}", std::process::id());
    // A result bundle is replaced as a unit by Xcode. Send every member when
    // any member changed, including objects whose timestamps were preserved.
    let result_listing = shell_quote(
        r#"if [ -n "$(find "$1" -type f \( -newer "$2" -o -cnewer "$2" \) -print)" ]; then find "$1" -type f -print0; fi"#,
    );
    Ok(format!(
        "umask 077; cd {} || exit; find . \\( -name .git -o -name .build -o -name target -o -name node_modules -o -name DerivedData -o -name .DS_Store -o -name .sweetpad-tools -o -name '*.xcresult' \\) -prune -o -type f \\( -newer {} -o -cnewer {} \\) -print0 > {} || exit; find . \\( -name .git -o -name .build -o -name target -o -name node_modules -o -name DerivedData -o -name .DS_Store -o -name .sweetpad-tools \\) -prune -o -type d -name '*.xcresult' -exec sh -c {result_listing} sh '{{}}' {} \\; >> {} || exit; COPYFILE_DISABLE=1 tar --no-recursion -cf - --null -T {}; code=$?; rm -f {}; exit $code",
        shell_quote(remote_dir),
        shell_quote(&marker),
        shell_quote(&marker),
        shell_quote(&listing),
        shell_quote(&marker),
        shell_quote(&listing),
        shell_quote(&listing),
        shell_quote(&listing)
    ))
}

fn result_bundle(relative: &Path) -> Option<PathBuf> {
    let mut prefix = PathBuf::new();
    for component in relative.components() {
        if let Component::Normal(name) = component {
            prefix.push(name);
            if Path::new(name)
                .extension()
                .is_some_and(|ext| ext == "xcresult")
            {
                return Some(prefix);
            }
        }
    }
    None
}

fn prune_stale_results(
    root: &Path,
    bundles: &BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    started: SystemTime,
) -> Result<(), String> {
    for (bundle, expected) in bundles {
        let mut ancestor = root.to_path_buf();
        for component in bundle.components() {
            ancestor.push(component);
            if std::fs::symlink_metadata(&ancestor).is_ok_and(|meta| meta.file_type().is_symlink())
            {
                return Err(format!(
                    "refusing to replace a result bundle through a symlink: {}",
                    ancestor.display()
                ));
            }
        }
        for entry in WalkDir::new(root.join(bundle)).follow_links(false) {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = entry.path().strip_prefix(root).map_err(|e| e.to_string())?;
            if !expected.contains(relative)
                && entry
                    .metadata()
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .is_some_and(|modified| modified <= started)
            {
                std::fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

pub(crate) fn receive_archive(
    root: &Path,
    mac: &Mac,
    remote_dir: &str,
    started: SystemTime,
    baseline: &str,
) -> Result<(), String> {
    let args = download_command(remote_dir, baseline)?;
    let mut ssh = mac.ssh();
    let mut child = ssh
        .args(["-T", &mac.host, &args])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start ssh: {e}"))?;
    let result = (|| {
        let stdout = child.stdout.take().ok_or("cannot read SSH archive")?;
        let mut archive = tar::Archive::new(stdout);
        let mut result_files: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
        for entry in archive.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let relative = entry.path().map_err(|e| e.to_string())?.into_owned();
            if relative
                .components()
                .any(|part| !matches!(part, Component::CurDir | Component::Normal(_)))
                || is_excluded(&relative, false, true)
            {
                continue;
            }
            let path = root.join(&relative);
            if entry.header().entry_type().is_dir() {
                std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            } else if entry.header().entry_type().is_file() {
                if let Some(bundle) = result_bundle(&relative) {
                    let normalized = relative
                        .components()
                        .filter(|part| matches!(part, Component::Normal(_)))
                        .collect::<PathBuf>();
                    result_files.entry(bundle).or_default().insert(normalized);
                }
                let local_modified = std::fs::metadata(&path)
                    .and_then(|meta| meta.modified())
                    .ok();
                // The marker already selects remote changes. Comparing the two
                // machines' clocks can discard a freshly generated artifact.
                if local_modified.is_some_and(|time| time > started) {
                    continue;
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                entry.unpack_in(root).map_err(|e| e.to_string())?;
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        if status.success() {
            prune_stale_results(root, &result_files, started)
        } else {
            Err(format!("download from {} failed ({status})", mac.host))
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if result.is_ok() {
        let _ = ssh_status(mac, &format!("rm -f {}", shell_quote(baseline)));
    }
    result
}

#[allow(clippy::too_many_lines)] // SSH setup, sync lifecycle, and teardown share one error path.
fn execute(args: &[String], parsed: &Parsed) -> Result<u8, String> {
    let mut mac = Registry::load()?.resolve(
        parsed
            .remote
            .as_deref()
            .ok_or("--remote needs a Mac name or SSH host")?,
    )?;
    mac.batch_mode = parsed.non_interactive;
    if let Some(dir) = &parsed.chdir {
        std::env::set_current_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let cwd = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|e| e.to_string())?;
    let mode = workspace(args);
    let root = if matches!(mode, Workspace::Files | Workspace::None) {
        cwd.clone()
    } else {
        find_project_root(&cwd)
    };
    if matches!(mode, Workspace::Snapshot | Workspace::Live)
        && (root.parent().is_none()
            || std::env::var_os("USERPROFILE")
                .and_then(|home| std::fs::canonicalize(home).ok())
                .is_some_and(|home| root == home))
    {
        return Err("refusing to sync the whole home or root directory; run from a project folder or use -C".into());
    }
    let remote_dir = remote_workspace(id(&root, &mac), mode == Workspace::Files);
    let started = SystemTime::now();
    let forwarded = forward_args(args, &root, &cwd)?;
    let command = remote_invocation(&forwarded);
    let remote_cwd = if cwd == root {
        remote_dir.clone()
    } else {
        let suffix = cwd
            .strip_prefix(&root)
            .map_err(|e| e.to_string())?
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        format!("{remote_dir}/{suffix}")
    };
    let mut baseline = None;
    if mode != Workspace::None {
        if mode == Workspace::Files {
            let prep = format!(
                "umask 077; rm -rf {} && mkdir -p {} && chmod 700 {}",
                shell_quote(&remote_dir),
                shell_quote(&remote_dir),
                shell_quote(&remote_dir)
            );
            if !ssh_status(&mac, &prep)? {
                return Err(format!("cannot prepare workspace on {}", mac.host));
            }
            let files = inputs(args, &root)?;
            send_inputs(&root, &mac, &remote_dir, &files)?;
        } else {
            sync_project(&root, &mac, &remote_dir)?;
        }
        baseline = Some(mark_download_baseline(&mac, &remote_dir)?);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let worker = (mode == Workspace::Live).then(|| {
        let stop = Arc::clone(&stop);
        let root = root.clone();
        let mac = mac.clone();
        let remote_dir = remote_dir.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(2));
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(error) = sync_project(&root, &mac, &remote_dir) {
                    eprintln!("SweetPad sync: {error}");
                }
            }
        })
    });
    let invocation = if mode == Workspace::None {
        command
    } else {
        format!("cd {} && {command}", shell_quote(&remote_cwd))
    };
    let interactive = std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && !parsed.json
        && !parsed.ndjson;
    let result = run_session(&mac, &invocation, interactive);
    stop.store(true, Ordering::Relaxed);
    if let Some(worker) = worker {
        let _ = worker.join();
    }
    let status = result?;
    if let Some(baseline) = &baseline
        && let Err(error) = receive_archive(&root, &mac, &remote_dir, started, baseline)
    {
        if status.success() {
            return Err(error);
        }
        eprintln!("SweetPad sync: {error}");
    }
    Ok(status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1))
}

#[must_use]
pub fn is_bridge(argv: &[String]) -> bool {
    parse(argv)
        .is_ok_and(|parsed| resource(&parsed.forwarded).is_some_and(|(name, _)| name == "bridge"))
}

fn bridge_workspace(parsed: &Parsed, mac: &Mac) -> Result<(PathBuf, String), String> {
    if let Some(dir) = &parsed.chdir {
        std::env::set_current_dir(dir).map_err(|e| e.to_string())?;
    }
    let cwd = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|e| e.to_string())?;
    let root = find_project_root(&cwd);
    if root.parent().is_none()
        || ["USERPROFILE", "HOME"].iter().any(|key| {
            std::env::var_os(key)
                .and_then(|p| std::fs::canonicalize(p).ok())
                .is_some_and(|p| p == root)
        })
    {
        return Err("run bridge from a project folder or use -C".into());
    }
    let remote = remote_workspace(id(&root, mac), false);
    sync_project(&root, mac, &remote)?;
    Ok((root, remote))
}

fn stream_options(args: &mut Vec<String>) -> Result<Vec<String>, String> {
    if args.len() < 4 {
        return Err(
            "bridge stream expects DEVICE LOCAL_PORT REMOTE_PORT PROGRAM [ARGS] after --".into(),
        );
    }
    let device = args.remove(0);
    if device.is_empty()
        || !device
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("invalid simulator UDID".into());
    }
    let local_port: u16 = args
        .remove(0)
        .parse()
        .map_err(|_| "invalid local stream port")?;
    let remote_port: u16 = args
        .remove(0)
        .parse()
        .map_err(|_| "invalid remote stream port")?;
    if local_port == 0 || remote_port == 0 {
        return Err("stream ports must be nonzero".into());
    }
    args.extend([device, "--port".into(), remote_port.to_string()]);
    Ok(vec![
        "-o".into(),
        "ExitOnForwardFailure=yes".into(),
        "-L".into(),
        format!("127.0.0.1:{local_port}:127.0.0.1:{remote_port}"),
    ])
}

fn bridge(parsed: &Parsed) -> Result<u8, String> {
    let action = resource(&parsed.forwarded)
        .and_then(|(_, action)| action)
        .unwrap_or("context");
    if parsed
        .forwarded
        .iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        println!(
            "Usage: sweetpad bridge SERVICE --remote MAC [--developer-dir PATH] [-- PROGRAM ARGS]\n\nServices:\n  context  Sync project and return local/remote roots (supports --json).\n  lsp      SourceKit-LSP over framed stdio; prepares the build server.\n  dap      LLDB DAP over framed stdio.\n  exec     Run PROGRAM ARGS in the synchronized project.\n  stream   DEVICE LOCAL_PORT REMOTE_PORT PROGRAM ARGS, with loopback SSH forwarding."
        );
        return Ok(0);
    }
    if !matches!(action, "context" | "lsp" | "dap" | "exec" | "stream") {
        return Err(format!(
            "unknown bridge service {action}; use context, lsp, dap, exec or stream"
        ));
    }
    if action != "context" && (parsed.json || parsed.ndjson) {
        return Err(
            "bridge services own stdout; JSON output is only available for bridge context".into(),
        );
    }
    let reference = parsed
        .remote
        .as_deref()
        .ok_or("bridge requires --remote MAC")?;
    let mut mac = Registry::load()?.resolve(reference)?;
    mac.batch_mode = true;
    let (root, remote) = bridge_workspace(parsed, &mac)?;
    if action == "context" {
        print_data(
            parsed.json,
            parsed.ndjson,
            &serde_json::json!({"localRoot":root,"remoteRoot":remote}),
        );
        return Ok(0);
    }
    let mut args: Vec<String> = match action {
        "lsp" => vec!["xcrun".into(), "sourcekit-lsp".into()],
        "dap" => vec!["xcrun".into(), "lldb-dap".into()],
        _ => Vec::new(), // Service names were validated before syncing.
    };
    if let Some(index) = parsed.forwarded.iter().position(|arg| arg == "--") {
        args.extend_from_slice(&parsed.forwarded[index + 1..]);
    }
    if args.is_empty() {
        return Err("bridge exec needs a program after --".into());
    }
    let developer_dir = parsed
        .forwarded
        .windows(2)
        .find(|pair| pair[0] == "--developer-dir")
        .map(|pair| pair[1].as_str());
    let env = developer_dir.map_or_else(String::new, |dir| {
        format!("DEVELOPER_DIR={} ", shell_quote(dir))
    });
    if action == "lsp" {
        let init = format!(
            "cd {} && {env}{}",
            shell_quote(&remote),
            remote_invocation(&["bsp".into(), "init".into()])
        );
        if !ssh_status(&mac, &format!("{init} >&2"))? {
            return Err("cannot prepare remote build server".into());
        }
    }
    let ssh_options = if action == "stream" {
        stream_options(&mut args)?
    } else {
        Vec::new()
    };
    let command = format!(
        "cd {} && {env}NPM_CONFIG_CACHE={} PATH=/opt/homebrew/bin:/usr/local/bin:$PATH exec {}",
        shell_quote(&remote),
        shell_quote(&format!("{remote}/.sweetpad-tools/npm-cache")),
        args.iter()
            .map(|arg| shell_quote(arg))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let status = if matches!(action, "exec" | "stream") {
        #[cfg(unix)]
        crate::cli::signals::install();
        run_session_options(&mac, &command, false, &ssh_options)?
    } else {
        mac.ssh()
            .args(["-T", &mac.host, &command])
            .status()
            .map_err(|e| format!("cannot start ssh: {e}"))?
    };
    Ok(status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1))
}

pub fn run(argv: &[String]) -> ExitCode {
    if matches!(argv.first().map(String::as_str), Some("--version" | "-V")) {
        println!("sweetpad {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if argv.is_empty() || matches!(argv.first().map(String::as_str), Some("--help" | "-h")) {
        println!(
            "SweetPad native Windows remote client\n\nUsage:\n  sweetpad remote add NAME USER@HOST [--identity-file PATH] [--port PORT]\n  sweetpad remote list|show NAME|remove NAME\n  sweetpad COMMAND [ARGS] --remote NAME\n\nThe receiving Mac needs the updated SweetPad CLI and Xcode."
        );
        return ExitCode::SUCCESS;
    }
    let parsed = match parse(argv) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("sweetpad: {error}");
            return ExitCode::from(2);
        }
    };
    let name = resource(&parsed.forwarded).map(|(name, _)| name);
    let result = if name == Some("bridge") {
        bridge(&parsed)
    } else if name == Some("remote") {
        let index = parsed
            .forwarded
            .iter()
            .position(|arg| arg == "remote")
            .unwrap_or(0);
        manage(
            &registry_args(&parsed.forwarded[index + 1..]),
            parsed.json,
            parsed.ndjson,
        )
        .map(|()| 0)
    } else if parsed.remote.is_some() {
        execute(&parsed.forwarded, &parsed)
    } else {
        Err("this Windows build runs Xcode commands on a Mac; use --remote MAC or `sweetpad remote add`".into())
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            if parsed.ndjson {
                println!(
                    "{}",
                    serde_json::json!({"event":"result","ok":false,"error":{"message":error}})
                );
            } else if parsed.json {
                println!(
                    "{}",
                    serde_json::json!({"schema":1,"ok":false,"error":{"message":error}})
                );
            } else {
                eprintln!("sweetpad: {error}");
            }
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_help_and_invalid_services_do_not_connect_or_sync() {
        let help = parse(&["bridge", "--help"].map(str::to_string)).unwrap();
        assert!(is_bridge(&["bridge", "--help"].map(str::to_string)));
        assert_eq!(bridge(&help).unwrap(), 0);
        let invalid = parse(&["bridge", "invalid"].map(str::to_string)).unwrap();
        assert!(
            bridge(&invalid)
                .unwrap_err()
                .contains("unknown bridge service")
        );
        let json = parse(&["--json", "bridge", "dap"].map(str::to_string)).unwrap();
        assert!(bridge(&json).unwrap_err().contains("services own stdout"));
    }

    #[cfg(unix)]
    #[test]
    fn project_archives_preserve_links_and_explicit_inputs_send_contents() {
        let root = std::env::temp_dir().join(format!("sweetpad-links-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("payload.json"), "payload").unwrap();
        std::os::unix::fs::symlink("payload.json", root.join("input.json")).unwrap();
        std::os::unix::fs::symlink("missing", root.join("broken")).unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        append_archive(&mut builder, &root, None, false).unwrap();
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(bytes.as_slice());
        let links: Vec<_> = archive
            .entries()
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.path().unwrap().into_owned(),
                    e.header().entry_type().is_symlink(),
                )
            })
            .collect();
        assert!(links.contains(&(PathBuf::from("broken"), true)));
        assert!(links.contains(&(PathBuf::from("input.json"), true)));
        let mut builder = tar::Builder::new(Vec::new());
        append_archive(&mut builder, &root, Some(&[root.join("input.json")]), true).unwrap();
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(bytes.as_slice());
        let entry = archive.entries().unwrap().next().unwrap().unwrap();
        assert!(entry.header().entry_type().is_file());
        assert_eq!(entry.size(), 7);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn manifest_detects_chmod_and_edits_with_preserved_mtime() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("sweetpad-modes-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("script");
        std::fs::write(&file, "first").unwrap();
        let mtime = std::fs::metadata(&file).unwrap().modified().unwrap();
        let initial = scan_project(&root).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let executable = scan_project(&root).unwrap();
        assert_eq!(changed_paths(&initial, &executable).0, ["script"]);
        std::thread::sleep(Duration::from_millis(10));
        std::fs::write(&file, "other").unwrap();
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(mtime))
            .unwrap();
        assert_eq!(
            changed_paths(&executable, &scan_project(&root).unwrap()).0,
            ["script"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_workspace_classification_handles_global_values_and_detach() {
        let args = [
            "--developer-dir",
            "/Applications/Xcode.app",
            "simulator",
            "open",
        ]
        .map(str::to_string);
        assert_eq!(workspace(&args), Workspace::None);
        for command in [vec!["run", "--detach"], vec!["app", "run", "--no-logs"]] {
            assert_eq!(
                workspace(&command.into_iter().map(str::to_string).collect::<Vec<_>>()),
                Workspace::Snapshot
            );
        }
    }

    #[test]
    fn windows_file_commands_upload_only_explicit_inputs() {
        let root = std::env::temp_dir().join(format!("sweetpad-inputs-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        std::fs::write(root.join("old.mp4"), "large old recording").unwrap();
        std::fs::write(root.join("push.json"), "{}").unwrap();
        let record = ["simulator", "record", "--output-file", "old.mp4"].map(str::to_string);
        assert!(inputs(&record, &root).unwrap().is_empty());
        let push = [
            "-o",
            "json",
            "simulator",
            "push",
            "com.example",
            "push.json",
            "device",
        ]
        .map(str::to_string);
        assert_eq!(inputs(&push, &root).unwrap(), [root.join("push.json")]);
        let media = ["simulator", "media-add", "missing.png"].map(str::to_string);
        assert!(inputs(&media, &root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_normal_and_verbatim_paths_map_to_the_same_workspace() {
        let root = Path::new(r"\\?\C:\Work\Project");
        let path = Path::new(r"c:\work\project\shots\phone.png");
        assert_eq!(
            local_relative(path, root).unwrap(),
            PathBuf::from(r"shots\phone.png")
        );
        assert_eq!(
            remote_relative(path, root, root).unwrap(),
            "shots/phone.png"
        );
        assert!(local_relative(Path::new(r"C:\Work\ProjectOther\file"), root).is_err());
        let unc_root = Path::new(r"\\?\UNC\server\share\Project");
        assert_eq!(
            local_relative(Path::new(r"\\server\share\Project\file"), unc_root).unwrap(),
            PathBuf::from("file")
        );
    }

    #[test]
    fn preserves_names_and_keys_in_registry_format() {
        let text = "[macs.\"Studio Mac\"]\nhost = \"dev@mac.local\"\nidentity_file = \"C:\\\\Keys\\\\dev key\"\nport = 2222\n";
        let registry: Registry = toml::from_str(text).unwrap();
        assert_eq!(registry.macs["Studio Mac"].port, Some(2222));
        assert_eq!(
            toml::from_str::<Registry>(&toml::to_string(&registry).unwrap())
                .unwrap()
                .macs["Studio Mac"]
                .host,
            "dev@mac.local"
        );
    }

    #[test]
    fn global_remote_is_removed_before_forwarding() {
        let args = ["build", "--remote", "Studio Mac", "--scheme", "App"].map(str::to_string);
        let parsed = parse(&args).unwrap();
        assert_eq!(parsed.remote.as_deref(), Some("Studio Mac"));
        assert_eq!(parsed.forwarded, ["build", "--scheme", "App"]);
        assert_eq!(workspace(&parsed.forwarded), Workspace::Snapshot);
    }

    #[test]
    fn non_interactive_disables_ssh_prompts_without_persisting_it() {
        let args = ["doctor", "--non-interactive", "--remote", "Test Mac"].map(str::to_string);
        let parsed = parse(&args).unwrap();
        assert!(parsed.non_interactive);
        assert!(
            parsed
                .forwarded
                .iter()
                .any(|arg| arg == "--non-interactive")
        );
        let passthrough = ["build", "--", "--non-interactive"].map(str::to_string);
        assert!(!parse(&passthrough).unwrap().non_interactive);
        let registry: Registry = toml::from_str("[macs.test]\nhost = 'dev@mac'\n").unwrap();
        let mut mac = registry.resolve("test").unwrap();
        assert!(!mac.batch_mode);
        assert!(!mac.ssh().get_args().any(|arg| arg == "BatchMode=yes"));
        mac.batch_mode = parsed.non_interactive;
        assert!(mac.ssh().get_args().any(|arg| arg == "BatchMode=yes"));
        assert!(!toml::to_string(&mac).unwrap().contains("batch_mode"));
    }

    #[test]
    fn windows_output_modes_match_global_precedence() {
        let args = ["-o", "ndjson", "remote", "list", "--json"].map(str::to_string);
        let parsed = parse(&args).unwrap();
        assert!(parsed.ndjson);
        assert!(!parsed.json);
        let args = ["--json", "--output=quiet", "remote", "list"].map(str::to_string);
        let parsed = parse(&args).unwrap();
        assert!(!parsed.json && !parsed.ndjson);
        let args = ["-ojson", "doctor", "--remote", "mac"].map(str::to_string);
        assert!(parse(&args).unwrap().json);
        assert!(parse(&["--output=invalid".into()]).is_err());
        let args = [
            "add",
            "Test Mac",
            "dev@mac",
            "--replace",
            "--json",
            "-o",
            "json",
        ]
        .map(str::to_string);
        assert_eq!(
            registry_args(&args),
            ["add", "Test Mac", "dev@mac", "--replace"]
        );
    }

    #[test]
    fn host_and_shell_quote_reject_injection() {
        assert!(!valid_host("mac;touch /tmp/no"));
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn download_keeps_build_artifacts_but_upload_excludes_them() {
        assert!(is_excluded(Path::new("build/App.ipa"), false, false));
        assert!(!is_excluded(Path::new("build/App.ipa"), false, true));
        assert!(is_excluded(Path::new(".git/config"), false, true));
        for path in [
            "sweetpad-shots/video.mp4",
            "Reports/Test.xcresult/Data/data",
        ] {
            assert!(is_excluded(Path::new(path), true, false));
            assert!(!is_excluded(Path::new(path), false, true));
        }
    }

    #[test]
    fn manifest_detects_edits_additions_and_deletions_without_build_outputs() {
        let root = std::env::temp_dir().join(format!(
            "sweetpad-manifest-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::write(root.join("App.swift"), "first").unwrap();
        std::fs::write(root.join("build/App.ipa"), "generated").unwrap();
        let first = scan_project(&root).unwrap();
        assert!(!first.files.contains_key("build/App.ipa"));
        std::fs::remove_file(root.join("App.swift")).unwrap();
        std::fs::write(root.join("New.swift"), "second").unwrap();
        let second = scan_project(&root).unwrap();
        let (changed, deleted) = changed_paths(&first, &second);
        assert!(changed.contains(&"New.swift".to_string()));
        assert!(deleted.contains(&"App.swift".to_string()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_manifest_detects_equal_size_edits_with_restored_mtime() {
        let root =
            std::env::temp_dir().join(format!("sweetpad-preserved-time-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("source.swift");
        std::fs::write(&path, "first").unwrap();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        let before = scan_project(&root).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&path, "other").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(mtime))
            .unwrap();
        assert_eq!(
            changed_paths(&before, &scan_project(&root).unwrap()).0,
            ["source.swift"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replacing_result_bundles_removes_old_objects_but_keeps_local_edits() {
        let root = std::env::temp_dir().join(format!(
            "sweetpad-replace-result-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bundle = Path::new("reports/Test ü.xcresult");
        std::fs::create_dir_all(root.join(bundle).join("Data")).unwrap();
        let expected = bundle.join("Data/current");
        let obsolete = root.join(bundle).join("Data/obsolete");
        let edited = root.join(bundle).join("Data/local-edit");
        std::fs::write(root.join(&expected), "new result").unwrap();
        std::fs::write(&obsolete, "previous result").unwrap();
        std::fs::write(&edited, "user edit during remote test").unwrap();
        let started = SystemTime::now();
        File::options()
            .write(true)
            .open(&edited)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(started + Duration::from_secs(60)))
            .unwrap();
        let bundles = BTreeMap::from([(bundle.to_path_buf(), BTreeSet::from([expected.clone()]))]);
        prune_stale_results(&root, &bundles, started).unwrap();
        assert!(!obsolete.exists());
        assert_eq!(
            std::fs::read_to_string(root.join(expected)).unwrap(),
            "new result"
        );
        assert_eq!(
            std::fs::read_to_string(edited).unwrap(),
            "user edit during remote test"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn download_archive_contains_only_files_changed_after_the_marker() {
        let base = std::env::temp_dir().join(format!(
            "sweetpad-pull-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = base.join("project with spaces");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("old.swift"), "old").unwrap();
        let bundle = root.join("Reports/Test ü.xcresult");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(
            bundle.join("preserved-object"),
            "old object reused by Xcode",
        )
        .unwrap();
        let unchanged = root.join("Reports/Unchanged.xcresult");
        std::fs::create_dir_all(&unchanged).unwrap();
        std::fs::write(unchanged.join("untouched-object"), "unchanged result").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(base.join("project with spaces.pull-stamp"), "").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(root.join("new.swift"), "new").unwrap();
        std::fs::write(bundle.join("new-object"), "new result object").unwrap();
        let output = Command::new("sh")
            .arg("-c")
            .arg(
                download_command(
                    root.to_str().unwrap(),
                    base.join("project with spaces.pull-stamp")
                        .to_str()
                        .unwrap(),
                )
                .unwrap(),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut archive = tar::Archive::new(output.stdout.as_slice());
        let names: Vec<_> = archive
            .entries()
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(
            names.iter().any(|name| name.ends_with("new.swift")),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|name| name.ends_with("old.swift")),
            "{names:?}"
        );
        assert!(
            names.iter().any(|name| name.ends_with("preserved-object")),
            "{names:?}"
        );
        assert!(
            names.iter().any(|name| name.ends_with("new-object")),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|name| name.ends_with("untouched-object")),
            "{names:?}"
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}
