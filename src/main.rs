use clap::Parser;

use csd::cli::{Cli, Command};
use csd::commands::{approve, kill, ps, send, spawn, state};
use csd::Result;

fn main() {
    let cli = Cli::parse();
    if let Err(error) = run(cli) {
        // Structured error on stderr so stdout stays clean for JSON consumers.
        eprintln!("{}", serde_json::json!({ "error": error.to_string() }));
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Spawn {
            cwd,
            session_id,
            permission_mode,
            name,
            backend,
            auto_accept,
            trust,
        } => {
            let session = spawn::run(spawn::SpawnArgs {
                cwd,
                session_id,
                permission_mode,
                name,
                backend,
                auto_accept,
                trust,
                ..Default::default()
            })?;
            print_json(&session);
        }
        Command::Send {
            session,
            prompt,
            no_submit,
            retries,
        } => {
            let result = send::run(send::SendArgs {
                session,
                prompt: prompt.join(" "),
                submit: !no_submit,
                retries,
            })?;
            print_json(&result);
        }
        Command::State { session } => {
            print_json(&state::run(&session)?);
        }
        Command::Approve { session, option } => {
            print_json(&approve::run(&session, option)?);
        }
        Command::Ps { json } => {
            let result = ps::run()?;
            if json {
                print_json(&result);
            } else {
                print_ps_table(&result);
            }
        }
        Command::Kill { session } => {
            print_json(&kill::run(&session)?);
        }
    }
    Ok(())
}

fn print_json<T: serde::Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("{}", serde_json::json!({ "error": e.to_string() })),
    }
}

fn print_ps_table(result: &ps::PsResult) {
    if result.sessions.is_empty() {
        println!("no tracked sessions");
        return;
    }
    println!("{:<28} {:<6} {:<14} CWD", "NAME", "ALIVE", "STATUS");
    for entry in &result.sessions {
        let status = serde_json::to_value(&entry.state)
            .ok()
            .and_then(|v| v.get("status").and_then(|s| s.as_str()).map(String::from))
            .unwrap_or_else(|| "unknown".to_string());
        println!(
            "{:<28} {:<6} {:<14} {}",
            entry.session.name, entry.alive, status, entry.session.cwd
        );
    }
}
