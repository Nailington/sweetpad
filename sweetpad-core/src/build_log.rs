//! Unfiltered build evidence from `xcresulttool get log --type build`.
//! Signing tools and scripts often emit failures that aren't diagnostics.

use serde_json::Value;

/// Parse and render Xcode's JSON build log for the native extension binding.
pub fn render_json(json: &str) -> Result<String, String> {
    let log = serde_json::from_str(json).map_err(|e| format!("invalid Xcode build log: {e}"))?;
    render(&log)
}

/// Render every command and its output, including nested sections and
/// messages that have no corresponding command output.
pub fn render(log: &Value) -> Result<String, String> {
    if !log.is_object()
        || ![
            "title",
            "subsections",
            "commandInvocationDetails",
            "messages",
        ]
        .iter()
        .any(|key| log.get(key).is_some())
    {
        return Err("xcresulttool returned an unrecognized build log".into());
    }
    let mut text = String::new();
    section(log, &mut text);
    Ok(text)
}

fn append(text: &mut String, value: &str) {
    if !value.is_empty() {
        text.push_str(value);
        if !value.ends_with('\n') {
            text.push('\n');
        }
    }
}

fn section(log: &Value, text: &mut String) {
    let details = &log["commandInvocationDetails"];
    let command = details["commandDetails"].as_str().unwrap_or_default();
    let output = details["emittedOutput"].as_str().unwrap_or_default();
    if command.is_empty() {
        append(text, log["title"].as_str().unwrap_or_default());
    } else {
        append(text, command);
    }
    append(text, output);
    if let Some(messages) = log["messages"].as_array() {
        for message in messages {
            let title = message["title"]
                .as_str()
                .or_else(|| message["shortTitle"].as_str())
                .unwrap_or_default();
            // Xcode repeats a command's last output line as a message.
            if !title.is_empty() && !output.contains(title) && !command.contains(title) {
                append(text, title);
            }
        }
    }
    if let Some(children) = log["subsections"].as_array() {
        for child in children {
            section(child, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_signing_failures_and_script_output_in_nested_sections() {
        let log = json!({"title": "Build App", "subsections": [
            {"title": "Target App", "subsections": [{
                "commandInvocationDetails": {
                    "commandDetails": "CodeSign /tmp/App.app/App.debug.dylib\n    /usr/bin/codesign --sign ID /tmp/App.app/App.debug.dylib\n",
                    "emittedOutput": "/tmp/App.app/App.debug.dylib: errSecInternalComponent\nCommand CodeSign failed with a nonzero exit code\n",
                    "exitCode": 1
                },
                "messages": [{"title": "Command CodeSign failed with a nonzero exit code"}]
            }]},
            {"title": "Run script", "commandInvocationDetails": {"emittedOutput": "custom failure without an error: prefix"}},
            {"messages": [{"shortTitle": "standalone diagnostic"}]}
        ]});
        let text = render(&log).unwrap();
        assert!(text.contains("/usr/bin/codesign --sign ID"));
        assert!(text.contains("errSecInternalComponent"));
        assert_eq!(text.matches("Command CodeSign failed").count(), 1);
        assert!(text.contains("custom failure without an error: prefix\n"));
        assert!(text.ends_with("standalone diagnostic\n"));
    }

    #[test]
    fn refuses_unrelated_json_instead_of_silently_returning_an_empty_log() {
        for log in [json!(null), json!([]), json!({}), json!({"error": "bad"})] {
            assert!(render(&log).is_err());
        }
    }
}
