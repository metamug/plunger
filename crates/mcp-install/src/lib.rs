//! Register an MCP server in AI coding tools' own config files, and add the always-on instructions that
//! tell the agent to use it.
//!
//! Every tool wants a different file in a different place in a different shape: Claude Code reads
//! `.mcp.json`, Cursor `.cursor/mcp.json`, Kiro `.kiro/settings/mcp.json`, Codex a TOML table in
//! `~/.codex/config.toml`, VS Code `.vscode/mcp.json` with a `servers` key, and so on. This crate knows
//! those, so a server's own installer is a few lines:
//!
//! ```no_run
//! use mcp_install::{install, Agent, Options, Scope, Server, Steering};
//!
//! let server = Server::new("acme", "acme-mcp").arg("serve");
//! let steering = Steering::new(
//!     "Use Acme for deployments",
//!     "# Use Acme\n\nFor anything about deployments, call the `acme` MCP tools instead of running the CLI.\n",
//! );
//! let home = mcp_install::home_dir().expect("a home folder");
//! let project = std::env::current_dir().unwrap();
//! let options = Options { scope: Scope::Project, mcp: true, steering: true, dry_run: false };
//! for change in install(&Agent::ALL, &home, &project, &server, Some(&steering), &options) {
//!     println!("{} {} {}", change.agent, change.what, change.action);
//! }
//! ```
//!
//! It is careful with other people's files:
//!
//! - an existing config is parsed and merged, so other servers and settings are kept; a file that does
//!   not parse is left alone and reported;
//! - the first time a file is changed, the original is copied next to it as `<name>.<server>-backup`;
//! - instructions in a file the user also writes (`CLAUDE.md`, `AGENTS.md`, ...) go between
//!   `<!-- name:start -->` and `<!-- name:end -->` markers, and are replaced in place next time;
//! - running it twice changes nothing the second time;
//! - `dry_run` reports exactly what would change and writes nothing.

#![forbid(unsafe_code)]

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// An AI coding tool this crate knows how to configure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Agent {
    ClaudeCode,
    Cursor,
    Kiro,
    Codex,
    Windsurf,
    VsCode,
    Gemini,
}

/// Where a setting applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// For every project of this user (files under the home folder).
    User,
    /// For one project folder (files under the project).
    Project,
}

/// The MCP server to register: its name and how a tool starts it (a stdio server).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    /// The key it is registered under, such as `acme`. It also names the rule and skill files.
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

impl Server {
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self { name: name.into(), command: command.into(), args: Vec::new() }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }
}

/// Instructions an agent should always follow, written in each tool's own rule format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Steering {
    /// One line saying when the rule applies, for tools that show or match on it.
    pub description: String,
    /// The Markdown text itself.
    pub body: String,
}

impl Steering {
    pub fn new(description: impl Into<String>, body: impl Into<String>) -> Self {
        Self { description: description.into(), body: body.into() }
    }
}

/// What to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub scope: Scope,
    /// Register the server.
    pub mcp: bool,
    /// Write the instructions (needs a [`Steering`] to be passed to [`install`]).
    pub steering: bool,
    /// Report what would change and write nothing.
    pub dry_run: bool,
}

/// What happened (or, in a dry run, would happen) to one file.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Change {
    /// The tool's id, for example `claude-code`.
    pub agent: String,
    /// `mcp` or `steering`.
    pub what: String,
    pub file: String,
    /// `created`, `updated`, `unchanged` or `skipped` (with `note` saying why).
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Agent {
    /// Every tool, in a stable order.
    pub const ALL: [Agent; 7] = [Agent::ClaudeCode, Agent::Cursor, Agent::Kiro, Agent::Codex, Agent::Windsurf, Agent::VsCode, Agent::Gemini];

    /// A short stable name: `claude-code`, `cursor`, `kiro`, `codex`, `windsurf`, `vscode`, `gemini`.
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

    /// The name people know it by.
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

    /// Reads an id or a casual spelling (`Claude Code`, `claude`, `copilot`, `VSCODE`).
    pub fn parse(text: &str) -> Option<Agent> {
        let t = text.trim().to_ascii_lowercase().replace([' ', '_'], "-");
        Agent::ALL.into_iter().find(|a| a.id() == t || (t == "claude" && *a == Agent::ClaudeCode) || (t == "copilot" && *a == Agent::VsCode))
    }

    /// Whether the tool looks installed for this user (its folder under `home` exists).
    pub fn detected(self, home: &Path) -> bool {
        match self {
            Agent::ClaudeCode => home.join(".claude"),
            Agent::Cursor => home.join(".cursor"),
            Agent::Kiro => home.join(".kiro"),
            Agent::Codex => home.join(".codex"),
            Agent::Windsurf => home.join(".codeium").join("windsurf"),
            Agent::VsCode => home.join(".vscode"),
            Agent::Gemini => home.join(".gemini"),
        }
        .exists()
    }

    /// Where the MCP server is registered, and in what shape. None when the tool has no file for this scope.
    fn mcp_target(self, scope: Scope, home: &Path, project: &Path) -> Option<(PathBuf, Format)> {
        let mcp_servers = Format::Json { key: "mcpServers", typed: false };
        Some(match (self, scope) {
            (Agent::ClaudeCode, Scope::User) => (home.join(".claude.json"), mcp_servers),
            (Agent::ClaudeCode, Scope::Project) => (project.join(".mcp.json"), mcp_servers),
            (Agent::Cursor, Scope::User) => (home.join(".cursor").join("mcp.json"), mcp_servers),
            (Agent::Cursor, Scope::Project) => (project.join(".cursor").join("mcp.json"), mcp_servers),
            (Agent::Kiro, Scope::User) => (home.join(".kiro").join("settings").join("mcp.json"), mcp_servers),
            (Agent::Kiro, Scope::Project) => (project.join(".kiro").join("settings").join("mcp.json"), mcp_servers),
            (Agent::Codex, Scope::User) => (home.join(".codex").join("config.toml"), Format::Toml),
            (Agent::Codex, Scope::Project) => return None,
            (Agent::Windsurf, Scope::User) => (home.join(".codeium").join("windsurf").join("mcp_config.json"), mcp_servers),
            (Agent::Windsurf, Scope::Project) => return None,
            (Agent::VsCode, Scope::Project) => (project.join(".vscode").join("mcp.json"), Format::Json { key: "servers", typed: true }),
            (Agent::VsCode, Scope::User) => return None,
            (Agent::Gemini, Scope::User) => (home.join(".gemini").join("settings.json"), mcp_servers),
            (Agent::Gemini, Scope::Project) => (project.join(".gemini").join("settings.json"), mcp_servers),
        })
    }

    /// The instruction files for this tool and scope. `name` is the server's name, used in file names.
    fn steering_targets(self, scope: Scope, home: &Path, project: &Path, name: &str) -> Vec<(PathBuf, SteeringKind)> {
        match (self, scope) {
            (Agent::ClaudeCode, Scope::User) => vec![
                (home.join(".claude").join("CLAUDE.md"), SteeringKind::Block),
                (home.join(".claude").join("skills").join(name).join("SKILL.md"), SteeringKind::Skill),
            ],
            (Agent::ClaudeCode, Scope::Project) => vec![
                (project.join("CLAUDE.md"), SteeringKind::Block),
                (project.join(".claude").join("skills").join(name).join("SKILL.md"), SteeringKind::Skill),
            ],
            (Agent::Cursor, Scope::Project) => vec![(project.join(".cursor").join("rules").join(format!("{name}.mdc")), SteeringKind::CursorRule)],
            (Agent::Kiro, Scope::User) => vec![(home.join(".kiro").join("steering").join(format!("{name}.md")), SteeringKind::KiroAlways)],
            (Agent::Kiro, Scope::Project) => vec![(project.join(".kiro").join("steering").join(format!("{name}.md")), SteeringKind::KiroAlways)],
            (Agent::Codex, Scope::User) => vec![(home.join(".codex").join("AGENTS.md"), SteeringKind::Block)],
            (Agent::Codex, Scope::Project) => vec![(project.join("AGENTS.md"), SteeringKind::Block)],
            (Agent::Windsurf, Scope::Project) => vec![(project.join(".windsurf").join("rules").join(format!("{name}.md")), SteeringKind::WindsurfRule)],
            (Agent::VsCode, Scope::Project) => vec![(project.join(".github").join("copilot-instructions.md"), SteeringKind::Block)],
            (Agent::Gemini, Scope::User) => vec![(home.join(".gemini").join("GEMINI.md"), SteeringKind::Block)],
            (Agent::Gemini, Scope::Project) => vec![(project.join("GEMINI.md"), SteeringKind::Block)],
            // Cursor's user rules live in its settings UI, Windsurf's global rules in one shared file, and
            // VS Code has no user-level instructions file.
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
enum SteeringKind {
    /// A block between markers, in a file that holds other instructions too.
    Block,
    /// A whole file of our own, in each tool's rule format.
    Skill,
    CursorRule,
    KiroAlways,
    WindsurfRule,
}

fn block_start(name: &str) -> String {
    format!("<!-- {name}:start -->")
}

fn block_end(name: &str) -> String {
    format!("<!-- {name}:end -->")
}

fn change(agent: Agent, what: &str, file: &Path, action: &str, note: Option<String>) -> Change {
    Change { agent: agent.id().into(), what: what.into(), file: file.display().to_string(), action: action.into(), note }
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::User => "user-level",
        Scope::Project => "project-level",
    }
}

/// Registers `server` in each of `agents` and, with `steering`, writes the instructions.
///
/// `home` is the user's home folder (used for [`Scope::User`]) and `project` the project folder (for
/// [`Scope::Project`]). Nothing is written when `options.dry_run` is set.
pub fn install(agents: &[Agent], home: &Path, project: &Path, server: &Server, steering: Option<&Steering>, options: &Options) -> Vec<Change> {
    let mut out = Vec::new();
    for &agent in agents {
        if options.mcp {
            match agent.mcp_target(options.scope, home, project) {
                Some((file, format)) => out.push(write_mcp(agent, &file, format, server, options.dry_run)),
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
            let Some(steering) = steering else {
                out.push(change(agent, "steering", Path::new(""), "skipped", Some("no instructions were given".into())));
                continue;
            };
            let targets = agent.steering_targets(options.scope, home, project, &server.name);
            if targets.is_empty() {
                out.push(change(
                    agent,
                    "steering",
                    Path::new(""),
                    "skipped",
                    Some(format!("{} has no {} instructions file; use the other scope.", agent.label(), scope_name(options.scope))),
                ));
            }
            for (file, kind) in targets {
                out.push(write_steering(agent, &file, kind, &server.name, steering, options.dry_run));
            }
        }
    }
    out
}

// ---- the MCP entry ------------------------------------------------------------------------------

fn server_entry(server: &Server, typed: bool) -> Value {
    let mut entry = Map::new();
    if typed {
        entry.insert("type".into(), json!("stdio"));
    }
    entry.insert("command".into(), json!(server.command));
    entry.insert("args".into(), json!(server.args));
    Value::Object(entry)
}

fn write_mcp(agent: Agent, file: &Path, format: Format, server: &Server, dry_run: bool) -> Change {
    let result = match format {
        Format::Json { key, typed } => merge_json(file, key, &server.name, &server_entry(server, typed)),
        Format::Toml => merge_toml(file, server),
    };
    finish(agent, "mcp", file, result, dry_run, &server.name)
}

/// The new text for a file, or None when it is already right; Err when it must not be touched.
type Planned = Result<Option<String>, String>;

fn merge_json(file: &Path, key: &str, name: &str, entry: &Value) -> Planned {
    let existing = std::fs::read_to_string(file).ok();
    let mut root: Value = match existing.as_deref().map(str::trim) {
        None | Some("") => json!({}),
        Some(text) => serde_json::from_str(text).map_err(|e| format!("it is not valid JSON ({e}), so it was left alone"))?,
    };
    let Value::Object(map) = &mut root else { return Err("it is not a JSON object, so it was left alone".into()) };
    let servers = map.entry(key.to_string()).or_insert_with(|| json!({}));
    let Value::Object(servers) = servers else { return Err(format!("`{key}` is not an object, so the file was left alone")) };
    if servers.get(name) == Some(entry) {
        return Ok(None);
    }
    servers.insert(name.to_string(), entry.clone());
    let mut text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    text.push('\n');
    Ok(Some(text))
}

fn toml_quote(s: &str) -> String {
    // A JSON string is a valid TOML basic string for everything we write.
    serde_json::to_string(s).unwrap_or_default()
}

fn merge_toml(file: &Path, server: &Server) -> Planned {
    let existing = std::fs::read_to_string(file).unwrap_or_default();
    let header = format!("[mcp_servers.{}]", server.name);
    if existing.lines().any(|l| l.trim() == header) {
        return Ok(None);
    }
    let args = server.args.iter().map(|a| toml_quote(a)).collect::<Vec<_>>().join(", ");
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&format!("{header}\ncommand = {}\nargs = [{}]\n", toml_quote(&server.command), args));
    Ok(Some(text))
}

// ---- instructions --------------------------------------------------------------------------------

fn steering_text(kind: SteeringKind, name: &str, steering: &Steering) -> String {
    let body = steering.body.trim_end();
    let description = steering.description.replace('\n', " ");
    match kind {
        SteeringKind::Block => format!("{}\n{body}\n{}\n", block_start(name), block_end(name)),
        SteeringKind::Skill => format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n"),
        SteeringKind::CursorRule => format!("---\ndescription: {description}\nalwaysApply: true\n---\n\n{body}\n"),
        SteeringKind::KiroAlways => format!("---\ninclusion: always\n---\n\n{body}\n"),
        SteeringKind::WindsurfRule => format!("---\ntrigger: always_on\ndescription: {description}\n---\n\n{body}\n"),
    }
}

fn write_steering(agent: Agent, file: &Path, kind: SteeringKind, name: &str, steering: &Steering, dry_run: bool) -> Change {
    let wanted = steering_text(kind, name, steering);
    let existing = std::fs::read_to_string(file).ok();
    let planned: Planned = match kind {
        SteeringKind::Block => Ok(with_block(existing.as_deref().unwrap_or(""), &wanted, name)),
        _ => Ok((existing.as_deref() != Some(wanted.as_str())).then_some(wanted)),
    };
    finish(agent, "steering", file, planned, dry_run, name)
}

/// `text` with our block added, or replaced in place; None when it is already there and current.
fn with_block(text: &str, block: &str, name: &str) -> Option<String> {
    let (start_marker, end_marker) = (block_start(name), block_end(name));
    if let (Some(start), Some(end)) = (text.find(&start_marker), text.find(&end_marker)) {
        if start < end {
            let end = end + end_marker.len();
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

fn finish(agent: Agent, what: &str, file: &Path, planned: Planned, dry_run: bool, name: &str) -> Change {
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
    match write_with_backup(file, &new_text, existed, name) {
        Ok(backup) => change(agent, what, file, action, backup.map(|b| format!("the previous file is kept as {}", b.display()))),
        Err(e) => change(agent, what, file, "skipped", Some(format!("could not write it: {e}"))),
    }
}

fn write_with_backup(file: &Path, text: &str, existed: bool, name: &str) -> std::io::Result<Option<PathBuf>> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut backup = None;
    if existed {
        let mut file_name = file.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        file_name.push(format!(".{name}-backup"));
        let path = file.with_file_name(file_name);
        // Keep the first copy: it is the file as the user had it, not as an earlier run left it.
        if !path.exists() {
            std::fs::copy(file, &path)?;
        }
        backup = Some(path);
    }
    std::fs::write(file, text)?;
    Ok(backup)
}

/// The home folder of the user running the program: `HOME`, or `USERPROFILE` on Windows.
pub fn home_dir() -> Option<PathBuf> {
    let from = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        from("USERPROFILE").or_else(|| from("HOME"))
    } else {
        from("HOME").or_else(|| from("USERPROFILE"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mcp-install-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn server() -> Server {
        Server::new("acme", "/opt/acme/acme-mcp").arg("serve")
    }

    fn steering() -> Steering {
        Steering::new("Use Acme for deployments", "# Use Acme\n\nCall the `acme` MCP tools for deployments, not the CLI.\n")
    }

    fn options(scope: Scope, dry_run: bool) -> Options {
        Options { scope, mcp: true, steering: true, dry_run }
    }

    #[test]
    fn a_project_setup_writes_the_server_and_the_instructions_for_every_tool() {
        let (home, project) = (temp("h1"), temp("p1"));
        let changes = install(&Agent::ALL, &home, &project, &server(), Some(&steering()), &options(Scope::Project, false));
        assert!(changes.iter().all(|c| c.action == "created" || c.action == "skipped"), "{changes:?}");

        let mcp: Value = serde_json::from_str(&std::fs::read_to_string(project.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["acme"], json!({"command": "/opt/acme/acme-mcp", "args": ["serve"]}));
        let vscode: Value = serde_json::from_str(&std::fs::read_to_string(project.join(".vscode/mcp.json")).unwrap()).unwrap();
        assert_eq!(vscode["servers"]["acme"]["type"], "stdio");
        assert!(std::fs::read_to_string(project.join(".cursor/rules/acme.mdc")).unwrap().contains("alwaysApply: true"));
        assert!(std::fs::read_to_string(project.join(".kiro/steering/acme.md")).unwrap().contains("inclusion: always"));
        assert!(std::fs::read_to_string(project.join(".windsurf/rules/acme.md")).unwrap().contains("trigger: always_on"));
        let claude_md = std::fs::read_to_string(project.join("CLAUDE.md")).unwrap();
        assert!(claude_md.contains("<!-- acme:start -->") && claude_md.contains("Call the `acme` MCP tools"));
        let skill = std::fs::read_to_string(project.join(".claude/skills/acme/SKILL.md")).unwrap();
        assert!(skill.starts_with("---\nname: acme\ndescription: Use Acme for deployments\n---"));
        // Codex has no project-level MCP file, and says so.
        assert!(changes.iter().any(|c| c.agent == "codex" && c.what == "mcp" && c.action == "skipped"));
    }

    #[test]
    fn running_it_twice_changes_nothing_the_second_time() {
        let (home, project) = (temp("h2"), temp("p2"));
        install(&Agent::ALL, &home, &project, &server(), Some(&steering()), &options(Scope::User, false));
        let again = install(&Agent::ALL, &home, &project, &server(), Some(&steering()), &options(Scope::User, false));
        assert!(again.iter().all(|c| c.action == "unchanged" || c.action == "skipped"), "{again:?}");
        let toml = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert_eq!(toml.matches("[mcp_servers.acme]").count(), 1);
        assert!(toml.contains("command = \"/opt/acme/acme-mcp\"") && toml.contains("args = [\"serve\"]"), "{toml}");
    }

    #[test]
    fn other_servers_and_settings_are_kept_and_a_copy_is_left() {
        let (home, project) = (temp("h3"), temp("p3"));
        std::fs::write(project.join(".mcp.json"), r#"{"theme": "dark", "mcpServers": {"other": {"command": "x"}}}"#).unwrap();
        let changes = install(&[Agent::ClaudeCode], &home, &project, &server(), None, &Options { steering: false, ..options(Scope::Project, false) });
        assert_eq!(changes[0].action, "updated");
        let merged: Value = serde_json::from_str(&std::fs::read_to_string(project.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(merged["theme"], "dark");
        assert_eq!(merged["mcpServers"]["other"]["command"], "x");
        assert!(merged["mcpServers"]["acme"].is_object());
        let backup = std::fs::read_to_string(project.join(".mcp.json.acme-backup")).unwrap();
        assert!(backup.contains("\"other\"") && !backup.contains("acme"));
    }

    #[test]
    fn a_file_that_is_not_json_is_left_alone() {
        let (home, project) = (temp("h4"), temp("p4"));
        std::fs::write(project.join(".mcp.json"), "{ not json").unwrap();
        let changes = install(&[Agent::ClaudeCode], &home, &project, &server(), None, &Options { steering: false, ..options(Scope::Project, false) });
        assert_eq!(changes[0].action, "skipped");
        assert_eq!(std::fs::read_to_string(project.join(".mcp.json")).unwrap(), "{ not json");
    }

    #[test]
    fn instructions_in_a_shared_file_stay_and_our_block_is_replaced_not_duplicated() {
        let (home, project) = (temp("h5"), temp("p5"));
        std::fs::write(project.join("AGENTS.md"), "# My rules\n\nBe kind.\n").unwrap();
        let only_steering = Options { mcp: false, ..options(Scope::Project, false) };
        install(&[Agent::Codex], &home, &project, &server(), Some(&steering()), &only_steering);
        let first = std::fs::read_to_string(project.join("AGENTS.md")).unwrap();
        assert!(first.starts_with("# My rules\n\nBe kind.\n") && first.contains("<!-- acme:start -->"));
        // an out-of-date block is refreshed in place
        std::fs::write(project.join("AGENTS.md"), first.replace("Use Acme", "OLD TEXT")).unwrap();
        install(&[Agent::Codex], &home, &project, &server(), Some(&steering()), &only_steering);
        let second = std::fs::read_to_string(project.join("AGENTS.md")).unwrap();
        assert_eq!(second.matches("<!-- acme:start -->").count(), 1);
        assert!(second.contains("# Use Acme") && !second.contains("OLD TEXT") && second.contains("Be kind."));
    }

    #[test]
    fn two_servers_do_not_trample_each_others_blocks() {
        let (home, project) = (temp("h8"), temp("p8"));
        let only_steering = Options { mcp: false, ..options(Scope::Project, false) };
        let other = Server::new("beta", "beta-mcp");
        install(&[Agent::Codex], &home, &project, &server(), Some(&steering()), &only_steering);
        install(&[Agent::Codex], &home, &project, &other, Some(&Steering::new("Use Beta", "# Use Beta\n")), &only_steering);
        let text = std::fs::read_to_string(project.join("AGENTS.md")).unwrap();
        assert!(text.contains("<!-- acme:start -->") && text.contains("<!-- beta:start -->"), "{text}");
        assert!(text.contains("# Use Acme") && text.contains("# Use Beta"));
    }

    #[test]
    fn a_dry_run_reports_and_writes_nothing() {
        let (home, project) = (temp("h6"), temp("p6"));
        let changes = install(&Agent::ALL, &home, &project, &server(), Some(&steering()), &options(Scope::Project, true));
        assert!(changes.iter().any(|c| c.action == "created"));
        assert!(std::fs::read_dir(&project).unwrap().next().is_none(), "no file was created");
    }

    #[test]
    fn steering_without_text_is_reported_not_invented() {
        let (home, project) = (temp("h9"), temp("p9"));
        let changes = install(&[Agent::Cursor], &home, &project, &server(), None, &options(Scope::Project, true));
        assert!(changes.iter().any(|c| c.what == "steering" && c.action == "skipped"), "{changes:?}");
    }

    #[test]
    fn tool_names_are_forgiving() {
        assert_eq!(Agent::parse("Claude Code"), Some(Agent::ClaudeCode));
        assert_eq!(Agent::parse("claude"), Some(Agent::ClaudeCode));
        assert_eq!(Agent::parse("VSCODE"), Some(Agent::VsCode));
        assert_eq!(Agent::parse("copilot"), Some(Agent::VsCode));
        assert_eq!(Agent::parse("emacs"), None);
        for agent in Agent::ALL {
            assert_eq!(Agent::parse(agent.id()), Some(agent));
        }
    }

    #[test]
    fn arguments_with_quotes_and_backslashes_stay_valid_toml_and_json() {
        let (home, project) = (temp("h10"), temp("p10"));
        let tricky = Server::new("acme", r"C:\Program Files\Acme\acme.exe").arg(r#"say "hi""#);
        install(&[Agent::Codex], &home, &project, &tricky, None, &Options { steering: false, ..options(Scope::User, false) });
        let toml = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(toml.contains(r#"command = "C:\\Program Files\\Acme\\acme.exe""#), "{toml}");
        assert!(toml.contains(r#"args = ["say \"hi\""]"#), "{toml}");
    }
}
