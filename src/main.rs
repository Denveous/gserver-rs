use gserver_rs::config::FileSystem;
use gserver_rs::logger::{init_logger, open_log_file};
use gserver_rs::server::Server;
use gserver_rs::{log_error, log_info, log_raw};
use std::env;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::signal;

const APP_VENDOR: &str = "OpenGraal";
const APP_NAME: &str = "GS2Emu";
const APP_VERSION: &str = "3.0.9-RS";
const APP_CREDITS: &str = "Terry A. Davis";

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let mut server_dir = String::new();
    let mut server_name = String::new();
    let mut port = String::new();
    let mut server_ip = String::new();
    let mut local_ip = String::new();
    let mut server_interface = String::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--server" | "-server" if i + 1 < args.len() => {
                server_dir = args[i + 1].clone();
                i += 1;
            }
            "--name" | "-name" if i + 1 < args.len() => {
                server_name = args[i + 1].clone();
                i += 1;
            }
            "--port" | "-port" if i + 1 < args.len() => {
                port = args[i + 1].clone();
                i += 1;
            }
            "--serverip" | "-serverip" if i + 1 < args.len() => {
                server_ip = args[i + 1].clone();
                i += 1;
            }
            "--localip" | "-localip" if i + 1 < args.len() => {
                local_ip = args[i + 1].clone();
                i += 1;
            }
            "--interface" | "-interface" if i + 1 < args.len() => {
                server_interface = args[i + 1].clone();
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    init_logger("", true);
    if let Err(e) = open_log_file("GServer.log") {
        eprintln!("Failed to open log file: {}", e);
    }

    log_raw!("{} {}", APP_VENDOR, APP_NAME);
    log_raw!("{}", APP_VERSION);
    log_raw!("Programmed by {}.", APP_CREDITS);

    let mut server = "default".to_string();
    if !server_dir.is_empty() {
        server = server_dir;
    } else {
        log_raw!(":: Determining the server to start... ");
        if let Ok(data) = fs::read_to_string("startupserver.txt") {
            server = data.trim().to_string();
            log_raw!("success! (startupserver.txt)");
        } else {
            let entries = fs::read_dir("servers").map(|res| {
                res.filter_map(Result::ok).collect::<Vec<_>>()
            }).unwrap_or_default();
            
            if entries.len() == 1 {
                server = entries[0].file_name().to_string_lossy().to_string();
                log_raw!("success! (servers/ search)");
            } else if Path::new("Content").is_dir() {
                server = "Content".to_string();
                log_raw!("success! (found Content folder)");
            } else {
                log_raw!("FAILED!");
                std::process::exit(1);
            }
        }
    }

    loop {
        let mut srv = Server::new("GServer-RS");
        if Path::new(&format!("servers/{}", server)).exists() {
            srv.config_base_path = format!("servers/{}/", server);
        } else {
            srv.config_base_path = format!("{}/", server);
        }
        log_raw!(":: Starting server: {}.", server);
        
        if let Err(e) = srv.init(&server_ip, &port, &local_ip, &server_interface).await {
            log_error!("Failed to start server {}: {}", server, e);
            std::process::exit(1);
        }
        
        srv.load_settings();
        if !server_name.is_empty() {
            srv.name = server_name.clone();
        }
        
        let display_name = srv.name.clone();
        log_raw!(":: Started server {} ({})", server, display_name);
        log_raw!(":: Press CTRL+C to close the program.  DO NOT CLICK THE X, you will LOSE data!");

        let srv = Arc::new(RwLock::new(srv));
        let srv_clone = srv.clone();
        
        let run_handle = tokio::spawn(async move {
            Server::run(srv_clone).await
        });

        tokio::select! {
            _ = signal::ctrl_c() => {
                log_raw!(":: The server is now shutting down...\n-------------------------------------\n");
                // TODO: gracefully shut down srv
                break;
            }
            res = run_handle => {
                match res {
                    Ok(Err(e)) => log_error!("Server error: {}", e),
                    Err(e) => log_error!("Server task panicked: {}", e),
                    _ => {}
                }
            }
        }

        // Add proper restart requested check here once implemented on srv
        break;
    }
}
