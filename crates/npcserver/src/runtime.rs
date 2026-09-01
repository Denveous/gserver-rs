use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

pub const ACCOUNT_NAME: &str = "(npcserver)";
pub const DEFAULT_PM_REPLY: &str =
    "I am the npcserver for\nthis game server. Almost\nall npc actions are controlled\nby me.";

pub type Logger = Arc<dyn Fn(String) + Send + Sync + 'static>;
pub type FileEventHandler = Arc<dyn Fn(String, String) + Send + Sync + 'static>;

struct RuntimeInner {
    stopped: AtomicBool,
    watching: AtomicBool,
    stop: Mutex<Option<mpsc::Sender<()>>>,
    debounce: Mutex<HashMap<PathBuf, Instant>>,
}

pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RuntimeInner {
                stopped: AtomicBool::new(false),
                watching: AtomicBool::new(false),
                stop: Mutex::new(None),
                debounce: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn enabled(&self, serverside: bool) -> bool {
        serverside && !self.inner.stopped.load(Ordering::Acquire)
    }

    pub fn set_stopped(&self, stopped: bool) {
        self.inner.stopped.store(stopped, Ordering::Release);
    }

    pub fn start_watching(
        &self,
        base: impl AsRef<Path>,
        warn: Option<Logger>,
        info: Option<Logger>,
        handler: Option<FileEventHandler>,
    ) {
        if self.inner.watching.swap(true, Ordering::AcqRel) {
            return;
        }
        let base = base.as_ref().to_path_buf();
        for directory in ["weapons", "scripts"] {
            if let Err(error) = fs::metadata(base.join(directory)) {
                if let Some(callback) = &warn {
                    callback(format!("Could not watch {directory}: {error}"));
                }
            }
        }
        if let Some(callback) = &info {
            callback("Watching weapons/ and scripts/ for live reload".to_string());
        }
        let (sender, receiver) = mpsc::channel();
        *self.inner.stop.lock().expect("runtime stop mutex poisoned") = Some(sender);
        let inner = Arc::clone(&self.inner);
        thread::spawn(move || watch_loop(inner, base, warn, handler, receiver));
    }

    pub fn stop_watching(&self) {
        if !self.inner.watching.swap(false, Ordering::AcqRel) {
            return;
        }
        if let Some(sender) = self
            .inner
            .stop
            .lock()
            .expect("runtime stop mutex poisoned")
            .take()
        {
            let _ = sender.send(());
        }
    }

    #[allow(non_snake_case)]
    pub fn Enabled(&self, serverside: bool) -> bool {
        self.enabled(serverside)
    }

    #[allow(non_snake_case)]
    pub fn SetStopped(&self, stopped: bool) {
        self.set_stopped(stopped)
    }

    #[allow(non_snake_case)]
    pub fn StartWatching(
        &self,
        base: impl AsRef<Path>,
        warn: Option<Logger>,
        info: Option<Logger>,
        handler: Option<FileEventHandler>,
    ) {
        self.start_watching(base, warn, info, handler)
    }

    #[allow(non_snake_case)]
    pub fn StopWatching(&self) {
        self.stop_watching()
    }
}

fn watch_loop(
    inner: Arc<RuntimeInner>,
    base: PathBuf,
    warn: Option<Logger>,
    handler: Option<FileEventHandler>,
    receiver: mpsc::Receiver<()>,
) {
    let mut known = HashMap::<PathBuf, SystemTime>::new();
    let mut initialized = false;
    while inner.watching.load(Ordering::Acquire) {
        if receiver.try_recv().is_ok() {
            break;
        }
        for directory in ["weapons", "scripts"] {
            scan_directory(
                &inner,
                &base,
                &base.join(directory),
                &mut known,
                warn.as_ref(),
                if initialized { handler.as_ref() } else { None },
            );
        }
        initialized = true;
        thread::sleep(Duration::from_millis(100));
    }
}

fn scan_directory(
    inner: &RuntimeInner,
    base: &Path,
    directory: &Path,
    known: &mut HashMap<PathBuf, SystemTime>,
    warn: Option<&Logger>,
    handler: Option<&FileEventHandler>,
) {
    let entries = match fs::read_dir(directory) {
        Ok(value) => value,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(value) if value.is_file() => value,
            _ => continue,
        };
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let changed = known.get(&path).is_none_or(|old| *old != modified);
        known.insert(path.clone(), modified);
        if !changed {
            continue;
        }
        let now = Instant::now();
        {
            let mut debounce = inner
                .debounce
                .lock()
                .expect("runtime debounce mutex poisoned");
            if debounce
                .get(&path)
                .is_some_and(|last| now.duration_since(*last) < Duration::from_millis(500))
            {
                continue;
            }
            debounce.insert(path.clone(), now);
        }
        thread::sleep(Duration::from_millis(100));
        let relative = match path.strip_prefix(base) {
            Ok(value) => value.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        if let Some(callback) = handler {
            callback(
                relative,
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
    let _ = warn;
}

pub fn configured_nickname(nickname: &str) -> String {
    let mut value = nickname.trim().to_string();
    if value.is_empty() {
        value = "NPC-Server".to_string();
    }
    if !value.to_ascii_lowercase().contains("(server)") {
        value.push_str(" (Server)");
    }
    value
}

pub fn is_location_query(packet: &[u8], query_packet_id: u8) -> bool {
    if packet.is_empty() {
        return true;
    }
    let mut value = packet;
    if value[0] == query_packet_id {
        value = &value[1..];
    }
    if value.len() >= 2 {
        value = &value[2..];
    }
    let message = String::from_utf8_lossy(value)
        .trim_matches(|x: char| x == '\0' || x == '\r' || x == '\n' || x == '\t' || x == ' ')
        .to_string();
    message.is_empty() || message.eq_ignore_ascii_case("location")
}

pub fn address_for(
    admin_setting: Option<&dyn Fn(&str) -> String>,
    setting: Option<&dyn Fn(&str) -> String>,
    requester_ip: &str,
) -> String {
    let mut host = admin_setting.map(|f| f("ns_ip")).unwrap_or_default();
    if host.is_empty() {
        host = setting.map(|f| f("ns_ip")).unwrap_or_default();
    }
    if host.is_empty() || host.eq_ignore_ascii_case("auto") {
        host = setting.map(|f| f("serverip")).unwrap_or_default();
    }
    if !requester_ip.is_empty()
        && let Some(get_setting) = setting
        && host == get_setting("localip")
    {
        host = requester_ip.to_string();
    }
    if host.is_empty() || host.eq_ignore_ascii_case("auto") {
        host = "127.0.0.1".to_string();
    }
    let port = setting
        .map(|f| f("serverport"))
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| "14802".to_string());
    format!("{host},{port}")
}

#[allow(non_snake_case)]
pub fn New() -> Runtime {
    Runtime::new()
}

#[allow(non_snake_case)]
pub fn ConfiguredNickname(nickname: &str) -> String {
    configured_nickname(nickname)
}

#[allow(non_snake_case)]
pub fn IsLocationQuery(packet: &[u8], query_packet_id: u8) -> bool {
    is_location_query(packet, query_packet_id)
}

#[allow(non_snake_case)]
pub fn AddressFor(
    admin_setting: Option<&dyn Fn(&str) -> String>,
    setting: Option<&dyn Fn(&str) -> String>,
    requester_ip: &str,
) -> String {
    address_for(admin_setting, setting, requester_ip)
}
