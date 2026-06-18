mod passthrough;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let result: Result<(), String> = match args.get(1).map(String::as_str) {
        Some("--server") => {
            let socket_path = args
                .get(2)
                .map(String::as_str)
                .unwrap_or_else(|| default_socket_path_static());
            server_bin::run(socket_path).map_err(|e| e.to_string())
        }
        Some("--attach") => {
            // Phase 8: client-side attach protocol.
            Err("--attach is not implemented until Phase 8".to_string())
        }
        Some("--passthrough") => {
            // Escape hatch: raw PTY passthrough without a window (useful in headless envs).
            passthrough::run().map_err(|e| e.to_string())
        }
        None | Some(_) => {
            // Default: open a window and connect to (or auto-start) the server.
            let socket_path = args
                .get(1)
                .cloned()
                .unwrap_or_else(default_socket_path);
            render::run_window(&socket_path).map_err(|e| e.to_string())
        }
    };

    if let Err(e) = result {
        eprintln!("termd: {e}");
        std::process::exit(1);
    }
}

/// Default socket path as an owned String (used in the closure above).
fn default_socket_path() -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    format!("/tmp/termd-{user}.sock")
}

/// Default socket path as a `&'static str` — only usable when we know the value at compile time.
/// Used in the --server branch where we need a `&str` but don't have a String yet.
fn default_socket_path_static() -> &'static str {
    "/tmp/termd.sock"
}
