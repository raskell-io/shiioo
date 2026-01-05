// MCP (Model Context Protocol) server implementation
// This will provide tools to agent clients (Claude Code, etc.)

pub mod protocol;
pub mod server;
pub mod tools;

pub use server::McpServer;
