//! The sync engine: discover Zed's settings file, pull models, merge, write.
//!
//! Design notes worth keeping:
//!
//! * A sync that fails must never damage the existing settings file. Every
//!   failure path leaves the file untouched.
//! * `watch` re-reads settings from disk each cycle, so edits you make in Zed
//!   between cycles are picked up and merged with, not clobbered.
//! * The first successful write backs up the original file once.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use crate::client;
use crate::config::Config;
use crate::jsonc_merge;
use crate::models::ZedModel;
use crate::settings;

/// Resolves which settings file to use.
pub fn resolve_settings_path(config: &Config) -> Result<PathBuf, String> {
    match config.settings_path.clone() {
        Some(path) => Ok(path),
        None => settings::default_settings_path(),
    }
}

/// Result of one sync pass.
#[derive(Debug)]
pub struct SyncReport {
    /// Number of models OmniRoute advertised.
    pub model_count: usize,
    /// Whether the settings file was actually rewritten.
    pub wrote: bool,
    /// Path that was synced.
    pub settings_path: PathBuf,
    /// Whether a one-time backup was created.
    pub backup_created: bool,
    /// Per-category merge counts, for honest logging.
    pub stats: jsonc_merge::MergeStats,
}

/// Performs exactly one sync: fetch, merge, write.
///
/// Returns an error *before* touching disk if the fetch fails, so a stopped
/// gateway can never truncate or empty the settings file.
pub fn sync_once(config: &Config) -> Result<SyncReport, String> {
    let settings_path = resolve_settings_path(config)?;

    let models: Vec<ZedModel> = client::fetch_models(config).map_err(|err| err.to_string())?;
    if models.is_empty() {
        return Err(format!(
            "{} returned no models; refusing to write an empty model list",
            config.models_url()
        ));
    }

    // Re-read every cycle so edits made in Zed between cycles are merged with
    // rather than clobbered.
    let current = settings::read_text(&settings_path)?;

    // The key is only persisted on explicit opt-in: settings.json is plaintext
    // and Zed reads OMNIROUTE_API_KEY from its own environment anyway.
    let api_key = config
        .write_api_key
        .then_some(config.api_key.as_deref())
        .flatten();

    let (merged, stats) = jsonc_merge::merge_into_settings(
        &current,
        &config.provider_id,
        &config.zed_api_url(),
        &models,
        api_key,
    )
    .map_err(|err| format!("{}: {err}", settings_path.display()))?;

    // Back up before the first mutation, so the pristine file is always kept.
    let backup_created = settings::backup_once(&settings_path)?.is_some();
    let wrote = settings::write_atomic(&settings_path, &merged)?;

    Ok(SyncReport {
        model_count: models.len(),
        wrote,
        settings_path,
        backup_created,
        stats,
    })
}

/// Sleeps in one-second ticks so shutdown is never a multi-second hang.
///
/// We deliberately do not install a signal handler: a single long
/// `thread::sleep` is the only thing Ctrl-C would have to interrupt, and the
/// OS default (terminate) already handles that. Ticks exist so the loop can
/// check for a stop condition between them.
pub fn interruptible_sleep(total: Duration) {
    let one_second = Duration::from_secs(1);
    let mut remaining = total;
    while !remaining.is_zero() {
        let step = one_second.min(remaining);
        thread::sleep(step);
        remaining -= step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interruptible_sleep_completes_short_waits() {
        let started = std::time::Instant::now();
        interruptible_sleep(Duration::from_millis(50));
        assert!(started.elapsed() >= Duration::from_millis(40));
    }

    #[test]
    fn default_settings_path_has_a_settings_json_name() {
        let path = settings::default_settings_path().expect("a home directory exists");
        assert!(path.ends_with("settings.json"));
    }
}
