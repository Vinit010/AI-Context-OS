//! `aicontext plugin list|install|remove`: the local plugin registry.
//!
//! `docs/PLUGIN_SPEC.md` §8 fixes the order for `install`: validate the manifest, show the
//! permission diff (requested against already granted), show the auth requirements, require
//! confirmation unless `--yes` was passed, then record the result in the local lockfile. This
//! module is that order. `list` reads the record, and `remove` takes one entry out of it, deletes
//! the per-plugin state tree, and drops the lockfile when the last plugin goes.
//!
//! # What the grant means
//!
//! The lockfile records what the developer confirmed at install. It is not a bypass of
//! `.ai/permissions/permissions.yaml`: every tool call is still resolved through the permission
//! resolver, and the policy may narrow the manifest's outer jail. The lockfile answers "what did
//! this checkout install and what did the developer agree to?", not "may this call proceed?".
//!
//! # Why the diff is shown before the prompt
//!
//! A human cannot approve a permission they have not seen. The preview is printed first, on stdout
//! in human mode and on stderr under `--json` (where stdout is the machine object), and the prompt
//! follows it. Only the literal answer `yes` approves; every other answer, an absent terminal, and
//! an unreadable stdin are denials (`PLUGIN-004`, exit 5). `--yes` is the one non-interactive
//! confirmation, and it is accepted here because installing a plugin is not itself a tool call.
//!
//! # Deliberate limits
//!
//! This build installs local paths only. A registry reference is refused by name (`PLUGIN-003`)
//! rather than guessed at, because there is no registry client and no signature verifier. Every
//! local install therefore displays `unsigned`, never `trusted`. `minimum_aicontext_version` is
//! recorded and shown but not resolved: the workspace version and the canonical fixture disagree,
//! and inventing a range comparison without a SemVer dependency would be a second opinion about a
//! question this task does not own.

mod error;
mod lockfile;
mod manifest;

use std::path::Path;

use aicontext_plugin_runtime::approval::{APPROVAL_ANSWER, PromptError, Prompter};
use serde::Serialize;

use crate::approval::TerminalPrompter;
use crate::args::{GlobalArgs, PluginArgs, PluginCommand, PluginInstallArgs, PluginRemoveArgs};
use crate::exit::Exit;
use crate::output::{Terminal, envelope};
use crate::plugin::error::PluginCommandError;
use crate::plugin::lockfile::{
    InstalledPlugin, LOCKFILE_DISPLAY, Lockfile, Permission, PermissionDiff,
};
use crate::plugin::manifest::ValidatedManifest;
use crate::project::{self, ProjectError};

/// Runs `aicontext plugin` and returns the exit code.
pub(crate) fn run(args: &PluginArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    match &args.command {
        PluginCommand::List => run_list(global, terminal),
        PluginCommand::Install(install) => {
            let mut prompter = TerminalPrompter::process();
            run_install(install, global, terminal, &mut prompter)
        }
        PluginCommand::Remove(remove) => run_remove(remove, global, terminal),
    }
}

/// `aicontext plugin list`.
fn run_list(global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return PluginCommandError::from(error).report(terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return PluginCommandError::from(error).report(terminal),
    };

    match Lockfile::load(&root) {
        Ok(lockfile) => {
            if global.json {
                report_list_json(&lockfile, terminal)
            } else {
                print_list_human(&lockfile, &name, &root, terminal);
                Exit::Ok
            }
        }
        Err(error) => error.report(terminal),
    }
}

/// `aicontext plugin install <path>`.
fn run_install(
    args: &PluginInstallArgs,
    global: &GlobalArgs,
    terminal: &mut Terminal,
    prompter: &mut dyn Prompter,
) -> Exit {
    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return PluginCommandError::from(error).report(terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return PluginCommandError::from(error).report(terminal),
    };

    if !global.json {
        terminal.identity("aicontext", &name, project::vcs_label(&root));
    }

    match execute_install(args, global.yes, &root, terminal, global.json, prompter) {
        Ok(outcome) => {
            if global.json {
                report_install_json(&outcome, terminal)
            } else {
                print_install_human(&outcome, terminal);
                Exit::Ok
            }
        }
        Err(error) => error.report(terminal),
    }
}

/// `aicontext plugin remove <name>`.
fn run_remove(args: &PluginRemoveArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return PluginCommandError::from(error).report(terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return PluginCommandError::from(error).report(terminal),
    };

    match execute_remove(&args.name, &root) {
        Ok(outcome) => {
            if global.json {
                report_remove_json(&outcome, terminal)
            } else {
                print_remove_human(&outcome, &name, &root, terminal);
                Exit::Ok
            }
        }
        Err(error) => error.report(terminal),
    }
}

/// What an install would do, assembled before a human is asked.
struct InstallPreview {
    /// The manifest that passed every check.
    validated: ValidatedManifest,
    /// The entry already recorded for this name, if any.
    existing: Option<InstalledPlugin>,
    /// Requested against already granted.
    diff: PermissionDiff,
}

/// The result of a confirmed install.
struct InstallOutcome {
    /// The entry written to the lockfile.
    plugin: InstalledPlugin,
    /// What the install added, removed, and left unchanged.
    diff: PermissionDiff,
}

/// The result of a remove.
struct RemoveOutcome {
    /// The entry that was taken out of the lockfile.
    plugin: InstalledPlugin,
    /// The permissions that are no longer granted.
    revoked: Vec<Permission>,
    /// Whether a per-plugin state tree existed and was deleted.
    state_removed: bool,
    /// How many plugins remain recorded.
    remaining: usize,
}

/// Validate the manifest, load the record, and build the diff without writing anything.
fn prepare_install(root: &Path, reference: &str) -> Result<InstallPreview, PluginCommandError> {
    require_ai_tree(root)?;
    let validated = ValidatedManifest::load(root, reference)?;
    let lockfile = Lockfile::load(root)?;
    let existing = lockfile.find(&validated.manifest.name).cloned();
    let granted = existing
        .as_ref()
        .map_or(&[][..], |plugin| plugin.permissions.as_slice());
    let diff = PermissionDiff::between(granted, &validated.permissions);
    Ok(InstallPreview {
        validated,
        existing,
        diff,
    })
}

/// The whole install: preview, confirm, then write the manifest copy and the record.
fn execute_install(
    args: &PluginInstallArgs,
    yes: bool,
    root: &Path,
    terminal: &mut Terminal,
    json: bool,
    prompter: &mut dyn Prompter,
) -> Result<InstallOutcome, PluginCommandError> {
    let preview = prepare_install(root, &args.path)?;
    print_preview(&preview, terminal, json);

    if !yes {
        let name = &preview.validated.manifest.name;
        let prompt = format!(
            "\nInstall `{name}` and record these permissions? Type `yes` to confirm: "
        );
        match prompter.ask(&prompt) {
            Ok(answer) if answer.trim() == APPROVAL_ANSWER => {}
            Ok(_) => {
                return Err(PluginCommandError::NotConfirmed { name: name.clone() });
            }
            Err(PromptError::Unavailable) => {
                return Err(PluginCommandError::ConfirmationUnavailable { name: name.clone() });
            }
            Err(_) => {
                return Err(PluginCommandError::NotConfirmed { name: name.clone() });
            }
        }
    }

    let plugin = preview.validated.installed_plugin();
    lockfile::write_state(root, &plugin.name, &preview.validated.bytes)?;

    // Reload after the prompt: the preview may have been on screen for a while, and another
    // process may have changed the record in the meantime. The last writer still records one
    // consistent entry rather than an in-memory copy of a file that no longer exists.
    let mut record = Lockfile::load(root)?;
    record.upsert(plugin.clone());
    record.save(root)?;

    Ok(InstallOutcome {
        plugin,
        diff: preview.diff,
    })
}

/// Remove the record, delete the state tree, and drop the lockfile when it becomes empty.
fn execute_remove(name: &str, root: &Path) -> Result<RemoveOutcome, PluginCommandError> {
    let mut record = Lockfile::load(root)?;
    let plugin = record
        .remove(name)
        .ok_or_else(|| PluginCommandError::UnknownPlugin {
            name: name.to_owned(),
            known: record.known_names(),
        })?;

    // Delete the state before the record. If deletion fails, the grant is still recorded and a
    // second `remove` can finish the job; the recoverable direction is to keep the record.
    let state_removed = lockfile::remove_state(root, name)?;
    let remaining = record.plugins.len();
    if remaining == 0 {
        Lockfile::delete(root)?;
    } else {
        record.save(root)?;
    }

    Ok(RemoveOutcome {
        revoked: plugin.permissions.clone(),
        plugin,
        state_removed,
        remaining,
    })
}

/// Refuse a root with no `.ai/` tree: a plugin record outside a project would be local state with
/// no project to belong to.
fn require_ai_tree(root: &Path) -> Result<(), PluginCommandError> {
    if root.join(".ai").is_dir() {
        return Ok(());
    }
    Err(PluginCommandError::Root(ProjectError::UnusableRoot {
        path: project::display(root),
        reason: "it has no `.ai/` directory yet; run `aicontext init` first".to_owned(),
    }))
}

/// Print the install preview, on stdout in human mode and on stderr under `--json`.
fn print_preview(preview: &InstallPreview, terminal: &mut Terminal, json: bool) {
    for line in preview_lines(preview) {
        if json {
            terminal.note(&line);
        } else {
            terminal.say(&line);
        }
    }
}

/// The preview as plain lines, so the human and JSON paths show exactly the same facts.
fn preview_lines(preview: &InstallPreview) -> Vec<String> {
    let manifest = &preview.validated.manifest;
    let mut lines = vec![
        format!("plugin `{}` {}", manifest.name, manifest.version),
        format!("  source      {}", preview.validated.source),
        format!("  signature   {}", "unsigned"),
        format!("  api         {}", manifest.plugin_api_version),
    ];
    if let Some(description) = &manifest.description {
        lines.push(format!("  description {description}"));
    }
    if let Some(minimum) = &manifest.minimum_aicontext_version {
        lines.push(format!(
            "  requires    aicontext {minimum} (not checked by this build)"
        ));
    }
    lines.push(format!("  status      {}", preview_status(preview)));
    lines.push(String::new());
    lines.push(format!("permissions ({})", preview.diff.summary()));
    for permission in &preview.diff.added {
        lines.push(format!("  + {permission}"));
    }
    for permission in &preview.diff.removed {
        lines.push(format!("  - {permission}"));
    }
    for permission in &preview.diff.unchanged {
        lines.push(format!("  = {permission}"));
    }
    if preview.diff.is_empty() {
        lines.push("  no changes to the grant".to_owned());
    }

    lines.push(String::new());
    let auth = preview.validated.auth_lines();
    if auth.is_empty() {
        lines.push("auth: none".to_owned());
    } else {
        lines.push("auth requirements:".to_owned());
        for line in auth {
            lines.push(format!("  {line}"));
        }
    }
    lines
}

/// Whether this is a new install, a reinstall, or an upgrade.
fn preview_status(preview: &InstallPreview) -> String {
    let version = &preview.validated.manifest.version;
    match &preview.existing {
        None => "new install".to_owned(),
        Some(existing) if existing.version == *version => format!("reinstall {version}"),
        Some(existing) => format!("upgrade {} -> {version}", existing.version),
    }
}

/// Print the human install result.
fn print_install_human(outcome: &InstallOutcome, terminal: &mut Terminal) {
    let ok = terminal.mark_ok();
    let info = terminal.mark_info();
    terminal.row(
        &ok,
        "installed",
        &format!("`{}` {}", outcome.plugin.name, outcome.plugin.version),
    );
    if !outcome.diff.removed.is_empty() {
        terminal.row(
            &info,
            "revoked",
            &format!(
                "{} permission(s) no longer requested",
                outcome.diff.removed.len()
            ),
        );
    }
    terminal.row(&info, "lockfile", LOCKFILE_DISPLAY);
    terminal.row(
        &info,
        "state",
        &format!(".aicontext/plugins/{}/manifest.json", outcome.plugin.name),
    );
    terminal.summary(
        &format!(
            "{} permission(s) granted",
            outcome.plugin.permissions.len()
        ),
        Exit::Ok,
        "`aicontext plugin list`",
    );
}

/// Print the human list.
fn print_list_human(lockfile: &Lockfile, name: &str, root: &Path, terminal: &mut Terminal) {
    let ok = terminal.mark_ok();
    let info = terminal.mark_info();
    terminal.identity("aicontext", name, project::vcs_label(root));
    if lockfile.plugins.is_empty() {
        terminal.row(&info, "plugins", "none installed");
    } else {
        for plugin in &lockfile.plugins {
            terminal.row(
                &ok,
                &plugin.name,
                &format!(
                    "{}  {}  {} permission(s)",
                    plugin.version,
                    plugin.signature,
                    plugin.permissions.len()
                ),
            );
        }
    }
    let counts = match lockfile.plugins.len() {
        0 => "no plugins installed".to_owned(),
        1 => "1 plugin installed".to_owned(),
        count => format!("{count} plugins installed"),
    };
    terminal.summary(
        &counts,
        Exit::Ok,
        "`aicontext plugin install <path>`",
    );
}

/// Print the human remove result.
fn print_remove_human(
    outcome: &RemoveOutcome,
    name: &str,
    root: &Path,
    terminal: &mut Terminal,
) {
    let ok = terminal.mark_ok();
    let info = terminal.mark_info();
    terminal.identity("aicontext", name, project::vcs_label(root));
    terminal.row(
        &ok,
        "removed",
        &format!("`{}` {}", outcome.plugin.name, outcome.plugin.version),
    );
    terminal.row(
        &info,
        "revoked",
        &format!("{} permission(s)", outcome.revoked.len()),
    );
    if outcome.state_removed {
        terminal.row(
            &ok,
            "state",
            &format!(".aicontext/plugins/{} deleted", outcome.plugin.name),
        );
    } else {
        terminal.row(&info, "state", "no local state remained");
    }
    let counts = match outcome.remaining {
        0 => "no plugins remain".to_owned(),
        1 => "1 plugin remains".to_owned(),
        count => format!("{count} plugins remain"),
    };
    terminal.summary(&counts, Exit::Ok, "`aicontext plugin list`");
}

/// The `--json` payload for `list`.
#[derive(Serialize)]
struct ListData<'a> {
    action: &'static str,
    lockfile: &'static str,
    plugins: &'a [InstalledPlugin],
}

/// The `--json` payload for `install`.
#[derive(Serialize)]
struct InstallData<'a> {
    action: &'static str,
    lockfile: &'static str,
    plugin: &'a InstalledPlugin,
    diff: &'a PermissionDiff,
}

/// The `--json` payload for `remove`.
#[derive(Serialize)]
struct RemoveData<'a> {
    action: &'static str,
    lockfile: &'static str,
    plugin: &'a InstalledPlugin,
    revoked: &'a [Permission],
    state_removed: bool,
    remaining: usize,
}

/// Report `list` as one JSON object.
fn report_list_json(lockfile: &Lockfile, terminal: &mut Terminal) -> Exit {
    let data = ListData {
        action: "list",
        lockfile: LOCKFILE_DISPLAY,
        plugins: &lockfile.plugins,
    };
    let summary = match lockfile.plugins.len() {
        0 => "no plugins installed".to_owned(),
        1 => "1 plugin installed".to_owned(),
        count => format!("{count} plugins installed"),
    };
    report_json(data, summary, terminal)
}

/// Report `install` as one JSON object.
fn report_install_json(outcome: &InstallOutcome, terminal: &mut Terminal) -> Exit {
    let data = InstallData {
        action: "install",
        lockfile: LOCKFILE_DISPLAY,
        plugin: &outcome.plugin,
        diff: &outcome.diff,
    };
    let summary = format!(
        "installed `{}` {} with {} permission(s)",
        outcome.plugin.name,
        outcome.plugin.version,
        outcome.plugin.permissions.len()
    );
    report_json(data, summary, terminal)
}

/// Report `remove` as one JSON object.
fn report_remove_json(outcome: &RemoveOutcome, terminal: &mut Terminal) -> Exit {
    let data = RemoveData {
        action: "remove",
        lockfile: LOCKFILE_DISPLAY,
        plugin: &outcome.plugin,
        revoked: &outcome.revoked,
        state_removed: outcome.state_removed,
        remaining: outcome.remaining,
    };
    let summary = format!(
        "removed `{}` and revoked {} permission(s)",
        outcome.plugin.name,
        outcome.revoked.len()
    );
    report_json(data, summary, terminal)
}

/// Write one envelope to stdout, or report that even that failed.
fn report_json<T: Serialize>(data: T, summary: String, terminal: &mut Terminal) -> Exit {
    let body = envelope("plugin", Exit::Ok.code(), data, Vec::new(), summary);
    match terminal.json(&body) {
        Ok(()) => Exit::Ok,
        Err(error) => PluginCommandError::JsonReport {
            detail: error.to_string(),
        }
        .report(terminal),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use aicontext_plugin_runtime::approval::{PromptError, Prompter};
    use tempfile::TempDir;

    use super::{
        Lockfile, execute_install, execute_remove, prepare_install,
    };
    use crate::args::PluginInstallArgs;
    use crate::output::Terminal;

    const MANIFEST_ONE: &str = r#"{
  "manifest_version": 1,
  "plugin_api_version": "1",
  "name": "demo",
  "version": "1.0.0",
  "entry": { "command": "demo-plugin" },
  "capabilities": [{ "tool": "demo.items.read", "effect": "read" }],
  "filesystem": { "read": [], "write": [] },
  "network": { "egress": [] },
  "limits": { "call_timeout_ms": 1000, "max_output_bytes": 1024 }
}"#;

    const MANIFEST_TWO: &str = r#"{
  "manifest_version": 1,
  "plugin_api_version": "1",
  "name": "demo",
  "version": "1.1.0",
  "entry": { "command": "demo-plugin" },
  "capabilities": [{ "tool": "demo.items.write", "effect": "write" }],
  "filesystem": { "read": [], "write": [] },
  "network": { "egress": [] },
  "limits": { "call_timeout_ms": 1000, "max_output_bytes": 1024 }
}"#;

    enum Answer {
        Yes,
        No,
        Unavailable,
    }

    struct FakePrompter {
        answer: Answer,
    }

    impl Prompter for FakePrompter {
        fn ask(&mut self, _prompt: &str) -> Result<String, PromptError> {
            match self.answer {
                Answer::Yes => Ok("yes\n".to_owned()),
                Answer::No => Ok("no\n".to_owned()),
                Answer::Unavailable => Err(PromptError::Unavailable),
            }
        }
    }

    fn project() -> TempDir {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir_all(root.path().join(".ai")).expect("create .ai");
        root
    }

    fn write_plugin(root: &Path, text: &str) -> PathBuf {
        let directory = root.join("plugin");
        fs::create_dir_all(&directory).expect("create plugin");
        fs::write(directory.join("manifest.json"), text).expect("write manifest");
        directory
    }

    fn args() -> PluginInstallArgs {
        PluginInstallArgs {
            path: "plugin".to_owned(),
        }
    }

    fn terminal() -> Terminal {
        Terminal::sink(
            Box::new(Vec::new()),
            Box::new(Vec::new()),
            false,
            crate::args::ColorChoice::Never,
        )
    }

    #[test]
    fn a_confirmed_install_writes_the_manifest_copy_and_the_record() {
        let root = project();
        write_plugin(root.path(), MANIFEST_ONE);
        let mut terminal = terminal();
        let mut prompter = FakePrompter {
            answer: Answer::Yes,
        };

        let outcome = execute_install(
            &args(),
            false,
            root.path(),
            &mut terminal,
            false,
            &mut prompter,
        )
        .expect("installed");

        assert_eq!(outcome.plugin.name, "demo");
        assert_eq!(outcome.diff.added.len(), 1);
        assert!(
            root.path()
                .join(".aicontext/plugins/demo/manifest.json")
                .is_file()
        );
        let record = Lockfile::load(root.path()).expect("record");
        assert_eq!(record.plugins.len(), 1);
        assert_eq!(record.plugins[0].version, "1.0.0");
        assert_eq!(record.plugins[0].signature.as_str(), "unsigned");
    }

    #[test]
    fn yes_skips_the_prompt_and_a_denial_writes_nothing() {
        let root = project();
        write_plugin(root.path(), MANIFEST_ONE);
        let mut terminal = terminal();

        let mut unavailable = FakePrompter {
            answer: Answer::Unavailable,
        };
        execute_install(
            &args(),
            true,
            root.path(),
            &mut terminal,
            false,
            &mut unavailable,
        )
        .expect("--yes does not ask");

        let denied_root = project();
        write_plugin(denied_root.path(), MANIFEST_ONE);
        let mut denied = FakePrompter { answer: Answer::No };
        let error = execute_install(
            &args(),
            false,
            denied_root.path(),
            &mut terminal,
            false,
            &mut denied,
        )
        .expect_err("a denial is not an install");
        assert_eq!(error.code(), "PLUGIN-004");
        assert!(!denied_root.path().join(".aicontext").exists());
    }

    #[test]
    fn a_missing_terminal_is_an_approval_failure() {
        let root = project();
        write_plugin(root.path(), MANIFEST_ONE);
        let mut terminal = terminal();
        let mut prompter = FakePrompter {
            answer: Answer::Unavailable,
        };

        let error = execute_install(
            &args(),
            false,
            root.path(),
            &mut terminal,
            false,
            &mut prompter,
        )
        .expect_err("a pipe is not a human");

        assert_eq!(error.code(), "PLUGIN-004");
        assert_eq!(error.exit().code(), 5);
        assert!(!root.path().join(".aicontext").exists());
    }

    #[test]
    fn a_reinstall_reports_the_permission_that_was_added_and_removed() {
        let root = project();
        write_plugin(root.path(), MANIFEST_ONE);
        let mut terminal = terminal();
        let mut yes = FakePrompter {
            answer: Answer::Yes,
        };
        execute_install(
            &args(),
            true,
            root.path(),
            &mut terminal,
            false,
            &mut yes,
        )
        .expect("first install");

        write_plugin(root.path(), MANIFEST_TWO);
        let outcome = execute_install(
            &args(),
            true,
            root.path(),
            &mut terminal,
            false,
            &mut yes,
        )
        .expect("second install");

        assert_eq!(outcome.diff.added.len(), 1);
        assert_eq!(outcome.diff.removed.len(), 1);
        assert!(outcome.diff.unchanged.is_empty());
        let record = Lockfile::load(root.path()).expect("record");
        assert_eq!(record.plugins[0].version, "1.1.0");
        assert_eq!(record.plugins[0].permissions.len(), 1);
        assert!(record.plugins[0]
            .permissions
            .iter()
            .any(|permission| matches!(
                permission,
                crate::plugin::lockfile::Permission::Tool { tool, .. }
                    if tool == "demo.items.write"
            )));
    }

    #[test]
    fn remove_deletes_the_state_tree_and_drops_the_lockfile_when_empty() {
        let root = project();
        write_plugin(root.path(), MANIFEST_ONE);
        let mut terminal = terminal();
        execute_install(
            &args(),
            true,
            root.path(),
            &mut terminal,
            false,
            &mut FakePrompter {
                answer: Answer::Unavailable,
            },
        )
        .expect("install");

        let outcome = execute_remove("demo", root.path()).expect("remove");
        assert_eq!(outcome.revoked.len(), 1);
        assert!(outcome.state_removed);
        assert_eq!(outcome.remaining, 0);
        assert!(!root.path().join(".aicontext/plugins/demo").exists());
        assert!(!Lockfile::path(root.path()).exists());

        let error = execute_remove("demo", root.path()).expect_err("gone");
        assert_eq!(error.code(), "PLUGIN-010");
    }

    #[test]
    fn a_remove_with_no_state_tree_still_drops_the_record() {
        let root = project();
        write_plugin(root.path(), MANIFEST_ONE);
        let mut terminal = terminal();
        execute_install(
            &args(),
            true,
            root.path(),
            &mut terminal,
            false,
            &mut FakePrompter {
                answer: Answer::Unavailable,
            },
        )
        .expect("install");
        fs::remove_dir_all(root.path().join(".aicontext/plugins/demo")).expect("delete by hand");

        let outcome = execute_remove("demo", root.path()).expect("remove");
        assert!(!outcome.state_removed);
        assert!(!Lockfile::path(root.path()).exists());
    }

    #[test]
    fn install_requires_an_ai_tree() {
        let root = TempDir::new().expect("temp dir");
        write_plugin(root.path(), MANIFEST_ONE);
        let error = prepare_install(root.path(), "plugin").expect_err("no .ai");
        assert_eq!(error.code(), "PROJ-001");
    }
}
