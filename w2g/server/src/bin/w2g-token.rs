//! Mint a room token (dev/ops helper; the account service does this in production).
//! usage: W2G_SECRET=... w2g-token <sub> <room> <controller|watcher> <ttl_seconds>
use w2g_server::{auth::now_unix, mint};
use w2g_sync::Role;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let secret = std::env::var("W2G_SECRET").unwrap_or_default();
    if a.len() != 4 || secret.len() < 32 {
        eprintln!("usage: W2G_SECRET=<>=32 chars> w2g-token <sub> <room> <controller|watcher> <ttl_seconds>");
        std::process::exit(2);
    }
    let role = match a[2].as_str() { "controller" => Role::Controller, "watcher" => Role::Watcher,
        _ => { eprintln!("role must be controller|watcher"); std::process::exit(2) } };
    let ttl: u64 = a[3].parse().unwrap_or_else(|_| { eprintln!("ttl must be seconds"); std::process::exit(2) });
    println!("{}", mint(secret.as_bytes(), &a[0], &a[1], role, now_unix() + ttl));
}
