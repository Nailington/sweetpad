//! Thin wrapper over `xcrun devicectl` — listing and driving physical devices.
//! Shared by the `device` command and the `app … --device` path. `devicectl`
//! writes its listing to a `--json-output` file rather than stdout, so [`list`]
//! routes through a temp file.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::cli::{CliError, ErrorContext, process};

#[derive(Debug, Deserialize)]
struct ListOutput {
    result: ListResult,
}

#[derive(Debug, Deserialize)]
struct ListResult {
    #[serde(default)]
    devices: Vec<RawDevice>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDevice {
    #[serde(default)]
    connection_properties: ConnectionProperties,
    #[serde(default)]
    device_properties: DeviceProperties,
    #[serde(default)]
    hardware_properties: HardwareProperties,
    #[serde(default)]
    identifier: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionProperties {
    #[serde(default)]
    tunnel_state: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceProperties {
    #[serde(default)]
    name: String,
    #[serde(default)]
    os_version_number: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HardwareProperties {
    #[serde(default)]
    udid: String,
    #[serde(default)]
    marketing_name: String,
    #[serde(default)]
    platform: String,
}

/// A connected physical device.
#[derive(Debug, Clone)]
pub struct Device {
    pub udid: String,
    pub name: String,
    pub model: String,
    pub platform: String,
    pub os_version: String,
    pub connection: String,
}

impl Device {
    /// `"My iPhone (iPhone 15 Pro, iOS 17.0)"`.
    #[must_use]
    pub fn label(&self) -> String {
        format!(
            "{} ({}, {} {})",
            self.name, self.model, self.platform, self.os_version
        )
    }
}

/// Enumerate connected physical devices.
pub fn list() -> Result<Vec<Device>, CliError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp: PathBuf = std::env::temp_dir().join(format!(
        "sweetpad-devices-{}-{nanos}.json",
        std::process::id()
    ));

    let ok = process::run(
        "xcrun",
        &[
            "devicectl",
            "list",
            "devices",
            "--json-output",
            &tmp.to_string_lossy(),
            "--timeout",
            "10",
        ],
        None,
        true,
    )?;
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(CliError::new("`xcrun devicectl list devices` failed"));
    }

    let raw = std::fs::read_to_string(&tmp)
        .map_err(|e| CliError::new(format!("reading devicectl output: {e}")))?;
    let _ = std::fs::remove_file(&tmp);

    parse_devices(&raw)
}

/// Parse `devicectl list devices` JSON into sorted devices. Split out from
/// [`list`] so it's testable without `devicectl`. Devices missing a UDID
/// (devicectl returns empty hardwareProperties for some USB iOS ≤16 devices)
/// fall back to their `identifier`, and are dropped only if both are empty.
fn parse_devices(raw: &str) -> Result<Vec<Device>, CliError> {
    let parsed: ListOutput = serde_json::from_str(raw)
        .map_err(|e| CliError::new(format!("parsing devicectl output: {e}")))?;

    let mut devices: Vec<Device> = parsed
        .result
        .devices
        .into_iter()
        .filter_map(|d| {
            let udid = if d.hardware_properties.udid.is_empty() {
                d.identifier
            } else {
                d.hardware_properties.udid
            };
            if udid.is_empty() {
                return None;
            }
            Some(Device {
                udid,
                name: d.device_properties.name,
                model: d.hardware_properties.marketing_name,
                platform: if d.hardware_properties.platform.is_empty() {
                    "iOS".to_string()
                } else {
                    d.hardware_properties.platform
                },
                os_version: d.device_properties.os_version_number,
                connection: d.connection_properties.tunnel_state,
            })
        })
        .collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(devices)
}

/// Find a device by UDID (case-insensitive) or exact name.
#[must_use]
pub fn find<'a>(devices: &'a [Device], query: &str) -> Option<&'a Device> {
    devices
        .iter()
        .find(|d| d.udid.eq_ignore_ascii_case(query))
        .or_else(|| devices.iter().find(|d| d.name == query))
}

/// Install an `.app` bundle onto a device. Captures stdout (stderr stays
/// visible): `devicectl` prints install progress there, which is noise under
/// the caller's step line — and in `--json` mode it would interleave with the
/// envelope on the same stream and break the consumer's parse.
pub fn install(device_id: &str, app_path: &str) -> Result<(), CliError> {
    process::capture(
        "xcrun",
        &[
            "devicectl",
            "device",
            "install",
            "app",
            "--device",
            device_id,
            app_path,
        ],
        None,
    )
    .map(|_| ())
    .context("installing the app on the device")
}

/// Launch an installed app on a device, terminating any existing instance.
pub fn launch(
    device_id: &str,
    bundle_id: &str,
    args: &[String],
    env: &[(String, String)],
    wait_for_debugger: bool,
) -> Result<String, CliError> {
    let cmd_args = launch_args(device_id, bundle_id, args, wait_for_debugger, false);
    // devicectl forwards `DEVICECTL_CHILD_*` from its own environment to the
    // app, the same shape simctl uses for `SIMCTL_CHILD_*`.
    process::capture_env("xcrun", &cmd_args, None, env).context("launching the app on the device")
}

/// Build one launch command for both detached and attached console paths. The
/// separator keeps an app argument such as `--console` from becoming a
/// devicectl option instead.
fn launch_args<'a>(
    device_id: &'a str,
    bundle_id: &'a str,
    args: &'a [String],
    wait_for_debugger: bool,
    console: bool,
) -> Vec<&'a str> {
    let mut cmd_args = vec![
        "devicectl",
        "device",
        "process",
        "launch",
        "--terminate-existing",
    ];
    if console {
        cmd_args.push("--console");
    }
    if wait_for_debugger {
        cmd_args.push("--start-stopped");
    }
    cmd_args.extend_from_slice(&["--device", device_id, bundle_id]);
    if !args.is_empty() {
        cmd_args.push("--");
        cmd_args.extend(args.iter().map(String::as_str));
    }
    cmd_args
}

/// Launch with the console attached, streaming the app's stdout/stderr and
/// os_log output to the terminal until it exits (Xcode 16+). This is how device
/// log following works — `devicectl` has no attach-to-running-process console.
pub fn launch_console(
    device_id: &str,
    bundle_id: &str,
    args: &[String],
    env: &[(String, String)],
    wait_for_debugger: bool,
) -> Result<(), CliError> {
    let cmd_args = launch_args(device_id, bundle_id, args, wait_for_debugger, true);
    process::stream_env("xcrun", &cmd_args, None, env)
}

/// Like [`launch_console`] but spawned in the background with stdout/stderr piped,
/// handing back the child so the interactive `app run` session can render the device
/// console (the app's own output) while watching for the rebuild key.
pub fn spawn_console(
    device_id: &str,
    bundle_id: &str,
    args: &[String],
    env: &[(String, String)],
    wait_for_debugger: bool,
) -> Result<std::process::Child, CliError> {
    let cmd_args = launch_args(device_id, bundle_id, args, wait_for_debugger, true);
    process::spawn_piped_both_env("xcrun", &cmd_args, None, env)
}

/// Uninstall an app from a device. Stdout is captured for the same reason as
/// [`install`] — keep `devicectl`'s progress chatter off the stream the
/// `--json` envelope goes to.
pub fn uninstall(device_id: &str, bundle_id: &str) -> Result<(), CliError> {
    process::capture(
        "xcrun",
        &[
            "devicectl",
            "device",
            "uninstall",
            "app",
            "--device",
            device_id,
            bundle_id,
        ],
        None,
    )
    .map(|_| ())
    .context("uninstalling the app from the device")
}

/// Terminate a running app on a device. `devicectl` has no
/// terminate-by-bundle-id — processes are addressed by pid — so the app's
/// pid(s) are looked up in the device's process list by the `.app` directory
/// name in the executable path, then signalled. Nothing running is success,
/// mirroring `simctl terminate`'s idempotence.
pub fn terminate(device_id: &str, app_dir_name: &str) -> Result<(), CliError> {
    if app_dir_name.is_empty() {
        return Err(CliError::new(
            "cannot terminate: the app bundle's name is unknown",
        ));
    }
    for pid in app_pids(device_id, app_dir_name)? {
        // Best-effort per pid: the process may have exited between the list
        // and the signal, which devicectl reports as a failure.
        let _ = process::run(
            "xcrun",
            &[
                "devicectl",
                "device",
                "process",
                "signal",
                "--signal",
                "15",
                "--pid",
                &pid.to_string(),
                "--device",
                device_id,
            ],
            None,
            true,
        );
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ProcessesOutput {
    result: ProcessesResult,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessesResult {
    #[serde(default)]
    running_processes: Vec<RawProcess>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProcess {
    #[serde(default)]
    process_identifier: i64,
    #[serde(default)]
    executable: String,
}

/// Pids of running processes whose executable lives inside the named `.app`
/// directory. `devicectl device info processes` routes through a
/// `--json-output` temp file like [`list`].
fn app_pids(device_id: &str, app_dir_name: &str) -> Result<Vec<i64>, CliError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp: PathBuf = std::env::temp_dir().join(format!(
        "sweetpad-processes-{}-{nanos}.json",
        std::process::id()
    ));
    let ok = process::run(
        "xcrun",
        &[
            "devicectl",
            "device",
            "info",
            "processes",
            "--device",
            device_id,
            "--json-output",
            &tmp.to_string_lossy(),
        ],
        None,
        true,
    )?;
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(CliError::new(
            "`xcrun devicectl device info processes` failed",
        ));
    }
    let raw = std::fs::read_to_string(&tmp)
        .map_err(|e| CliError::new(format!("reading devicectl output: {e}")))?;
    let _ = std::fs::remove_file(&tmp);
    parse_app_pids(&raw, app_dir_name)
}

/// Parse the process list, keeping pids whose executable path contains the
/// `.app` directory (executables are reported as `file://` URLs, e.g.
/// `file:///private/var/.../My.app/My`).
fn parse_app_pids(raw: &str, app_dir_name: &str) -> Result<Vec<i64>, CliError> {
    let parsed: ProcessesOutput = serde_json::from_str(raw)
        .map_err(|e| CliError::new(format!("parsing devicectl output: {e}")))?;
    Ok(parsed
        .result
        .running_processes
        .into_iter()
        .filter(|p| p.process_identifier > 0 && executable_is_in_app(&p.executable, app_dir_name))
        .map(|p| p.process_identifier)
        .collect())
}

/// Match complete directory components, decoding file URLs exactly once. A
/// literal `%20` in a bundle name is reported as `%2520`, while an unescaped
/// filesystem path must retain its literal percent characters.
fn executable_is_in_app(executable: &str, app_dir_name: &str) -> bool {
    if app_dir_name.is_empty() {
        return false;
    }
    let (path, is_url) = if let Some(url) = executable.strip_prefix("file://") {
        // Skip the URL authority; query/fragment text is not part of the path.
        let Some((_, path)) = url.split_once('/') else {
            return false;
        };
        (path.split(['?', '#']).next().unwrap_or_default(), true)
    } else {
        (executable, false)
    };
    let mut components = path.split('/').peekable();
    while let Some(component) = components.next() {
        let matches = if is_url {
            decode_url_component(component) == app_dir_name.as_bytes()
        } else {
            component == app_dir_name
        };
        if matches && components.peek().is_some_and(|next| !next.is_empty()) {
            return true;
        }
    }
    false
}

/// Decode a single URL component rather than the whole path, so an escaped
/// slash cannot introduce a directory boundary and match another app.
fn decode_url_component(component: &str) -> Vec<u8> {
    let bytes = component.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(encoded) = bytes.get(index + 1..index + 3)
            && let (Some(high), Some(low)) = (hex_digit(encoded[0]), hex_digit(encoded[1]))
        {
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    decoded
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "result": {
        "devices": [
          {
            "identifier": "ID-1",
            "connectionProperties": {"tunnelState": "connected"},
            "deviceProperties": {"name": "My iPhone", "osVersionNumber": "17.0"},
            "hardwareProperties": {"udid": "UDID-1", "marketingName": "iPhone 15 Pro", "platform": "iOS"}
          },
          {
            "identifier": "ID-2",
            "connectionProperties": {"tunnelState": "disconnected"},
            "deviceProperties": {"name": "Alpha iPad"},
            "hardwareProperties": {}
          }
        ]
      }
    }"#;

    #[test]
    fn parses_devices_with_fallbacks() {
        let devices = parse_devices(SAMPLE).unwrap();
        assert_eq!(devices.len(), 2);
        // Sorted by name: "Alpha iPad" before "My iPhone".
        assert_eq!(devices[0].name, "Alpha iPad");
        // Empty hardwareProperties → udid falls back to identifier, platform to iOS.
        assert_eq!(devices[0].udid, "ID-2");
        assert_eq!(devices[0].platform, "iOS");

        let iphone = &devices[1];
        assert_eq!(iphone.udid, "UDID-1");
        assert_eq!(iphone.model, "iPhone 15 Pro");
        assert_eq!(iphone.connection, "connected");
        assert_eq!(iphone.label(), "My iPhone (iPhone 15 Pro, iOS 17.0)");
    }

    #[test]
    fn drops_devices_without_any_id() {
        let raw = r#"{"result":{"devices":[{"identifier":"","hardwareProperties":{}}]}}"#;
        assert!(parse_devices(raw).unwrap().is_empty());
    }

    #[test]
    fn launch_separates_app_arguments_from_devicectl_options() {
        let args = vec![
            "--console".to_owned(),
            "--start-stopped".to_owned(),
            "--".to_owned(),
            "value with spaces 猫".to_owned(),
        ];
        for console in [false, true] {
            let actual = launch_args("UDID", "com.example.Game", &args, false, console);
            let mut expected = vec![
                "devicectl",
                "device",
                "process",
                "launch",
                "--terminate-existing",
            ];
            if console {
                expected.push("--console");
            }
            expected.extend_from_slice(&[
                "--device",
                "UDID",
                "com.example.Game",
                "--",
                "--console",
                "--start-stopped",
                "--",
                "value with spaces 猫",
            ]);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn launch_without_app_arguments_needs_no_separator() {
        let actual = launch_args("UDID", "com.example.Game", &[], false, false);
        assert_eq!(
            actual,
            vec![
                "devicectl",
                "device",
                "process",
                "launch",
                "--terminate-existing",
                "--device",
                "UDID",
                "com.example.Game",
            ]
        );
        let console = launch_args("UDID", "com.example.Game", &[], false, true);
        assert!(console.contains(&"--console"));
        assert_eq!(console.last(), Some(&"com.example.Game"));
        assert!(!console.contains(&"--"));
    }

    #[test]
    fn launch_waits_for_debugger_with_and_without_console() {
        for console in [false, true] {
            let args = launch_args("UDID", "com.example.Game", &[], true, console);
            assert_eq!(
                args.iter().filter(|arg| **arg == "--start-stopped").count(),
                1
            );
            assert!(
                args.iter().position(|arg| *arg == "--start-stopped")
                    < args.iter().position(|arg| *arg == "--device")
            );
            assert!(
                !launch_args("UDID", "com.example.Game", &[], false, console)
                    .contains(&"--start-stopped")
            );
        }
    }

    #[test]
    fn app_pids_match_the_bundle_directory() {
        let raw = r#"{
          "result": {
            "runningProcesses": [
              {"processIdentifier": 496, "executable": "file:///usr/libexec/backboardd"},
              {"processIdentifier": 1201, "executable": "file:///private/var/containers/Bundle/Application/AAAA/My.app/My"},
              {"processIdentifier": 1300, "executable": "file:///private/var/containers/Bundle/Application/BBBB/MyOther.app/MyOther"},
              {"processIdentifier": 0, "executable": "file:///x/My.app/My"}
            ]
          }
        }"#;
        assert_eq!(parse_app_pids(raw, "My.app").unwrap(), vec![1201]);
        assert_eq!(parse_app_pids(raw, "MyOther.app").unwrap(), vec![1300]);
        assert!(parse_app_pids(raw, "Absent.app").unwrap().is_empty());
    }

    #[test]
    fn app_pids_decode_spaces_and_unicode_in_file_urls() {
        let raw = r#"{"result":{"runningProcesses":[
            {"processIdentifier": 101, "executable": "file:///private/var/Idle%20Game.app/Idle%20Game"},
            {"processIdentifier": 102, "executable": "file:///private/var/Caf%C3%A9%20%E7%8C%AB.app/Caf%C3%A9"},
            {"processIdentifier": 103, "executable": "file:///private/var/Caf%c3%a9%20%e7%8c%ab.app/Caf%c3%a9"},
            {"processIdentifier": 104, "executable": "/private/var/Café 猫.app/Café"}
        ]}}"#;
        assert_eq!(parse_app_pids(raw, "Idle Game.app").unwrap(), vec![101]);
        assert_eq!(
            parse_app_pids(raw, "Café 猫.app").unwrap(),
            vec![102, 103, 104]
        );
    }

    #[test]
    fn app_pids_preserve_literal_percent_characters() {
        let raw = r#"{"result":{"runningProcesses":[
            {"processIdentifier": 101, "executable": "file:///x/Game%2520.app/Game"},
            {"processIdentifier": 102, "executable": "/x/Game%20.app/Game"},
            {"processIdentifier": 103, "executable": "file:///x/Game%20.app/Game"},
            {"processIdentifier": 104, "executable": "file:///x/100%25.app/Game"}
        ]}}"#;
        assert_eq!(parse_app_pids(raw, "Game%20.app").unwrap(), vec![101, 102]);
        assert_eq!(parse_app_pids(raw, "Game .app").unwrap(), vec![103]);
        assert_eq!(parse_app_pids(raw, "100%.app").unwrap(), vec![104]);
    }

    #[test]
    fn app_pids_match_only_real_path_components() {
        let raw = r#"{"result":{"runningProcesses":[
            {"processIdentifier": 101, "executable": "file:///x/My.app/My"},
            {"processIdentifier": 102, "executable": "file:///x/My.app.backup/My"},
            {"processIdentifier": 103, "executable": "file:///x/OtherMy.app/My"},
            {"processIdentifier": 104, "executable": "file:///x/Other.app/My?path=/My.app/My"},
            {"processIdentifier": 105, "executable": "file:///x/Other.app/My#/My.app/My"},
            {"processIdentifier": 106, "executable": "file://My.app/x/Other.app/My"},
            {"processIdentifier": 107, "executable": "file:///x%2FMy.app%2FMy"},
            {"processIdentifier": 108, "executable": "file:///x/My.app"},
            {"processIdentifier": 109, "executable": "file:///x/My.app/"},
            {"processIdentifier": -1, "executable": "file:///x/My.app/My"}
        ]}}"#;
        assert_eq!(parse_app_pids(raw, "My.app").unwrap(), vec![101]);
        assert!(parse_app_pids(raw, "").unwrap().is_empty());
    }

    #[test]
    fn find_matches_udid_and_name() {
        let devices = parse_devices(SAMPLE).unwrap();
        assert_eq!(find(&devices, "udid-1").unwrap().name, "My iPhone");
        assert_eq!(find(&devices, "Alpha iPad").unwrap().udid, "ID-2");
        assert!(find(&devices, "nope").is_none());
    }
}
