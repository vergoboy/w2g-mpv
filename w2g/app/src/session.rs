//! The login session lives in a file, NOT in a webview's storage: it survives restarts,
//! updates and webview profile changes. The refresh token ROTATES on every refresh, so
//! every write is atomic (tmp + fsync + rename) and happens before the new token is used.
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Session {
    pub api: String,
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: String,
    pub username: String,
}

pub fn state_dir() -> PathBuf {
    if let Ok(d) = std::env::var("W2G_STATE_DIR") { return d.into(); }
    if let Ok(d) = std::env::var("XDG_STATE_HOME") { return Path::new(&d).join("w2g"); }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    Path::new(&home).join(".local/state/w2g")
}

fn file(dir: &Path) -> PathBuf { dir.join("session.json") }

pub fn load(dir: &Path) -> std::io::Result<Option<Session>> {
    match std::fs::read_to_string(file(dir)) {
        Ok(s) => Ok(serde_json::from_str(&s).ok()), // a corrupt file means "not logged in", not a crash
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn save(dir: &Path, s: &Session) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt;
      std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?; }
    let tmp = dir.join(format!("session.json.tmp{}", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; opts.mode(0o600); }
    let mut f = opts.open(&tmp)?;
    f.write_all(serde_json::to_string_pretty(s).unwrap().as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, file(dir))
}

pub fn clear(dir: &Path) { let _ = std::fs::remove_file(file(dir)); }
