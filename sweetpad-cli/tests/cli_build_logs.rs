//! Extract a saved signing failure without invoking xcodebuild or rebuilding.
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use common::TempDir;
use serde_json::{Value, json};

const SIGNING: &str = "/tmp/App.app/App.debug.dylib: errSecInternalComponent\nCommand CodeSign failed with a nonzero exit code\n";

struct Fixture {
    root: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = TempDir::new("sweetpad-build-logs");
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("saved result ü.xcresult")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(
            root.join("log.json"),
            json!({
                "title": "Build App", "subsections": [{
                    "commandInvocationDetails": {
                        "commandDetails": "CodeSign /tmp/App.app/App.debug.dylib\n",
                        "emittedOutput": SIGNING, "exitCode": 1,
                    }
                }],
            })
            .to_string(),
        )
        .unwrap();
        fs::write(root.join("bin/xcrun"), "#!/bin/sh\n[ \"$1 $2 $3\" = 'xcresulttool get log' ] || exit 99\nprintf '%s\\n' \"$@\" > \"$INVOCATION\"\ncat \"$LOG_FIXTURE\"\n").unwrap();
        fs::set_permissions(root.join("bin/xcrun"), fs::Permissions::from_mode(0o755)).unwrap();
        // A log read must never try to compile or resolve build settings.
        fs::write(root.join("bin/xcodebuild"), "#!/bin/sh\nexit 99\n").unwrap();
        fs::set_permissions(
            root.join("bin/xcodebuild"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        Self { root }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_sweetpad"));
        cmd.current_dir(&self.root)
            .args(["--non-interactive", "build", "logs"])
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("LOG_FIXTURE", self.root.join("log.json"))
            .env("INVOCATION", self.root.join("invocation"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        cmd
    }

    fn explicit(&self) -> Command {
        let mut cmd = self.command();
        cmd.args(["--result-bundle", "saved result ü.xcresult"]);
        cmd
    }
}

#[test]
fn extracts_text_json_and_a_saved_file_without_building() {
    let fixture = Fixture::new();
    let text = fixture.explicit().output().unwrap();
    assert!(text.status.success(), "{text:?}");
    assert!(String::from_utf8(text.stdout).unwrap().contains(SIGNING));
    assert_eq!(
        fs::read_to_string(fixture.root.join("invocation")).unwrap(),
        "xcresulttool\nget\nlog\n--path\nsaved result ü.xcresult\n--type\nbuild\n"
    );

    let json = fixture.explicit().arg("--json").output().unwrap();
    assert!(json.status.success(), "{json:?}");
    let envelope: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(envelope["ok"], true);
    assert!(envelope["data"]["text"].as_str().unwrap().contains(SIGNING));
    assert_eq!(
        envelope["data"]["log"]["subsections"][0]["commandInvocationDetails"]["exitCode"],
        1
    );

    let saved = fixture
        .explicit()
        .args(["--output-file", "logs/failed build.log"])
        .output()
        .unwrap();
    assert!(saved.status.success(), "{saved:?}");
    assert_eq!(
        fs::read_to_string(fixture.root.join("logs/failed build.log")).unwrap(),
        envelope["data"]["text"].as_str().unwrap()
    );
}

#[test]
fn reads_the_projects_retained_result_without_a_scheme_or_destination() {
    let fixture = Fixture::new();
    let project = fixture.root.join("App.xcodeproj");
    fs::create_dir_all(&project).unwrap();
    let hash = fs::canonicalize(&project)
        .unwrap()
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    let bundle = fixture.root.join(format!(
        "state/sweetpad/results/App-{hash:016x}-build.xcresult"
    ));
    fs::create_dir_all(&bundle).unwrap();
    let output = fixture
        .command()
        .args(["--project", "App.xcodeproj"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8(output.stdout).unwrap().contains(SIGNING));
}

#[test]
fn missing_bundle_and_bad_log_are_actionable_errors() {
    let fixture = Fixture::new();
    let missing = fixture
        .command()
        .args(["--result-bundle", "missing.xcresult", "--json"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let envelope: Value = serde_json::from_slice(&missing.stderr).unwrap();
    assert!(
        envelope["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no saved Xcode build result")
    );
    assert!(!fixture.root.join("invocation").exists());

    fs::write(fixture.root.join("log.json"), "{}").unwrap();
    let bad = fixture.explicit().arg("--json").output().unwrap();
    assert!(!bad.status.success());
    let envelope: Value = serde_json::from_slice(&bad.stderr).unwrap();
    assert!(
        envelope["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unrecognized build log")
    );
}

#[test]
fn readback_refuses_build_flags_on_both_sides_of_logs() {
    let fixture = Fixture::new();
    for args in [
        vec!["build", "--clean", "logs"],
        vec!["build", "logs", "--scheme", "App"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
            .current_dir(&fixture.root)
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        let envelope: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert!(
            envelope["error"]["message"]
                .as_str()
                .unwrap()
                .contains("builds nothing")
        );
    }
}

#[test]
fn remote_extraction_downloads_the_log_to_the_local_project() {
    let fixture = Fixture::new();
    let project = fixture.root.join("project");
    fs::create_dir_all(project.join("App.xcodeproj")).unwrap();
    // Exercise the actual sync, forwarding, extraction and download protocol,
    // with SSH executing locally and xcrun returning a saved signing failure.
    fs::write(
        fixture.root.join("bin/ssh"),
        "#!/bin/sh\nfor arg do command=$arg; done\nexec sh -c \"$command\"\n",
    )
    .unwrap();
    fs::set_permissions(
        fixture.root.join("bin/ssh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let receiver = fixture.root.join("receiver");
    fs::write(
        &receiver,
        format!(
            "#!/bin/sh\nmkdir -p saved.xcresult\nexec '{}' \"$@\"\n",
            env!("CARGO_BIN_EXE_sweetpad")
        ),
    )
    .unwrap();
    fs::set_permissions(&receiver, fs::Permissions::from_mode(0o755)).unwrap();
    let output = fixture
        .command()
        .current_dir(&project)
        .args([
            "--remote",
            "test-mac",
            "--result-bundle",
            "saved.xcresult",
            "--output-file",
            "logs/build.log",
        ])
        .env("SWEETPAD_REMOTE_ROOT", fixture.root.join("remote"))
        .env("SWEETPAD_REMOTE_EXECUTABLE", &receiver)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        fs::read_to_string(project.join("logs/build.log"))
            .unwrap()
            .contains(SIGNING)
    );
}
