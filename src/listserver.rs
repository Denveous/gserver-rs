use std::time::SystemTime;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::server::Server;
use crate::encryption::{Encryption, ENCRYPT_GEN_1, ENCRYPT_GEN_2};
use crate::buffer::Buffer;
use crate::protocol::*;

pub struct CachedListserverServer {
    pub name: String,
    pub server_type: String,
    pub player_count: i32,
    pub language: String,
    pub description: String,
    pub url: String,
    pub version: String,
    pub game_versions: String,
    pub latency: i32,
    pub updated: SystemTime,
}

#[derive(Clone)]
pub struct ListserverEndpoint {
    pub host: String,
    pub port: String,
}

pub struct ListserverManager {
    pub cache: DashMap<String, CachedListserverServer>,
    pub endpoints: Vec<ListserverEndpoint>,
    pub enabled: bool,
}

impl Default for ListserverManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ListserverManager {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
            endpoints: Vec::new(),
            enabled: true,
        }
    }

    pub fn configure_endpoints(&mut self, hosts: &str, ports: &str) {
        let host_parts: Vec<&str> = hosts.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        let mut port_parts: Vec<&str> = ports.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        
        if host_parts.is_empty() {
            return;
        }
        
        if port_parts.is_empty() {
            port_parts.push("14900");
        }

        self.endpoints.clear();
        for (i, &host) in host_parts.iter().enumerate() {
            let port = if i < port_parts.len() { port_parts[i] } else { port_parts[0] };
            self.endpoints.push(ListserverEndpoint {
                host: host.to_string(),
                port: port.to_string(),
            });
        }
    }

    pub fn cache_listserver_text(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(data);
        let text = text.trim_end_matches('\0').trim();
        if text.is_empty() {
            return;
        }

        if text.to_lowercase().starts_with("listserver,modify,server,") {
            self.cache_listserver_modify(text);
            return;
        }

        for record in self.listserver_records(text) {
            self.cache_listserver_record(&record);
        }
    }

    fn cache_listserver_modify(&self, text: &str) {
        let fields: Vec<&str> = text.split(',').collect();
        if fields.len() < 4 {
            return;
        }
        
        let name = fields[3].trim();
        if name.is_empty() {
            return;
        }

        let mut server = match self.cache.get(name) {
            Some(existing) => CachedListserverServer {
                name: existing.name.clone(),
                server_type: existing.server_type.clone(),
                player_count: existing.player_count,
                language: existing.language.clone(),
                description: existing.description.clone(),
                url: existing.url.clone(),
                version: existing.version.clone(),
                game_versions: existing.game_versions.clone(),
                latency: existing.latency,
                updated: SystemTime::now(),
            },
            None => CachedListserverServer {
                name: name.to_string(),
                server_type: String::new(),
                player_count: 0,
                language: String::new(),
                description: String::new(),
                url: String::new(),
                version: String::new(),
                game_versions: String::new(),
                latency: 0,
                updated: SystemTime::now(),
            },
        };

        for field in &fields[4..] {
            let field = field.trim();
            if let Some((key, value)) = field.split_once('=') {
                self.apply_field(&mut server, key, value);
            }
        }
        
        self.cache.insert(name.to_lowercase(), server);
    }

    fn cache_listserver_record(&self, record: &str) {
        let fields = self.split_listserver_fields(record);
        if fields.len() < 3 {
            return;
        }

        let name = fields[0].trim();
        if name.is_empty() || name.eq_ignore_ascii_case("Listserver") || name.eq_ignore_ascii_case("GraalEngine") {
            return;
        }

        let mut server = CachedListserverServer {
            name: name.to_string(),
            server_type: String::new(),
            player_count: 0,
            language: String::new(),
            description: String::new(),
            url: String::new(),
            version: String::new(),
            game_versions: String::new(),
            latency: 0,
            updated: SystemTime::now(),
        };

        if fields.len() > 1 { server.server_type = fields[1].clone(); }
        if fields.len() > 2 { server.player_count = fields[2].trim().parse().unwrap_or(0); }
        if fields.len() > 3 { server.language = fields[3].clone(); }
        if fields.len() > 4 { server.description = fields[4].clone(); }
        if fields.len() > 5 { server.url = fields[5].clone(); }
        if fields.len() > 6 { server.version = fields[6].clone(); }
        if fields.len() > 7 { server.game_versions = fields[7].clone(); }
        if fields.len() > 8 { server.latency = fields[8].trim().parse().unwrap_or(0); }

        self.cache.insert(name.to_lowercase(), server);
    }

    fn listserver_records(&self, text: &str) -> Vec<String> {
        let mut t = text.replace("\r\n", "\n").replace('\r', "\n");
        if t.contains('\x01') {
            t = t.replace('\x01', "\n");
        }
        t.split('\n').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    }

    fn split_listserver_fields(&self, record: &str) -> Vec<String> {
        if record.contains('\x01') {
            record.split('\x01').map(|s| s.trim().to_string()).collect()
        } else if record.contains(',') {
            // A simplified assumption of guntokenizeText equivalent for now
            record.split('\n').map(|s| s.trim().to_string()).collect()
        } else {
            Vec::new()
        }
    }

    fn apply_field(&self, server: &mut CachedListserverServer, key: &str, value: &str) {
        match key.trim().to_lowercase().as_str() {
            "name" => server.name = value.to_string(),
            "type" => server.server_type = value.to_string(),
            "players" | "playercount" => server.player_count = value.trim().parse().unwrap_or(0),
            "language" => server.language = value.to_string(),
            "description" | "desc" => server.description = value.to_string(),
            "url" | "website" => server.url = value.to_string(),
            "version" | "serverversion" => server.version = value.to_string(),
            "gameversions" | "allowedversions" => server.game_versions = value.to_string(),
            "latency" | "ping" => server.latency = value.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
}

pub struct ServerListConnection {
    pub server: Arc<RwLock<Server>>,
    pub endpoint: ListserverEndpoint,
    pub encryption: Encryption,
}

impl ServerListConnection {
    pub fn new(server: Arc<RwLock<Server>>, endpoint: ListserverEndpoint) -> Self {
        Self {
            server,
            endpoint,
            encryption: Encryption::new(),
        }
    }

    async fn send_packet(&mut self, stream: &mut TcpStream, mut data: Vec<u8>) {
        if data.is_empty() || *data.last().unwrap() != b'\n' {
            data.push(b'\n');
        }

        if self.encryption.get_gen() == ENCRYPT_GEN_2 {
            use flate2::write::ZlibEncoder;
            use flate2::Compression;
            use std::io::Write;
            
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&data).unwrap();
            let compressed = encoder.finish().unwrap();
            
            let mut buf = Buffer::new();
            buf.write_short(compressed.len() as i16);
            buf.write_bytes(&compressed);
            data = buf.bytes().to_vec();
        }

        let _ = stream.write_all(&data).await;
    }

    pub async fn run(&mut self) {
        let addr = format!("{}:{}", self.endpoint.host, self.endpoint.port);
        crate::log_info!(":: Initializing listserver socket ({}).", addr);
        
        let mut stream = match tokio::time::timeout(std::time::Duration::from_secs(5), TcpStream::connect(&addr)).await {
            Ok(Ok(s)) => s,
            _ => {
                crate::log_error!("Could not connect listserver socket");
                return;
            }
        };

        crate::log_info!(":: listserver - Connected ({}).", addr);

        self.encryption.set_gen(ENCRYPT_GEN_1);
        
        let mut buf = Buffer::new();
        buf.write_gchar(SVO_REGISTERV3);
        buf.write_string("3.0.9-RS");
        self.send_packet(&mut stream, buf.bytes().to_vec()).await;
        
        self.encryption.set_gen(ENCRYPT_GEN_2);
        
        let (hq_password, name, desc, lang, url, ip, port, localip) = {
            let srv = self.server.read().await;
            (
                srv.settings.get("hq_password").unwrap_or_else(|| "".to_string()),
                srv.settings.get("name").unwrap_or_else(|| srv.name.clone()),
                srv.settings.get("description").unwrap_or_else(|| srv.name.clone()),
                srv.settings.get("language").unwrap_or_else(|| "English".to_string()),
                srv.settings.get("url").unwrap_or_else(|| "http://www.graal.in/".to_string()),
                srv.settings.get("serverip").unwrap_or_else(|| "AUTO".to_string()),
                srv.settings.get("serverport").unwrap_or_else(|| "14802".to_string()),
                srv.settings.get("localip").unwrap_or_else(|| "AUTO".to_string()),
            )
        };
        
        let mut buf = Buffer::new();
        buf.write_gchar(SVO_SERVERHQPASS);
        buf.write_string8(&hq_password);
        self.send_packet(&mut stream, buf.bytes().to_vec()).await;
        
        let mut buf = Buffer::new();
        let version = "3.0.9-RS";
        
        buf.write_gchar(SVO_NEWSERVER);
        buf.write_string8_encoded(&name);
        buf.write_string8_encoded(&desc);
        buf.write_string8_encoded(&lang);
        buf.write_string8_encoded(version);
        buf.write_string8_encoded(&url);
        buf.write_string8_encoded(&ip);
        buf.write_string8_encoded(&port);
        buf.write_string8_encoded(&localip);
        self.send_packet(&mut stream, buf.bytes().to_vec()).await;
        
        let mut buf = Buffer::new();
        buf.write_gchar(SVO_SERVERHQLEVEL);
        buf.write_gchar(1);
        self.send_packet(&mut stream, buf.bytes().to_vec()).await;
        
        let versions = {
            let srv = self.server.read().await;
            srv.settings.get("allowedclientversions").unwrap_or_else(|| "V3,V4,V5,V6,V7,V8".to_string())
        };
        
        let text = format!("Listserver,settings,allowedversions,{}", versions);
        let mut buf = Buffer::new();
        buf.write_gchar(SVO_SENDTEXT);
        buf.write_bytes(text.as_bytes());
        self.send_packet(&mut stream, buf.bytes().to_vec()).await;
        
        let mut read_buffer = Vec::new();
        let mut read_buf = vec![0; 4096];
        loop {
            match stream.read(&mut read_buf).await {
                Ok(0) => break,
                Ok(n) => {
                    read_buffer.extend_from_slice(&read_buf[..n]);
                    
                    while !read_buffer.is_empty() {
                        if read_buffer[0] >= 32 {
                            if let Some(nl) = read_buffer.iter().position(|&b| b == b'\n') {
                                let packet = read_buffer[..nl].to_vec();
                                read_buffer.drain(..nl + 1);
                                if !packet.is_empty() {
                                    let packet_id = packet[0].saturating_sub(32);
                                    if packet_id == SVI_PING {
                                        let mut buf = Buffer::new();
                                        buf.write_gchar(SVO_PING);
                                        self.send_packet(&mut stream, buf.bytes().to_vec()).await;
                                    }
                                }
                                continue;
                            }
                            break;
                        }
                        
                        if read_buffer.len() < 2 {
                            break;
                        }
                        
                        let length = ((read_buffer[0] as usize) << 8) | (read_buffer[1] as usize);
                        if read_buffer.len() < length + 2 {
                            break;
                        }
                        
                        let compressed = read_buffer[2..length + 2].to_vec();
                        read_buffer.drain(..length + 2);
                        
                        use flate2::read::ZlibDecoder;
                        use std::io::Read;
                        let mut decoder = ZlibDecoder::new(&compressed[..]);
                        let mut decompressed = Vec::new();
                        if decoder.read_to_end(&mut decompressed).is_ok() {
                            let mut start = 0;
                            while let Some(nl) = decompressed[start..].iter().position(|&b| b == b'\n') {
                                let packet = &decompressed[start..start + nl];
                                start += nl + 1;
                                if !packet.is_empty() {
                                    let packet_id = packet[0].saturating_sub(32);
                                    if packet_id == SVI_PING {
                                        let mut buf = Buffer::new();
                                        buf.write_gchar(SVO_PING);
                                        self.send_packet(&mut stream, buf.bytes().to_vec()).await;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
    }
}
