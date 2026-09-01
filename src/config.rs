use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Default)]
pub struct Settings {
    values: Arc<RwLock<HashMap<String, String>>>,
}

impl Settings {
    pub fn new() -> Self {
        Self {
            values: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    pub fn NewSettings() -> Self {
        Self::new()
    }
    pub fn load<P: AsRef<Path>>(&self, filename: P) -> io::Result<()> {
        let file = match File::open(filename.as_ref()) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let loaded = parse_settings(BufReader::new(file).lines())?;
        *self.values.write().unwrap() = loaded;
        Ok(())
    }
    pub fn Load<P: AsRef<Path>>(&self, filename: P) -> io::Result<()> {
        self.load(filename)
    }
    pub fn save<P: AsRef<Path>>(&self, filename: P) -> io::Result<()> {
        let mut file = File::create(filename)?;
        for (key, value) in self.values.read().unwrap().iter() {
            writeln!(file, "{key}={value}")?;
        }
        Ok(())
    }
    pub fn Save<P: AsRef<Path>>(&self, filename: P) -> io::Result<()> {
        self.save(filename)
    }
    pub fn get(&self, key: &str) -> String {
        self.values
            .read()
            .unwrap()
            .get(key)
            .cloned()
            .unwrap_or_default()
    }
    pub fn Get(&self, key: &str) -> String {
        self.get(key)
    }
    pub fn set(&self, key: &str, value: &str) {
        self.values
            .write()
            .unwrap()
            .insert(key.to_string(), value.to_string());
    }
    pub fn Set(&self, key: &str, value: &str) {
        self.set(key, value)
    }
    pub fn get_int(&self, key: &str, default: i32) -> i32 {
        let value = self.get(key);
        let bytes = value.as_bytes();
        let mut start = 0;
        while start < bytes.len() && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        let mut end = start;
        if matches!(bytes.get(end), Some(b'+' | b'-')) {
            end += 1;
        }
        let digits = end;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == digits {
            return default;
        }
        value[start..end].parse().unwrap_or(default)
    }
    pub fn GetInt(&self, key: &str, default: i32) -> i32 {
        self.get_int(key, default)
    }
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        let value = self.values.read().unwrap().get(key).cloned();
        match value {
            Some(value) => value.eq_ignore_ascii_case("true") || value == "1",
            None => default,
        }
    }
    pub fn GetBool(&self, key: &str, default: bool) -> bool {
        self.get_bool(key, default)
    }
    pub fn exists(&self, key: &str) -> bool {
        self.values.read().unwrap().contains_key(key)
    }
    pub fn Exists(&self, key: &str) -> bool {
        self.exists(key)
    }
    pub fn get_all(&self) -> HashMap<String, String> {
        self.values.read().unwrap().clone()
    }
    pub fn GetAll(&self) -> HashMap<String, String> {
        self.get_all()
    }
    pub fn load_from_string(&self, data: &str) -> io::Result<()> {
        let loaded = parse_settings(data.lines().map(|line| Ok(line.to_string())))?;
        *self.values.write().unwrap() = loaded;
        Ok(())
    }
    pub fn LoadFromString(&self, data: &str) -> io::Result<()> {
        self.load_from_string(data)
    }
}

fn parse_settings<I>(lines: I) -> io::Result<HashMap<String, String>>
where
    I: IntoIterator<Item = io::Result<String>>,
{
    let mut result = HashMap::new();
    for line in lines {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            result.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    Ok(result)
}

pub struct Logger {
    file: Mutex<Option<File>>,
    prefix: String,
    log_to_file: bool,
    silent: Mutex<bool>,
}

impl Logger {
    pub fn new(prefix: &str, log_to_file: bool) -> Self {
        Self {
            file: Mutex::new(None),
            prefix: prefix.to_string(),
            log_to_file,
            silent: Mutex::new(false),
        }
    }
    pub fn NewLogger(prefix: &str, log_to_file: bool) -> Self {
        Self::new(prefix, log_to_file)
    }
    pub fn set_silent(&self, silent: bool) {
        *self.silent.lock().unwrap() = silent;
    }
    pub fn SetSilent(&self, silent: bool) {
        self.set_silent(silent)
    }
    pub fn open<P: AsRef<Path>>(&self, filename: P) -> io::Result<()> {
        if !self.log_to_file {
            return Ok(());
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(filename)?;
        *self.file.lock().unwrap() = Some(file);
        Ok(())
    }
    pub fn Open<P: AsRef<Path>>(&self, filename: P) -> io::Result<()> {
        self.open(filename)
    }
    pub fn close(&self) -> io::Result<()> {
        let mut file = self.file.lock().unwrap();
        if let Some(file) = file.as_mut() {
            file.flush()?;
        }
        *file = None;
        Ok(())
    }
    pub fn Close(&self) -> io::Result<()> {
        self.close()
    }
    pub fn write(&self, message: &str) {
        self.write_args(message);
    }
    pub fn write_args(&self, message: &str) {
        let message = message.trim_end_matches('\n');
        let mut file_message = String::new();
        let mut console_message = String::new();
        for line in message.split('\n') {
            let timestamp = format_timestamp();
            file_message.push_str(&format!("{timestamp} {}{line}\n", self.prefix));
            console_message.push_str(&format!(
                "{timestamp} {}{}\n",
                self.prefix,
                color_log_tags(line)
            ));
        }
        let mut file = self.file.lock().unwrap();
        if !*self.silent.lock().unwrap() {
            print!("{console_message}");
        }
        if let Some(output) = file.as_mut() {
            let _ = output.write_all(file_message.as_bytes());
            let _ = output.flush();
        }
    }
    pub fn Write(&self, message: &str) {
        self.write(message)
    }
    pub fn error(&self, message: &str) {
        self.write(&format!("[ERROR] {message}"));
    }
    pub fn Error(&self, message: &str) {
        self.error(message)
    }
    pub fn warning(&self, message: &str) {
        self.write(&format!("[WARNING] {message}"));
    }
    pub fn Warning(&self, message: &str) {
        self.warning(message)
    }
    pub fn info(&self, message: &str) {
        self.write(&format!("[INFO] {message}"));
    }
    pub fn Info(&self, message: &str) {
        self.info(message)
    }
    pub fn debug(&self, message: &str) {
        if crate::model::debug_mode() {
            self.write(&format!("[DEBUG] {message}"));
        }
    }
    pub fn Debug(&self, message: &str) {
        self.debug(message)
    }
    pub fn packet_debug(&self, message: &str) {
        if crate::model::debug_mode() && crate::model::packet_debug_mode() {
            self.write(&format!("[DEBUG] {message}"));
        }
    }
    pub fn PacketDebug(&self, message: &str) {
        self.packet_debug(message)
    }
}

fn format_timestamp() -> String {
    // Keep the formatter dependency-free and stable for log parsing.
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let minutes = (seconds / 60) % (24 * 60);
    let hour24 = minutes / 60;
    let hour12 = match hour24 % 12 {
        0 => 12,
        value => value,
    };
    let suffix = if hour24 < 12 { "AM" } else { "PM" };
    format!("[{hour12:02}:{:02} {suffix}]", minutes % 60)
}

fn color_log_tags(line: &str) -> String {
    line.to_string()
}

#[derive(Clone, Debug)]
pub struct FileInfo {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: SystemTime,
}

pub struct FileSystem {
    base_path: RwLock<PathBuf>,
    cache: RwLock<HashMap<String, Vec<u8>>>,
}

impl FileSystem {
    pub fn new<P: AsRef<Path>>(base_path: P) -> Self {
        Self {
            base_path: RwLock::new(base_path.as_ref().to_path_buf()),
            cache: RwLock::new(HashMap::new()),
        }
    }
    pub fn NewFileSystem<P: AsRef<Path>>(base_path: P) -> Self {
        Self::new(base_path)
    }
    pub fn set_base_path<P: AsRef<Path>>(&self, path: P) {
        *self.base_path.write().unwrap() = path.as_ref().to_path_buf();
    }
    pub fn SetBasePath<P: AsRef<Path>>(&self, path: P) {
        self.set_base_path(path)
    }
    pub fn get_base_path(&self) -> PathBuf {
        self.base_path.read().unwrap().clone()
    }
    pub fn GetBasePath(&self) -> PathBuf {
        self.get_base_path()
    }
    pub fn resolve_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        clean_join(&self.get_base_path(), path.as_ref())
    }
    pub fn ResolvePath<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        self.resolve_path(path)
    }
    pub fn resolve_existing_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        let path = path.as_ref();
        let base = self.get_base_path();
        let normalized = path.to_string_lossy().replace('\\', "/");
        if normalized.trim_matches('/').is_empty() {
            let world = clean_join(&base, Path::new("world"));
            if world.exists() {
                return world;
            }
        }
        let direct = clean_join(&base, path);
        if direct.exists() {
            return direct;
        }
        if let Some(rest) = normalized.strip_prefix("world/") {
            let server_path = clean_join(&base, Path::new(rest));
            if server_path.exists() {
                return server_path;
            }
            return direct;
        }
        let world = clean_join(&clean_join(&base, Path::new("world")), path);
        if world.exists() {
            return world;
        }
        direct
    }
    pub fn ResolveExistingPath<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        self.resolve_existing_path(path)
    }
    pub fn file_exists<P: AsRef<Path>>(&self, path: P) -> bool {
        self.resolve_existing_path(path).exists()
    }
    pub fn FileExists<P: AsRef<Path>>(&self, path: P) -> bool {
        self.file_exists(path)
    }
    pub fn load_file<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<u8>> {
        fs::read(self.resolve_existing_path(path))
    }
    pub fn LoadFile<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<u8>> {
        self.load_file(path)
    }
    pub fn save_file<P: AsRef<Path>>(&self, path: P, data: &[u8]) -> io::Result<()> {
        let full = self.resolve_path(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(full, data)
    }
    pub fn SaveFile<P: AsRef<Path>>(&self, path: P, data: &[u8]) -> io::Result<()> {
        self.save_file(path, data)
    }
    pub fn delete_file<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let full = self.resolve_path(path);
        match fs::remove_file(&full) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::IsADirectory => fs::remove_dir(full),
            Err(error) => Err(error),
        }
    }
    pub fn DeleteFile<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        self.delete_file(path)
    }
    pub fn list_files<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<String>> {
        let mut result = Vec::new();
        for entry in fs::read_dir(self.resolve_existing_path(path))? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                result.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        result.sort();
        Ok(result)
    }
    pub fn ListFiles<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<String>> {
        self.list_files(path)
    }
    pub fn list_dirs<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<String>> {
        let mut result = Vec::new();
        for entry in fs::read_dir(self.resolve_existing_path(path))? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                result.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        result.sort();
        Ok(result)
    }
    pub fn ListDirs<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<String>> {
        self.list_dirs(path)
    }
    pub fn file_mod_time<P: AsRef<Path>>(&self, path: P) -> io::Result<SystemTime> {
        fs::metadata(self.resolve_existing_path(path))?.modified()
    }
    pub fn FileModTime<P: AsRef<Path>>(&self, path: P) -> io::Result<SystemTime> {
        self.file_mod_time(path)
    }
    pub fn file_info<P: AsRef<Path>>(&self, path: P) -> io::Result<FileInfo> {
        let full = self.resolve_existing_path(path);
        let meta = fs::metadata(&full)?;
        Ok(FileInfo {
            name: full
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified: meta.modified().unwrap_or(UNIX_EPOCH),
        })
    }
    pub fn FileInfo<P: AsRef<Path>>(&self, path: P) -> io::Result<FileInfo> {
        self.file_info(path)
    }
    pub fn file_size<P: AsRef<Path>>(&self, path: P) -> io::Result<u64> {
        Ok(self.file_info(path)?.size)
    }
    pub fn FileSize<P: AsRef<Path>>(&self, path: P) -> io::Result<u64> {
        self.file_size(path)
    }
    pub fn cache_file<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let key = path.as_ref().to_string_lossy().into_owned();
        let data = self.load_file(path)?;
        self.cache.write().unwrap().insert(key, data);
        Ok(())
    }
    pub fn CacheFile<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        self.cache_file(path)
    }
    pub fn get_cached(&self, path: &str) -> Option<Vec<u8>> {
        self.cache.read().unwrap().get(path).cloned()
    }
    pub fn GetCached(&self, path: &str) -> Option<Vec<u8>> {
        self.get_cached(path)
    }
    pub fn clear_cache(&self) {
        self.cache.write().unwrap().clear();
    }
    pub fn ClearCache(&self) {
        self.clear_cache()
    }
    pub fn load_file_as_lines<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<String>> {
        let data = self.load_file(path)?;
        // Match bufio.Scanner with its default ScanLines split function.
        // ScanLines omits the trailing newline (and an optional preceding
        // carriage return), does not manufacture an empty line after a final
        // newline, and exposes non-UTF-8 input as lossy text. Scanner also
        // rejects tokens at its
        // default 64 KiB limit.
        const MAX_SCAN_TOKEN_SIZE: usize = 64 * 1024;
        let mut lines = Vec::new();
        let mut start = 0usize;
        while start < data.len() {
            let end = data[start..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|offset| start + offset)
                .unwrap_or(data.len());
            let mut line = &data[start..end];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            if line.len() >= MAX_SCAN_TOKEN_SIZE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "bufio.Scanner: token too long",
                ));
            }
            lines.push(String::from_utf8_lossy(line).into_owned());
            if end == data.len() {
                break;
            }
            start = end + 1;
        }
        Ok(lines)
    }
    pub fn LoadFileAsLines<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<String>> {
        self.load_file_as_lines(path)
    }
    pub fn save_lines_as_file<P: AsRef<Path>>(&self, path: P, lines: &[String]) -> io::Result<()> {
        let mut data = String::new();
        for line in lines {
            data.push_str(line);
            data.push('\n');
        }
        self.save_file(path, data.as_bytes())
    }
    pub fn SaveLinesAsFile<P: AsRef<Path>>(&self, path: P, lines: &[String]) -> io::Result<()> {
        self.save_lines_as_file(path, lines)
    }
    pub fn copy_file<P: AsRef<Path>>(&self, src: P, dst: P) -> io::Result<()> {
        let src = self.resolve_path(src);
        let dst = self.resolve_path(dst);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut input = File::open(src)?;
        let mut output = File::create(dst)?;
        io::copy(&mut input, &mut output).map(|_| ())
    }
    pub fn CopyFile<P: AsRef<Path>>(&self, src: P, dst: P) -> io::Result<()> {
        self.copy_file(src, dst)
    }
}

/// `filepath.Join` on the reference platform concatenates and then lexically
/// cleans the path.  `Path::join` would discard the base for an absolute
/// second component and would retain `..`, so keep the reference behavior
/// explicit here.
fn clean_join(base: &Path, path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    append_clean_components(&mut result, base, true);
    append_clean_components(&mut result, path, false);
    result
}

fn append_clean_components(result: &mut PathBuf, path: &Path, keep_root: bool) {
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::RootDir => {
                if keep_root && result.as_os_str().is_empty() {
                    result.push(Path::new("/"));
                }
            }
            Component::Prefix(prefix) => {
                if keep_root {
                    result.push(prefix.as_os_str());
                }
            }
            Component::ParentDir => {
                if result.file_name().is_some() {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            Component::Normal(value) => result.push(value),
        }
    }
}
