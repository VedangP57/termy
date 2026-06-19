mod passthrough;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let result: Result<(), String> = match args.get(1).map(String::as_str) {
        Some("--server") => {
            let socket_path = args
                .get(2)
                .map(String::as_str)
                .unwrap_or_else(|| default_socket_path_static());
            server_bin::run(socket_path, false).map_err(|e| e.to_string())
        }
        // SSH remote-attach mode: server speaks the binary protocol over stdio.
        // Usage: ssh host termd --server --attach-stdio
        Some("--attach-stdio") => {
            let socket_path = args
                .get(2)
                .map(String::as_str)
                .unwrap_or_else(|| default_socket_path_static());
            server_bin::run(socket_path, true).map_err(|e| e.to_string())
        }
        Some("--passthrough") => {
            // Escape hatch: raw PTY passthrough without a window (useful in headless envs).
            passthrough::run().map_err(|e| e.to_string())
        }
        None | Some(_) => {
            // Default: open a GPU window and attach to (or auto-start) the local server.
            let socket_path = args
                .get(1)
                .filter(|a| !a.starts_with('-'))
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

fn default_socket_path() -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    format!("/tmp/termd-{user}.sock")
}

fn default_socket_path_static() -> &'static str {
    "/tmp/termd.sock"
}
