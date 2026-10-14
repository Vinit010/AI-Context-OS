//! `aicontext context`: assemble one context packet for a task or a query, and print it.
//!
//! The command is the join between the retrieval, budget, and assembly the context crate provides and
//! the process contract in `docs/CLI_SPEC.md` §4: it resolves the project, reads the task register
//! when `--task` seeds the plan, scans (or reuses) the index cache, assembles, and renders. It writes
//! no document; the only file it may touch is the rebuildable, git-ignored index cache in
//! `.aicontext/`.
//!
//! # Output
//!
//! Under the global `--json` — or the equivalent `--format json` — stdout is the standard envelope
//! whose `data` is the `ContextPacket` itself, exactly the shape `docs/CONTEXT_SPEC.md` §7.6
//! publishes. `--format prompt` writes provider input, with every retrieved document wrapped in an
//! untrusted-data envelope, and nothing else. The default `--format text` writes the human manifest.
//!
//! # Exit
//!
//! `0` for a packet; `3` when assembly cannot fit the never-dropped tiers (`CTX-030`); `2` for a root
//! that cannot be used, as every command reports it.

use std::fmt::Write as _;
use std::path::Path;

use aicontext_context::{
    ContextAssembler, ContextBudget, ContextIndex, ContextPacket, INDEX_PATH, PlanOptions,
    RetrievalPlan, RetrievalTrace,
};

use crate::args::{ContextArgs, GlobalArgs, OutputFormat};
use crate::exit::Exit;
use crate::output::{Finding, Terminal, envelope};
use crate::project::{self, ProjectError};

/// Runs `context` and returns the exit code.
pub(crate) fn run(args: &ContextArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return report_root_failure(&error, terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return report_root_failure(&error, terminal),
    };

    let mut findings = Vec::new();
    let index = read_index(&root, &mut findings);
    let register = aicontext_context::register::read(&root);

    let options = plan_options(args);
    let plan = match &args.task {
        Some(id) => RetrievalPlan::for_task(&register, id, &options),
        None => RetrievalPlan::new(&options),
    };

    let packet = match ContextAssembler::new().assemble(&plan, &index) {
        Ok(packet) => packet,
        Err(error) => {
            terminal.error_report(
                &error.code().to_string(),
                &error.to_string(),
                error.remediation(),
            );
            return Exit::Validation;
        }
    };

    // The global `--json` is the machine contract, so it wins over a contradictory `--format prompt`.
    if global.json || args.format == OutputFormat::Json {
        let body = envelope(
            "context",
            Exit::Ok.code(),
            &packet,
            findings,
            summary_line(&packet),
        );
        if let Err(error) = terminal.json(&body) {
            terminal.error_report(
                "CONTEXT-001",
                "the JSON envelope could not be written",
                &format!("{error}; check that stdout is a pipe rather than a full device"),
            );
            return Exit::General;
        }
    } else if args.format == OutputFormat::Prompt {
        terminal.say(packet.render_prompt(&index).trim_end());
    } else {
        print_human(&name, &root, &plan, &packet, args, terminal);
    }
    Exit::Ok
}

/// Builds the plan options from the command's flags.
fn plan_options(args: &ContextArgs) -> PlanOptions {
    PlanOptions {
        query: args.query.clone(),
        include: args.include.clone(),
        exclude: args.exclude.clone(),
        budget: args
            .budget
            .map(|tokens| ContextBudget::default().with_max_tokens(tokens)),
        ..PlanOptions::default()
    }
}

/// Reads the index cache, scanning the tree and writing the cache back so the next run is warm.
///
/// A cache that cannot be written is reported, not fatal: the packet is built from the scan either
/// way, and a read-only checkout is a directory a developer can still assemble context in.
fn read_index(root: &Path, findings: &mut Vec<Finding>) -> ContextIndex {
    let cache = root.join(INDEX_PATH);
    let previous = ContextIndex::load(&cache).ok();
    let (index, _report) = ContextIndex::build(root, previous.as_ref());
    if let Err(error) = index.save(&cache) {
        findings.push(Finding::new(
            "CTX-019",
            "info",
            INDEX_PATH,
            format!("the index cache could not be written: {error}"),
            "check that the project directory is writable; the packet is still assembled from a scan",
        ));
    }
    index
}

/// Prints the human manifest: the identity line, the packet, and the summary.
fn print_human(
    name: &str,
    root: &Path,
    plan: &RetrievalPlan,
    packet: &ContextPacket,
    args: &ContextArgs,
    terminal: &mut Terminal,
) {
    terminal.identity("aicontext", name, project::vcs_label(root));

    if args.explain {
        terminal.say(plan.explain().trim_end());
        terminal.say("");
    }
    terminal.say(manifest(packet, args.explain).trim_end());
    terminal.summary(&summary_line(packet), Exit::Ok, "`aicontext doctor`");
}

/// The packet as a human manifest; `explain` adds every provenance trace, not just the included ones.
fn manifest(packet: &ContextPacket, explain: bool) -> String {
    let mut out = String::new();

    let task = packet.request.task.as_deref().unwrap_or("(none)");
    let instruction = packet.request.instruction.as_deref().unwrap_or("(none)");
    let _ = writeln!(out, "request:   task {task}");
    let _ = writeln!(out, "           query {instruction}");

    let _ = writeln!(
        out,
        "policy:    {} document(s), tiers 1-2, always kept",
        packet.policy.len()
    );
    for item in &packet.policy {
        let _ = writeln!(
            out,
            "  {:<16} tier {}  {}",
            item.id,
            item.tier,
            item.content_ref.display()
        );
    }

    let _ = writeln!(
        out,
        "retrieved: {} document(s), untrusted data",
        packet.retrieved.len()
    );
    for item in &packet.retrieved {
        let _ = writeln!(
            out,
            "  {}  tier {}  score {:.2}  {}",
            item.id,
            item.tier,
            item.score,
            item.content_ref.display()
        );
        let _ = writeln!(out, "      why: {}", item.reason);
    }

    if !packet.conflicts.is_empty() {
        let _ = writeln!(out, "conflicts: {}", packet.conflicts.len());
        for conflict in &packet.conflicts {
            let _ = writeln!(
                out,
                "  {}: {} (tier {}) vs {} (tier {})",
                conflict.key,
                conflict.asserted_by.id,
                conflict.asserted_by.tier,
                conflict.conflicts_with.id,
                conflict.conflicts_with.tier
            );
        }
    }

    let budget = &packet.budget;
    let _ = writeln!(
        out,
        "budget:    requested {}, used {}, {} dropped",
        budget.requested,
        budget.used,
        budget.dropped.len()
    );
    for dropped in &budget.dropped {
        let _ = writeln!(
            out,
            "  {}  tier {}  {}",
            dropped.id, dropped.tier, dropped.reason
        );
    }

    if explain {
        let _ = writeln!(out, "provenance: {}", packet.provenance.len());
        for trace in &packet.provenance {
            let _ = writeln!(out, "  {}", trace_line(trace));
        }
    } else {
        let _ = writeln!(
            out,
            "provenance: {} candidate(s); run with --explain for every trace",
            packet.provenance.len()
        );
    }

    out
}

/// One provenance trace line.
fn trace_line(trace: &RetrievalTrace) -> String {
    let dropped = match &trace.dropped_reason {
        Some(reason) => format!(" | dropped: {reason}"),
        None => String::new(),
    };
    format!(
        "[{}] {}  tier {}  score {:.2}  {}{}",
        if trace.included { "kept" } else { "out" },
        trace.id,
        trace.tier,
        trace.score,
        trace.source_path.display(),
        dropped
    )
}

/// The closing counts line.
fn summary_line(packet: &ContextPacket) -> String {
    format!(
        "{} policy, {} retrieved, {} conflict(s), {} dropped",
        packet.policy.len(),
        packet.retrieved.len(),
        packet.conflicts.len(),
        packet.budget.dropped.len()
    )
}

/// A root that cannot be used, reported through the surface every other command reports through.
fn report_root_failure(error: &ProjectError, terminal: &mut Terminal) -> Exit {
    terminal.error_report(error.code(), &error.to_string(), error.remediation());
    error.exit()
}
