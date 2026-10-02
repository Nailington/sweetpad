//! Run the ordinary CLI on a Mac over SSH. Project commands sync the working
//! tree; host-only commands need no copy. The Mac owns Xcode and devices.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use crate::cli::{Cli, CliError, Resource, commands, output::Output, remote_registry};
use crate::portable_remote::{self, Mac as TransportMac};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Workspace {
    /// Commands that only touch the Mac's devices, tools, or user settings.
    None,
    /// Device commands with named input or output files, without a project.
    Files,
    /// Commands that read or create files in the current working directory.
    Snapshot,
    /// Sessions that can consume later edits from the local working tree.
    Live,
}

pub(crate) fn local_only(resource: Option<&Resource>) -> bool {
    matches!(
        resource,
        Some(
            Resource::Merge { .. }
                | Resource::Remote { .. }
                | Resource::Spm { .. }
                | Resource::Pbxproj { .. }
                | Resource::Project {
                    action: commands::project::Action::New(_)
                }
        )
    )
}

fn workspace(resource: Option<&Resource>) -> Workspace {
    match resource {
        Some(
            Resource::Run(args)
            | Resource::App {
                action: Some(commands::app::Action::Run(args)),
            },
        ) if args.detach || args.no_logs => Workspace::Snapshot,
        Some(
            Resource::Run(_)
            | Resource::App {
                action: None | Some(commands::app::Action::Run(_)),
            },
        ) => Workspace::Live,
        Some(Resource::Build {
            args,
            action: None | Some(commands::build::Action::Start),
        }) if args.watch => Workspace::Live,
        Some(Resource::Test {
            args,
            action: None | Some(commands::test::Action::Run),
        }) if args.watch => Workspace::Live,
        Some(Resource::Simulator { action }) => match action {
            commands::simulator::Action::Push { .. }
            | commands::simulator::Action::MediaAdd { .. }
            | commands::simulator::Action::Record { .. }
            | commands::simulator::Action::Screenshot { .. } => Workspace::Files,
            _ => Workspace::None,
        },
        Some(
            Resource::Device { .. }
            | Resource::Devices { .. }
            | Resource::Doctor
            | Resource::SelfUpdate
            | Resource::Help { .. }
            | Resource::Completions { .. }
            | Resource::Hot { .. }
            | Resource::Open {
                what: commands::open::What::Sim | commands::open::What::Config,
                ..
            },
        ) => Workspace::None,
        _ => Workspace::Snapshot,
    }
}

#[must_use]
pub fn run(cli: &Cli, argv: &[String], host: &str, out: &Output) -> ExitCode {
    match run_inner(cli, argv, host, out) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            super::render_early_error(out, &CliError::new(message));
            ExitCode::from(1)
        }
    }
}

#[allow(clippy::too_many_lines)] // SSH session setup and teardown share one error path
fn run_inner(cli: &Cli, argv: &[String], reference: &str, out: &Output) -> Result<u8, String> {
    let mac = remote_registry::resolve(reference)?;
    let host = mac.host.as_str();
    let workspace = workspace(cli.resource.as_ref());
    let cwd = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|e| format!("cannot locate the local project: {e}"))?;
    let root = if matches!(workspace, Workspace::Snapshot | Workspace::Live) {
        find_project_root(&cwd)
    } else {
        cwd.clone()
    };
    if matches!(workspace, Workspace::Snapshot | Workspace::Live)
        && (root == Path::new("/")
            || std::env::var_os("HOME").is_some_and(|home| root == Path::new(&home)))
    {
        return Err("refusing to sync the whole home or root directory; run from your project directory or pass -C".into());
    }
    let transport_mac = TransportMac {
        host: mac.host.clone(),
        identity_file: mac.identity_file.clone(),
        port: mac.port,
        batch_mode: cli.global.non_interactive,
    };
    let id = portable_remote::id(&root, &transport_mac);
    let remote_dir = portable_remote::remote_workspace(id, workspace == Workspace::Files);
    let forwarded = forward_args(argv, &cwd, &root)?;
    let invocation = portable_remote::remote_invocation(&forwarded);
    let remote_cwd = if cwd == root {
        remote_dir.clone()
    } else {
        format!(
            "{remote_dir}/{}",
            cwd.strip_prefix(&root).unwrap().display()
        )
    };
    let remote_command = if workspace == Workspace::None {
        invocation
    } else {
        format!("cd {} && {invocation}", shell_quote(&remote_cwd))
    };
    let started = SystemTime::now();

    let mut baseline = None;
    if workspace == Workspace::None {
        out.note(&format!("running on {host}"));
    } else {
        out.note(&format!("syncing {} to {host}", root.display()));
        match workspace {
            Workspace::Files => {
                let quoted = shell_quote(&remote_dir);
                let prep = format!(
                    "umask 077; rm -rf {quoted} && mkdir -p {quoted} && chmod 700 {quoted}"
                );
                if !portable_remote::ssh_status(&transport_mac, &prep)? {
                    return Err(format!("cannot prepare workspace on {host}"));
                }
                sync_inputs(cli.resource.as_ref(), &cwd, &transport_mac, &remote_dir)?;
            }
            Workspace::Snapshot | Workspace::Live => {
                portable_remote::sync_project(&root, &transport_mac, &remote_dir)?;
            }
            Workspace::None => unreachable!(),
        }
        baseline = Some(portable_remote::mark_download_baseline(
            &transport_mac,
            &remote_dir,
        )?);
    }

    // A run session can rebuild with `r`, and hot reload watches files on the
    // Mac. Keep sending local saves while that session is alive.
    let stop = Arc::new(AtomicBool::new(false));
    let worker = (workspace == Workspace::Live).then(|| {
        let stop = Arc::clone(&stop);
        let root = root.clone();
        let mac = transport_mac.clone();
        let remote_dir = remote_dir.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(2));
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(e) = portable_remote::sync_project(&root, &mac, &remote_dir) {
                    sync_warning(&e);
                }
            }
        })
    });

    let interactive = !out.is_json()
        && !out.is_ndjson()
        && unsafe {
            libc::isatty(libc::STDIN_FILENO) == 1 && libc::isatty(libc::STDOUT_FILENO) == 1
        };
    let result =
        portable_remote::run_session(&transport_mac, &remote_command, interactive).map(exit_status);
    stop.store(true, Ordering::Relaxed);
    if let Some(worker) = worker {
        let _ = worker.join();
    }
    let code = result?;
    if let Some(baseline) = &baseline
        && let Err(e) =
            portable_remote::receive_archive(&root, &transport_mac, &remote_dir, started, baseline)
    {
        if code == 0 {
            return Err(e);
        }
        sync_warning(&e);
    }
    Ok(code)
}

pub(crate) fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_@".contains(&b))
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
                    Some("xcworkspace" | "xcodeproj")
                )
            })
        }) {
            return dir.to_path_buf();
        }
    }
    cwd.to_path_buf()
}

fn sync_inputs(
    resource: Option<&Resource>,
    root: &Path,
    mac: &TransportMac,
    remote_dir: &str,
) -> Result<(), String> {
    let inputs: Vec<&Path> = match resource {
        Some(Resource::Simulator {
            action: commands::simulator::Action::Push { payload, .. },
        }) => vec![payload.as_path()],
        Some(Resource::Simulator {
            action: commands::simulator::Action::MediaAdd { paths, .. },
        }) => paths.iter().map(PathBuf::as_path).collect(),
        _ => Vec::new(),
    };
    let mut files = Vec::new();
    for input in inputs {
        let absolute = if input.is_absolute() {
            input.to_path_buf()
        } else {
            root.join(input)
        };
        let canonical = std::fs::canonicalize(&absolute)
            .map_err(|e| format!("cannot read input file {}: {e}", input.display()))?;
        if canonical.strip_prefix(root).is_err() {
            return Err(format!(
                "input file {} is outside the working directory; use -C to select its directory",
                input.display()
            ));
        }
        let relative = absolute.strip_prefix(root).map_err(|_| {
            format!(
                "input file {} is outside the working directory; use -C to select its directory",
                input.display()
            )
        })?;
        if relative
            .components()
            .any(|part| part == std::path::Component::ParentDir)
        {
            return Err(format!(
                "input path {} cannot contain `..`",
                input.display()
            ));
        }
        files.push(absolute);
    }
    portable_remote::send_inputs(root, mac, remote_dir, &files)
}

#[allow(clippy::print_stderr)]
fn sync_warning(message: &str) {
    eprintln!("sweetpad remote sync: {message}");
}

fn forward_args(argv: &[String], cwd: &Path, sync_root: &Path) -> Result<Vec<String>, String> {
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
    let mut result = Vec::new();
    let mut iter = argv.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            result.push(arg.clone());
            result.extend(iter.cloned());
            break;
        }
        if arg == "--remote" || arg == "-C" {
            iter.next().ok_or_else(|| format!("{arg} needs a value"))?;
            continue;
        }
        if arg.starts_with("--remote=") || (arg.starts_with("-C") && arg.len() > 2) {
            continue;
        }
        if PATH_FLAGS.contains(&arg.as_str()) {
            let path = iter.next().ok_or_else(|| format!("{arg} needs a path"))?;
            result.push(arg.clone());
            result.push(map_path(path, cwd, sync_root)?);
            continue;
        }
        if let Some((flag, path)) = arg.split_once('=')
            && PATH_FLAGS.contains(&flag)
        {
            result.push(format!("{flag}={}", map_path(path, cwd, sync_root)?));
            continue;
        }
        // Positional file inputs (e.g. `simulator push /path/payload.json`)
        // and project paths may be absolute. Translate those that live in the
        // copied tree; leave Mac-only absolute values such as `/Applications`
        // alone.
        if Path::new(arg).is_absolute() && Path::new(arg).starts_with(sync_root) {
            result.push(map_path(arg, cwd, sync_root)?);
        } else {
            result.push(arg.clone());
        }
    }
    Ok(result)
}

fn map_path(path: &str, cwd: &Path, sync_root: &Path) -> Result<String, String> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        let relative = path
            .strip_prefix(sync_root)
            .map_err(|_| format!("{} is outside the synced workspace", path.display()))?;
        let depth = cwd
            .strip_prefix(sync_root)
            .map_err(|_| "current directory is outside the synced workspace".to_string())?
            .components()
            .count();
        let mut mapped = PathBuf::new();
        for _ in 0..depth {
            mapped.push("..");
        }
        mapped.push(relative);
        Ok(mapped.to_string_lossy().into_owned())
    } else {
        Ok(path.to_string_lossy().into_owned())
    }
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn exit_status(status: std::process::ExitStatus) -> u8 {
    status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn profile(args: &[&str]) -> Workspace {
        let cli =
            Cli::try_parse_from(std::iter::once("sweetpad").chain(args.iter().copied())).unwrap();
        workspace(cli.resource.as_ref())
    }

    #[test]
    fn simulator_control_does_not_copy_a_project_but_file_commands_do() {
        assert_eq!(profile(&["simulator", "boot", "iPhone"]), Workspace::None);
        assert_eq!(profile(&["simulator", "open"]), Workspace::None);
        assert_eq!(
            profile(&["simulator", "location", "-33.8688", "-122.009"]),
            Workspace::None
        );
        assert_eq!(profile(&["simulator", "screenshot"]), Workspace::Files);
        assert_eq!(
            profile(&["simulator", "push", "com.example", "payload.json"]),
            Workspace::Files
        );
        assert_eq!(profile(&["doctor"]), Workspace::None);
        assert_eq!(profile(&["build"]), Workspace::Snapshot);
        assert_eq!(profile(&["build", "--watch"]), Workspace::Live);
        assert_eq!(profile(&["run"]), Workspace::Live);
        assert_eq!(profile(&["run", "--detach"]), Workspace::Snapshot);
    }

    #[test]
    fn local_git_commands_stay_on_the_client_checkout() {
        let cli = Cli::try_parse_from(["sweetpad", "merge", "install", "--remote", "mac"]).unwrap();
        assert!(local_only(cli.resource.as_ref()));
    }

    #[test]
    fn forwarding_keeps_the_passthrough_and_maps_local_paths() {
        let root = Path::new("/tmp/My App");
        let argv = [
            "-C",
            "/tmp/My App",
            "build",
            "--remote",
            "mac-mini",
            "--project=/tmp/My App/My App.xcodeproj",
            "--",
            "--remote",
            "literal",
        ];
        let forwarded = forward_args(&argv.map(str::to_string), root, root).unwrap();
        assert_eq!(
            forwarded,
            [
                "build",
                "--project=My App.xcodeproj",
                "--",
                "--remote",
                "literal"
            ]
        );
    }

    #[test]
    fn file_outputs_and_positional_inputs_are_mapped_into_the_copy() {
        let root = Path::new("/tmp/My App");
        let argv = [
            "simulator",
            "screenshot",
            "--output-file=/tmp/My App/shots/phone.png",
        ];
        assert_eq!(
            forward_args(&argv.map(str::to_string), root, root).unwrap(),
            ["simulator", "screenshot", "--output-file=shots/phone.png"]
        );
        let argv = [
            "simulator",
            "push",
            "com.example.App",
            "/tmp/My App/push.json",
        ];
        assert_eq!(
            forward_args(&argv.map(str::to_string), root, root).unwrap(),
            ["simulator", "push", "com.example.App", "push.json"]
        );
    }

    #[test]
    fn absolute_project_path_from_a_nested_directory_maps_to_parent() {
        let root = Path::new("/work/App");
        let cwd = root.join("Sources");
        let argv = ["build", "--project", "/work/App/App.xcodeproj"];
        assert_eq!(
            forward_args(&argv.map(str::to_string), &cwd, root).unwrap(),
            ["build", "--project", "../App.xcodeproj"]
        );
    }

    #[test]
    fn host_and_quoting_reject_shell_injection() {
        assert!(valid_host("dev@mac-mini.local"));
        assert!(!valid_host("mac;touch /tmp/pwn"));
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
