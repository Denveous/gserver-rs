use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::sync::Mutex;
use chrono::Local;

lazy_static::lazy_static! {
    static ref GLOBAL_LOGGER: Mutex<Option<Logger>> = Mutex::new(None);
}

pub struct Logger {
    file: Option<File>,
    prefix: String,
    log_to_file: bool,
    debug_mode: bool,
    packet_debug_mode: bool,
}

pub fn init_logger(prefix: &str, log_to_file: bool) {
    let mut global = GLOBAL_LOGGER.lock().unwrap();
    *global = Some(Logger {
        file: None,
        prefix: prefix.to_string(),
        log_to_file,
        debug_mode: true,
        packet_debug_mode: false,
    });
}

pub fn open_log_file(filename: &str) -> io::Result<()> {
    let mut global = GLOBAL_LOGGER.lock().unwrap();
    if let Some(logger) = global.as_mut() {
        if !logger.log_to_file {
            return Ok(());
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(filename)?;
        logger.file = Some(file);
    }
    Ok(())
}

fn color_log_tags(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '[' {
            out.push(c);
            continue;
        }
        
        let mut tag = String::from("[");
        let mut found_end = false;
        while let Some(&next) = chars.peek() {
            tag.push(next);
            chars.next();
            if next == ']' {
                found_end = true;
                break;
            }
        }
        
        if found_end {
            out.push('[');
            out.push_str(&log_tag_color(&tag));
            out.push_str(&tag[1..tag.len()-1]);
            out.push_str("\x1b[0m");
            out.push(']');
        } else {
            out.push_str(&tag);
        }
    }
    out
}

fn log_tag_color(tag: &str) -> String {
    if tag.starts_with("[ERROR") {
        "\x1b[91m".to_string()
    } else if tag.starts_with("[WARNING") {
        "\x1b[93m".to_string()
    } else if tag.starts_with("[INFO") {
        "\x1b[92m".to_string()
    } else if tag.starts_with("[DEBUG") {
        "\x1b[35m".to_string()
    } else if tag.starts_with("[LISTSERVER") {
        "\x1b[96m".to_string()
    } else if tag.starts_with("[PACKET") {
        "\x1b[95m".to_string()
    } else if tag.starts_with("[GS2") {
        "\x1b[94m".to_string()
    } else {
        "\x1b[36m".to_string()
    }
}

pub fn log_write(args: std::fmt::Arguments) {
    let mut global = GLOBAL_LOGGER.lock().unwrap();
    let message = format!("{}", args);
    let message = message.trim_end_matches('\n');
    let lines = message.split('\n');

    let timestamp = Local::now().format("[%I:%M %p]").to_string();
    let color_ts = format!("[\x1b[96m{}\x1b[0m]", Local::now().format("%I:%M %p"));

    if let Some(logger) = global.as_mut() {
        for line in lines {
            println!("{} {}{}", color_ts, logger.prefix, color_log_tags(line));
            if let Some(file) = &mut logger.file {
                let _ = writeln!(file, "{} {}{}", timestamp, logger.prefix, line);
            }
        }
    } else {
        // Fallback if logger is not initialized
        for line in lines {
            println!("{} {}", color_ts, color_log_tags(line));
        }
    }
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logger::log_write(format_args!("[ERROR] {}", format_args!($($arg)*)))
    };
}
#[macro_export]
macro_rules! log_warning {
    ($($arg:tt)*) => {
        $crate::logger::log_write(format_args!("[WARNING] {}", format_args!($($arg)*)))
    };
}
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::logger::log_write(format_args!("[INFO] {}", format_args!($($arg)*)))
    };
}
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::logger::log_write(format_args!("[DEBUG] {}", format_args!($($arg)*)))
    };
}
#[macro_export]
macro_rules! log_raw {
    ($($arg:tt)*) => {
        $crate::logger::log_write(format_args!("{}", format_args!($($arg)*)))
    };
}
