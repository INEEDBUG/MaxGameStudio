use maxgamestudio_core::native_http::{NativeHttpServer, DEFAULT_PORT};
use serde_json::json;
use std::{
    env,
    io::{self, BufRead, Write},
    thread,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut port = env::var("CS2_INSIGHT_PORT")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let mut version = "unknown".to_owned();
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--port" => port = args.next().ok_or("missing --port value")?.parse()?,
            "--app-version" => version = args.next().ok_or("missing --app-version value")?,
            "-h" | "--help" => {
                println!("maxgamestudio-native-http [--port PORT] [--app-version VERSION]");
                println!(
                    "Opt-in loopback health and GSI service. Close stdin or press Enter to stop."
                );
                return Ok(());
            }
            _ => return Err("unknown argument; see --help".into()),
        }
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let server = NativeHttpServer::bind(port, version)?;
    println!("{}", json!({"backendUrl": server.backend_url()}));
    io::stdout().flush()?;
    let (send, receive) = tokio::sync::oneshot::channel();
    thread::Builder::new()
        .name("native-http-stop".to_owned())
        .spawn(move || {
            let _ = io::stdin().lock().read_line(&mut String::new());
            let _ = send.send(());
        })?;
    runtime.block_on(server.serve(async {
        let _ = receive.await;
    }))?;
    Ok(())
}
