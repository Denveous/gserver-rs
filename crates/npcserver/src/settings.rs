use std::collections::HashMap;
use std::fs;
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    values: HashMap<String, String>,
}

pub fn parse_settings(data: &str) -> Settings {
    let mut values = HashMap::new();
    for line in data.replace("\r\n", "\n").split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(separator) = line.find('=') else {
            continue;
        };
        let key = line[..separator].trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        values.insert(key, line[separator + 1..].trim().to_string());
    }
    Settings { values }
}

pub fn load_settings(filename: impl AsRef<std::path::Path>) -> io::Result<Settings> {
    Ok(parse_settings(&fs::read_to_string(filename)?))
}

impl Settings {
    pub fn get(&self, key: &str) -> String {
        self.values
            .get(&key.trim().to_ascii_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    pub fn get_int(&self, key: &str, fallback: i32) -> i32 {
        self.get(key).parse::<i32>().unwrap_or(fallback)
    }

    pub fn game_server_account(&self) -> String {
        let account = self.get("account");
        if account.is_empty() {
            "npcserver".to_string()
        } else {
            account
        }
    }

    pub fn values(&self) -> HashMap<String, String> {
        self.values.clone()
    }

    #[allow(non_snake_case)]
    pub fn Get(&self, key: &str) -> String {
        self.get(key)
    }

    #[allow(non_snake_case)]
    pub fn GetInt(&self, key: &str, fallback: i32) -> i32 {
        self.get_int(key, fallback)
    }

    #[allow(non_snake_case)]
    pub fn GameServerAccount(&self) -> String {
        self.game_server_account()
    }

    #[allow(non_snake_case)]
    pub fn Values(&self) -> HashMap<String, String> {
        self.values()
    }
}

#[allow(non_snake_case)]
pub fn ParseSettings(data: &str) -> Settings {
    parse_settings(data)
}

#[allow(non_snake_case)]
pub fn LoadSettings(filename: impl AsRef<std::path::Path>) -> io::Result<Settings> {
    load_settings(filename)
}
