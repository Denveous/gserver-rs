use std::collections::HashMap;

pub struct Weapon {
    pub name: String,
    pub image: String,
    pub script: String,
    pub bytecode_file: String,
    pub bytecode: Option<Vec<u8>>,
}

pub struct ScriptClass {
    pub name: String,
    pub script: String,
}

impl Weapon {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            image: String::new(),
            script: String::new(),
            bytecode_file: String::new(),
            bytecode: None,
        }
    }
}
