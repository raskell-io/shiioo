// MCP (Model Context Protocol) server implementation
// This provides tools to agent clients (Claude Code, etc.) via JSON-RPC 2.0 over stdio

pub mod protocol;
pub mod server;
pub mod tools;

pub use server::McpServer;
pub use tools::{Tool, ToolRegistry, ToolTier};
