//! `aicontext` — the command-line entry point.
//!
//! The CLI is the only crate that assembles dependencies, converts errors into human output, and
//! maps outcomes to exit codes (`ARCHITECTURE.md` §9). Everything it can do is specified in
//! `docs/CLI_SPEC.md`; nothing here may be improvised.
//!
//! # What exists today
//!
//! `init` (`TASK-012`) and the global flags. Every other command in the §3 tree is listed in
//! [`args::PENDING`] and exits 2 naming the task that will build it, which is what §3 requires of a
//! command that is not implemented yet — a named refusal rather than an unknown-word usage error.
//!
//! The command word is read from the raw arguments *before* clap validates them, so
//! `aicontext doctor --json` reports "not implemented, TASK-014" rather than complaining about the
//! flag: the developer is told what is missing before being told what else they typed.

#![forbid(unsafe_code)]

mod args;
mod exit;
mod init;
mod output;

use std::process::ExitCode;

use args::{Cli, Command, GlobalArgs, PendingCommand, first_command_word, pending};
use clap::Parser;
use exit::Exit;
use output::{Terminal, Verbosity, envelope};

fn main() -> ExitCode {
    let raw: Vec<_> = std::env::args_os().skip(1).collect();
    let json = raw.iter().any(|arg| arg == "--json");

    // Read the command word before parsing: an unimplemented command must be refused on its own
    // terms rather than through clap's usage error.
    if raw.iter().any(|arg| arg == "--version" || arg == "-V") {
        return report_version();
    }
    if let Some(word) = first_command_word(raw) {
        if let Some(pending) = pending(&word) {
            return report_pending(pending, json);
        }
    }

    let cli = Cli::parse();
    let mut terminal = Terminal::process(verbosity(&cli.global), cli.global.color_choice());

    let exit = match &cli.command {
        Command::Init(init_args) => init::run(init_args, &cli.global, &mut terminal),
    };

    terminal.flush();
    ExitCode::from(exit.code())
}

/// How much of the human report to print.
fn verbosity(global: &GlobalArgs) -> Verbosity {
    if global.quiet {
        Verbosity::Quiet
    } else {
        Verbosity::Normal
    }
}

/// `--version` reports the three things a bug report needs: the version, the plugin API version, and
/// the build profile (`docs/CLI_SPEC.md` §3.6).
fn report_version() -> ExitCode {
    println!(
        "aicontext {}\nplugin-api {}\nprofile {}",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_VERSION"),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    ExitCode::from(Exit::Ok.code())
}

/// A command that is specified but not built: exit 2 naming the task, in human or JSON form.
fn report_pending(command: &PendingCommand, json: bool) -> ExitCode {
    let task = command.task.unwrap_or("not scheduled yet");
    if json {
        let body = envelope(
            command.name,
            Exit::Usage.code(),
            serde_json::json!({ "task": task, "summary": command.summary }),
            Vec::new(),
            format!("{} is not implemented yet ({task})", command.name),
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| String::from("{}"))
        );
        return ExitCode::from(Exit::Usage.code());
    }
    eprintln!("aicontext {} is not implemented yet ({task})", command.name);
    eprintln!("  does: {}", command.summary);
    eprintln!("  try: `aicontext --help` for what works today, or .ai/TASKS.md for the task order");
    ExitCode::from(Exit::Usage.code())
}
