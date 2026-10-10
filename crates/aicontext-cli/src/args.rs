//! Argument parsing for the whole command tree (`docs/CLI_SPEC.md` §2 and §3).
//!
//! Only the commands that exist — `init`, `status`, `doctor`, `export`, and `import` — are declared
//! to `clap`. The rest of the tree is listed in [`PENDING`] with the task that will provide it, so
//! `--help` shows what exists and what does not, and an unimplemented command exits 2 naming its task
//! instead of a usage error about an unknown word. Declaring the tree as stub subcommands would put
//! a second copy of every command name in this file, and the two copies would drift.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::init::templates::TemplateName;

/// Global flags, accepted before or after the subcommand (`docs/CLI_SPEC.md` §8 puts `--json` after
/// it), and scoped here so they never reach a command that has not asked for them.
///
/// `Default` is what `clap` builds for a command line that passes no global flags, and what the tests
/// start from so each one states only the flag it is about.
// The lint is wrong here: these are independent switches a developer either passes or does not, and
// there is no state to be in. Collapsing them into a state machine would invent combinations no
// invocation can reach.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Args, Default)]
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
    ///
    /// Not declared as conflicting with `--color`: a developer who types both almost always means
    /// this one, and failing the run over it would be pedantry about a preference, not a safety check.
    #[arg(long, global = true)]
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
///
/// `--version` is deliberately absent from this struct and handled from the raw arguments in `main`:
/// it must work without a subcommand, and it prints three lines rather than one, so `clap`'s built-in
/// action is disabled instead of being allowed to answer first.
#[derive(Debug, Parser)]
#[command(
    name = "aicontext",
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
    /// Report where this project is: branch, phase, current task, changes, and pending tasks.
    Status(StatusArgs),
    /// Report what is wrong with this `.ai/` tree and what to do about it. Writes nothing.
    Doctor(DoctorArgs),
    /// Write a deterministic, verifiable archive of `.ai/`.
    Export(ExportArgs),
    /// Read an archive, verify it, and write it into `.ai/`.
    Import(ImportArgs),
}

/// `aicontext status`.
///
/// Reports project name, branch, current phase, current task, modified files, and pending tasks, and
/// degrades gracefully outside a Git repository rather than failing. It takes no flags of its own:
/// everything it answers is already in the register and the working tree.
#[derive(Debug, Args)]
pub(crate) struct StatusArgs {}

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

/// Rejects a blank value at the parser, where a usage error belongs.
///
/// `--only ""` would otherwise be a filter that matches nothing and exits 0, which reads exactly like
/// a clean project. Refusing it at the edge is the only place a developer can still be told they
/// asked the wrong question.
fn code_or_prefix(text: &str) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("give a check code such as CTX-007".to_string());
    }
    Ok(trimmed.to_string())
}

/// `aicontext doctor`.
///
/// Reads the `.ai/` tree, reports what does not line up with `docs/CONTEXT_SPEC.md`, and exits 3 if
/// any finding is an error. It writes nothing: the only version that does is `init`, and a check that
/// repairs itself cannot be trusted to report the damage.
#[derive(Debug, Args)]
pub(crate) struct DoctorArgs {
    /// Treat warnings as errors, so the command fails a pipeline.
    #[arg(long)]
    pub(crate) strict: bool,

    /// Explain one check code and exit, without examining the project.
    #[arg(long, value_name = "CODE", value_parser = code_or_prefix)]
    pub(crate) explain: Option<String>,

    /// Run only the checks whose code starts with this prefix, for example `CTX-01`.
    #[arg(long, value_name = "PREFIX", value_parser = code_or_prefix)]
    pub(crate) only: Option<String>,

    /// Discard the index cache and rebuild it. Accepted in v1 with a report saying it did nothing,
    /// because the cache itself arrives with TASK-031.
    #[arg(long)]
    pub(crate) rebuild_index: bool,
}

impl Default for DoctorArgs {
    /// No flags, which is what every test needs before it sets one.
    fn default() -> Self {
        Self {
            strict: false,
            explain: None,
            only: None,
            rebuild_index: false,
        }
    }
}

impl DoctorArgs {
    /// Whether a code is in `--only`'s selection.
    ///
    /// A prefix rather than an exact code, so `--only CTX-01` can ask for a family of checks at once.
    /// An empty or nonsensical prefix therefore selects nothing and reports nothing: a filter that
    /// matched everything would be a silent way to believe you had narrowed the run.
    pub(crate) fn selects(&self, code: &str) -> bool {
        match self.only.as_deref() {
            None => true,
            Some(prefix) => {
                let prefix = prefix.trim().to_uppercase();
                !prefix.is_empty() && code.to_uppercase().starts_with(&prefix)
            }
        }
    }
}

/// `aicontext export`.
///
/// Writes one deterministic archive of `.ai/`. A relative `<archive>` is resolved against the project
/// root, not the working directory, so `--cwd` and a plain run agree on where it lands.
#[derive(Debug, Args)]
pub(crate) struct ExportArgs {
    /// Where to write the archive.
    #[arg(value_name = "archive")]
    pub(crate) path: PathBuf,

    /// Print what would be archived and write nothing.
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Replace an existing archive. Needs an interactive terminal.
    #[arg(long)]
    pub(crate) force: bool,
}

/// `aicontext import`.
///
/// Verifies the archive's manifest and every per-file digest before writing anything, and refuses
/// rather than overwriting a file that differs unless `--force` is given in an interactive terminal.
#[derive(Debug, Args)]
pub(crate) struct ImportArgs {
    /// The archive to read.
    #[arg(value_name = "archive")]
    pub(crate) path: PathBuf,

    /// Print the plan and write nothing.
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Replace files that differ from the archive. Needs an interactive terminal.
    #[arg(long)]
    pub(crate) force: bool,
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
/// `init`, `status`, `doctor`, `export`, and `import` are deliberately absent: they are implemented. A
/// command here exits 2 with the code below and names its task, which §3 requires — it is never a
/// silent no-op.
pub(crate) const PENDING: &[PendingCommand] = &[
    PendingCommand {
        name: "health",
        task: Some("TASK-038"),
        summary: "Transparent context metrics",
    },
    PendingCommand {
        name: "context",
        task: Some("TASK-037"),
        summary: "Assemble a context packet (show, explain)",
    },
    PendingCommand {
        name: "plan",
        task: None,
        summary: "Produce an implementation plan for a task",
    },
    PendingCommand {
        name: "task",
        task: None,
        summary: "Task management",
    },
    PendingCommand {
        name: "memory",
        task: None,
        summary: "Memory register",
    },
    PendingCommand {
        name: "decision",
        task: None,
        summary: "Architecture decision records",
    },
    PendingCommand {
        name: "bug",
        task: None,
        summary: "Bug memory",
    },
    PendingCommand {
        name: "change",
        task: None,
        summary: "Change history",
    },
    PendingCommand {
        name: "workflow",
        task: None,
        summary: "Run a documented workflow",
    },
    PendingCommand {
        name: "agent",
        task: Some("TASK-055"),
        summary: "Agent profiles and runs (plan mode only)",
    },
    PendingCommand {
        name: "ai",
        task: None,
        summary: "Provider configuration",
    },
    PendingCommand {
        name: "plugin",
        task: Some("TASK-077"),
        summary: "Plugin management",
    },
    PendingCommand {
        name: "connect",
        task: Some("TASK-053"),
        summary: "Configure a provider or integration",
    },
    PendingCommand {
        name: "disconnect",
        task: None,
        summary: "Remove a stored reference",
    },
    PendingCommand {
        name: "audit",
        task: Some("TASK-075"),
        summary: "Audit log (show, verify, tail)",
    },
];

/// Looks up a command name in the pending tree.
pub(crate) fn pending(name: &str) -> Option<&'static PendingCommand> {
    PENDING.iter().find(|command| command.name == name)
}

/// The first argument that is not a global flag or a flag's value, which is the command word.
///
/// This is what makes `aicontext diff --json` report "not implemented, TASK-027" rather than a
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
  status                   Project, branch, phase, current task, changes
                                                             available
  doctor                   Report what is wrong with .ai/ and how to fix it
                                                             available
  export <archive>         Write a deterministic archive of .ai/        available
  import <archive>         Verify an archive and write it into .ai/     available

Planned, not yet built (each exits 2 naming its task in .ai/TASKS.md):
  health                   Transparent context metrics                            TASK-038
  context                  Assemble a context packet (show, explain)             TASK-037
  plan                     Produce an implementation plan for a task
  task | memory | decision | bug | change | workflow
                           Project registers
  agent                    Agent profiles and runs (plan mode only)               TASK-055
  ai                       Provider configuration
  plugin                   Plugin management                                      TASK-077
  connect | disconnect     Configure or remove an integration                     TASK-053
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

doctor flags:
      --strict             Treat warnings as errors
      --explain <code>     Explain one check code and exit; no project is read
      --only <prefix>      Run only checks whose code starts with <prefix>
      --rebuild-index      Accepted in v1; reports that the cache does not exist yet

export / import flags:
      --dry-run            Print the plan; write nothing
      --force              Replace existing files. Needs an interactive terminal

Exit codes are documented in docs/CLI_SPEC.md section 5. A run always prints its exit code.";

#[cfg(test)]
mod tests {
    use super::{
        Cli, ColorChoice, Command, DoctorArgs, GlobalArgs, code_or_prefix, first_command_word,
        pending,
    };
    use clap::Parser;

    fn os(text: &str) -> std::ffi::OsString {
        std::ffi::OsString::from(text)
    }

    fn parse(args: &[&str]) -> GlobalArgs {
        Cli::try_parse_from(args).expect("arguments parse").global
    }

    #[test]
    fn global_flags_are_accepted_after_the_subcommand() {
        let args = parse(&["aicontext", "init", "--json", "--dry-run"]);
        assert!(args.json);
        assert!(!args.offline);
    }

    #[test]
    fn a_flag_that_takes_a_value_does_not_swallow_the_command_word() {
        // The program name is skipped, exactly as `main` skips it.
        let args = vec![
            os("aicontext"),
            os("--color"),
            os("always"),
            os("init"),
            os("--cwd"),
            os("/tmp/x"),
        ];
        assert_eq!(
            first_command_word(args.into_iter().skip(1)).as_deref(),
            Some("init")
        );
    }

    #[test]
    fn a_leading_command_word_is_found_past_the_flags() {
        assert_eq!(
            first_command_word(
                vec![os("aicontext"), os("--json"), os("doctor")]
                    .into_iter()
                    .skip(1)
            )
            .as_deref(),
            Some("doctor")
        );
        assert_eq!(
            first_command_word(vec![os("aicontext")].into_iter().skip(1)),
            None
        );
    }

    #[test]
    fn no_color_wins_over_color() {
        let args = parse(&["aicontext", "init", "--color", "always", "--no-color"]);
        assert_eq!(args.color_choice(), ColorChoice::Never);
        let args = parse(&["aicontext", "init"]);
        assert_eq!(args.color_choice(), ColorChoice::Auto);
    }

    #[test]
    fn every_pending_command_appears_in_the_help_text() {
        for command in super::PENDING {
            assert!(!command.name.is_empty());
            assert!(
                !command.summary.is_empty(),
                "{} has no explanation",
                command.name
            );
            assert!(
                super::HELP.contains(command.name),
                "{} is in the pending tree but missing from --help",
                command.name
            );
        }
        assert!(
            super::HELP.contains("available"),
            "the help must say what works today"
        );
    }

    #[test]
    fn a_pending_command_is_found_by_name() {
        assert_eq!(pending("health").map(|c| c.task), Some(Some("TASK-038")));
        assert!(pending("init").is_none());
        assert!(
            pending("status").is_none(),
            "status is implemented, so it must not also be pending"
        );
        assert!(
            pending("doctor").is_none(),
            "doctor is implemented, so it must not also be pending"
        );
        assert!(
            pending("export").is_none(),
            "export is implemented, so it must not also be pending"
        );
        assert!(
            pending("import").is_none(),
            "import is implemented, so it must not also be pending"
        );
        assert!(pending("nonsense").is_none());
    }

    #[test]
    fn export_and_import_require_an_archive_and_default_to_no_flags() {
        let Cli {
            command: Command::Export(export),
            ..
        } = Cli::try_parse_from(["aicontext", "export", "backup.aix"]).expect("export parses")
        else {
            panic!("expected the export command");
        };
        assert_eq!(export.path, std::path::PathBuf::from("backup.aix"));
        assert!(!export.dry_run && !export.force);

        let Cli {
            command: Command::Import(import),
            ..
        } = Cli::try_parse_from(["aicontext", "import", "backup.aix", "--dry-run", "--force"])
            .expect("import parses")
        else {
            panic!("expected the import command");
        };
        assert_eq!(import.path, std::path::PathBuf::from("backup.aix"));
        assert!(import.dry_run && import.force);

        assert!(
            Cli::try_parse_from(["aicontext", "export"]).is_err(),
            "the archive path is required"
        );
    }

    #[test]
    fn doctor_flags_parse_and_their_defaults_are_off() {
        let Cli {
            command: Command::Doctor(doctor),
            ..
        } = Cli::try_parse_from(["aicontext", "doctor"]).expect("doctor parses")
        else {
            panic!("expected the doctor command");
        };
        assert!(!doctor.strict);
        assert_eq!(doctor.explain, None);
        assert_eq!(doctor.only, None);
        assert!(!doctor.rebuild_index);

        let Cli {
            command:
                Command::Doctor(DoctorArgs {
                    strict,
                    explain,
                    only,
                    rebuild_index,
                }),
            ..
        } = Cli::try_parse_from([
            "aicontext",
            "doctor",
            "--strict",
            "--explain",
            "CTX-007",
            "--only",
            "CTX-01",
            "--rebuild-index",
        ])
        .expect("every doctor flag parses")
        else {
            panic!("expected the doctor command");
        };
        assert!(strict && rebuild_index);
        assert_eq!(explain.as_deref(), Some("CTX-007"));
        assert_eq!(only.as_deref(), Some("CTX-01"));
    }

    #[test]
    fn only_takes_a_prefix_so_a_family_of_checks_can_be_asked_for_at_once() {
        let doctor = |only: &str| DoctorArgs {
            only: Some(only.to_string()),
            ..DoctorArgs::default()
        };
        assert!(
            doctor("CTX-01").selects("CTX-012"),
            "a prefix selects a family, not one code"
        );
        assert!(doctor("CTX-01").selects("CTX-018"));
        assert!(
            !doctor("CTX-01").selects("CTX-007"),
            "CTX-007 does not start with CTX-01"
        );
        assert!(!doctor("CTX-01").selects("CTX-001"));
        assert!(!doctor("CTX-01").selects("PRD-001"));
        assert!(
            doctor("ctx-01").selects("CTX-012"),
            "the catalogue spells codes uppercase; typing them otherwise is the same request"
        );
        assert!(doctor("CTX-0").selects("CTX-001"));
        assert!(doctor("CTX-0").selects("CTX-002"));
        assert!(doctor("CTX-0").selects("CTX-007"));
        assert!(doctor("CTX-0").selects("CTX-012"));
        assert!(doctor("CTX-007").selects("CTX-007"));
        assert!(!doctor("CTX-007").selects("CTX-012"));
        assert!(
            !doctor("  ").selects("CTX-012"),
            "a blank filter selects nothing, rather than everything"
        );
    }

    #[test]
    fn a_blank_code_or_prefix_is_refused_at_the_parser() {
        // `--only ""` would otherwise run nothing and exit 0, which is indistinguishable from a clean
        // project; the parser is the only place that can still say the question was malformed.
        for flag in ["--only", "--explain"] {
            assert!(
                Cli::try_parse_from(["aicontext", "doctor", flag, "   "]).is_err(),
                "{flag} \"\" must be a usage error"
            );
        }
        assert_eq!(
            code_or_prefix(" CTX-007 ").unwrap(),
            "CTX-007",
            "surrounding whitespace is trimmed rather than treated as part of the code"
        );
        assert!(code_or_prefix(" ").is_err());
    }
}
