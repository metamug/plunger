//! A tiny command line over the library, to try it or to use from a script.
//!
//! cargo run -p mcp-install --example add -- --name acme --command acme-mcp --arg serve --dry-run
//! cargo run -p mcp-install --example add -- --name acme --command acme-mcp --scope user cursor kiro

use mcp_install::{home_dir, install, Agent, Options, Scope, Server, Steering};

fn main() {
    let mut name = None;
    let mut command = None;
    let mut args = Vec::new();
    let mut scope = Scope::Project;
    let mut dry_run = false;
    let mut steering_text: Option<String> = None;
    let mut agents = Vec::new();

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--name" => name = it.next(),
            "--command" => command = it.next(),
            "--arg" => args.extend(it.next()),
            "--scope" => {
                scope = match it.next().as_deref() {
                    Some("user") => Scope::User,
                    _ => Scope::Project,
                }
            }
            "--steering" => steering_text = it.next().map(|path| std::fs::read_to_string(&path).expect("a readable steering file")),
            "--dry-run" => dry_run = true,
            other => match Agent::parse(other) {
                Some(agent) => agents.push(agent),
                None => {
                    eprintln!("unknown argument `{other}`");
                    std::process::exit(2);
                }
            },
        }
    }
    let (Some(name), Some(command)) = (name, command) else {
        eprintln!("usage: add --name NAME --command COMMAND [--arg ARG]... [--scope user|project] [--steering FILE] [--dry-run] [tool...]");
        std::process::exit(2);
    };
    if agents.is_empty() {
        agents = Agent::ALL.to_vec();
    }
    let server = Server::new(&name, command).args(args);
    let steering = steering_text.map(|body| Steering::new(format!("Use the {name} MCP server"), body));
    let options = Options { scope, mcp: true, steering: steering.is_some(), dry_run };
    let home = home_dir().expect("a home folder");
    let project = std::env::current_dir().expect("a current folder");
    for change in install(&agents, &home, &project, &server, steering.as_ref(), &options) {
        println!("{:12} {:9} {:9} {}{}", change.agent, change.what, change.action, change.file, change.note.map(|n| format!("  ({n})")).unwrap_or_default());
    }
}
