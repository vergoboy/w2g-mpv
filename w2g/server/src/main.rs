use std::{net::TcpListener, time::Duration};
use w2g_server::{serve, PrefixAuth, TokenAuth};

fn main() {
    let addr = std::env::var("W2G_BIND").unwrap_or_else(|_| "127.0.0.1:8765".into());
    let l = TcpListener::bind(&addr).expect("bind");
    let secret = std::env::var("W2G_SECRET").unwrap_or_default();
    if secret.len() >= 32 {
        eprintln!("w2g-server listening on ws://{addr} (signed room tokens)");
        serve(l, TokenAuth::new(secret.as_bytes()), Duration::from_secs(1));
    } else if std::env::var("W2G_DEV_AUTH").as_deref() == Ok("1") {
        eprintln!("w2g-server listening on ws://{addr} -- DEV AUTH: anyone can be a controller. NEVER expose this.");
        serve(l, PrefixAuth, Duration::from_secs(1));
    } else {
        eprintln!("refusing to start: set W2G_SECRET (>= 32 chars) or, for local dev only, W2G_DEV_AUTH=1");
        std::process::exit(2);
    }
    loop { std::thread::sleep(Duration::from_secs(3600)); }
}
