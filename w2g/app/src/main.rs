use std::io::Write;
use std::path::PathBuf;
use w2g_app::{api::{Api, ApiError}, launch, session};

const DEFAULT_API: &str = "https://vergoboy.ir/w2g";

fn usage() -> ! {
    eprintln!("usage: w2g <login [user] | logout | whoami | join <code> | play [code]>\n\
               env: W2G_API (default {DEFAULT_API}), W2G_MPV, W2G_PLUGIN (default: next to this binary), W2G_MPV_ARGS");
    std::process::exit(2)
}

fn exe_sibling(name: &str) -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name))).unwrap_or_else(|| name.into())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let api_base = std::env::var("W2G_API").unwrap_or_else(|_| DEFAULT_API.into());
    let dir = session::state_dir();
    let mut api = Api::open(&dir, &api_base).map_err(|e| e.to_string())?;
    let e2s = |e: ApiError| e.to_string();
    match args.first().map(String::as_str) {
        Some("login") => {
            let user = match args.get(1) {
                Some(u) => u.clone(),
                None => { eprint!("username or email: "); std::io::stderr().flush().ok();
                          let mut s = String::new(); std::io::stdin().read_line(&mut s).map_err(|e| e.to_string())?; s.trim().into() }
            };
            let pass = match std::env::var("W2G_PASSWORD") {
                Ok(p) => p,
                Err(_) => rpassword::prompt_password("password: ").map_err(|e| e.to_string())?,
            };
            let s = api.login(&user, &pass).map_err(e2s)?;
            println!("logged in as {} ({})", s.username, dir.display());
        }
        Some("logout") => { api.logout(); println!("logged out"); }
        Some("whoami") => {
            let me = api.me().map_err(e2s)?;
            println!("{}", serde_json::to_string_pretty(&me).unwrap());
        }
        Some("join") => {
            let code = args.get(1).ok_or("usage: w2g join <code>")?;
            let r = api.join_room(code).map_err(e2s)?;
            println!("joined room {}", r["room"]["name"].as_str().unwrap_or(code));
        }
        Some("play") => {
            if let Some(code) = args.get(1) { api.join_room(code).map_err(e2s)?; }
            let sync = api.sync_token().map_err(e2s)?;
            let state = api.state().map_err(e2s)?;
            let (title, src) = launch::pick_item(&state)?;
            let url = launch::media_url(api.base(), &src);
            let mpv = std::env::var("W2G_MPV").map(PathBuf::from).unwrap_or_else(|_| exe_sibling("mpv"));
            let plugin = std::env::var("W2G_PLUGIN").map(PathBuf::from).unwrap_or_else(|_| exe_sibling("w2g.so"));
            for (what, p) in [("mpv", &mpv), ("plugin", &plugin)] {
                if !p.exists() { return Err(format!("{what} not found at {} (set W2G_{})", p.display(), what.to_uppercase())); }
            }
            let extra: Vec<String> = std::env::var("W2G_MPV_ARGS").unwrap_or_default().split_whitespace().map(String::from).collect();
            let token = api.session().map(|s| s.access_token.clone()).unwrap_or_default();
            let l = launch::build(&mpv, &plugin, &sync, &url, &title, &token, &extra);
            eprintln!("w2g: room {} as {} — {}", sync.room, sync.role, title);
            let status = std::process::Command::new(&l.program).args(&l.args).envs(l.env.iter().cloned())
                .status().map_err(|e| format!("cannot start {}: {e}", l.program.display()))?;
            if !status.success() { return Err(format!("player exited with {status}")); }
        }
        _ => usage(),
    }
    Ok(())
}

fn main() { if let Err(e) = run() { eprintln!("w2g: {e}"); std::process::exit(1); } }
