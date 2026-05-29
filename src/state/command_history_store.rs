//! Persisted command history for the `:`-command prompt.
//!
//! Sibling to [`crate::state::read_store`] and
//! [`crate::state::pin_store`]: same atomic-rename-0600 write discipline
//! (via [`crate::state::persist::write_atomic`]), same XDG path
//! resolution. Differs only in shape — command history is an ordered
//! flat `Vec<String>` rather than a `HashMap<StoryId, _>`, so it lives
//! outside [`crate::state::persist::JsonStore`].
//!
//! On save, entries are tail-truncated to [`MAX_ENTRIES`] so a runaway
//! power-user session can't grow the file unboundedly.
//!
//! Failures on load and save are silently swallowed — command history
//! is a convenience, not a correctness boundary.

use crate::state::persist::{write_json_atomic, xdg_data_path};

/// Maximum retained history entries. Matches the user-visible scrollback
/// affordance most shells expose by default; deeper recall isn't useful
/// for a TUI command palette.
pub const MAX_ENTRIES: usize = 100;

const FILE: &str = "commands.json";

/// Loads persisted command history. Empty `Vec` on missing/corrupt file
/// or when no XDG path could be resolved. Thin wrapper over [`load_from`]
/// that resolves the XDG path.
pub fn load() -> Vec<String> {
    match xdg_data_path(FILE) {
        Some(path) => load_from(&path),
        None => Vec::new(),
    }
}

/// Writes `entries` to the XDG history file. No-op when the path can't be
/// resolved (e.g. headless containers with no `HOME`). Thin wrapper over
/// [`save_to`] that resolves the XDG path.
pub fn save(entries: &[String]) {
    if let Some(path) = xdg_data_path(FILE) {
        save_to(entries, &path);
    }
}

/// Writes `entries` to an explicit path via the shared
/// [`write_json_atomic`] discipline (0700 dir, 0600 tmp + rename). Trims to
/// the most recent [`MAX_ENTRIES`] entries — older ones dropped. This is the
/// single write core; [`save`] just resolves the XDG path and delegates here
/// (tests pass an explicit temp path).
pub fn save_to(entries: &[String], path: &std::path::Path) {
    let tail: &[String] = if entries.len() > MAX_ENTRIES {
        &entries[entries.len() - MAX_ENTRIES..]
    } else {
        entries
    };
    let Ok(json) = serde_json::to_string(tail) else {
        return;
    };
    let _ = write_json_atomic(path, &json);
}

/// Reads command history from an explicit path. Empty `Vec` on
/// missing/corrupt file. The single read core; [`load`] resolves the XDG
/// path and delegates here (tests pass an explicit temp path).
pub fn load_from(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hnt_cmdhist_test_{}_{}.json",
            name,
            std::process::id()
        ))
    }

    #[test]
    fn roundtrip_through_disk() {
        let p = tmp("roundtrip");
        let _ = std::fs::remove_file(&p);
        save_to(
            &[
                "quit".to_string(),
                "feed best".to_string(),
                "filter author dang".to_string(),
            ],
            &p,
        );
        let loaded = load_from(&p);
        assert_eq!(
            loaded,
            vec![
                "quit".to_string(),
                "feed best".to_string(),
                "filter author dang".to_string()
            ]
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn corrupt_file_loads_as_empty() {
        let p = tmp("corrupt");
        std::fs::write(&p, "{not valid json").unwrap();
        assert!(load_from(&p).is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn save_trims_to_max_entries() {
        let p = tmp("trim");
        let _ = std::fs::remove_file(&p);
        let many: Vec<String> = (0..MAX_ENTRIES + 50).map(|i| format!("cmd-{i}")).collect();
        save_to(&many, &p);
        let loaded = load_from(&p);
        assert_eq!(loaded.len(), MAX_ENTRIES);
        assert_eq!(loaded.first().map(String::as_str), Some("cmd-50"));
        assert_eq!(
            loaded.last().map(String::as_str),
            Some(format!("cmd-{}", MAX_ENTRIES + 49)).as_deref()
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let p = tmp("missing");
        let _ = std::fs::remove_file(&p);
        assert!(load_from(&p).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn save_creates_file_with_user_only_perms() {
        use std::os::unix::fs::PermissionsExt;
        let p = tmp("perms");
        let _ = std::fs::remove_file(&p);
        save_to(&["secret".to_string()], &p);
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "saved file mode should be 0o600, got {:o}",
            mode
        );
        let _ = std::fs::remove_file(&p);
    }
}
