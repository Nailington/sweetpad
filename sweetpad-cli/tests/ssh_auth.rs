//! Exercise the native askpass executable used by Unix and Windows OpenSSH.
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;
use std::time::Duration;

fn helper(answer: serde_json::Value, confirm: bool) -> (std::process::Output, serde_json::Value) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        writeln!(stream, "{}", serde_json::json!({ "answer": answer })).unwrap();
        serde_json::from_str::<serde_json::Value>(&line).unwrap()
    });
    let output = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .arg("dev@mac.local's password: ")
        .env("SWEETPAD_ASKPASS_HELPER", "1")
        .env("SWEETPAD_ASKPASS_ADDR", address.to_string())
        .env("SWEETPAD_ASKPASS_TOKEN", "test-token")
        .env("SWEETPAD_ASKPASS_HOST", "dev@mac.local")
        .env("SWEETPAD_ASKPASS_PORT", "2222")
        .env("SWEETPAD_ASKPASS_CONNECTION", "123-1")
        .env("SSH_ASKPASS_PROMPT", if confirm { "confirm" } else { "" })
        .output()
        .unwrap();
    (output, server.join().unwrap())
}

#[test]
fn askpass_transports_unicode_and_shell_characters_without_interpolation() {
    let secret = "p@ss ' $ & ü";
    let (output, request) = helper(serde_json::json!(secret), false);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{secret}\n")
    );
    assert!(output.stderr.is_empty());
    assert_eq!(request["host"], "dev@mac.local");
    assert_eq!(request["port"], "2222");
    assert_eq!(request["token"], "test-token");
    assert_eq!(request["connection"], "123-1");
    assert_eq!(request["confirm"], false);
    assert!(!request.to_string().contains(secret));
}

#[test]
fn askpass_fails_quietly_on_cancel_or_multiline_answers() {
    for answer in [
        serde_json::Value::Null,
        serde_json::json!("password\nextra"),
    ] {
        let (output, _) = helper(answer, false);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn askpass_passes_host_confirmation_separately() {
    let (output, request) = helper(serde_json::json!("yes"), true);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"yes\n");
    assert_eq!(request["confirm"], true);
}

#[test]
fn askpass_rejects_non_loopback_brokers() {
    let output = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .arg("Password: ")
        .env("SWEETPAD_ASKPASS_HELPER", "1")
        .env("SWEETPAD_ASKPASS_ADDR", "192.0.2.1:1234")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn saved_password_auth_contains_preferences_but_no_credentials() {
    let root = std::env::temp_dir().join(format!("sweetpad-auth-registry-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_sweetpad"))
            .args(args)
            .env("XDG_CONFIG_HOME", &root)
            .env("APPDATA", &root)
            .output()
            .unwrap()
    };
    let output = cli(&[
        "-o",
        "json",
        "remote",
        "add",
        "Password Mac",
        "dev@mac.local",
        "--auth",
        "password",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["data"]["macs"][0]["auth"], "password");
    let registry = std::fs::read_to_string(root.join("sweetpad/remotes.toml")).unwrap();
    assert!(registry.contains("auth = \"password\""));
    assert!(!registry.contains("token"));
    assert!(!registry.contains("secret"));
    assert!(
        cli(&["-o", "json", "remote", "show", "Password Mac"])
            .status
            .success()
    );
    assert!(
        !cli(&[
            "remote",
            "add",
            "Invalid Mac",
            "dev@mac.local",
            "--auth",
            "invalid"
        ])
        .status
        .success()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn remote_commands_use_the_native_helper_with_password_preferences() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!(
        "sweetpad password transport {}",
        std::process::id()
    ));
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::create_dir_all(root.join("config/sweetpad")).unwrap();
    std::fs::write(
        root.join("config/sweetpad/remotes.toml"),
        "[macs.test]\nhost = \"dev@mac.local\"\nport = 2222\nauth = \"password\"\n",
    )
    .unwrap();
    let ssh = root.join("bin/ssh");
    std::fs::write(
        &ssh,
        r#"#!/bin/sh
printf '%s\n' "$@" > "$TEST_ARGUMENTS"
test "$SSH_ASKPASS_REQUIRE" = force || exit 1
answer=$("$SSH_ASKPASS" "dev@mac.local's password: ") || exit 1
test "$answer" = "p@ss word" || exit 1
printf '%s\n' '{"ok":true,"data":{"devices":[]}}'
"#,
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cli = root.join("bin/sweetpad with spaces");
    std::fs::hard_link(env!("CARGO_BIN_EXE_sweetpad"), &cli).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = String::new();
        BufReader::new(&stream).read_line(&mut request).unwrap();
        writeln!(stream, "{}", serde_json::json!({ "answer": "p@ss word" })).unwrap();
        serde_json::from_str::<serde_json::Value>(&request).unwrap()
    });
    let output = Command::new(cli)
        .args([
            "-o",
            "json",
            "--non-interactive",
            "devices",
            "--remote",
            "test",
        ])
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("TEST_ARGUMENTS", root.join("arguments"))
        .env("SWEETPAD_ASKPASS_ADDR", address.to_string())
        .env("SWEETPAD_ASKPASS_TOKEN", "integration-token")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = server.join().unwrap();
    assert_eq!(request["host"], "dev@mac.local");
    assert_eq!(request["port"], "2222");
    assert_eq!(request["token"], "integration-token");
    let arguments = std::fs::read_to_string(root.join("arguments")).unwrap();
    assert!(arguments.contains("PreferredAuthentications=keyboard-interactive,password"));
    assert!(arguments.contains("PubkeyAuthentication=no"));
    assert!(!arguments.lines().any(|arg| arg == "BatchMode=yes"));
    assert!(!arguments.contains("p@ss word"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("p@ss word"));
    std::fs::remove_dir_all(root).unwrap();
}
