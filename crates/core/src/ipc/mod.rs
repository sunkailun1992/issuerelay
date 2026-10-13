//! Local JSON-RPC between the engine host and the CLI / MCP control server: one JSON object
//! per line over a Unix domain socket (macOS) or a named pipe (Windows).
pub mod client;
#[cfg(windows)]
pub mod pipe_security;
pub mod protocol;
pub mod server;
