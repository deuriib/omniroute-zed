//! `omniroute-zed` — wire OmniRoute into Zed.
//!
//! Zed extensions cannot register LLM providers (the extension surface is
//! Languages, Debuggers, Themes, Icon Themes, Snippets, MCP Servers and Agent
//! Servers), and a WASM guest has no timers, so the provider entry and the
//! 5-minute refresh are done here instead: a small host binary that talks to
//! OmniRoute's OpenAI-compatible `/v1` API and merges the result into Zed's
//! `settings.json`.
//!
//! Two pieces of Zed behaviour make this work with no custom provider code:
//!
//! * `language_models.openai_compatible.<id>` accepts an arbitrary base URL,
//!   and Zed appends `/chat/completions` to it, so we write the `/v1` root.
//! * Zed derives the API-key env var from the provider id as
//!   `format!("{}_API_KEY", id).to_upper_snake()`, so the provider id
//!   `omniroute` means Zed reads `OMNIROUTE_API_KEY` on its own.

mod client;
mod config;
mod jsonc_merge;
mod models;
mod settings;
mod sync;

use std::time::Duration;

use clap::{Parser, Subcommand};
use config::{Config, DEFAULT_BASE_URL, DEFAULT_PROVIDER_ID, DEFAULT_SYNC_INTERVAL_SECS};

/// Sync OmniRoute models into Zed's settings.json.
#[derive(Debug, Parser)]
#[command(name = "omniroute-zed", version, about, long_about = None)]
struct Cli {
    /// OmniRoute base URL.
    #[arg(long, env = "OMNIROUTE_BASE_URL", default_value = DEFAULT_BASE_URL)]
    base_url: String,

    /// Zed provider id. Zed reads `<ID>_API_KEY` (upper snake) for the key,
    /// so the default `omniroute` means `OMNIROUTE_API_KEY`.
    #[arg(long, env = "OMNIROUTE_ZED_PROVIDER_ID", default_value = DEFAULT_PROVIDER_ID)]
    provider_id: String,

    /// Path to Zed's settings.json. Defaults to the per-user config path.
    #[arg(long, env = "OMNIROUTE_ZED_SETTINGS")]
    settings: Option<std::path::PathBuf>,

    /// Write OMNIROUTE_API_KEY into Zed's settings.json as a
    /// `custom_headers` entry. Off by default: Zed already reads the env var,
    /// and settings.json is plaintext.
    #[arg(long)]
    write_api_key: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Fetch models once and update Zed's settings.json.
    Sync,
    /// Sync on a loop, refreshing every interval (default 5 minutes).
    Watch {
        /// Seconds between syncs.
        #[arg(long, env = "OMNIROUTE_SYNC_INTERVAL", default_value_t = DEFAULT_SYNC_INTERVAL_SECS)]
        interval: u64,
    },
    /// Check OmniRoute and report the model count without writing anything.
    Status,
}

/// Builds the resolved config from CLI input.
fn build_config(cli: &Cli, interval: Duration) -> Result<Config, String> {
    let base_url = config::normalize_base_url(&cli.base_url)?;
    let api_key = std::env::var("OMNIROUTE_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());

    Ok(Config {
        base_url,
        sync_interval: interval,
        provider_id: cli.provider_id.clone(),
        api_key,
        settings_path: cli.settings.clone(),
        write_api_key: cli.write_api_key,
    })
}

/// Human-readable one-line summary of a sync.
fn report_sync(config: &Config, result: &sync::SyncReport) {
    let destination = if result.wrote {
        format!("updated {}", result.settings_path.display())
    } else {
        format!("already up to date ({})", result.settings_path.display())
    };

    println!(
        "{} models -> {} [{} curated kept, {} added]",
        result.model_count, destination, result.stats.preserved, result.stats.added
    );

    if result.backup_created {
        println!("  backup written to {}.bak", result.settings_path.display());
    }

    if config.write_api_key && config.api_key.is_some() {
        println!("  api_key + MCP bearer header written to settings.json (plaintext)");
    } else if config.api_key.is_none() {
        println!(
            "  OMNIROUTE_API_KEY not set in this shell. Zed still reads it from\n  \
             its own environment at runtime; the syncer only needs it to fetch\n  \
             /v1/models."
        );
    } else {
        println!(
            "  no secrets written to settings.json. Zed reads OMNIROUTE_API_KEY\n  \
             from its own environment; pass --write-api-key to persist it here."
        );
    }
}

fn run_sync(config: &Config) -> Result<(), String> {
    match sync::sync_once(config) {
        Ok(result) => {
            report_sync(config, &result);
            Ok(())
        }
        Err(error) => {
            eprintln!("sync failed: {error}");
            eprintln!("settings.json was left untouched.");
            Err(error)
        }
    }
}

/// The watch loop. Runs until interrupted.
///
/// Ctrl-C is left to the OS default (terminate the process). Sub-minute
/// ticks keep the sleep interruptible so shutdown is never a multi-second
/// hang, and the loop re-reads settings from disk each pass so edits made in
/// Zed between cycles are merged rather than clobbered.
fn run_watch(config: &Config) -> Result<(), String> {
    println!(
        "watching {} every {}s — Ctrl-C to stop",
        config.models_url(),
        config.sync_interval.as_secs()
    );

    loop {
        if let Err(_error) = run_sync(config) {
            // Keep going: a stopped gateway should not end the watch.
            eprintln!("  will retry in {}s", config.sync_interval.as_secs());
        }
        sync::interruptible_sleep(config.sync_interval);
    }
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    let interval = match &cli.command {
        Command::Watch { interval } => match config::parse_interval_secs(&interval.to_string()) {
            Ok(seconds) => Duration::from_secs(seconds),
            Err(error) => {
                eprintln!("{error}");
                return std::process::ExitCode::from(2);
            }
        },
        _ => Duration::from_secs(DEFAULT_SYNC_INTERVAL_SECS),
    };

    let config = match build_config(&cli, interval) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::from(2);
        }
    };

    let outcome = match &cli.command {
        Command::Sync => run_sync(&config),
        Command::Watch { .. } => run_watch(&config),
        Command::Status => match client::probe(&config) {
            Ok(count) => {
                println!("{} is reachable — {count} models", config.models_url());
                Ok(())
            }
            Err(error) => {
                eprintln!("{} is unreachable: {error}", config.models_url());
                Err(error.to_string())
            }
        },
    };

    match outcome {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::FAILURE,
    }
}
