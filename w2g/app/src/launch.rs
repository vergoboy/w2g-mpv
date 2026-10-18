//! Builds the player command. No PATH lookup: mpv and the plugin are explicit paths.
use crate::api::SyncInfo;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct Launch { pub program: PathBuf, pub args: Vec<String>, pub env: Vec<(String, String)> }

/// "https://host:port/w2g" -> "https://host:port"
pub fn origin(base: &str) -> String {
    match base.find("://") {
        Some(i) => { let rest = &base[i + 3..]; format!("{}{}", &base[..i + 3], rest.split('/').next().unwrap_or("")) }
        None => base.to_string(),
    }
}

/// Item `src` is a server path such as `/w2g/media/hls/<id>/master.m3u8` (already prefixed).
pub fn media_url(api_base: &str, src: &str) -> String {
    if src.starts_with("http://") || src.starts_with("https://") { src.to_string() }
    else { format!("{}{}", origin(api_base), if src.starts_with('/') { src.to_string() } else { format!("/{src}") }) }
}

/// The room's current item: `(title, src)`. Items that are not `ready` have no playable source yet.
pub fn pick_item(state: &Value) -> Result<(String, String), String> {
    let list = state.get("playlist").and_then(Value::as_array).ok_or("the room has no playlist")?;
    let idx = state.get("current_index").and_then(Value::as_i64).unwrap_or(0);
    let item = usize::try_from(idx).ok().and_then(|i| list.get(i)).ok_or("nothing is selected in the room")?;
    let title = item.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    if item.get("status").and_then(Value::as_str).map_or(false, |s| s != "ready") {
        return Err(format!("“{title}” is not ready yet (still processing on the server)"));
    }
    match item.get("src").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => Ok((title, s.to_string())),
        _ => Err(format!("“{title}” has no playable source yet")),
    }
}

pub fn build(mpv: &Path, plugin: &Path, sync: &SyncInfo, url: &str, title: &str, access_token: &str, extra: &[String]) -> Launch {
    let mut args = vec![
        "--no-config".to_string(),                       // the app must not depend on a user's mpv.conf
        format!("--script={}", plugin.display()),
        format!("--force-media-title={title}"),
        "--keep-open=yes".into(),
        "--pause".into(),                                // start paused; the room decides when to play
    ];
    if url.contains("://") { args.push(format!("--http-header-fields=Authorization: Bearer {access_token}")); }
    args.extend(extra.iter().cloned());
    args.push("--".into());
    args.push(url.to_string());
    let role = if sync.role == "controller" { "controller" } else { "watcher" };
    Launch {
        program: mpv.to_path_buf(),
        args,
        env: vec![
            ("W2G_URL".into(), sync.url.clone()), ("W2G_TOKEN".into(), sync.token.clone()),
            ("W2G_ROOM".into(), sync.room.clone()), ("W2G_ROLE".into(), role.into()),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn origin_and_media_url() {
        assert_eq!(origin("https://vergoboy.ir/w2g"), "https://vergoboy.ir");
        assert_eq!(origin("http://127.0.0.1:8080/w2g/"), "http://127.0.0.1:8080");
        assert_eq!(media_url("https://vergoboy.ir/w2g", "/w2g/media/hls/a/master.m3u8"),
                   "https://vergoboy.ir/w2g/media/hls/a/master.m3u8");
        assert_eq!(media_url("https://x.y/w2g", "https://cdn.z/a.mkv"), "https://cdn.z/a.mkv");
    }

    #[test]
    fn pick_item_rules() {
        let ok = json!({"current_index":1,"playlist":[{"title":"a","src":"/x"},{"title":"b","status":"ready","src":"/w2g/m.m3u8"}]});
        assert_eq!(pick_item(&ok).unwrap(), ("b".into(), "/w2g/m.m3u8".into()));
        assert!(pick_item(&json!({})).is_err());
        assert!(pick_item(&json!({"current_index":-1,"playlist":[{"title":"a","src":"/x"}]})).is_err());
        let busy = json!({"current_index":0,"playlist":[{"title":"t","status":"encoding","src":null}]});
        assert!(pick_item(&busy).unwrap_err().contains("not ready"));
        let nosrc = json!({"current_index":0,"playlist":[{"title":"t","status":"ready","src":null}]});
        assert!(pick_item(&nosrc).unwrap_err().contains("no playable source"));
    }

    #[test]
    fn launch_uses_explicit_paths_and_room_env() {
        let s = SyncInfo { url: "wss://h/sync".into(), token: "v1.a.b".into(), room: "R1".into(), role: "controller".into() };
        let l = build(Path::new("/opt/w2g/mpv"), Path::new("/opt/w2g/w2g.so"), &s, "https://h/w2g/m.m3u8", "Film", "ACC", &[]);
        assert_eq!(l.program, PathBuf::from("/opt/w2g/mpv"));
        assert!(l.args.contains(&"--no-config".to_string()) && l.args.contains(&"--pause".to_string()));
        assert!(l.args.contains(&"--script=/opt/w2g/w2g.so".to_string()));
        assert_eq!(l.args.last().unwrap(), "https://h/w2g/m.m3u8");
        assert!(l.env.contains(&("W2G_ROLE".into(), "controller".into())));
        assert!(l.env.contains(&("W2G_TOKEN".into(), "v1.a.b".into())));
        let w = SyncInfo { role: "anything-else".into(), ..s };
        assert!(build(Path::new("m"), Path::new("p"), &w, "f.mkv", "t", "A", &[]).env.contains(&("W2G_ROLE".into(), "watcher".into())));
    }
}
