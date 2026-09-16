use jyotish_mcp::config::Config;

#[tokio::main]
async fn main() {
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{{\"level\":\"fatal\",\"msg\":\"config error\",\"error\":{}}}",
                serde_json::json!(e.to_string())
            );
            std::process::exit(2);
        }
    };
    let state = match jyotish_mcp::boot(config.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "{{\"level\":\"fatal\",\"msg\":\"boot guard failed\",\"error\":{}}}",
                serde_json::json!(e.to_string())
            );
            std::process::exit(3);
        }
    };
    // TLS settings are read here, not in `Config`, so the in-process
    // integration tests (plain HTTP on a random port) are untouched.
    let tls_paths = match jyotish_mcp::tls::tls_paths_from_env() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "{{\"level\":\"fatal\",\"msg\":\"config error\",\"error\":{}}}",
                serde_json::json!(e.to_string())
            );
            std::process::exit(2);
        }
    };
    let served = match &tls_paths {
        Some(paths) => jyotish_mcp::serve_tls(state, config.bind, paths).await,
        None => {
            eprintln!("{{\"level\":\"warn\",\"msg\":\"JYOTISH_TLS=off: serving plain HTTP\"}}");
            jyotish_mcp::serve(state, config.bind).await
        }
    };
    let (_addr, handle) = match served {
        Ok(x) => x,
        // Boot guard: key mode / SAN / PEM failures are config errors (exit 2);
        // anything else (port in use, ...) is a bind failure (exit 4).
        Err(e) if e.is::<jyotish_mcp::tls::TlsError>() => {
            eprintln!(
                "{{\"level\":\"fatal\",\"msg\":\"tls boot guard failed\",\"error\":{}}}",
                serde_json::json!(e.to_string())
            );
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!(
                "{{\"level\":\"fatal\",\"msg\":\"bind failed\",\"error\":{}}}",
                serde_json::json!(e.to_string())
            );
            std::process::exit(4);
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            eprintln!("{{\"level\":\"info\",\"msg\":\"shutdown on SIGINT\"}}");
        }
        _ = handle => {}
    }
}
