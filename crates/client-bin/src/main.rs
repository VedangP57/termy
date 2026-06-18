mod passthrough;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let result = match args.get(1).map(String::as_str) {
        Some("--server") => {
            let socket_path = args
                .get(2)
                .map(String::as_str)
                .unwrap_or("/tmp/termd.sock");
            server_bin::run(socket_path).map_err(|e| e.to_string())
        }
        Some("--attach") => {
            // Phase 8: implement the client-side attach protocol.
            // For now emit a clear not-implemented message.
            Err("--attach is not implemented until Phase 8".to_string())
        }
        None | Some(_) => passthrough::run().map_err(|e| e.to_string()),
    };

    if let Err(e) = result {
        eprintln!("termd: {e}");
        std::process::exit(1);
    }
}
