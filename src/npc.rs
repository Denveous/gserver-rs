use std::collections::HashMap;
use crate::player::Character;
use crate::level::Level;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NPCType {
    DBNPC = 0,
    LEVELNPC = 1,
}

#[derive(Clone)]
pub struct NPC {
    pub id: u32,
    pub npc_type: NPCType,
    
    pub x: i16,
    pub y: i16,
    pub z: i16,
    
    pub width: i32,
    pub height: i32,
    
    pub image: String,
    pub script: String,
    pub npc_name: String,
    pub scripter: String,
    pub script_type: String,
    
    pub timeout: i32,
    pub sprite: u8,
    pub vis_flags: u8,
    pub block_flags: u8,
    
    pub hurt_x: f32,
    pub hurt_y: f32,
    
    pub saves: [u8; 10],
    pub flag_list: HashMap<String, String>,
    
    pub character: Character,
    pub weapon_name: String,
    pub script_data: String,
    pub level_name: String,
}

impl Default for NPC {
    fn default() -> Self {
        Self {
            id: 0,
            npc_type: NPCType::LEVELNPC,
            x: 480, // 30 * 16
            y: 480, // 30 * 16
            z: 0,
            width: 0,
            height: 0,
            image: String::new(),
            script: String::new(),
            npc_name: String::new(),
            scripter: String::new(),
            script_type: String::new(),
            timeout: 0,
            sprite: 0,
            vis_flags: 1, // NPCVISFLAG_VISIBLE
            block_flags: 0,
            hurt_x: 0.0,
            hurt_y: 0.0,
            saves: [0; 10],
            flag_list: HashMap::new(),
            character: Character {
                nick_name: String::new(),
                chat_message: String::new(),
                head_image: String::new(),
                body_image: String::new(),
                sword_image: String::new(),
                shield_image: String::new(),
                colors: [0; 5],
            },
            weapon_name: String::new(),
            script_data: String::new(),
            level_name: String::new(),
        }
    }
}

impl NPC {
    pub fn new(npc_type: NPCType) -> Self {
        let mut npc = Self::default();
        npc.npc_type = npc_type;
        npc
    }
    
    pub fn set_id(&mut self, id: u32) {
        self.id = id;
    }
    
    pub fn get_id(&self) -> u32 {
        self.id
    }
}
