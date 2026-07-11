pub const ENCRYPT_GEN_1: u32 = 0;
pub const ENCRYPT_GEN_2: u32 = 1;
pub const ENCRYPT_GEN_3: u32 = 2;
pub const ENCRYPT_GEN_4: u32 = 3;
pub const ENCRYPT_GEN_5: u32 = 4;
pub const ENCRYPT_GEN_6: u32 = 5;

pub const COMPRESS_UNCOMPRESSED: u8 = 0x02;
pub const COMPRESS_ZLIB: u8 = 0x04;
pub const COMPRESS_BZ2: u8 = 0x06;

pub const ITERATOR_START: [u32; 6] = [0, 0, 0x04A80B38, 0x4A80B38, 0x4A80B38, 0];

pub struct Encryption {
    pub key: u8,
    pub iterator: u32,
    pub limit: i32,
    pub generation: u32,
}

impl Default for Encryption {
    fn default() -> Self {
        Self::new()
    }
}

impl Encryption {
    pub fn new() -> Self {
        Self {
            key: 0,
            iterator: ITERATOR_START[ENCRYPT_GEN_3 as usize],
            limit: -1,
            generation: ENCRYPT_GEN_3,
        }
    }

    pub fn reset(&mut self, key: u8) {
        self.key = key;
        self.iterator = ITERATOR_START[self.generation as usize];
        self.limit = -1;
    }

    pub fn set_gen(&mut self, new_gen: u32) {
        if new_gen > 6 {
            self.generation = 6;
        } else {
            self.generation = new_gen;
        }
        self.iterator = ITERATOR_START[self.generation as usize];
    }

    pub fn get_gen(&self) -> u32 {
        self.generation
    }

    pub fn set_limit(&mut self, limit: i32) {
        self.limit = limit;
    }

    pub fn limit_from_type(&mut self, packet_type: u8) -> i32 {
        let limits = [(0x02, 0x0C), (0x04, 0x04), (0x06, 0x04)];
        for (pt, l) in limits.iter() {
            if *pt == packet_type {
                self.limit = *l;
                return 0;
            }
        }
        1
    }

    pub fn decrypt(&mut self, data: &mut Vec<u8>) {
        if data.is_empty() {
            return;
        }
        match self.generation {
            ENCRYPT_GEN_1 | ENCRYPT_GEN_2 => return,
            ENCRYPT_GEN_3 => {
                self.iterator = self.iterator.wrapping_mul(0x8088405).wrapping_add(self.key as u32);
                let pos = ((self.iterator & 0xFFFF) as usize) % data.len();
                data.remove(pos);
            }
            ENCRYPT_GEN_4 | ENCRYPT_GEN_5 => {
                for i in 0..data.len() {
                    if i % 4 == 0 {
                        if self.limit == 0 {
                            return;
                        }
                        self.iterator = self.iterator.wrapping_mul(0x8088405).wrapping_add(self.key as u32);
                        if self.limit > 0 {
                            self.limit -= 1;
                        }
                    }
                    let iterator_bytes = self.iterator.to_le_bytes();
                    data[i] ^= iterator_bytes[i % 4];
                }
            }
            ENCRYPT_GEN_6 => return,
            _ => return,
        }
    }

    pub fn encrypt(&mut self, data: &[u8]) -> Vec<u8> {
        if data.is_empty() {
            return data.to_vec();
        }
        let mut result = data.to_vec();
        match self.generation {
            ENCRYPT_GEN_1 | ENCRYPT_GEN_2 => result,
            ENCRYPT_GEN_3 => {
                self.iterator = self.iterator.wrapping_mul(0x8088405).wrapping_add(self.key as u32);
                let pos = ((self.iterator & 0xFFFF) as usize) % result.len();
                result.insert(pos, b')');
                result
            }
            ENCRYPT_GEN_4 | ENCRYPT_GEN_5 => {
                for i in 0..result.len() {
                    if i % 4 == 0 {
                        if self.limit == 0 {
                            return result;
                        }
                        self.iterator = self.iterator.wrapping_mul(0x8088405).wrapping_add(self.key as u32);
                        if self.limit > 0 {
                            self.limit -= 1;
                        }
                    }
                    let iterator_bytes = self.iterator.to_le_bytes();
                    result[i] ^= iterator_bytes[i % 4];
                }
                result
            }
            _ => result,
        }
    }
}
