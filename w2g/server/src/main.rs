use std::{net::TcpListener, time::Duration};
fn main() {
    let addr = std::env::var("W2G_BIND").unwrap_or_else(|_| "127.0.0.1:8765".into());
    let l = TcpListener::bind(&addr).expect("bind");
    eprintln!("w2g-server listening on ws://{addr} (PLACEHOLDER AUTH: ctl:*/watch:* tokens)");
    w2g_server::serve(l, w2g_server::PrefixAuth, Duration::from_secs(1));
    loop { std::thread::sleep(Duration::from_secs(3600)); }
}
