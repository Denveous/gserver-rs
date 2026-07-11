use std::collections::HashMap;
use std::time::SystemTime;

pub struct LevelTiles {}
#[derive(Clone)]
pub struct LevelBaddy {
    pub id: u8,
    pub x: f32,
    pub y: f32,
    pub baddy_type: u8,
    pub props: String,
}

impl LevelBaddy {
    pub fn new(x: f32, y: f32, baddy_type: u8) -> Self {
        Self {
            id: 0,
            x,
            y,
            baddy_type,
            props: String::new(),
        }
    }
    pub fn set_props(&mut self, props: &[u8]) {
        self.props = String::from_utf8_lossy(props).to_string();
    }
}

#[derive(Clone)]
pub struct Chest {
    pub x: f32,
    pub y: f32,
    pub item_type: u8,
}

pub struct Level {
    pub level_name: String,
    pub file_name: String,
    pub actual_level_name: String,
    pub file_version: String,
    
    pub mod_time: SystemTime,
    pub is_sparring_zone: bool,
    pub is_singleplayer: bool,
    
    pub map_x: i32,
    pub map_y: i32,
    
    pub tiles: HashMap<u8, LevelTiles>,
    pub baddies: HashMap<u8, LevelBaddy>,
    pub npcs: HashMap<u32, crate::npc::NPC>,
    pub chests: Vec<Chest>,
    pub players: Vec<u16>,
}

impl Level {
    pub fn get_chest_key(&self, chest: &Chest) -> String {
        format!("{}_{}_{}", self.level_name, chest.x, chest.y)
    }

    pub fn remove_item(&mut self, _x: f32, _y: f32) -> i32 {
        // TODO: find and remove item, return type
        0
    }

    pub fn add_item(&mut self, _x: f32, _y: f32, _item_type: i32) {
        // TODO: Add item
    }

    pub fn new(name: &str) -> Self {
        Self {
            level_name: name.to_string(),
            file_name: String::new(),
            actual_level_name: String::new(),
            file_version: String::new(),
            mod_time: SystemTime::now(),
            is_sparring_zone: false,
            is_singleplayer: false,
            map_x: 0,
            map_y: 0,
            tiles: HashMap::new(),
            baddies: HashMap::new(),
            npcs: HashMap::new(),
            chests: Vec::new(),
            players: Vec::new(),
        }
    }
}
