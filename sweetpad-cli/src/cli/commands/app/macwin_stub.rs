//! Window capture is only available on macOS.

use crate::cli::CliError;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub number: u32,
    pub pid: i32,
}

pub fn has_screen_capture_access() -> bool {
    false
}
pub fn request_screen_capture_access() {}
pub fn permission_error() -> CliError {
    unsupported()
}
pub fn list_windows() -> Result<Vec<WindowInfo>, CliError> {
    Err(unsupported())
}
pub fn pids_for_executable(_executable: &Path) -> Result<Vec<i32>, CliError> {
    Err(unsupported())
}
pub fn pick_window(
    _windows: &[WindowInfo],
    _pids: &[i32],
    _index: Option<usize>,
) -> Result<(WindowInfo, usize), String> {
    Err("window capture requires macOS".into())
}
pub fn capture_window(_window: u32, _path: &Path) -> Result<(), CliError> {
    Err(unsupported())
}

fn unsupported() -> CliError {
    CliError::new("window capture requires macOS")
}
