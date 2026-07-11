use std::cmp;
use base64::{engine::general_purpose, Engine as _};

pub struct Buffer {
    pub data: Vec<u8>,
    pub read: usize,
    pub write: usize,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            data: Vec::with_capacity(256),
            read: 0,
            write: 0,
        }
    }

    pub fn from_bytes(data: Vec<u8>) -> Self {
        let len = data.len();
        Self {
            data,
            read: 0,
            write: len,
        }
    }

    pub fn write_bytes(&mut self, data: &[u8]) -> &mut Self {
        self.data.extend_from_slice(data);
        self.write = self.data.len();
        self
    }

    pub fn write_byte(&mut self, v: u8) -> &mut Self {
        self.data.push(v);
        self.write = self.data.len();
        self
    }

    pub fn write_gchar(&mut self, mut v: u8) -> &mut Self {
        if v > 223 {
            v = 223;
        }
        self.write_byte(v + 32)
    }

    pub fn write_char(&mut self, v: i8) -> &mut Self {
        self.write_byte(v as u8)
    }

    pub fn write_short(&mut self, v: i16) -> &mut Self {
        self.data.extend_from_slice(&v.to_be_bytes());
        self.write = self.data.len();
        self
    }

    pub fn write_short_u(&mut self, v: u16) -> &mut Self {
        self.data.extend_from_slice(&v.to_be_bytes());
        self.write = self.data.len();
        self
    }

    pub fn write_int(&mut self, v: i32) -> &mut Self {
        self.data.extend_from_slice(&v.to_be_bytes());
        self.write = self.data.len();
        self
    }

    pub fn write_int64(&mut self, v: i64) -> &mut Self {
        self.data.extend_from_slice(&v.to_be_bytes());
        self.write = self.data.len();
        self
    }

    pub fn write_int3(&mut self, v: i32) -> &mut Self {
        self.data.extend_from_slice(&v.to_be_bytes()[1..4]);
        self.write = self.data.len();
        self
    }

    pub fn write_gbyte(&mut self, v: u8) -> &mut Self {
        self.write_byte(v)
    }

    pub fn write_gshort(&mut self, mut v: u16) -> &mut Self {
        if v > 28767 {
            v = 28767;
        }
        let mut val0 = (v >> 7) as u8;
        if val0 > 223 {
            val0 = 223;
        }
        let mut val1 = (v - ((val0 as u16) << 7)) as u8;
        val0 += 32;
        val1 += 32;
        self.data.push(val0);
        self.data.push(val1);
        self.write = self.data.len();
        self
    }

    pub fn write_gint(&mut self, mut v: u32) -> &mut Self {
        if v > 3682399 {
            v = 3682399;
        }
        let mut val0 = (v >> 14) as u8;
        if val0 > 223 {
            val0 = 223;
        }
        v -= (val0 as u32) << 14;
        let mut val1 = (v >> 7) as u8;
        if val1 > 223 {
            val1 = 223;
        }
        let mut val2 = (v - ((val1 as u32) << 7)) as u8;
        val0 += 32;
        val1 += 32;
        val2 += 32;
        self.data.push(val0);
        self.data.push(val1);
        self.data.push(val2);
        self.write = self.data.len();
        self
    }

    pub fn write_gint4(&mut self, mut v: u32) -> &mut Self {
        if v > 471347295 {
            v = 471347295;
        }
        let mut val0 = (v >> 21) as u8;
        if val0 > 223 {
            val0 = 223;
        }
        v -= (val0 as u32) << 21;
        let mut val1 = (v >> 14) as u8;
        if val1 > 223 {
            val1 = 223;
        }
        v -= (val1 as u32) << 14;
        let mut val2 = (v >> 7) as u8;
        if val2 > 223 {
            val2 = 223;
        }
        let val3 = (v - ((val2 as u32) << 7)) as u8;
        self.data.extend_from_slice(&[val0 + 32, val1 + 32, val2 + 32, val3 + 32]);
        self.write = self.data.len();
        self
    }

    pub fn write_gint5(&mut self, mut v: u64) -> &mut Self {
        if v > 0xFFFFFFFF {
            v = 0xFFFFFFFF;
        }
        let mut val0 = (v >> 28) as u8;
        if val0 > 15 {
            val0 = 15;
        }
        v -= (val0 as u64) << 28;
        let mut val1 = (v >> 21) as u8;
        if val1 > 223 {
            val1 = 223;
        }
        v -= (val1 as u64) << 21;
        let mut val2 = (v >> 14) as u8;
        if val2 > 223 {
            val2 = 223;
        }
        v -= (val2 as u64) << 14;
        let mut val3 = (v >> 7) as u8;
        if val3 > 223 {
            val3 = 223;
        }
        let val4 = (v - ((val3 as u64) << 7)) as u8;
        self.data.extend_from_slice(&[val0 + 32, val1 + 32, val2 + 32, val3 + 32, val4 + 32]);
        self.write = self.data.len();
        self
    }

    pub fn write_gstring(&mut self, s: &str) -> &mut Self {
        let bytes = s.as_bytes();
        self.write_gint(bytes.len() as u32);
        self.write_bytes(bytes)
    }

    pub fn write_string8(&mut self, s: &str) -> &mut Self {
        let bytes = s.as_bytes();
        self.write_byte(bytes.len() as u8);
        self.write_bytes(bytes)
    }

    pub fn write_string8_encoded(&mut self, s: &str) -> &mut Self {
        let bytes = s.as_bytes();
        self.write_gchar(bytes.len() as u8);
        self.write_bytes(bytes)
    }

    pub fn write_string(&mut self, s: &str) -> &mut Self {
        self.write_bytes(s.as_bytes());
        self.write_byte(0)
    }

    pub fn read_byte(&mut self) -> u8 {
        if self.read >= self.data.len() {
            return 0;
        }
        let v = self.data[self.read];
        self.read += 1;
        v
    }

    pub fn read_char(&mut self) -> i8 {
        self.read_byte() as i8
    }

    pub fn read_short(&mut self) -> i16 {
        if self.read + 2 > self.data.len() {
            return 0;
        }
        let v = i16::from_be_bytes([self.data[self.read], self.data[self.read + 1]]);
        self.read += 2;
        v
    }

    pub fn read_int(&mut self) -> i32 {
        if self.read + 4 > self.data.len() {
            return 0;
        }
        let v = i32::from_be_bytes([
            self.data[self.read],
            self.data[self.read + 1],
            self.data[self.read + 2],
            self.data[self.read + 3],
        ]);
        self.read += 4;
        v
    }

    pub fn read_int3(&mut self) -> i32 {
        if self.read + 3 > self.data.len() {
            return 0;
        }
        let v = i32::from_be_bytes([
            0,
            self.data[self.read],
            self.data[self.read + 1],
            self.data[self.read + 2],
        ]);
        self.read += 3;
        v
    }

    pub fn read_gbyte(&mut self) -> u8 {
        self.read_byte()
    }

    pub fn read_gshort(&mut self) -> u16 {
        if self.read + 2 > self.data.len() {
            return 0;
        }
        let first = self.read_gchar() as u16;
        let second = self.read_gchar() as u16;
        (first << 7) | second
    }

    pub fn read_gint(&mut self) -> u32 {
        if self.read + 3 > self.data.len() {
            return 0;
        }
        let first = self.read_gchar() as u32;
        let second = self.read_gchar() as u32;
        let third = self.read_gchar() as u32;
        (first << 14) | (second << 7) | third
    }

    pub fn read_gint4(&mut self) -> u32 {
        if self.read + 4 > self.data.len() {
            return 0;
        }
        let v = ((self.data[self.read] as u32) << 21)
            | ((self.data[self.read + 1] as u32) << 14)
            | ((self.data[self.read + 2] as u32) << 7)
            | (self.data[self.read + 3] as u32);
        self.read += 4;
        v.wrapping_sub(0x4081020)
    }

    pub fn read_gint5(&mut self) -> u64 {
        if self.read + 5 > self.data.len() {
            return 0;
        }
        let first = self.read_gchar() as u64;
        let second = self.read_gchar() as u64;
        let third = self.read_gchar() as u64;
        let fourth = self.read_gchar() as u64;
        let fifth = self.read_gchar() as u64;
        (first << 28) | (second << 21) | (third << 14) | (fourth << 7) | fifth
    }

    pub fn read_gstring(&mut self) -> String {
        let str_len = self.read_gint() as usize;
        if self.read + str_len > self.data.len() {
            return String::new();
        }
        let start = self.read;
        self.read += str_len;
        String::from_utf8_lossy(&self.data[start..self.read]).into_owned()
    }

    pub fn read_gchar_string(&mut self) -> String {
        let mut str_len = self.read_gchar() as usize;
        if str_len > self.bytes_left() {
            str_len = self.bytes_left();
        }
        let bytes = self.read_bytes(str_len);
        String::from_utf8_lossy(&bytes).into_owned()
    }

    pub fn read_string(&mut self) -> String {
        let start = self.read;
        while self.read < self.data.len() && self.data[self.read] != 0 {
            self.read += 1;
        }
        let s = String::from_utf8_lossy(&self.data[start..self.read]).into_owned();
        if self.read < self.data.len() {
            self.read += 1;
        }
        s
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn remaining(&self) -> usize {
        if self.read > self.data.len() {
            0
        } else {
            self.data.len() - self.read
        }
    }

    pub fn bytes_left(&self) -> usize {
        self.remaining()
    }

    pub fn read_gchar(&mut self) -> u8 {
        let v = self.read_gbyte();
        if v < 32 {
            0
        } else {
            v - 32
        }
    }

    pub fn read_bytes(&mut self, count: usize) -> Vec<u8> {
        let mut result = vec![0; count];
        for i in 0..count {
            result[i] = self.read_byte();
        }
        result
    }

    pub fn reset(&mut self) {
        self.read = 0;
    }

    pub fn clear(&mut self) {
        self.data.clear();
        self.read = 0;
        self.write = 0;
    }

    pub fn base64_encode(&mut self) -> &mut Self {
        let encoded = general_purpose::STANDARD.encode(&self.data);
        self.data = encoded.into_bytes();
        self.read = 0;
        self.write = self.data.len();
        self
    }

    pub fn base64_decode(&mut self) -> &mut Self {
        if let Ok(decoded) = general_purpose::STANDARD.decode(&self.data) {
            self.data = decoded;
            self.read = 0;
            self.write = self.data.len();
        }
        self
    }
}
