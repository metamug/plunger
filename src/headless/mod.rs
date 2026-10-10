//! The ways in that have no window: the `plunger <command>` CLI, `plunger curl`, the MCP server over stdio,
//! and `plunger install`.

pub mod cli;
pub mod curl_cli;
pub mod mcp;
pub mod mcp_content;
pub mod install;
