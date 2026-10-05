//! Locating, backing up and atomically rewriting Zed's `settings.json`.
//!
//! Zed's settings file is **JSONC**, not strict JSON: real files carry
//! comments and trailing commas. This module therefore treats the file as
//! opaque text and hands it to [`crate::jsonc_merge`] for editing; nothing
//! here parses it. That keeps a strict-JSON bug from silently eating a user's
//! comments.
//!
//! Two safety properties hold regardless of how the merge goes:
//!
//! 1. **Never destructive** — a failed merge leaves the file untouched.
//! 2. **Atomic** — we write a temp file in the same directory and rename over
//!    the original, so a crash mid-write cannot truncate it.
//!
//! 3. **Quiet when unchanged** — if the rendered text equals what is on disk we
//!    skip the write entirely, because Zed watches this file and an
//!    unconditional write would trigger pointless reloads every cycle.

use std::path::{Path, PathBuf};

/// Locates Zed's `settings.json` across platforms.
///
/// Zed stores settings per-user in the OS config directory:
/// * Windows: `%APPDATA%\Zed\settings.json`
/// * macOS:   `~/Library/Application Support/Zed/settings.json`
/// * Linux:   `~/.config/zed/settings.json`
pub fn default_settings_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "could not determine home directory".to_string())?;

    if cfg!(target_os = "windows") {
        if let Some(app_data) = std::env::var_os("APPDATA") {
            return Ok(PathBuf::from(app_data).join("Zed").join("settings.json"));
        }
        return Ok(home
            .join("AppData")
            .join("Roaming")
            .join("Zed")
            .join("settings.json"));
    }

    if cfg!(target_os = "macos") {
        return Ok(home
            .join("Library")
            .join("Application Support")
            .join("Zed")
            .join("settings.json"));
    }

    Ok(home.join(".config").join("zed").join("settings.json"))
}

/// Reads a settings file as text, returning `""` when it does not exist.
///
/// Reading is deliberately total: an unreadable or missing file yields the
/// empty string so a first run can create a fresh file, while the merge layer
/// reports parse failures for content that is present but malformed.
pub fn read_text(path: &Path) -> Result<String, String> {
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(path).map_err(|err| format!("could not read {}: {err}", path.display()))
}

/// Writes `contents` to `path` atomically, creating parent directories.
///
/// Returns whether the file was actually rewritten. An unchanged file is left
/// alone so Zed does not reload needlessly.
pub fn write_atomic(path: &Path, contents: &str) -> Result<bool, String> {
    if let Ok(existing) = read_text(path)
        && existing == contents
    {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("could not create {}: {err}", parent.display()))?;
    }

    // Same directory as the target, so the rename cannot cross filesystems.
    let temp = path.with_extension("json.omniroute-tmp");
    std::fs::write(&temp, contents)
        .map_err(|err| format!("could not write {}: {err}", temp.display()))?;

    // rename over an existing file is atomic on Windows and POSIX alike.
    if let Err(err) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("could not replace {}: {err}", path.display()));
    }
    Ok(true)
}

/// Backs the file up to `<path>.bak`, but only if that backup does not exist.
///
/// Keeping exactly one backup preserves the *pristine* original. Rotating it
/// on every run would replace the only untouched copy after the first sync.
pub fn backup_once(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let backup = path.with_extension("json.bak");
    if backup.exists() {
        return Ok(None);
    }
    std::fs::copy(path, &backup)
        .map_err(|err| format!("could not back up {}: {err}", path.display()))?;
    Ok(Some(backup))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_back_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");

        assert_eq!(read_text(&path).expect("missing reads empty"), "");
        assert!(write_atomic(&path, "{}\n").expect("first write"));
        assert_eq!(read_text(&path).expect("read"), "{}\n");
    }

    #[test]
    fn skips_the_write_when_content_is_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");

        assert!(write_atomic(&path, "a").expect("first write"));
        assert!(!write_atomic(&path, "a").expect("identical content"));
        assert!(write_atomic(&path, "b").expect("changed content"));
    }

    #[test]
    fn leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        write_atomic(&path, "content").expect("write");

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("readdir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "settings.json")
            .collect();
        assert!(
            leftovers.is_empty(),
            "unexpected files left behind: {leftovers:?}"
        );
    }

    #[test]
    fn backs_up_once_and_keeps_the_original() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        write_atomic(&path, "original").expect("write");

        let first = backup_once(&path).expect("backup");
        assert!(first.is_some(), "first call must create a backup");

        // A second call must not clobber the pristine copy.
        write_atomic(&path, "modified").expect("write");
        assert!(backup_once(&path).expect("second backup").is_none());

        let backup = path.with_extension("json.bak");
        assert_eq!(read_text(&backup).expect("read backup"), "original");
    }

    #[test]
    fn backup_is_skipped_for_a_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        assert!(backup_once(&path).expect("no file").is_none());
    }
}
