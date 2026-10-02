//! Real subprocess regression for concurrent Windows registry updates.
#![cfg(windows)]

use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn simultaneous_additions_are_not_lost_and_replacements_remain_readable() {
    let root = std::env::temp_dir().join(format!(
        "sweetpad-registry-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut children = Vec::new();
    for index in 0..16 {
        children.push(
            Command::new(env!("CARGO_BIN_EXE_sweetpad"))
                .args([
                    "remote",
                    "add",
                    &format!("Concurrent {index}"),
                    "dev@fixture.local",
                    "--json",
                ])
                .env("APPDATA", &root)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(["remote", "list", "--json"])
        .env("APPDATA", &root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["data"]["macs"].as_array().unwrap().len(), 16);
    for _ in 0..4 {
        let status = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
            .args([
                "remote",
                "add",
                "Concurrent 0",
                "dev@replacement.local",
                "--replace",
                "--json",
            ])
            .env("APPDATA", &root)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let text = std::fs::read_to_string(root.join("sweetpad/remotes.toml")).unwrap();
        let registry: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(registry["macs"].as_table().unwrap().len(), 16);
        assert_eq!(
            registry["macs"]["Concurrent 0"]["host"].as_str(),
            Some("dev@replacement.local")
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
