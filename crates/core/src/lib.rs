//! IssueRelay engine: watches Linear issues assigned to the current user and hands each one
//! to a fresh headless Claude Code or Codex run.
pub mod config;
pub mod model;
pub mod paths;
pub mod trigger;
