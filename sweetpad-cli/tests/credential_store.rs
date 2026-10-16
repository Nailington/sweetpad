//! Run with --ignored inside an isolated, unlocked Secret Service to exercise
//! native persistence and SSH/keychain transport without using real credentials.
#![cfg(unix)]
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output, Stdio};

#[test]
#[ignore = "requires an unlocked OS credential store; use an isolated Secret Service"]
fn cli_passwords_persist_and_unlock_in_the_build_session_without_disclosure() {
    let root =
        std::env::temp_dir().join(format!("sweetpad-credential-store-{}", std::process::id()));
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(root.join("project")).unwrap();
    fs::write(root.join("project/sweetpad.toml"), "").unwrap();
    let host = format!("dev@credential-test-{}", std::process::id());
    let secret = "login p@ss '$ ü";
    let separate = "keychain '$ ü";
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let cli = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_sweetpad"));
        c.env_remove("SWEETPAD_ASKPASS_ADDR")
            .env_remove("SWEETPAD_ASKPASS_TOKEN")
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("SWEETPAD_REMOTE_ROOT", root.join("remote"))
            .env("SWEETPAD_REMOTE_EXECUTABLE", root.join("receiver"))
            .env("TEST_ARGUMENTS", root.join("arguments"))
            .env("TEST_UNLOCK", root.join("unlocked"))
            .env("TEST_BUILD", root.join("built"))
            .env("TEST_LOGIN", secret)
            .env("TEST_KEYCHAIN", secret)
            .env("PATH", &path)
            .current_dir(root.join("project"));
        c
    };
    let checked = |output: Output| {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for bytes in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains(secret));
            assert!(!text.contains(separate));
        }
    };
    checked(
        cli()
            .args(["remote", "add", "test", &host, "--auth", "password"])
            .output()
            .unwrap(),
    );
    let save = |keychain: bool, password: &str| {
        let mut command = cli();
        command.args(["remote", "password", "test", "--stdin"]);
        if keychain {
            command.arg("--keychain");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "{password}").unwrap();
        checked(child.wait_with_output().unwrap());
    };
    save(false, secret);
    checked(
        cli()
            .args(["remote", "add", "alias", &host, "--auth", "password"])
            .output()
            .unwrap(),
    );
    // Removing one of two names must preserve their shared endpoint passwords.
    checked(cli().args(["remote", "remove", "alias"]).output().unwrap());
    fs::write(
        root.join("bin/ssh"),
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$TEST_ARGUMENTS"
answer=$("$SSH_ASKPASS" "$SWEETPAD_ASKPASS_HOST's password: ") || exit 10
test "$answer" = "$TEST_LOGIN" || exit 11
unset answer
for arg do command=$arg; done
exec sh -c "$command"
"#,
    )
    .unwrap();
    fs::set_permissions(root.join("bin/ssh"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        root.join("receiver"),
        r#"#!/usr/bin/env python3
import json,os,sys
from pathlib import Path
if sys.argv[1:] == ['__unlock-keychain']:
    actual=json.loads(sys.stdin.buffer.readline())
    if actual != os.environ['TEST_KEYCHAIN']:
        print('keychain unlock rejected',file=sys.stderr)
        sys.exit(78)
    Path(os.environ['TEST_UNLOCK']).write_text(str(os.getppid()))
    sys.exit(0)
# The build execs the shell whose child unlocked the keychain.
assert Path(os.environ['TEST_UNLOCK']).read_text() == str(os.getpid())
Path(os.environ['TEST_BUILD']).write_text('built')
print('{"ok":true,"data":{"built":true}}')
"#,
    )
    .unwrap();
    fs::set_permissions(root.join("receiver"), fs::Permissions::from_mode(0o755)).unwrap();
    let build = |expected: &str| {
        cli()
            .args([
                "--non-interactive",
                "-o",
                "json",
                "build",
                "--remote",
                "test",
            ])
            .env("TEST_KEYCHAIN", expected)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    checked(build(secret)); // Saved SSH password is the keychain default.
    save(true, separate);
    checked(build(separate)); // A separate saved credential takes precedence.
    fs::remove_file(root.join("built")).unwrap();
    let failure = build("different keychain password");
    assert!(!failure.status.success());
    assert!(!root.join("built").exists());
    assert!(!String::from_utf8_lossy(&failure.stderr).contains(separate));
    checked(
        cli()
            .args(["remote", "password", "test", "--keychain", "--forget"])
            .output()
            .unwrap(),
    );
    checked(build(secret));
    let arguments = fs::read_to_string(root.join("arguments")).unwrap();
    assert!(!arguments.contains(secret));
    assert!(!arguments.contains(separate));
    assert!(!arguments.contains("security unlock-keychain -p"));
    checked(
        cli()
            .args(["remote", "password", "test", "--forget"])
            .output()
            .unwrap(),
    );
    let failure = build(secret);
    assert!(!failure.status.success()); // Noninteractive SSH does not prompt after forgetting.
    fs::remove_dir_all(root).unwrap();
}
