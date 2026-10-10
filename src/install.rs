//! Plunger's side of `plunger install`: which program starts the server, and the instructions that tell an
//! agent to use Plunger instead of curl. Finding each tool's files and writing them safely is the
//! `mcp-install` crate (crates/mcp-install), which any MCP server can use.

use std::path::{Path, PathBuf};

pub use mcp_install::{Agent, Change, Scope};

/// Every tool it can set up.
pub const ALL: [Agent; 7] = Agent::ALL;

/// The steering text every tool gets (also published as docs/steering/plunger.md).
pub const STEERING: &str = include_str!("../docs/steering/plunger.md");

const STEERING_DESCRIPTION: &str = "Use whenever you need to call an HTTP API or test an endpoint: send the request through Plunger (MCP or CLI) instead of curl, Invoke-RestMethod, Invoke-WebRequest or a script. Keeps secrets out of the transcript and returns only the values you select.";

/// How the agent starts the server: this program, or `uvx plunger-cli` (fetched on first use).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    Exe(PathBuf),
    Uvx,
}

impl Via {
    fn server(&self) -> mcp_install::Server {
        match self {
            Via::Exe(path) => mcp_install::Server::new("plunger", path.to_string_lossy().into_owned()).arg("mcp"),
            Via::Uvx => mcp_install::Server::new("plunger", "uvx").args(["plunger-cli", "mcp"]),
        }
    }
}

pub struct Options {
    pub scope: Scope,
    pub via: Via,
    pub mcp: bool,
    pub steering: bool,
    pub dry_run: bool,
}

/// Sets Plunger up in `agents`: the folders under `home` (user scope) or `project` (project scope).
pub fn install(agents: &[Agent], home: &Path, project: &Path, options: &Options) -> Vec<Change> {
    let steering = mcp_install::Steering::new(STEERING_DESCRIPTION, STEERING);
    mcp_install::install(
        agents,
        home,
        project,
        &options.via.server(),
        Some(&steering),
        &mcp_install::Options { scope: options.scope, mcp: options.mcp, steering: options.steering, dry_run: options.dry_run },
    )
}

/// This program's own path, as the way to start the server.
pub fn default_via_exe() -> Via {
    std::env::current_exe().map(Via::Exe).unwrap_or(Via::Uvx)
}

/// How an agent should start the server: this program, unless it is running from uv's temporary
/// cache (`uvx plunger-cli ...`), whose path will not be there next time.
pub fn default_via() -> Via {
    match std::env::current_exe() {
        Ok(path) => {
            let text = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
            if text.contains("/uv/archive-") || text.contains("/.cache/uv/") || text.contains("/uv/cache/") {
                Via::Uvx
            } else {
                Via::Exe(path)
            }
        }
        Err(_) => Via::Uvx,
    }
}

/// The home folder of the user running Plunger.
pub fn home_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("plunger-install-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn options(scope: Scope, via: Via) -> Options {
        Options { scope, via, mcp: true, steering: true, dry_run: false }
    }

    #[test]
    fn plunger_registers_itself_under_its_own_name() {
        let (home, project) = (temp("h1"), temp("p1"));
        install(&ALL, &home, &project, &options(Scope::Project, Via::Exe(PathBuf::from("C:/Tools/plunger.exe"))));
        let mcp: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(project.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["plunger"], serde_json::json!({"command": "C:/Tools/plunger.exe", "args": ["mcp"]}));
        assert!(std::fs::read_to_string(project.join(".cursor/rules/plunger.mdc")).unwrap().contains("alwaysApply: true"));
        assert!(std::fs::read_to_string(project.join(".claude/skills/plunger/SKILL.md")).unwrap().starts_with("---\nname: plunger"));
        assert!(std::fs::read_to_string(project.join("CLAUDE.md")).unwrap().contains("<!-- plunger:start -->"));
    }

    #[test]
    fn uvx_starts_the_server_without_a_local_copy() {
        let (home, project) = (temp("h7"), temp("p7"));
        let o = Options { steering: false, ..options(Scope::Project, Via::Uvx) };
        install(&[Agent::Cursor], &home, &project, &o);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(project.join(".cursor/mcp.json")).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["plunger"], serde_json::json!({"command": "uvx", "args": ["plunger-cli", "mcp"]}));
    }

    #[test]
    fn the_steering_tells_agents_what_to_avoid_and_what_to_use() {
        for needle in ["curl", "Invoke-RestMethod", "send_request", "select", "extract", "set_variable", "get_history"] {
            assert!(STEERING.contains(needle), "the steering does not mention {needle}");
        }
    }
}
