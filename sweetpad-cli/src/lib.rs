//! The `sweetpad` command-line interface.
//!
//! A frontend over [`sweetpad_core`] / [`sweetpad_lib`]. Two halves share the
//! one binary: the standalone resource-first command tree ([`cli`], see
//! `CLI_DESIGN.md`) and the [`vscode_cli`] JSON-RPC client for the running
//! SweetPad extension.

#[cfg(not(windows))]
pub mod cli;
pub mod portable_remote;
#[cfg(not(windows))]
pub mod vscode_cli;
