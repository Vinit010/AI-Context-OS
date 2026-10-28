//! The `aicontext-plugin-github` executable.
//!
//! The host starts this binary and speaks the plugin protocol on stdin/stdout. The only option is
//! `--api-base`, which exists for GitHub Enterprise and for loopback tests; the production default
//! is the public API. The binary validates its embedded manifest before it reads a request, so a
//! broken declaration fails before any tool can run.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use aicontext_plugin_github::{GitHubClient, server};
use aicontext_plugin_sdk::PluginManifest;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aicontext-plugin-github: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut api_base: Option<String> = None;
    let mut arguments = std::env::args().skip(1);

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--api-base" => {
                let value = arguments
                    .next()
                    .ok_or("--api-base requires a URL after it")?;
                api_base = Some(value);
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            "--version" | "-V" => {
                println!("aicontext-plugin-github {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => {
                return Err(format!("unknown argument `{other}`; run with --help").into());
            }
        }
    }

    let manifest: PluginManifest = aicontext_plugin_github::manifest()?;
    let max_output_bytes = usize::try_from(manifest.limits.max_output_bytes).unwrap_or(usize::MAX);
    let client = GitHubClient::from_environment(api_base.as_deref())?;

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    server::serve(stdin.lock(), stdout.lock(), &client, max_output_bytes)?;
    Ok(())
}

fn print_help() {
    println!(
        "aicontext-plugin-github {}\n\
         \n\
         A read-only GitHub plugin for AI Context OS. It speaks JSON-RPC 2.0 over stdin/stdout\n\
         and is normally started by the host, not by hand.\n\
         \n\
         Usage:\n\
           aicontext-plugin-github [--api-base URL]\n\
         \n\
         Options:\n\
           --api-base URL  Use a GitHub Enterprise API base or a loopback test server.\n\
                           Default: https://api.github.com\n\
           -h, --help      Show this help.\n\
           -V, --version   Show the version.",
        env!("CARGO_PKG_VERSION")
    );
}
