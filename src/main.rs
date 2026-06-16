use std::io::Read;

use clap::Parser;

use csd::cli::{Cli, Command};
use csd::commands::{approve, kill, ps, run, send, spawn, state};
use csd::{Error, Result};

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
            bypass_permissions,
            yolo,
            trust,
        } => {
            let session = spawn::run(spawn::SpawnArgs {
                cwd,
                session_id,
                permission_mode,
                name,
                backend,
                auto_accept,
                bypass_permissions,
                yolo,
                trust,
                ..Default::default()
            })?;
            print_json(&session);
        }
        Command::Run {
            cwd,
            session_id,
            resume,
            session,
            permission_mode,
            auto_accept,
            bypass_permissions,
            yolo,
            keep,
            timeout,
            approve_plan,
            json,
            prompt,
        } => {
            let outcome = run::run(run::RunArgs {
                cwd,
                session_id,
                resume,
                session,
                permission_mode,
                auto_accept,
                bypass_permissions,
                yolo,
                keep,
                timeout,
                approve_plan,
                prompt: resolve_prompt(prompt)?,
            })?;
            // stdout stays the answer channel: text (or --json everything) on stdout; non-done
            // outcomes go to stderr as JSON so pipes never mistake a question for an answer.
            let code = outcome.exit_code();
            if json {
                print_json(&outcome);
            } else if let run::RunOutcome::Done { text, .. } = &outcome {
                println!("{text}");
            } else {
                eprintln!("{}", serde_json::to_string_pretty(&outcome)?);
            }
            if code != 0 {
                std::process::exit(code);
            }
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

/// Prompt from the positional args, or stdin when none were given (`cat prompt.md | csd run`).
///
/// Stdin is read as raw bytes and lossily decoded (invalid sequences → U+FFFD) rather than
/// requiring strict UTF-8: callers pipe large machine-generated prompts (e.g. life-assistant's
/// ~73K-char committee briefings, which embed news/RSS/financial text and may be byte-truncated
/// mid-codepoint), and `claude -p` tolerates those bytes. A hard "stream did not contain valid
/// UTF-8" error here would fail the whole run over a single stray byte.
fn resolve_prompt(args: Vec<String>) -> Result<String> {
    let prompt = if args.is_empty() {
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf).map_err(|e| Error::Io {
            path: "stdin".into(),
            source: e,
        })?;
        String::from_utf8_lossy(&buf).trim().to_string()
    } else {
        args.join(" ")
    };
    if prompt.is_empty() {
        return Err(Error::EmptyPrompt);
    }
    Ok(prompt)
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
