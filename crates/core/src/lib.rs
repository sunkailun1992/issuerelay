//! IssueRelay engine: watches Linear issues assigned to the current user and hands each one
//! to a fresh headless Claude Code or Codex run.
pub mod config;
pub mod doctor;
pub mod engine;
pub mod executor;
pub mod ipc;
pub mod linear;
pub mod model;
pub mod paths;
pub mod prompt;
pub mod runner;
pub mod scheduler;
pub mod store;
pub mod text;
pub mod trigger;
