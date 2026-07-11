use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::server::Server;
use crate::encryption::Encryption;
use crate::buffer::Buffer;
use crate::{log_info, log_error};

pub struct SocketSession {
    pub stream: TcpStream,
    pub encryption: Encryption,
    pub read_buf: Vec<u8>,
}

impl SocketSession {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            encryption: Encryption::new(),
            read_buf: vec![0; 8192],
        }
    }
    
    pub async fn read_packet(&mut self) -> Result<Option<Vec<u8>>, std::io::Error> {
        let n = tokio::io::AsyncReadExt::read(&mut self.stream, &mut self.read_buf).await?;
        if n == 0 {
            return Ok(None);
        }
        
        let mut packet = self.read_buf[..n].to_vec();
        self.encryption.decrypt(&mut packet);
        
        Ok(Some(packet))
    }
    
    pub async fn write_packet(&mut self, data: &[u8]) -> Result<(), std::io::Error> {
        let encrypted = self.encryption.encrypt(data);
        tokio::io::AsyncWriteExt::write_all(&mut self.stream, &encrypted).await
    }
}

pub async fn start_socket_server(server: Arc<RwLock<Server>>, bind_addr: &str) {
    let listener = match tokio::net::TcpListener::bind(bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            crate::log_error!("Failed to bind TCP listener on {}: {}", bind_addr, e);
            return;
        }
    };
    
    crate::log_info!("Listening for client connections on {}", bind_addr);
    
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                crate::log_info!("New connection from {}", addr);
                let server_clone = server.clone();
                tokio::spawn(async move {
                    handle_client(server_clone, stream).await;
                });
            }
            Err(e) => {
                crate::log_error!("Accept error: {}", e);
            }
        }
    }
}

async fn handle_client(server: Arc<RwLock<Server>>, stream: tokio::net::TcpStream) {
    let mut session = SocketSession::new(stream);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);
    
    // Find next available ID and create Player
    let player_arc = {
        let mut srv = server.write().await;
        let mut id = 1;
        while srv.players.contains_key(&id) {
            id += 1;
        }
        let mut player = crate::player::Player::new(server.clone(), id);
        player.tx = Some(tx);
        let arc = Arc::new(RwLock::new(player));
        srv.players.insert(id, arc.clone());
        arc
    };
    
    loop {
        tokio::select! {
            packet_opt_res = tokio::time::timeout(tokio::time::Duration::from_secs(300), session.read_packet()) => {
                match packet_opt_res {
                    Ok(Ok(Some(packet))) => {
                        if packet.is_empty() { continue; }
                        crate::control::handle_packet(server.clone(), player_arc.clone(), &packet).await;
                    }
                    Ok(Ok(None)) => break, // EOF
                    Ok(Err(e)) => {
                        crate::log_error!("Read error: {}", e);
                        break;
                    }
                    Err(_) => {
                        crate::log_info!("Client timed out");
                        break; // Timeout
                    }
                }
            }
            Some(data) = rx.recv() => {
                if let Err(e) = session.write_packet(&data).await {
                    crate::log_error!("Write error: {}", e);
                    break;
                }
            }
        }
    }
    
    // Cleanup
    let mut p = player_arc.write().await;
    p.disconnect().await;
}
