//! Argument parsing for the whole command tree (`docs/CLI_SPEC.md` §2 and §3).
//!
//! Only `init` is declared to `clap`. The rest of the tree is listed in [`PENDING`] with the task
//! that will provide it, so `--help` shows what exists and what does not, and an unimplemented
//! command exits 2 naming its task instead of a usage error about an unknown word. Declaring the
//! tree as stub subcommands would put a second copy of every command name in this file, and the two
//! copies would drift.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, ValueEnum};

use crate::init::templates::TemplateName;

/// Global flags, accepted before or after the subcommand (`docs/CLI_SPEC.md` §8 puts `--json` after
/// it), and scoped here so they never reach a command that has not asked for them.
#[derive(Debug, Args)]
pub(crate) struct GlobalArgs {
    /// Emit a single JSON object to stdout; all human text goes to stderr.
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// Errors only.
    #[arg(long, global = true, conflicts_with = "verbose")]
    pub(crate) quiet: bool,

    /// Diagnostics to stderr. Repeatable: `-vv` adds trace.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub(crate) verbose: u8,

    /// Disable colour. Also honoured: `NO_COLOR`, `AICON_TEXT_NO_COLOR`, a non-TTY stdout.
    #[arg(long, global = true, conflicts_with = "color")]
    pub(crate) no_color: bool,

    /// Force colour behaviour instead of auto-detecting it.
    #[arg(long, global = true, value_enum)]
    pub(crate) color: Option<ColorChoice>,

    /// Operate as if started in this directory.
    #[arg(long, global = true, value_name = "path")]
    pub(crate) cwd: Option<PathBuf>,

    /// Use this configuration file.
    #[arg(long, global = true, value_name = "path")]
    pub(crate) config: Option<PathBuf>,

    /// Fail rather than make any network call.
    #[arg(long, global = true)]
    pub(crate) offline: bool,

    /// Confirm non-destructive prompts. Never satisfies an explicit-approval prompt.
    #[arg(long, global = true)]
    pub(crate) yes: bool,
}

impl GlobalArgs {
    /// The colour choice after `--no-color` is taken into account.
    pub(crate) fn color_choice(&self) -> ColorChoice {
        if self.no_color {
            ColorChoice::Never
        } else {
            self.color.unwrap_or(ColorChoice::Auto)
        }
    }
}

/// When ANSI colour may be emitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ColorChoice {
    /// Emit colour only on a terminal, and honour the opt-out environment variables.
    Auto,
    /// Emit colour even into a pipe.
    Always,
    /// Never emit colour.
    Never,
}

/// The parsed command line.
#[derive(Debug, Parser)]
#[command(
    name = "aicontext",
    version,
    disable_version_flag = true,
    override_help = HELP,
    subcommand_required = true,
    arg_required_else_help = true
)]
pub(crate) struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub(crate) command: Command,
    /// Flags that apply to every command.
    #[command(flatten)]
    pub(crate) global: GlobalArgs,
}

/// The commands that exist today.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Create the `.ai` skeleton. Writes inside `.ai/` and one `.gitignore` entry, nothing else.
    Init(InitArgs),
}

/// `aicontext init`.
///
/// Creates the `.ai/` skeleton, adds `.aicontext/` to `.gitignore`, validates what it wrote, and
/// prints the next commands. It is idempotent: a document you have edited is never overwritten
/// unless you ask for it with `--force`, which needs a terminal to confirm in.
#[derive(Debug, Args)]
pub(crate) struct InitArgs {
    /// Print the plan and write nothing.
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Overwrite documents that differ from the template. Needs an interactive terminal.
    #[arg(long)]
    pub(crate) force: bool,

    /// Skip project discovery, so no stack hints are reported.
    #[arg(long)]
    pub(crate) no_detect: bool,

    /// Which skeleton to start from.
    #[arg(long, value_enum, default_value_t = TemplateName::Default)]
    pub(crate) template: TemplateName,
}

/// A command in the §3 tree that is specified but not built yet.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingCommand {
    /// The word as typed, for example `doctor`.
    pub(crate) name: &'static str,
    /// The task that will implement it, or `None` when the register has not scheduled it.
    pub(crate) task: Option<&'static str>,
    /// One line of help text.
    pub(crate) summary: &'static str,
}

/// Every command from `docs/CLI_SPEC.md` §3 that this binary does not implement yet.
///
/// `init` is deliberately absent: it is implemented. A command here exits 2 with the code below and
/// names its task, which §3 requires — it is never a silent no-op.
pub(crate) const PENDING: &[PendingCommand] = &[
    PendingCommand { name: "status", task: Some("TASK-013"), summary: "Project, branch, phase, current task, changes" },
    PendingCommand { name: "doctor", task: Some("TASK-014"), summary: "Validate and diagnose context" },
    PendingCommand { name: "health", task: Some("TASK-038"), summary: "Transparent context metrics" },
    PendingCommand { name: "context", task: Some("TASK-037"), summary: "Assemble a context packet (show, explain)" },
    PendingCommand { name: "plan", task: None, summary: "Produce an implementation plan for a task" },
    PendingCommand { name: "task", task: None, summary: "Task management" },
    PendingCommand { name: "memory", task: None, summary: "Memory register" },
    PendingCommand { name: "decision", task: None, summary: "Architecture decision records" },
    PendingCommand { name: "bug", task: None, summary: "Bug memory" },
    PendingCommand { name: "change", task: None, summary: "Change history" },
    PendingCommand { name: "workflow", task: None, summary: "Run a documented workflow" },
    PendingCommand { name: "agent", task: Some("TASK-055"), summary: "Agent profiles and runs (plan mode only)" },
    PendingCommand { name: "ai", task: None, summary: "Provider configuration" },
    PendingCommand { name: "plugin", task: Some("TASK-077"), summary: "Plugin management" },
    PendingCommand { name: "connect", task: Some("TASK-053"), summary: "Configure a provider or integration" },
    PendingCommand { name: "disconnect", task: None, summary: "Remove a stored reference" },
    PendingCommand { name: "export", task: Some("TASK-020"), summary: "Export `.ai` as a portable archive" },
    PendingCommand { name: "import", task: Some("TASK-020"), summary: "Import and verify an archive" },
    PendingCommand { name: "audit", task: Some("TASK-075"), summary: "Audit log (show, verify, tail)" },
];

/// Looks up a command name in the pending tree.
pub(crate) fn pending(name: &str) -> Option<&'static PendingCommand> {
    PENDING.iter().find(|command| command.name == name)
}

/// The first argument that is not a global flag or a flag's value, which is the command word.
///
/// This is what makes `aicontext doctor --json` report "not implemented, TASK-014" rather than a
/// complaint about the flag: the command is read before the flags are validated.
pub(crate) fn first_command_word<I: IntoIterator<Item = OsString>>(args: I) -> Option<String> {
    let mut expecting_value = false;
    for arg in args {
        let text = arg.to_string_lossy().into_owned();
        if expecting_value {
            expecting_value = false;
            continue;
        }
        if text == "--cwd" || text == "--config" || text == "--color" {
            expecting_value = true;
            continue;
        }
        if text.starts_with('-') {
            continue;
        }
        return Some(text);
    }
    None
}

const HELP: &str = "\
AI Context OS — git-native context, memory, governance, and tool integration

Usage: aicontext <command> [flags]

Commands:
  init                     Create the .ai skeleton                     available

Planned, not yet built (each exits 2 naming its task in .ai/TASKS.md):
  status                   Project, branch, phase, current task, changes        TASK-013
  doctor                   Validate and diagnose context                         TASK-014
  health                   Transparent context metrics                            TASK-038
  context                  Assemble a context packet (show, explain)             TASK-037
  plan                     Produce an implementation plan for a task
  task | memory | decision | bug | change | workflow
                           Project registers
  agent                    Agent profiles and runs (plan mode only)               TASK-055
  ai                       Provider configuration
  plugin                   Plugin management                                      TASK-077
  connect | disconnect     Configure or remove an integration                     TASK-053
  export | import          Portable .ai archives                                  TASK-020
  audit                    Audit log (show, verify, tail)                         TASK-075

Flags:
      --json               Single JSON object on stdout; human text to stderr
      --quiet              Errors only
  -v, --verbose            Diagnostics on stderr; repeatable
      --no-color           Disable colour (NO_COLOR and a non-TTY are honoured)
      --color <when>       auto | always | never
      --cwd <path>         Operate as if started in <path>
      --config <path>      Use a specific configuration file
      --offline            Fail rather than make any network call
      --yes                Confirm non-destructive prompts; never an explicit approval
      --version            Version, plugin API version, and build profile
  -h, --help               This text

init flags:
      --dry-run            Print the plan; write nothing
      --force              Overwrite edited documents. Needs an interactive terminal
      --no-detect          Skip project discovery
      --template <name>    default | rust | node | python | blank

Exit codes are documented in docs/CLI_SPEC.md section 5. A run always prints its exit code.";

#[cfg(test)]
mod tests {
    use super::{ColorChoice, GlobalArgs, first_command_word, pending};
    use clap::Parser;

    fn parse(args: &[&str]) -> GlobalArgs {
        Cli::try_parse_from(args)
            .expect("arguments parse")
            .global
    }

    #[test]
    fn global_flags_are_accepted_after_the_subcommand() {
        let args = parse(&["aicontext", "init", "--json", "--dry-run"]);
        assert!(args.json);
        assert!(!args.offline);
    }

    #[test]
    fn a_flag_that_takes_a_value_does_not_swallow_the_command_word() {
        let args = vec![
            os("aicontext"),
            os("--color"),
            os("always"),
            os("init"),
            os("--cwd"),
            os("/tmp/x"),
        ];
        assert_eq!(first_command_word(args).as_deref(), Some("init"));
    }

    #[test]
    fn a_leading_command_word_is_found_past_the_flags() {
        assert_eq!(
            first_command_word(vec![os("aicontext"), os("--json"), os("doctor")]).as_deref(),
            Some("doctor")
        );
        assert_eq!(first_command_word(vec![os("aicontext")]), None);
    }

    #[test]
    fn no_color_wins_over_color() {
        let args = parse(&["aicontext", "init", "--color", "always", "--no-color"]);
        assert_eq!(args.color_choice(), ColorChoice::Never);
        let args = parse(&["aicontext", "init"]);
        assert_eq!(args.color_choice(), ColorChoice::Auto);
    }

    #[test]
    fn every_planned_command_names_its_task_or_says_it_is_unscheduled() {
        for command in super::PENDING {
            assert!(!command.name.is_empty());
            assert!(!command.summary.is_empty());
            assert!(
                command.task.is_some() || command.summary.contains("register"),
                "{} has no task and no explanation",
                command.name
            );
        }
    }

    #[test]
    fn a_pending_command_is_found_by_name() {
        assert_eq!(pending("doctor").map(|c| c.task), Some(Some("TASK-014")));
        assert!(pending("init").is_none());
        assert!(pending("nonsense").is_none());
    }

    fn os(text: &str) -> std::ffi::OsString {
        std::ffi::OsString::from(text)
    }
}