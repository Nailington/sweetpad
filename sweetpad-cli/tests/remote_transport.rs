//! Exercise cancellation through the real client process and SSH shell protocol.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn remote_interrupt_finalizes_and_downloads_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "sweetpad-remote-cancel-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(root.join("project")).unwrap();
    fs::write(root.join("project/movie.mp4"), "previous recording").unwrap();
    let ssh = root.join("bin/ssh");
    // Execute the SSH command locally, preserving its ordinary shell semantics.
    fs::write(
        &ssh,
        "#!/bin/sh\nfor arg do command=$arg; done\nexec sh -c \"$command\"\n",
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    let receiver = root.join("receiver");
    let ready = root.join("ready");
    fs::write(&receiver, format!(
        "#!/bin/sh\ntrap 'printf finalized > movie.mp4; touch -t 200001010000 movie.mp4; exit 0' INT\nprintf ready > '{}'\nwhile :; do sleep 0.1; done\n", ready.display()
    )).unwrap();
    fs::set_permissions(&receiver, fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args([
            "-o",
            "json",
            "simulator",
            "record",
            "--output-file",
            "movie.mp4",
            "--remote",
            "test-mac",
        ])
        .current_dir(root.join("project"))
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("SWEETPAD_REMOTE_ROOT", root.join("remote"))
        .env("SWEETPAD_REMOTE_EXECUTABLE", &receiver)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if !ready.exists() {
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        panic!(
            "receiver did not start: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // Signal only the CLI; it must keep its SSH stream alive to drain the result.
    unsafe {
        libc::kill(child.id().cast_signed(), libc::SIGINT);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if child.try_wait().unwrap().is_none() {
        let _ = child.kill();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(root.join("project/movie.mp4")).unwrap(),
        b"finalized"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn overlapping_file_commands_keep_both_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "sweetpad-remote-overlap-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(root.join("project")).unwrap();
    fs::write(
        root.join("bin/ssh"),
        "#!/bin/sh\nfor arg do command=$arg; done\nexec sh -c \"$command\"\n",
    )
    .unwrap();
    fs::set_permissions(root.join("bin/ssh"), fs::Permissions::from_mode(0o755)).unwrap();
    let receiver = root.join("receiver");
    fs::write(&receiver, format!("#!/bin/sh\nwhile [ $# -gt 0 ]; do if [ \"$1\" = --output-file ]; then shift; output=$1; fi; shift; done\ntouch '{}/ready-'\"$output\"\nsleep 0.5\nprintf '%s' \"$output\" > \"$output\"\n", root.display())).unwrap();
    fs::set_permissions(&receiver, fs::Permissions::from_mode(0o755)).unwrap();
    let start = |output: &str| {
        Command::new(env!("CARGO_BIN_EXE_sweetpad"))
            .args([
                "-o",
                "json",
                "simulator",
                "screenshot",
                "--output-file",
                output,
                "--remote",
                "test-mac",
            ])
            .current_dir(root.join("project"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    root.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("SWEETPAD_REMOTE_ROOT", root.join("remote"))
            .env("SWEETPAD_REMOTE_EXECUTABLE", &receiver)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let mut first = start("first.png");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.join("ready-first.png").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !root.join("ready-first.png").exists() {
        let _ = first.kill();
        let output = first.wait_with_output().unwrap();
        panic!(
            "first command did not start: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let second = start("second.png");
    for child in [first, second] {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(
        fs::read(root.join("project/first.png")).unwrap(),
        b"first.png"
    );
    assert_eq!(
        fs::read(root.join("project/second.png")).unwrap(),
        b"second.png"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn simulator_push_does_not_pollute_json_with_tool_output() {
    let root = std::env::temp_dir().join(format!(
        "sweetpad-push-json-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("bin")).unwrap();
    let xcrun = root.join("bin/xcrun");
    fs::write(&xcrun, r#"#!/bin/sh
if [ "$2" = list ]; then
  printf '%s\n' '{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-26-0":[{"udid":"TEST-UDID","name":"Test Phone","state":"Booted","isAvailable":true}]}}'
else
  printf '%s\n' 'Notification sent to app'
fi
"#).unwrap();
    fs::set_permissions(&xcrun, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(root.join("push.json"), "{}").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args([
            "--json",
            "simulator",
            "push",
            "com.example",
            "push.json",
            "TEST-UDID",
        ])
        .current_dir(&root)
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["udid"], "TEST-UDID");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn simultaneous_first_project_commands_initialize_the_workspace_once() {
    let root = std::env::temp_dir().join(format!(
        "sweetpad-sync-race-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(root.join("project/App.xcodeproj")).unwrap();
    fs::write(root.join("project/file.swift"), "source").unwrap();
    fs::write(root.join("bin/ssh"), format!("#!/bin/sh\nfor arg do command=$arg; done\ncase \"$command\" in *'rm -rf'*) printf 'reset\\n' >> '{}/resets'; sleep 0.2;; esac\nexec sh -c \"$command\"\n", root.display())).unwrap();
    fs::set_permissions(root.join("bin/ssh"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(root.join("receiver"), "#!/bin/sh\nprintf '{}\\n'\n").unwrap();
    fs::set_permissions(root.join("receiver"), fs::Permissions::from_mode(0o755)).unwrap();
    let start = || {
        Command::new(env!("CARGO_BIN_EXE_sweetpad"))
            .args(["-o", "json", "scheme", "list", "--remote", "test-mac"])
            .current_dir(root.join("project"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    root.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("SWEETPAD_REMOTE_ROOT", root.join("remote"))
            .env("SWEETPAD_REMOTE_EXECUTABLE", root.join("receiver"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let first = start();
    let second = start();
    for child in [first, second] {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(
        fs::read_to_string(root.join("resets"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn foreground_group_interrupt_keeps_transport_open_for_finalization() {
    use std::os::fd::FromRawFd;
    use std::os::unix::process::CommandExt;

    let root = std::env::temp_dir().join(format!("sweetpad-foreground-{}", std::process::id()));
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(root.join("project")).unwrap();
    let ssh = root.join("bin/ssh");
    fs::write(
        &ssh,
        "#!/bin/sh\nfor arg do command=$arg; done\nexec sh -c \"$command\"\n",
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    let receiver = root.join("receiver");
    let ready = root.join("ready");
    fs::write(&receiver, format!(
        "#!/bin/sh\ntrap 'printf finalized > movie.mp4; exit 0' INT\nprintf ready > '{}'\nwhile :; do sleep 0.1; done\n", ready.display()
    )).unwrap();
    fs::set_permissions(&receiver, fs::Permissions::from_mode(0o755)).unwrap();
    let mut master = -1;
    let mut slave = -1;
    // Give the actual CLI terminal stdio so it uses the foreground SSH path.
    assert_eq!(
        unsafe {
            libc::openpty(
                &raw mut master,
                &raw mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    let master = unsafe { fs::File::from_raw_fd(master) };
    let slave = unsafe { fs::File::from_raw_fd(slave) };
    let mut child = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args([
            "simulator",
            "record",
            "--output-file",
            "movie.mp4",
            "--remote",
            "test-mac",
        ])
        .current_dir(root.join("project"))
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("SWEETPAD_REMOTE_ROOT", root.join("remote"))
        .env("SWEETPAD_REMOTE_EXECUTABLE", &receiver)
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave.try_clone().unwrap())
        .process_group(0)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(ready.exists(), "remote recorder did not start");
    assert_eq!(
        unsafe { libc::kill(-i32::try_from(child.id()).unwrap(), libc::SIGINT) },
        0
    );
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("foreground cancellation did not finalize");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "foreground interrupt returned {status}");
    assert_eq!(
        fs::read_to_string(root.join("project/movie.mp4")).unwrap(),
        "finalized"
    );
    drop(slave);
    drop(master);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn commands_and_protocol_bridges_share_the_same_workspace() {
    let root = std::env::temp_dir().join(format!("sweetpad-workspace-{}", std::process::id()));
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(root.join("project")).unwrap();
    fs::write(root.join("project/Package.swift"), "// workspace fixture\n").unwrap();
    let ssh = root.join("bin/ssh");
    fs::write(
        &ssh,
        "#!/bin/sh\nfor arg do command=$arg; done\nexec sh -c \"$command\"\n",
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    let receiver = root.join("receiver");
    fs::write(
        &receiver,
        "#!/bin/sh\nprintf '{\"data\":{\"remoteRoot\":\"%s\"},\"ok\":true}\\n' \"$PWD\"\n",
    )
    .unwrap();
    fs::set_permissions(&receiver, fs::Permissions::from_mode(0o755)).unwrap();
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
            .args(["--json", "--remote", "test-mac"])
            .args(args)
            .current_dir(root.join("project"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    root.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("SWEETPAD_REMOTE_ROOT", root.join("remote"))
            .env("SWEETPAD_REMOTE_EXECUTABLE", &receiver)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"]["remoteRoot"]
            .clone()
    };
    assert_eq!(run(&["scheme", "list"]), run(&["bridge", "context"]));
    fs::remove_dir_all(root).unwrap();
}
