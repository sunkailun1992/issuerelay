//! IssueRelay engine: watches Linear issues assigned to the current user and hands each one
//! to a fresh headless Claude Code or Codex run.
pub mod config;
pub mod doctor;
pub mod engine;
pub mod env;
pub mod executor;
pub mod host;
pub mod ipc;
pub mod linear;
pub mod logs;
pub mod model;
pub mod paths;
pub mod prompt;
pub mod runner;
pub mod scheduler;
pub mod setup;
pub mod store;
pub mod text;
pub mod trigger;
