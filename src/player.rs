use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{SystemTime, Instant};

use crate::server::Server;
use crate::buffer::Buffer;
use crate::protocol::*;

#[derive(Clone, Default)]
pub struct Character {
    pub nick_name: String,
    pub chat_message: String,
    pub head_image: String,
    pub body_image: String,
    pub sword_image: String,
    pub shield_image: String,
    pub colors: [u8; 5],
}

pub struct Player {
    pub id: u16,
    pub account_name: String,
    pub email: String,
    pub is_banned: bool,
    pub is_load_only: bool,
    pub is_staff: bool,
    pub player_type: i32,
    pub character: Character,
    
    pub x: i16,
    pub y: i16,
    pub z: i16,
    pub status: i32,
    
    pub kills: u32,
    pub deaths: u32,
    pub online_time: u32,
    
    pub admin_rights: i32,
    pub guild: String,
    pub level_name: String,
    
    pub last_movement: Instant,
    pub last_nick: Instant,
    pub last_save: Instant,
    
    pub flag_list: HashMap<String, String>,
    pub chest_list: Vec<String>,
    pub weapon_list: Vec<String>,
    
    pub is_ftp: bool,
    pub last_folder: String,
    pub folder_list: String,

    pub tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,

    pub server: Arc<RwLock<Server>>,
}

impl Player {
    pub fn new(server: Arc<RwLock<Server>>, id: u16) -> Self {
        Self {
            id,
            account_name: String::new(),
            email: String::new(),
            is_banned: false,
            is_load_only: false,
            is_staff: false,
            player_type: 1, // PLTYPE_CLIENT
            character: Character {
                nick_name: String::new(),
                chat_message: String::new(),
                head_image: "head0.png".to_string(),
                body_image: "body.png".to_string(),
                sword_image: "sword1.png".to_string(),
                shield_image: "shield1.png".to_string(),
                colors: [0, 0, 0, 0, 0],
            },
            x: 0,
            y: 0,
            z: 0,
            status: 0,
            kills: 0,
            deaths: 0,
            online_time: 0,
            admin_rights: 0,
            guild: String::new(),
            level_name: String::new(),
            last_movement: Instant::now(),
            last_nick: Instant::now(),
            last_save: Instant::now(),
            flag_list: HashMap::new(),
            chest_list: Vec::new(),
            weapon_list: Vec::new(),
            is_ftp: false,
            last_folder: String::new(),
            folder_list: String::new(),
            tx: None,
            server,
        }
    }

    pub fn set_chat(&mut self, mut chat: String) {
        if chat.len() > 223 {
            chat.truncate(223);
        }
        self.character.chat_message = chat;
        // self.send_player_prop_changes(PLPROP_CURCHAT);
    }
    
    pub fn clear_chat_with_props(&mut self, _prop_id: u8) {
        self.character.chat_message.clear();
        // Send prop changes
    }
    
    pub fn warp(&mut self, level_name: &str, x: f64, y: f64) {
        self.level_name = level_name.to_string();
        self.x = (x * 16.0) as i16;
        self.y = (y * 16.0) as i16;
        // Broadcast warp
    }

    pub fn has_right(&self, right: i32) -> bool {
        (self.admin_rights & right) != 0
    }

    pub fn rc_self_props_from_packet(&self, props: &[u8]) -> Vec<u8> {
        let mut in_buf = Buffer::from_bytes(props.to_vec());
        let mut out_buf = Buffer::new();
        while in_buf.bytes_left() > 0 {
            let prop_id = in_buf.read_gchar();
            let start = in_buf.read;
            match prop_id as u8 {
                PLPROP_NICKNAME | PLPROP_GANI | PLPROP_BODYIMG | PLPROP_HORSEGIF |
                PLPROP_CURCHAT | PLPROP_PLANGUAGE | PLPROP_OSTYPE | PLPROP_COMMUNITYNAME => {
                    in_buf.read_gchar_string();
                }
                PLPROP_HEADGIF => {
                    let length = in_buf.read_gchar() as usize;
                    if length > 100 {
                        let skip = std::cmp::min(length - 100, in_buf.bytes_left());
                        in_buf.read_bytes(skip);
                    }
                }
                PLPROP_SWORDPOWER | PLPROP_SHIELDPOWER => {
                    let power = in_buf.read_gchar();
                    if (prop_id as u8 == PLPROP_SWORDPOWER && power > 4) ||
                       (prop_id as u8 == PLPROP_SHIELDPOWER && power > 3) {
                        in_buf.read_gchar_string();
                    }
                }
                PLPROP_COLORS => {
                    let skip = std::cmp::min(5, in_buf.bytes_left());
                    in_buf.read_bytes(skip);
                }
                PLPROP_ID | PLPROP_APCOUNTER | PLPROP_X2 | PLPROP_Y2 | PLPROP_Z2 => {
                    let skip = std::cmp::min(2, in_buf.bytes_left());
                    in_buf.read_bytes(skip);
                }
                PLPROP_X | PLPROP_Y | PLPROP_Z | PLPROP_SPRITE | PLPROP_STATUS |
                PLPROP_CARRYSPRITE | PLPROP_HORSEBUSHES | PLPROP_MAGICPOINTS |
                PLPROP_ALIGNMENT | PLPROP_ADDITFLAGS | PLPROP_GMAPLEVELX |
                PLPROP_GMAPLEVELY | PLPROP_JOINLEAVELVL | PLPROP_PSTATUSMSG |
                PLPROP_UNKNOWN77 | PLPROP_UNKNOWN81 => {
                    let skip = std::cmp::min(1, in_buf.bytes_left());
                    in_buf.read_bytes(skip);
                }
                PLPROP_CURLEVEL | PLPROP_ACCOUNTNAME | PLPROP_GATTRIB1 | PLPROP_GATTRIB2 |
                PLPROP_GATTRIB3 | PLPROP_GATTRIB4 | PLPROP_GATTRIB5 | PLPROP_GATTRIB6 |
                PLPROP_GATTRIB7 | PLPROP_GATTRIB8 | PLPROP_GATTRIB9 | PLPROP_GATTRIB10 |
                PLPROP_GATTRIB11 | PLPROP_GATTRIB12 | PLPROP_GATTRIB13 | PLPROP_GATTRIB14 |
                PLPROP_GATTRIB15 | PLPROP_GATTRIB16 | PLPROP_GATTRIB17 | PLPROP_GATTRIB18 |
                PLPROP_GATTRIB19 | PLPROP_GATTRIB20 | PLPROP_GATTRIB21 | PLPROP_GATTRIB22 |
                PLPROP_GATTRIB23 | PLPROP_GATTRIB24 | PLPROP_GATTRIB25 | PLPROP_GATTRIB26 |
                PLPROP_GATTRIB27 | PLPROP_GATTRIB28 | PLPROP_GATTRIB29 | PLPROP_GATTRIB30 => {
                    in_buf.read_gchar_string();
                }
                PLPROP_EFFECTCOLORS => {
                    if in_buf.read_gchar() > 0 {
                        let skip = std::cmp::min(4, in_buf.bytes_left());
                        in_buf.read_bytes(skip);
                    }
                }
                PLPROP_CARRYNPC | PLPROP_UDPPORT | PLPROP_KILLSCOUNT | PLPROP_DEATHSCOUNT |
                PLPROP_ONLINESECS | PLPROP_RATING | PLPROP_TEXTCODEPAGE => {
                    let skip = std::cmp::min(4, in_buf.bytes_left());
                    in_buf.read_bytes(skip);
                }
                PLPROP_IPADDR => {
                    let skip = std::cmp::min(5, in_buf.bytes_left());
                    in_buf.read_bytes(skip);
                }
                PLPROP_ATTACHNPC => {
                    let skip = std::cmp::min(5, in_buf.bytes_left());
                    in_buf.read_bytes(skip);
                }
                PLPROP_PCONNECTED => {}
                _ => return out_buf.data.clone(),
            }
            if in_buf.read == start && prop_id as u8 != PLPROP_PCONNECTED {
                return out_buf.data.clone();
            }
            match prop_id as u8 {
                PLPROP_X | PLPROP_Y | PLPROP_Z | PLPROP_X2 | PLPROP_Y2 | PLPROP_Z2 | PLPROP_CURLEVEL => continue,
                _ => {}
            }
            out_buf.write_gchar(prop_id as u8);
            out_buf.write_bytes(&self.get_prop(prop_id as u8));
        }
        out_buf.data.clone()
    }

    pub fn get_prop(&self, _prop_id: u8) -> Vec<u8> {
        // TODO: Full implementation of prop getters
        Vec::new()
    }

    pub async fn send_plo_npcprops(&self, _npc: &crate::npc::NPC) {
        // TODO: Serialize NPC props
    }

    pub async fn send_plo_npcdel(&self, id: u32) {
        let mut buf = crate::buffer::Buffer::new();
        buf.write_byte(crate::protocol::PLO_NPCDEL);
        buf.write_string(&id.to_string());
        self.send(&buf.bytes()).await;
    }

    pub async fn send_file(&self, _file_name: &str) {
        // TODO: Send file over socket
    }

    pub async fn send_plo_fileuptodate(&self, file_name: &str) {
        let mut buf = crate::buffer::Buffer::new();
        buf.write_byte(crate::protocol::PLO_FILEUPTODATE);
        buf.write_string(file_name);
        self.send(&buf.bytes()).await;
    }

    pub async fn send(&self, data: &[u8]) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(data.to_vec()).await;
        }
    }

    pub async fn apply_level_item(&mut self, _item_type: u8) {
        // TODO: Give player the item
    }

    pub async fn send_plo_levelchest(&self, _chest: &crate::level::Chest, _b: bool) {
        // TODO: Broadcast chest open
    }

    pub async fn send_plo_levelbaddyprops(&self, _baddy: &crate::level::LevelBaddy) {
        // TODO: Send baddy props
    }

    pub async fn add_chest(&mut self, chest_key: &str) {
        if !self.chest_list.contains(&chest_key.to_string()) {
            self.chest_list.push(chest_key.to_string());
        }
    }

    pub async fn has_chest(&self, chest_key: &str) -> bool {
        self.chest_list.contains(&chest_key.to_string())
    }

    pub async fn set_flag(&mut self, k: &str, v: &str) {
        self.flag_list.insert(k.to_string(), v.to_string());
    }

    pub async fn delete_flag(&mut self, k: &str) {
        self.flag_list.remove(k);
    }

    pub async fn handle_movement_flag(&mut self, _k: &str, _v: &str) -> bool {
        false // TODO
    }

    pub async fn disconnect(&mut self) {
        // TODO: Save account
        
        // Remove from level
        // TODO: Level remove player
        
        // Let server know
        let id = self.id;
        let mut srv = self.server.write().await;
        srv.players.remove(&id);
    }

    pub fn get_props_rc(&self) -> Vec<u8> {
        Vec::new()
    }

    pub async fn load_account(&mut self, _name: &str, _create: bool) -> bool {
        false
    }

    pub async fn save_account(&self) {
    }

    pub fn reset_account(&mut self) {
    }

    pub fn set_props_from_rc(&mut self, _buf: &mut crate::buffer::Buffer, _sender: &Player) {
    }
}
