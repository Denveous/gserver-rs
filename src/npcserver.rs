use std::sync::Arc;
use tokio::sync::RwLock;
use crate::server::Server;
use crate::player::Player;

pub struct NPCServer {
    pub host: Arc<RwLock<Server>>,
}

impl NPCServer {
    pub fn new(host: Arc<RwLock<Server>>) -> Self {
        Self { host }
    }
    
    pub async fn start(&mut self) -> Option<Player> {
        let host = self.host.read().await;
        let mut p = Player::new(self.host.clone(), 1);
        p.account_name = "npcserver".to_string();
        p.player_type = 2; // PLTYPE_NPCSERVER
        p.character.head_image = "head25.png".to_string();
        p.character.nick_name = "NPC Server".to_string();
        Some(p)
    }
}
