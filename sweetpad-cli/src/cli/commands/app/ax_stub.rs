//! Accessibility is only available on the Mac. These signatures let the
//! Linux CLI reach its remote transport without linking Apple frameworks.
#![allow(dead_code)]

use crate::cli::CliError;

#[derive(Debug)]
pub struct Node {
    pub path: Vec<usize>,
}

#[allow(clippy::unused_self)]
impl Node {
    pub fn count(&self) -> usize {
        0
    }
    pub fn describe(&self) -> String {
        String::new()
    }
}

pub struct Query {
    pub label: Option<String>,
    pub role: Option<String>,
    pub nth: Option<usize>,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.label.is_none() && self.role.is_none()
    }
}

pub enum Act<'a> {
    Perform(&'a str),
    SetValue(&'a str),
}

pub fn has_accessibility_access() -> bool {
    false
}
pub fn request_accessibility_access() {}
pub fn permission_error() -> CliError {
    unsupported()
}
pub fn snapshot(_pid: i32, _depth: usize) -> Result<Node, CliError> {
    Err(unsupported())
}
pub fn find<'a>(_root: &'a Node, _query: &Query, _tree: &str) -> Result<&'a Node, String> {
    Err("accessibility requires macOS".into())
}
pub fn act(_pid: i32, _target: &Node, _action: &Act<'_>, _tree: &str) -> Result<(), CliError> {
    Err(unsupported())
}
pub fn outline(_root: &Node) -> Vec<String> {
    Vec::new()
}
pub fn to_json(_root: &Node) -> serde_json::Value {
    serde_json::Value::Null
}

fn unsupported() -> CliError {
    CliError::new("accessibility requires macOS")
}
