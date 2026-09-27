//! `aicontext` — the command-line entry point.
//!
//! The CLI is the only crate that assembles dependencies, converts errors into human output, and
//! maps outcomes to exit codes (`ARCHITECTURE.md` §9). Everything it can do is specified in
//! `docs/CLI_SPEC.md`; nothing here may be improvised.
//!
//! # Status
//!
//! Argument parsing and the first command arrive in `TASK-012`. Until then the binary reports
//! honestly rather than pretending to work, which is itself the specified behaviour for a command
//! that is not yet implemented (`docs/CLI_SPEC.md` §3).

#![forbid(unsafe_code)]

/// Exit code for a command that does not exist yet (`docs/CLI_SPEC.md` §5).
const EXIT_NOT_IMPLEMENTED: u8 = 2;

fn main() -> std::process::ExitCode {
    eprintln!(
        "aicontext {}\n\
         \n\
         The command line is specified in docs/CLI_SPEC.md and is not implemented yet.\n\
         \n\
         Next: TASK-011 (core domain types), then TASK-012 (`aicontext init`).\n\
         \n\
         Meanwhile: the project knowledge for this tool is in .ai/, and starts with AI.md.",
        env!("CARGO_PKG_VERSION")
    );
    std::process::ExitCode::from(EXIT_NOT_IMPLEMENTED)
}
