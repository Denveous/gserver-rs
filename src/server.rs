use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::{Settings, FileSystem};
use crate::listserver::ListserverManager;
use crate::player::Player;
use crate::npc::NPC;
use crate::weapon::{Weapon, ScriptClass};
use crate::{log_info, log_error};

pub struct Server {
    pub name: String,
    pub config_base_path: String,
    pub settings: Settings,
    pub fs: FileSystem,
    pub listserver_manager: ListserverManager,
    pub players: HashMap<u16, Arc<tokio::sync::RwLock<Player>>>,
    pub npcs: HashMap<u32, Arc<tokio::sync::RwLock<NPC>>>,
    pub weapons: HashMap<String, Weapon>,
    pub classes: HashMap<String, ScriptClass>,
    pub flags: HashMap<String, String>,
    pub levels: HashMap<String, Arc<RwLock<crate::level::Level>>>,
    pub allowed_versions: Vec<String>,
    pub bind_addr: String,
    pub restart_requested: bool,
}

impl Server {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            config_base_path: String::new(),
            settings: Settings::new(),
            fs: FileSystem::new(""),
            listserver_manager: ListserverManager::new(),
            players: HashMap::new(),
            npcs: HashMap::new(),
            weapons: HashMap::new(),
            classes: HashMap::new(),
            flags: HashMap::new(),
            levels: HashMap::new(),
            allowed_versions: Vec::new(),
            bind_addr: String::new(),
            restart_requested: false,
        }
    }

    pub async fn init(&mut self, _server_ip: &str, _port: &str, _local_ip: &str, _interface: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.fs.set_base_path(&self.config_base_path);
        
        let server_ini = format!("{}config/server.ini", self.config_base_path); // Fixed path
        if let Err(e) = self.settings.load(&server_ini) {
            crate::log_error!("Failed to load {}: {}", server_ini, e);
        }
        
        self.load_flags();
        self.load_allowed_versions();

        self.listserver_manager.enabled = self.settings.get_bool("listserver", true);
        let listip = self.settings.get_or("listip", "listserver.graal.in");
        let listport = self.settings.get_or("listport", "14900");
        self.listserver_manager.configure_endpoints(&listip, &listport);

        let bind_ip = if !_server_ip.is_empty() { _server_ip.to_string() } else { self.settings.get_or("serverip", "0.0.0.0") };
        let bind_port = if !_port.is_empty() { _port.to_string() } else { self.settings.get_or("serverport", "14802") };
        self.bind_addr = format!("{}:{}", if bind_ip == "AUTO" { "0.0.0.0" } else { &bind_ip }, bind_port);

        Ok(())
    }

    pub fn load_flags(&mut self) {
        if let Ok(data) = self.fs.load_file("config/serverflags.txt") {
            let content = String::from_utf8_lossy(&data);
            for line in content.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    if !k.is_empty() {
                        self.flags.insert(k.to_string(), v.to_string());
                    }
                }
            }
        }
    }

    pub fn load_allowed_versions(&mut self) {
        self.allowed_versions.clear();
        let path = format!("{}config/allowedversions.txt", self.config_base_path);
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                let mut clean_line = line.to_string();
                if let Some(idx) = clean_line.find("//") {
                    clean_line = clean_line[..idx].to_string();
                }
                clean_line = clean_line.replace('\r', "").replace('\t', "").replace(' ', "");
                let clean_line = clean_line.trim();
                if clean_line.is_empty() {
                    continue;
                }
                self.allowed_versions.push(clean_line.to_string());
            }
        } else {
            crate::log_error!("Could not open config/allowedversions.txt. No client version list will be sent to the listserver.");
        }
    }

    pub fn allowed_versions_listserver_text(&self) -> String {
        self.allowed_versions.join(",")
    }

    pub fn load_settings(&mut self) {
        if let Some(name) = self.settings.get("name") {
            if !name.is_empty() {
                self.name = name;
            }
        }
    }

    pub async fn run(srv: Arc<RwLock<Self>>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        {
            let srv_read = srv.read().await;
            if srv_read.listserver_manager.enabled {
                for endpoint in srv_read.listserver_manager.endpoints.clone() {
                    let mut conn = crate::listserver::ServerListConnection::new(srv.clone(), endpoint);
                    tokio::spawn(async move {
                        conn.run().await;
                    });
                }
            }
            
            if !srv_read.bind_addr.is_empty() {
                let bind_addr = srv_read.bind_addr.clone();
                tokio::spawn(crate::network::start_socket_server(srv.clone(), bind_addr));
            }
        }
        
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(50));
        let mut last_second = std::time::Instant::now();
        let mut last_minute = std::time::Instant::now();
        let mut last_five_minute = std::time::Instant::now();
        
        loop {
            interval.tick().await;
            let now = std::time::Instant::now();
            
            if now.duration_since(last_second).as_secs() >= 1 {
                last_second = now;
                let mut s = srv.write().await;
                s.one_second_events().await;
            }
            if now.duration_since(last_minute).as_secs() >= 60 {
                last_minute = now;
                let mut s = srv.write().await;
                s.one_minute_events().await;
            }
            if now.duration_since(last_five_minute).as_secs() >= 300 {
                last_five_minute = now;
                let mut s = srv.write().await;
                s.five_minute_events().await;
            }
        }
    }

    pub async fn one_second_events(&mut self) {
        // Decrement NPC timeouts and broadcast if needed
        let mut npcs_to_wake = Vec::new();
        for (id, npc_arc) in &self.npcs {
            let mut npc = npc_arc.write().await;
            if npc.timeout > 0 {
                npc.timeout -= 1;
                if npc.timeout == 0 {
                    npcs_to_wake.push(*id);
                }
            }
        }
        // TODO: Handle awoken NPCs
    }

    pub async fn one_minute_events(&mut self) {
        crate::log_info!("One minute timer");
        // TODO: Save player accounts
    }

    pub async fn five_minute_events(&mut self) {
        crate::log_info!("Five minute timer - saving data");
        self.save_flags();
    }

    pub fn save_flags(&self) {
        let mut lines = Vec::new();
        for (key, value) in &self.flags {
            if !key.is_empty() && !key.contains('\n') && !value.contains('\n') {
                lines.push(format!("{}={}", key, value));
            }
        }
        let data = lines.join("\n") + "\n";
        if let Err(e) = self.fs.save_file("config/serverflags.txt", data.as_bytes()) {
            crate::log_error!("Could not save serverflags.txt: {}", e);
        }
    }

    pub fn is_default_client_file(&self, _file_name: &str) -> bool {
        false // TODO
    }

    pub fn resolve_requested_file(&self, file_name: &str) -> Result<String, String> {
        Ok(format!("{}{}", self.config_base_path, file_name)) // TODO: Security checks
    }

    pub fn run_server_side_event_for_active_scripts(&self, _event_name: &str, _args: &[&str]) {
        // TODO
    }

    pub fn stop(&mut self) {
        // Trigger shutdown signals
    }

    pub async fn send_rc_chat(&self, message: &str) {
        use crate::buffer::Buffer;
        use crate::protocol::*;
        let mut buf = Buffer::new();
        buf.write_byte(PLO_RC_CHAT);
        buf.write_bytes(message.as_bytes());
        let packet = buf.bytes();

        for player_lock in self.players.values() {
            let p = player_lock.read().await;
            if (p.player_type & PLTYPE_ANYRC) != 0 || p.player_type == PLTYPE_RC || p.player_type == PLTYPE_RC2 {
                if let Some(tx) = &p.tx {
                    let _ = tx.send(packet.to_vec()).await;
                }
            }
        }
    }
}

