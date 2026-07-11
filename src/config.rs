use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use parking_lot::RwLock;

pub struct Settings {
    settings: RwLock<HashMap<String, String>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl Settings {
    pub fn new() -> Self {
        Self {
            settings: RwLock::new(HashMap::new()),
        }
    }

    pub fn load_from_string(&self, data: &str) -> io::Result<()> {
        let mut loaded = HashMap::new();
        for line in data.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                loaded.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        *self.settings.write() = loaded;
        Ok(())
    }

    pub fn load(&self, filename: &str) -> io::Result<()> {
        let file = match File::open(filename) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };

        let mut loaded = HashMap::new();
        let reader = BufReader::new(file);

        for line_res in reader.lines() {
            let line = line_res?;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                loaded.insert(k.trim().to_string(), v.trim().to_string());
            }
        }

        *self.settings.write() = loaded;
        Ok(())
    }

    pub fn save(&self, filename: &str) -> io::Result<()> {
        let mut file = File::create(filename)?;
        let settings = self.settings.read();
        for (key, value) in settings.iter() {
            writeln!(file, "{}={}", key, value)?;
        }
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.settings.read().get(key).cloned()
    }

    pub fn get_or(&self, key: &str, default_value: &str) -> String {
        self.get(key).unwrap_or_else(|| default_value.to_string())
    }

    pub fn set(&self, key: &str, value: &str) {
        self.settings.write().insert(key.to_string(), value.to_string());
    }

    pub fn get_int(&self, key: &str, default_value: i32) -> i32 {
        if let Some(val) = self.get(key) {
            if let Ok(num) = val.parse::<i32>() {
                return num;
            }
        }
        default_value
    }

    pub fn get_bool(&self, key: &str, default_value: bool) -> bool {
        if let Some(val) = self.get(key) {
            let lower = val.to_lowercase();
            return lower == "true" || lower == "1";
        }
        default_value
    }

    pub fn exists(&self, key: &str) -> bool {
        self.settings.read().contains_key(key)
    }

    pub fn get_all(&self) -> HashMap<String, String> {
        self.settings.read().clone()
    }
}

pub struct FileSystem {
    base_path: RwLock<String>,
    cache: RwLock<HashMap<String, Vec<u8>>>,
}

impl FileSystem {
    pub fn new(base_path: &str) -> Self {
        Self {
            base_path: RwLock::new(base_path.to_string()),
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn set_base_path(&self, path: &str) {
        *self.base_path.write() = path.to_string();
    }

    pub fn get_base_path(&self) -> String {
        self.base_path.read().clone()
    }

    pub fn resolve_path(&self, path: &str) -> PathBuf {
        Path::new(&*self.base_path.read()).join(path)
    }

    pub fn resolve_existing_path(&self, path: &str) -> PathBuf {
        let base_path = self.base_path.read().clone();
        let stripped = path.trim_start_matches('/');
        let direct = Path::new(&base_path).join(stripped);
        if direct.exists() {
            return direct;
        }

        if stripped.starts_with("world/") {
            return direct; // Still direct
        }

        let world_path = Path::new(&base_path).join("world").join(stripped);
        if world_path.exists() {
            return world_path;
        }

        direct
    }

    pub fn load_file(&self, path: &str) -> io::Result<Vec<u8>> {
        let full_path = self.resolve_existing_path(path);
        fs::read(full_path)
    }

    pub fn file_exists(&self, path: &str) -> bool {
        self.resolve_existing_path(path).exists()
    }

    pub fn save_file(&self, path: &str, data: &[u8]) -> io::Result<()> {
        let full_path = self.resolve_path(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(full_path, data)
    }

    pub fn delete_file(&self, path: &str) -> io::Result<()> {
        fs::remove_file(self.resolve_path(path))
    }

    pub fn list_files(&self, path: &str) -> io::Result<Vec<String>> {
        let mut files = Vec::new();
        let full_path = self.resolve_existing_path(path);
        if full_path.is_dir() {
            for entry in fs::read_dir(full_path)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        files.push(name.to_string());
                    }
                }
            }
        }
        Ok(files)
    }

    pub fn list_dirs(&self, path: &str) -> io::Result<Vec<String>> {
        let mut dirs = Vec::new();
        let full_path = self.resolve_existing_path(path);
        if full_path.is_dir() {
            for entry in fs::read_dir(full_path)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        dirs.push(name.to_string());
                    }
                }
            }
        }
        Ok(dirs)
    }

    pub fn file_mod_time(&self, path: &str) -> io::Result<std::time::SystemTime> {
        let full_path = self.resolve_existing_path(path);
        fs::metadata(full_path)?.modified()
    }

    pub fn file_size(&self, path: &str) -> io::Result<u64> {
        let full_path = self.resolve_existing_path(path);
        Ok(fs::metadata(full_path)?.len())
    }

    pub fn cache_file(&self, path: &str) -> io::Result<()> {
        let data = self.load_file(path)?;
        self.cache.write().insert(path.to_string(), data);
        Ok(())
    }

    pub fn get_cached(&self, path: &str) -> Option<Vec<u8>> {
        self.cache.read().get(path).cloned()
    }

    pub fn clear_cache(&self) {
        self.cache.write().clear();
    }

    pub fn load_file_as_lines(&self, path: &str) -> io::Result<Vec<String>> {
        let full_path = self.resolve_existing_path(path);
        let file = File::open(full_path)?;
        let reader = BufReader::new(file);
        reader.lines().collect()
    }

    pub fn save_lines_as_file(&self, path: &str, lines: &[String]) -> io::Result<()> {
        let full_path = self.resolve_path(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(full_path)?;
        for line in lines {
            writeln!(file, "{}", line)?;
        }
        file.flush()
    }

    pub fn copy_file(&self, src: &str, dst: &str) -> io::Result<u64> {
        let src_path = self.resolve_path(src);
        let dst_path = self.resolve_path(dst);
        if let Some(parent) = dst_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src_path, dst_path)
    }
}
