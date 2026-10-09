//! Setting Plunger up in AI coding tools: adds the MCP server to each tool's config and writes the
//! always-on steering that tells the agent to use Plunger instead of curl or Invoke-RestMethod.
//!
//! Nothing is overwritten blindly: an existing config is parsed and merged (a file that does not parse
//! is left alone), a copy is kept next to anything changed, shared files like CLAUDE.md only get a
//! marked block, and running it twice changes nothing the second time. `dry_run` reports what would
//! change without touching a file.

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// The steering text every tool gets (also published as docs/steering/plunger.md).
pub const STEERING: &str = include_str!("../docs/steering/plunger.md");

const BLOCK_START: &str = "<!-- plunger:start -->";
const BLOCK_END: &str = "<!-- plunger:end -->";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
    Cursor,
    Kiro,
    Codex,
    Windsurf,
    VsCode,
    Gemini,
}

pub const ALL: [Agent; 7] = [Agent::ClaudeCode, Agent::Cursor, Agent::Kiro, Agent::Codex, Agent::Windsurf, Agent::VsCode, Agent::Gemini];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// For every project of this user.
    User,
    /// For one project folder.
    Project,
}

/// How the agent starts the server: this program, or `uvx plunger-cli` (fetched on first use).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    Exe(PathBuf),
    Uvx,
}

impl Via {
    fn command(&self) -> (String, Vec<String>) {
        match self {
            Via::Exe(path) => (path.to_string_lossy().into_owned(), vec!["mcp".into()]),
            Via::Uvx => ("uvx".into(), vec!["plunger-cli".into(), "mcp".into()]),
        }
    }
}

impl Agent {
    pub fn id(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "claude-code",
            Agent::Cursor => "cursor",
            Agent::Kiro => "kiro",
            Agent::Codex => "codex",
            Agent::Windsurf => "windsurf",
            Agent::VsCode => "vscode",
            Agent::Gemini => "gemini",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "Claude Code",
            Agent::Cursor => "Cursor",
            Agent::Kiro => "Kiro",
            Agent::Codex => "Codex",
            Agent::Windsurf => "Windsurf",
            Agent::VsCode => "VS Code (Copilot)",
            Agent::Gemini => "Gemini CLI",
        }
    }

    pub fn parse(text: &str) -> Option<Agent> {
        let t = text.trim().to_ascii_lowercase().replace([' ', '_'], "-");
        ALL.into_iter().find(|a| a.id() == t || (t == "claude" && *a == Agent::ClaudeCode) || (t == "copilot" && *a == Agent::VsCode))
    }

    /// The folder that exists when the tool is installed for this user.
    fn user_marker(self, home: &Path) -> PathBuf {
        match self {
            Agent::ClaudeCode => home.join(".claude"),
            Agent::Cursor => home.join(".cursor"),
            Agent::Kiro => home.join(".kiro"),
            Agent::Codex => home.join(".codex"),
            Agent::Windsurf => home.join(".codeium").join("windsurf"),
            Agent::VsCode => home.join(".vscode"),
            Agent::Gemini => home.join(".gemini"),
        }
    }

    /// Whether the tool looks installed for this user.
    pub fn detected(self, home: &Path) -> bool {
        self.user_marker(home).exists()
    }

    /// Where the MCP server is registered, and in what shape. None when the tool has no file for this scope.
    fn mcp_target(self, scope: Scope, home: &Path, project: &Path) -> Option<(PathBuf, Format)> {
        Some(match (self, scope) {
            (Agent::ClaudeCode, Scope::User) => (home.join(".claude.json"), Format::Json { key: "mcpServers", typed: false }),
            (Agent::ClaudeCode, Scope::Project) => (project.join(".mcp.json"), Format::Json { key: "mcpServers", typed: false }),
            (Agent::Cursor, Scope::User) => (home.join(".cursor").join("mcp.json"), Format::Json { key: "mcpServers", typed: false }),
            (Agent::Cursor, Scope::Project) => (project.join(".cursor").join("mcp.json"), Format::Json { key: "mcpServers", typed: false }),
            (Agent::Kiro, Scope::User) => (home.join(".kiro").join("settings").join("mcp.json"), Format::Json { key: "mcpServers", typed: false }),
            (Agent::Kiro, Scope::Project) => (project.join(".kiro").join("settings").join("mcp.json"), Format::Json { key: "mcpServers", typed: false }),
            (Agent::Codex, Scope::User) => (home.join(".codex").join("config.toml"), Format::Toml),
            (Agent::Codex, Scope::Project) => return None,
            (Agent::Windsurf, Scope::User) => (home.join(".codeium").join("windsurf").join("mcp_config.json"), Format::Json { key: "mcpServers", typed: false }),
            (Agent::Windsurf, Scope::Project) => return None,
            (Agent::VsCode, Scope::Project) => (project.join(".vscode").join("mcp.json"), Format::Json { key: "servers", typed: true }),
            (Agent::VsCode, Scope::User) => return None,
            (Agent::Gemini, Scope::User) => (home.join(".gemini").join("settings.json"), Format::Json { key: "mcpServers", typed: false }),
            (Agent::Gemini, Scope::Project) => (project.join(".gemini").join("settings.json"), Format::Json { key: "mcpServers", typed: false }),
        })
    }

    /// The steering files for this tool and scope.
    fn steering_targets(self, scope: Scope, home: &Path, project: &Path) -> Vec<(PathBuf, Steering)> {
        match (self, scope) {
            (Agent::ClaudeCode, Scope::User) => vec![
                (home.join(".claude").join("CLAUDE.md"), Steering::Block),
                (home.join(".claude").join("skills").join("plunger").join("SKILL.md"), Steering::Skill),
            ],
            (Agent::ClaudeCode, Scope::Project) => vec![
                (project.join("CLAUDE.md"), Steering::Block),
                (project.join(".claude").join("skills").join("plunger").join("SKILL.md"), Steering::Skill),
            ],
            (Agent::Cursor, Scope::Project) => vec![(project.join(".cursor").join("rules").join("plunger.mdc"), Steering::CursorRule)],
            (Agent::Kiro, Scope::User) => vec![(home.join(".kiro").join("steering").join("plunger.md"), Steering::KiroAlways)],
            (Agent::Kiro, Scope::Project) => vec![(project.join(".kiro").join("steering").join("plunger.md"), Steering::KiroAlways)],
            (Agent::Codex, Scope::User) => vec![(home.join(".codex").join("AGENTS.md"), Steering::Block)],
            (Agent::Codex, Scope::Project) => vec![(project.join("AGENTS.md"), Steering::Block)],
            (Agent::Windsurf, Scope::Project) => vec![(project.join(".windsurf").join("rules").join("plunger.md"), Steering::WindsurfRule)],
            (Agent::VsCode, Scope::Project) => vec![(project.join(".github").join("copilot-instructions.md"), Steering::Block)],
            (Agent::Gemini, Scope::User) => vec![(home.join(".gemini").join("GEMINI.md"), Steering::Block)],
            (Agent::Gemini, Scope::Project) => vec![(project.join("GEMINI.md"), Steering::Block)],
            // Cursor's user rules live in its settings UI, and Windsurf's global rules in one shared file.
            (Agent::Cursor, Scope::User) | (Agent::Windsurf, Scope::User) | (Agent::VsCode, Scope::User) => vec![],
        }
    }
}

#[derive(Clone, Copy)]
enum Format {
    /// A JSON file with a map of servers under `key`; `typed` adds `"type": "stdio"` (VS Code wants it).
    Json { key: &'static str, typed: bool },
    Toml,
}

#[derive(Clone, Copy)]
enum Steering {
    /// A block between markers in a file that holds other instructions too.
    Block,
    /// A whole file of our own, for each tool's rule format.
    Skill,
    CursorRule,
    KiroAlways,
    WindsurfRule,
}

/// What happened (or would happen) to one file.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Change {
    pub agent: String,
    /// `mcp` or `steering`.
    pub what: String,
    pub file: String,
    /// `created`, `updated`, `unchanged` or `skipped` (with `note` saying why).
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

pub struct Options {
    pub scope: Scope,
    pub via: Via,
    pub mcp: bool,
    pub steering: bool,
    pub dry_run: bool,
}

fn change(agent: Agent, what: &str, file: &Path, action: &str, note: Option<String>) -> Change {
    Change { agent: agent.id().into(), what: what.into(), file: file.display().to_string(), action: action.into(), note }
}

/// Sets up `agents` in the folders under `home` (user scope) or `project` (project scope).
pub fn install(agents: &[Agent], home: &Path, project: &Path, options: &Options) -> Vec<Change> {
    let mut out = Vec::new();
    for &agent in agents {
        if options.mcp {
            match agent.mcp_target(options.scope, home, project) {
                Some((file, format)) => out.push(write_mcp(agent, &file, format, &options.via, options.dry_run)),
                None => out.push(change(
                    agent,
                    "mcp",
                    Path::new(""),
                    "skipped",
                    Some(format!("{} has no {} MCP config file; use the other scope.", agent.label(), scope_name(options.scope))),
                )),
            }
        }
        if options.steering {
            let targets = agent.steering_targets(options.scope, home, project);
            if targets.is_empty() {
                out.push(change(agent, "steering", Path::new(""), "skipped", Some(format!("{} has no {} steering file; use the other scope.", agent.label(), scope_name(options.scope)))));
            }
            for (file, kind) in targets {
                out.push(write_steering(agent, &file, kind, options.dry_run));
            }
        }
    }
    out
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::User => "user-level",
        Scope::Project => "project-level",
    }
}

// ---- the MCP entry ------------------------------------------------------------------------------

fn server_entry(via: &Via, typed: bool) -> Value {
    let (command, args) = via.command();
    let mut entry = Map::new();
    if typed {
        entry.insert("type".into(), json!("stdio"));
    }
    entry.insert("command".into(), json!(command));
    entry.insert("args".into(), json!(args));
    Value::Object(entry)
}

fn write_mcp(agent: Agent, file: &Path, format: Format, via: &Via, dry_run: bool) -> Change {
    let result = match format {
        Format::Json { key, typed } => merge_json(file, key, &server_entry(via, typed)),
        Format::Toml => merge_toml(file, via),
    };
    finish(agent, "mcp", file, result, dry_run)
}

/// The new text for `file`, or None when it is already right; Err when it must not be touched.
type Planned = Result<Option<String>, String>;

fn merge_json(file: &Path, key: &str, entry: &Value) -> Planned {
    let existing = std::fs::read_to_string(file).ok();
    let mut root: Value = match existing.as_deref().map(str::trim) {
        None | Some("") => json!({}),
        Some(text) => serde_json::from_str(text).map_err(|e| format!("it is not valid JSON ({e}), so it was left alone"))?,
    };
    let Value::Object(map) = &mut root else { return Err("it is not a JSON object, so it was left alone".into()) };
    let servers = map.entry(key.to_string()).or_insert_with(|| json!({}));
    let Value::Object(servers) = servers else { return Err(format!("`{key}` is not an object, so the file was left alone")) };
    if servers.get("plunger") == Some(entry) {
        return Ok(None);
    }
    servers.insert("plunger".into(), entry.clone());
    let mut text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    text.push('\n');
    Ok(Some(text))
}

fn merge_toml(file: &Path, via: &Via) -> Planned {
    let existing = std::fs::read_to_string(file).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == "[mcp_servers.plunger]") {
        return Ok(None);
    }
    let (command, args) = via.command();
    let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let args = args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(", ");
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&format!("[mcp_servers.plunger]\ncommand = {}\nargs = [{}]\n", quote(&command), args));
    Ok(Some(text))
}

// ---- steering ------------------------------------------------------------------------------------

fn steering_text(kind: Steering) -> String {
    let body = STEERING.trim_end();
    match kind {
        Steering::Block => format!("{BLOCK_START}\n{body}\n{BLOCK_END}\n"),
        Steering::Skill => format!(
            "---\nname: plunger\ndescription: Use whenever you need to call an HTTP API or test an endpoint: send the request through Plunger (MCP or CLI) instead of curl, Invoke-RestMethod, Invoke-WebRequest or a script. Keeps secrets out of the transcript and returns only the values you select.\n---\n\n{body}\n"
        ),
        Steering::CursorRule => format!("---\ndescription: Send HTTP requests through Plunger, not curl or Invoke-RestMethod\nalwaysApply: true\n---\n\n{body}\n"),
        Steering::KiroAlways => format!("---\ninclusion: always\n---\n\n{body}\n"),
        Steering::WindsurfRule => format!("---\ntrigger: always_on\ndescription: Send HTTP requests through Plunger, not curl or Invoke-RestMethod\n---\n\n{body}\n"),
    }
}

fn write_steering(agent: Agent, file: &Path, kind: Steering, dry_run: bool) -> Change {
    let wanted = steering_text(kind);
    let existing = std::fs::read_to_string(file).ok();
    let planned: Planned = match kind {
        Steering::Block => Ok(with_block(existing.as_deref().unwrap_or(""), &wanted)),
        _ => Ok((existing.as_deref() != Some(wanted.as_str())).then_some(wanted)),
    };
    finish(agent, "steering", file, planned, dry_run)
}

/// `text` with our block added, or replaced in place; None when it is already there and current.
fn with_block(text: &str, block: &str) -> Option<String> {
    if let (Some(start), Some(end)) = (text.find(BLOCK_START), text.find(BLOCK_END)) {
        if start < end {
            let end = end + BLOCK_END.len();
            let current = text[start..end].trim_end();
            if current == block.trim_end() {
                return None;
            }
            let after = text[end..].strip_prefix('\n').unwrap_or(&text[end..]);
            return Some(format!("{}{}{}", &text[..start], block, after));
        }
    }
    let mut out = text.to_string();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(block);
    Some(out)
}

// ---- writing -------------------------------------------------------------------------------------

fn finish(agent: Agent, what: &str, file: &Path, planned: Planned, dry_run: bool) -> Change {
    let new_text = match planned {
        Err(why) => return change(agent, what, file, "skipped", Some(why)),
        Ok(None) => return change(agent, what, file, "unchanged", None),
        Ok(Some(text)) => text,
    };
    let existed = file.exists();
    let action = if existed { "updated" } else { "created" };
    if dry_run {
        return change(agent, what, file, action, Some("dry run: nothing was written".into()));
    }
    match write_with_backup(file, &new_text, existed) {
        Ok(backup) => change(agent, what, file, action, backup.map(|b| format!("the previous file is kept as {}", b.display()))),
        Err(e) => change(agent, what, file, "skipped", Some(format!("could not write it: {e}"))),
    }
}

fn write_with_backup(file: &Path, text: &str, existed: bool) -> std::io::Result<Option<PathBuf>> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut backup = None;
    if existed {
        let mut name = file.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(".plunger-backup");
        let path = file.with_file_name(name);
        // Keep the first copy: it is the file as the user had it, not as an earlier run left it.
        if !path.exists() {
            std::fs::copy(file, &path)?;
        }
        backup = Some(path);
    }
    std::fs::write(file, text)?;
    Ok(backup)
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

    fn options(scope: Scope, dry_run: bool) -> Options {
        Options { scope, via: Via::Exe(PathBuf::from("C:/Tools/plunger.exe")), mcp: true, steering: true, dry_run }
    }

    #[test]
    fn a_project_setup_writes_the_server_and_the_steering_for_every_tool() {
        let (home, project) = (temp("h1"), temp("p1"));
        let changes = install(&ALL, &home, &project, &options(Scope::Project, false));
        assert!(changes.iter().all(|c| c.action == "created" || c.action == "skipped"), "{changes:?}");

        let mcp: Value = serde_json::from_str(&std::fs::read_to_string(project.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["plunger"], json!({"command": "C:/Tools/plunger.exe", "args": ["mcp"]}));
        let vscode: Value = serde_json::from_str(&std::fs::read_to_string(project.join(".vscode/mcp.json")).unwrap()).unwrap();
        assert_eq!(vscode["servers"]["plunger"]["type"], "stdio");
        assert!(std::fs::read_to_string(project.join(".cursor/rules/plunger.mdc")).unwrap().contains("alwaysApply: true"));
        assert!(std::fs::read_to_string(project.join(".kiro/steering/plunger.md")).unwrap().contains("inclusion: always"));
        let claude_md = std::fs::read_to_string(project.join("CLAUDE.md")).unwrap();
        assert!(claude_md.contains(BLOCK_START) && claude_md.contains("instead of curl") || claude_md.contains("Do not use `curl`"));
        assert!(std::fs::read_to_string(project.join(".claude/skills/plunger/SKILL.md")).unwrap().starts_with("---\nname: plunger"));
        // Codex has no project-level MCP file, and says so.
        assert!(changes.iter().any(|c| c.agent == "codex" && c.what == "mcp" && c.action == "skipped"));
    }

    #[test]
    fn running_it_twice_changes_nothing_the_second_time() {
        let (home, project) = (temp("h2"), temp("p2"));
        install(&ALL, &home, &project, &options(Scope::User, false));
        let again = install(&ALL, &home, &project, &options(Scope::User, false));
        assert!(again.iter().all(|c| c.action == "unchanged" || c.action == "skipped"), "{again:?}");
        let toml = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert_eq!(toml.matches("[mcp_servers.plunger]").count(), 1);
        assert!(toml.contains("command = \"C:/Tools/plunger.exe\"") && toml.contains("args = [\"mcp\"]"), "{toml}");
    }

    #[test]
    fn other_servers_and_settings_in_a_config_are_kept_and_a_copy_is_left() {
        let (home, project) = (temp("h3"), temp("p3"));
        std::fs::write(project.join(".mcp.json"), r#"{"theme": "dark", "mcpServers": {"other": {"command": "x"}}}"#).unwrap();
        let changes = install(&[Agent::ClaudeCode], &home, &project, &Options { steering: false, ..options(Scope::Project, false) });
        assert_eq!(changes[0].action, "updated");
        let merged: Value = serde_json::from_str(&std::fs::read_to_string(project.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(merged["theme"], "dark");
        assert_eq!(merged["mcpServers"]["other"]["command"], "x");
        assert!(merged["mcpServers"]["plunger"].is_object());
        let backup = std::fs::read_to_string(project.join(".mcp.json.plunger-backup")).unwrap();
        assert!(backup.contains("\"other\"") && !backup.contains("plunger"));
    }

    #[test]
    fn a_file_that_is_not_json_is_left_alone() {
        let (home, project) = (temp("h4"), temp("p4"));
        std::fs::write(project.join(".mcp.json"), "{ not json").unwrap();
        let changes = install(&[Agent::ClaudeCode], &home, &project, &Options { steering: false, ..options(Scope::Project, false) });
        assert_eq!(changes[0].action, "skipped");
        assert_eq!(std::fs::read_to_string(project.join(".mcp.json")).unwrap(), "{ not json");
    }

    #[test]
    fn instructions_already_in_a_shared_file_stay_and_our_block_is_replaced_not_duplicated() {
        let (home, project) = (temp("h5"), temp("p5"));
        std::fs::write(project.join("AGENTS.md"), "# My rules\n\nBe kind.\n").unwrap();
        let only_steering = Options { mcp: false, ..options(Scope::Project, false) };
        install(&[Agent::Codex], &home, &project, &only_steering);
        let first = std::fs::read_to_string(project.join("AGENTS.md")).unwrap();
        assert!(first.starts_with("# My rules\n\nBe kind.\n") && first.contains(BLOCK_START));
        // an out-of-date block is refreshed in place
        std::fs::write(project.join("AGENTS.md"), first.replace("Use Plunger for HTTP requests", "OLD TEXT")).unwrap();
        install(&[Agent::Codex], &home, &project, &only_steering);
        let second = std::fs::read_to_string(project.join("AGENTS.md")).unwrap();
        assert_eq!(second.matches(BLOCK_START).count(), 1);
        assert!(second.contains("Use Plunger for HTTP requests") && !second.contains("OLD TEXT") && second.contains("Be kind."));
    }

    #[test]
    fn a_dry_run_reports_and_writes_nothing() {
        let (home, project) = (temp("h6"), temp("p6"));
        let changes = install(&ALL, &home, &project, &options(Scope::Project, true));
        assert!(changes.iter().any(|c| c.action == "created"));
        assert!(std::fs::read_dir(&project).unwrap().next().is_none(), "no file was created");
    }

    #[test]
    fn uvx_starts_the_server_without_a_local_copy() {
        let (home, project) = (temp("h7"), temp("p7"));
        let o = Options { via: Via::Uvx, steering: false, ..options(Scope::Project, false) };
        install(&[Agent::Cursor], &home, &project, &o);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(project.join(".cursor/mcp.json")).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["plunger"], json!({"command": "uvx", "args": ["plunger-cli", "mcp"]}));
    }

    #[test]
    fn agent_names_are_forgiving() {
        assert_eq!(Agent::parse("Claude Code"), Some(Agent::ClaudeCode));
        assert_eq!(Agent::parse("claude"), Some(Agent::ClaudeCode));
        assert_eq!(Agent::parse("VSCODE"), Some(Agent::VsCode));
        assert_eq!(Agent::parse("copilot"), Some(Agent::VsCode));
        assert_eq!(Agent::parse("emacs"), None);
    }

    #[test]
    fn the_steering_tells_agents_what_to_avoid_and_what_to_use() {
        for needle in ["curl", "Invoke-RestMethod", "send_request", "select", "extract", "set_variable", "get_history"] {
            assert!(STEERING.contains(needle), "the steering does not mention {needle}");
        }
    }
}
