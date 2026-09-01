use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use npcserver::{NewStandalone, load_settings};

fn default_settings_path() -> PathBuf {
    if let Ok(executable) = env::current_exe() {
        let candidate = executable
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("settings.ini");
        if candidate.is_file() {
            return candidate;
        }
    }
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("settings.ini")
}

fn main() {
    let mut config = None;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--config" {
            config = arguments.next().map(PathBuf::from);
        }
    }
    let path = config.unwrap_or_else(default_settings_path);
    let settings = load_settings(&path)
        .unwrap_or_else(|error| panic!("load settings {}: {error}", path.display()));
    let logger = Arc::new(|message: String| println!("[NPCServer] {message}"));
    let server = NewStandalone(settings, Some(logger));
    if let Err(error) = server.run() {
        panic!("{error}");
    }
}
