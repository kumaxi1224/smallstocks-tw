//! The Shioaji API key pair for the auto-started sidecar, stored on this
//! machine only. The keys live in their own file inside the data folder
//! (`shioaji-keys.json`) — deliberately NOT in the core's Saved/state.json,
//! so they never enter a snapshot, an export, or the automatic state
//! backups (those all travel through state.json; this file does not).
//! The only surfaces are the UI-only Tauri commands in lib.rs: set, clear,
//! and a status that reports which keys exist as booleans. The key values
//! themselves are read in exactly one place — lib.rs's ensure_sidecar,
//! which hands them to the spawned sidecar through its environment,
//! exactly like run-sidecar.bat does with its prompts.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const KEYS_FILE: &str = "shioaji-keys.json";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarKeys {
    pub api_key: String,
    pub secret_key: String,
}

pub fn keys_path(dir: &Path) -> PathBuf {
    dir.join(KEYS_FILE)
}

/// The stored key pair, or None when the file is missing, unreadable, or
/// either key is blank. A half-configured file counts as unconfigured:
/// the sidecar cannot log in with one key alone.
pub fn load(dir: &Path) -> Option<SidecarKeys> {
    let text = std::fs::read_to_string(keys_path(dir)).ok()?;
    let keys: SidecarKeys = serde_json::from_str(&text).ok()?;
    if keys.api_key.trim().is_empty() || keys.secret_key.trim().is_empty() {
        return None;
    }
    Some(keys)
}

/// Which keys exist, per key, without ever exposing a value. A missing or
/// unreadable file reports both unset; a file holding only one key
/// reports that one set (so the settings page can say precisely what is
/// missing), even though `load` treats the pair as unconfigured.
pub fn status(dir: &Path) -> (bool, bool) {
    let Ok(text) = std::fs::read_to_string(keys_path(dir)) else {
        return (false, false);
    };
    let Ok(keys) = serde_json::from_str::<SidecarKeys>(&text) else {
        return (false, false);
    };
    (
        !keys.api_key.trim().is_empty(),
        !keys.secret_key.trim().is_empty(),
    )
}

/// The status exactly as the `shioaji_keys_status` command returns it:
/// two booleans and nothing else — there is no masked or partial form of
/// a key anywhere in this module's output.
pub fn status_json(dir: &Path) -> Value {
    let (api_key_set, secret_key_set) = status(dir);
    json!({"apiKeySet": api_key_set, "secretKeySet": secret_key_set})
}

/// Store the pair. Both keys are required and stored trimmed; an empty
/// side is refused here so a blank field can never half-wipe the file
/// (clearing is `clear`'s explicit job).
pub fn save(dir: &Path, api_key: &str, secret_key: &str) -> Result<(), String> {
    let keys = SidecarKeys {
        api_key: api_key.trim().into(),
        secret_key: secret_key.trim().into(),
    };
    if keys.api_key.is_empty() || keys.secret_key.is_empty() {
        return Err("API Key 與 Secret Key 都要填，不能空白".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string(&keys).map_err(|e| e.to_string())?;
    std::fs::write(keys_path(dir), text).map_err(|e| e.to_string())
}

/// Delete the key file. A missing file is already the desired end state.
pub fn clear(dir: &Path) -> Result<(), String> {
    match std::fs::remove_file(keys_path(dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "maxistocks-sidecar-keys-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saved_pair_reports_both_set_and_loads_back() {
        let dir = temp_dir("roundtrip");
        assert_eq!(status(&dir), (false, false));
        assert!(load(&dir).is_none());
        save(&dir, "  api-key-1 ", " secret-key-1 ").unwrap();
        assert_eq!(status(&dir), (true, true));
        let keys = load(&dir).unwrap();
        assert_eq!(keys.api_key, "api-key-1");
        assert_eq!(keys.secret_key, "secret-key-1");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clear_removes_the_pair_and_is_idempotent() {
        let dir = temp_dir("clear");
        save(&dir, "api-key-1", "secret-key-1").unwrap();
        clear(&dir).unwrap();
        assert_eq!(status(&dir), (false, false));
        assert!(load(&dir).is_none());
        // Clearing again, with nothing stored, is not an error.
        clear(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn blank_side_is_refused_and_a_half_file_loads_as_unset() {
        let dir = temp_dir("half");
        assert!(save(&dir, "", "secret-key-1").is_err());
        assert!(save(&dir, "api-key-1", "  ").is_err());
        assert_eq!(status(&dir), (false, false));
        // A hand-edited file with only one key: status names the missing
        // side, but the pair counts as unconfigured for starting.
        std::fs::write(
            keys_path(&dir),
            r#"{"apiKey":"api-key-1","secretKey":""}"#,
        )
        .unwrap();
        assert_eq!(status(&dir), (true, false));
        assert!(load(&dir).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn status_output_never_contains_key_values() {
        let dir = temp_dir("leak");
        save(&dir, "api-key-VERY-SECRET-1", "secret-key-VERY-SECRET-2").unwrap();
        let shown = status_json(&dir).to_string();
        assert_eq!(shown, r#"{"apiKeySet":true,"secretKeySet":true}"#);
        assert!(!shown.contains("VERY-SECRET"));
        // And the pair never rides along in the core's snapshot: the keys
        // file is a sibling of state.json, not part of it.
        assert!(!keys_path(&dir).ends_with("state.json"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
