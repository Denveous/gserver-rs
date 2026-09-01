use std::env;
use std::fs;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use gscript_game_server::{Logger, Server, APP_NAME};

const APP_CREDITS: &str = "Terry A. Davis";

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn handle_signal(_signal: i32) {
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

#[cfg(unix)]
fn install_signal_handlers() {
    unsafe extern "C" {
        fn signal(signal: i32, handler: usize) -> usize;
    }
    unsafe {
        let handler = handle_signal as usize;
        let _ = signal(2, handler); // SIGINT
        let _ = signal(15, handler); // SIGTERM
    }
}

#[cfg(not(unix))]
fn install_signal_handlers() {}

fn select_server(server_dir: &str, logger: &Logger) -> String {
    if !server_dir.is_empty() {
        return server_dir.to_string();
    }
    logger.write(":: Determining the server to start... ");
    if let Ok(data) = fs::read("startupserver.txt") {
        if !data.is_empty() {
            let selected_server = String::from_utf8_lossy(&data).trim().to_string();
            logger.write("success! (startupserver.txt)");
            return selected_server;
        }
    }
    let entries = fs::read_dir("servers")
        .map(|entries| entries.flatten().collect::<Vec<_>>())
        .unwrap_or_default();
    if entries.len() == 1 {
        let selected = entries[0].file_name().to_string_lossy().into_owned();
        logger.write("success! (directory search)");
        return selected;
    }
    logger.write("FAILED!");
    process::exit(1);
}

fn print_usage() {
    println!("Usage: gameserver [options]");
    println!("  -server string");
    println!("  -name string");
    println!("  -port string");
    println!("  -serverip string");
    println!("  -localip string");
    println!("  -interface string");
    println!("  -silent");
}

fn main() {
    let mut server_dir = String::new();
    let mut server_name = String::new();
    let mut port = String::new();
    let mut server_ip = String::new();
    let mut local_ip = String::new();
    let mut interface = String::new();
    let mut silent = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = |args: &mut std::iter::Skip<std::env::Args>, name: &str| -> String {
            args.next().unwrap_or_else(|| {
                eprintln!("missing value for {name}");
                process::exit(2);
            })
        };
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return;
            }
            "--server" => server_dir = value(&mut args, "--server"),
            "-server" => server_dir = value(&mut args, "-server"),
            "--name" => server_name = value(&mut args, "--name"),
            "-name" => server_name = value(&mut args, "-name"),
            "--port" => port = value(&mut args, "--port"),
            "-port" => port = value(&mut args, "-port"),
            "--serverip" => server_ip = value(&mut args, "--serverip"),
            "-serverip" => server_ip = value(&mut args, "-serverip"),
            "--localip" => local_ip = value(&mut args, "--localip"),
            "-localip" => local_ip = value(&mut args, "-localip"),
            "--interface" => interface = value(&mut args, "--interface"),
            "-interface" => interface = value(&mut args, "-interface"),
            "--silent" => silent = true,
            "-silent" => silent = true,
            _ if arg.starts_with("-silent=") || arg.starts_with("--silent=") => {
                let value = arg
                    .split_once('=')
                    .map(|(_, value)| value)
                    .unwrap_or_default();
                silent = match value {
                    "1" | "t" | "T" | "true" | "TRUE" | "True" => true,
                    "0" | "f" | "F" | "false" | "FALSE" | "False" => false,
                    _ => {
                        eprintln!("invalid value {value:?} for flag `-silent'");
                        process::exit(2);
                    }
                };
            }
            _ if arg.starts_with("-server=") => server_dir = arg[8..].to_string(),
            _ if arg.starts_with("-name=") => server_name = arg[6..].to_string(),
            _ if arg.starts_with("-port=") => port = arg[6..].to_string(),
            _ if arg.starts_with("-serverip=") => server_ip = arg[10..].to_string(),
            _ if arg.starts_with("-localip=") => local_ip = arg[9..].to_string(),
            _ if arg.starts_with("-interface=") => interface = arg[11..].to_string(),
            _ if arg.starts_with("--server=") => server_dir = arg[9..].to_string(),
            _ if arg.starts_with("--name=") => server_name = arg[7..].to_string(),
            _ if arg.starts_with("--port=") => port = arg[7..].to_string(),
            _ if arg.starts_with("--serverip=") => server_ip = arg[11..].to_string(),
            _ if arg.starts_with("--localip=") => local_ip = arg[10..].to_string(),
            _ if arg.starts_with("--interface=") => interface = arg[12..].to_string(),
            _ => {
                eprintln!("unknown option: {arg}");
                process::exit(2);
            }
        }
    }

    let logger = Arc::new(Logger::new("", true));
    logger.set_silent(silent);
    let _ = logger.open("GServer.log");
    logger.write(APP_NAME);
    logger.write(&format!("Programmed by {APP_CREDITS}."));
    let selected_server = select_server(&server_dir, &logger);
    install_signal_handlers();

    loop {
        SHUTDOWN_REQUESTED.store(false, Ordering::Relaxed);
        let base = format!("servers/{selected_server}/");
        let server = Server::new_with_logger("GServer", base, logger.clone());
        logger.write(&format!(":: Starting server: {selected_server}."));
        if let Err(error) = server.init_with_args(&server_ip, &port, &local_ip, &interface) {
            logger.error(&format!(
                "Failed to start server: {selected_server}: {error}"
            ));
            process::exit(1);
        }
        server.reload_settings();
        if !server_name.is_empty() {
            *server.name.write().unwrap() = server_name.clone();
        }
        logger.write(&format!(
            ":: Started server {selected_server} ({}).",
            server.configured_name()
        ));
        logger.write(
            ":: Press CTRL+C to close the program.  DO NOT CLICK THE X, you will LOSE data!",
        );

        let run_server = server.clone();
        let run_thread = thread::spawn(move || run_server.run());
        while !run_thread.is_finished() {
            if SHUTDOWN_REQUESTED.load(Ordering::Relaxed) {
                logger.write(":: The server is now shutting down...\n-------------------------------------\n");
                server.stop();
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if let Ok(Err(error)) = run_thread.join() {
            logger.error(&format!("Server error: {error}"));
        }
        if !server.restart_requested() {
            break;
        }
        logger.write(":: Restarting server...\n-------------------------------------\n");
    }
}
