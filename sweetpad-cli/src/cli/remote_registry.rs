//! User-managed Mac names, separate from the hand-authored `config.toml`.
//! SSH owns credentials and host-key verification; this file stores only
//! endpoints and optional paths to keys already on disk.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Subcommand;
use serde::{Deserialize, Serialize};

use crate::cli::output::Output;
use crate::cli::{CliError, CommandResult, Context, Render, Rendered};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Save a name for an SSH-reachable Mac.
    Add {
        /// Local name, including spaces when quoted.
        name: String,
        /// SSH host, 'user@host', or alias from ~/.ssh/config.
        host: String,
        /// Existing private key file to use for this Mac.
        #[arg(long = "identity-file", value_name = "PATH")]
        identity_file: Option<PathBuf>,
        /// SSH port (otherwise SSH config/default port applies).
        #[arg(long)]
        port: Option<u16>,
        /// Update an existing saved Mac with the same name.
        #[arg(long)]
        replace: bool,
    },
    /// List saved Macs (the default action).
    List,
    /// Show one saved Mac and its SSH settings.
    Show { name: String },
    /// Remove a saved Mac without touching SSH keys or known_hosts.
    Remove { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mac {
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

impl Mac {
    #[must_use]
    pub fn ssh_command(&self) -> Command {
        let mut command = Command::new("ssh");
        if let Some(path) = &self.identity_file {
            command
                .arg("-i")
                .arg(path)
                .args(["-o", "IdentitiesOnly=yes"]);
        }
        if let Some(port) = self.port {
            command.arg("-p").arg(port.to_string());
        }
        command
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Registry {
    macs: BTreeMap<String, Mac>,
}

impl Registry {
    fn path() -> Result<PathBuf, String> {
        crate::cli::config::Config::path()
            .and_then(|path| path.parent().map(|dir| dir.join("remotes.toml")))
            .ok_or_else(|| {
                "cannot locate the SweetPad config directory (set HOME or XDG_CONFIG_HOME)"
                    .to_string()
            })
    }

    fn load() -> Result<Self, String> {
        let path = Self::path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn save(&self) -> Result<(), String> {
        let path = Self::path()?;
        let parent = path
            .parent()
            .ok_or_else(|| "invalid remotes path".to_string())?;
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        for attempt in 0..10 {
            let temporary =
                parent.join(format!(".remotes.toml.{}.{}", std::process::id(), attempt));
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary);
            let mut file = match file {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("{}: {e}", temporary.display())),
            };
            if let Err(e) = file
                .write_all(text.as_bytes())
                .and_then(|()| file.sync_all())
            {
                let _ = std::fs::remove_file(&temporary);
                return Err(format!("{}: {e}", temporary.display()));
            }
            drop(file);
            if let Err(e) = std::fs::rename(&temporary, &path) {
                let _ = std::fs::remove_file(&temporary);
                return Err(format!("{}: {e}", path.display()));
            }
            return Ok(());
        }
        Err("could not allocate a temporary remotes file".into())
    }
}

pub fn resolve(reference: &str) -> Result<Mac, String> {
    let registry = Registry::load()?;
    if let Some(mac) = registry.macs.get(reference) {
        if !super::remote::valid_host(&mac.host) || mac.port == Some(0) {
            return Err(format!(
                "saved Mac {reference:?} has an invalid SSH target or port"
            ));
        }
        return Ok(mac.clone());
    }
    if super::remote::valid_host(reference) {
        return Ok(Mac {
            host: reference.to_owned(),
            identity_file: None,
            port: None,
        });
    }
    Err(format!(
        "no saved Mac named {reference:?}; run 'sweetpad remote list' or 'sweetpad remote add'"
    ))
}

pub fn manage(_ctx: &mut Context, action: Option<&Action>) -> CommandResult {
    let _lock = if matches!(action, Some(Action::Add { .. } | Action::Remove { .. })) {
        Some(
            crate::portable_remote::lock_registry(&Registry::path().map_err(CliError::new)?)
                .map_err(CliError::new)?,
        )
    } else {
        None
    };
    let mut registry = Registry::load().map_err(CliError::new)?;
    let message = match action {
        None | Some(Action::List) => None,
        Some(Action::Add {
            name,
            host,
            identity_file,
            port,
            replace,
        }) => {
            validate_name(name).map_err(CliError::new)?;
            if !super::remote::valid_host(host) {
                return Err(CliError::new(
                    "SSH target must be a host, user@host, or SSH config alias",
                ));
            }
            if *port == Some(0) {
                return Err(CliError::new("SSH port must be greater than zero"));
            }
            if registry.macs.contains_key(name) && !replace {
                return Err(CliError::new(format!(
                    "Mac {name:?} already exists (pass --replace to update it)"
                )));
            }
            let identity_file = identity_file
                .as_ref()
                .map(|path| normalize_identity(path))
                .transpose()
                .map_err(CliError::new)?;
            registry.macs.insert(
                name.clone(),
                Mac {
                    host: host.clone(),
                    identity_file,
                    port: *port,
                },
            );
            registry.save().map_err(CliError::new)?;
            Some(format!("saved Mac {name:?}"))
        }
        Some(Action::Show { name }) => {
            let mac = registry
                .macs
                .get(name)
                .ok_or_else(|| CliError::new(format!("no saved Mac named {name:?}")))?
                .clone();
            return Ok(Rendered::data(RegistryReport {
                entries: vec![(name.clone(), mac)],
                message: None,
            }));
        }
        Some(Action::Remove { name }) => {
            if registry.macs.remove(name).is_none() {
                return Err(CliError::new(format!("no saved Mac named {name:?}")));
            }
            registry.save().map_err(CliError::new)?;
            Some(format!("removed Mac {name:?}"))
        }
    };
    Ok(Rendered::data(RegistryReport {
        entries: registry.macs.into_iter().collect(),
        message,
    }))
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.trim() != name || name.chars().any(char::is_control) {
        return Err(
            "Mac name must be nonempty, with no surrounding whitespace or control characters"
                .into(),
        );
    }
    Ok(())
}

fn normalize_identity(path: &Path) -> Result<PathBuf, String> {
    let expanded = if path == Path::new("~") || path.starts_with("~/") {
        let home =
            std::env::var_os("HOME").ok_or_else(|| "cannot expand ~ without HOME".to_string())?;
        PathBuf::from(home).join(path.strip_prefix("~").unwrap())
    } else {
        path.to_path_buf()
    };
    let absolute = std::fs::canonicalize(&expanded)
        .map_err(|e| format!("identity file {}: {e}", expanded.display()))?;
    if !absolute.is_file() {
        return Err(format!(
            "identity path {} is not a file",
            absolute.display()
        ));
    }
    Ok(absolute)
}

struct RegistryReport {
    entries: Vec<(String, Mac)>,
    message: Option<String>,
}

impl Render for RegistryReport {
    fn human(&self, out: &Output) {
        if let Some(message) = &self.message {
            out.note(message);
        }
        if self.entries.is_empty() {
            out.note("no saved Macs; add one with 'sweetpad remote add NAME SSH_TARGET'");
        } else {
            for (name, mac) in &self.entries {
                let mut details = format!("{name}  →  {}", mac.host);
                if let Some(port) = mac.port {
                    let _ = write!(details, "  (port {port})");
                }
                if let Some(path) = &mac.identity_file {
                    let _ = write!(details, "  (key {})", path.display());
                }
                out.line(&details);
            }
        }
    }

    fn json(&self) -> serde_json::Value {
        let macs: Vec<_> = self
            .entries
            .iter()
            .map(|(name, mac)| {
                serde_json::json!({
                    "name": name,
                    "host": mac.host,
                    "identityFile": mac.identity_file.as_ref().map(|p| p.display().to_string()),
                    "port": mac.port,
                })
            })
            .collect();
        serde_json::json!({ "macs": macs, "message": self.message })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_round_trip_preserves_spaced_names_and_key_paths() {
        let text = "[macs.\"Studio Mac\"]\nhost = \"dev@mac.local\"\nidentity_file = \"/tmp/a key\"\nport = 2222\n";
        let registry: Registry = toml::from_str(text).unwrap();
        assert_eq!(registry.macs["Studio Mac"].port, Some(2222));
        let serialized = toml::to_string(&registry).unwrap();
        let reparsed: Registry = toml::from_str(&serialized).unwrap();
        assert_eq!(
            reparsed.macs["Studio Mac"].identity_file,
            Some(PathBuf::from("/tmp/a key"))
        );
    }

    #[test]
    fn validation_catches_invisible_names() {
        assert!(validate_name("Studio Mac").is_ok());
        assert!(validate_name(" Studio Mac").is_err());
        assert!(validate_name("mac\nname").is_err());
    }

    #[test]
    fn ssh_uses_the_saved_key_and_port() {
        let mac = Mac {
            host: "dev@mac.local".into(),
            identity_file: Some(PathBuf::from("/tmp/key's name")),
            port: Some(2222),
        };
        assert_eq!(mac.ssh_command().get_args().count(), 6);
    }
}
