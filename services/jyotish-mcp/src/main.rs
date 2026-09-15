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
    let (_addr, handle) = match jyotish_mcp::serve(state, config.bind).await {
        Ok(x) => x,
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
