use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::{engine::general_purpose, Engine as _};
use bzip2::read::BzDecoder;
use bzip2::write::BzEncoder;
use bzip2::Compression as BzCompression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::protocol::*;

pub const ITERATOR_START: [u32; 6] = [0, 0, 0x04A8_0B38, 0x04A8_0B38, 0x04A8_0B38, 0];

/// Mutable byte buffer used by all protocol encoders and decoders.
#[derive(Clone, Debug, Default)]
pub struct Buffer {
    pub data: Vec<u8>,
    pub read: usize,
    pub write: usize,
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            data: Vec::with_capacity(256),
            read: 0,
            write: 0,
        }
    }
    pub fn from_bytes(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            read: 0,
            write: data.len(),
        }
    }
    pub fn NewBuffer() -> Self {
        Self::new()
    }
    pub fn NewBufferFromBytes(data: &[u8]) -> Self {
        Self::from_bytes(data)
    }

    pub fn write(&mut self, bytes: &[u8]) -> &mut Self {
        self.data.extend_from_slice(bytes);
        self.write = self.data.len();
        self
    }
    pub fn Write(&mut self, bytes: &[u8]) -> &mut Self {
        self.write(bytes)
    }
    pub fn write_byte(&mut self, value: u8) -> &mut Self {
        self.data.push(value);
        self.write = self.data.len();
        self
    }
    pub fn WriteByte(&mut self, value: u8) -> &mut Self {
        self.write_byte(value)
    }
    pub fn write_gchar(&mut self, value: u8) -> &mut Self {
        self.write_byte(value.min(223).wrapping_add(32))
    }
    pub fn WriteGChar(&mut self, value: u8) -> &mut Self {
        self.write_gchar(value)
    }
    pub fn write_char(&mut self, value: i8) -> &mut Self {
        self.write_byte(value as u8)
    }
    pub fn WriteChar(&mut self, value: i8) -> &mut Self {
        self.write_char(value)
    }
    pub fn write_short(&mut self, value: i16) -> &mut Self {
        self.write(&value.to_be_bytes())
    }
    pub fn WriteShort(&mut self, value: i16) -> &mut Self {
        self.write_short(value)
    }
    pub fn write_short_u(&mut self, value: u16) -> &mut Self {
        self.write(&value.to_be_bytes())
    }
    pub fn WriteShortU(&mut self, value: u16) -> &mut Self {
        self.write_short_u(value)
    }
    pub fn write_int(&mut self, value: i32) -> &mut Self {
        self.write(&value.to_be_bytes())
    }
    pub fn WriteInt(&mut self, value: i32) -> &mut Self {
        self.write_int(value)
    }
    pub fn write_int64(&mut self, value: i64) -> &mut Self {
        self.write(&value.to_be_bytes())
    }
    pub fn WriteInt64(&mut self, value: i64) -> &mut Self {
        self.write_int64(value)
    }
    pub fn write_int3(&mut self, value: i32) -> &mut Self {
        let bytes = value.to_be_bytes();
        self.write(&bytes[1..]);
        self
    }
    pub fn WriteInt3(&mut self, value: i32) -> &mut Self {
        self.write_int3(value)
    }

    pub fn write_gbyte(&mut self, value: u8) -> &mut Self {
        self.write_byte(value)
    }
    pub fn WriteGByte(&mut self, value: u8) -> &mut Self {
        self.write_gbyte(value)
    }
    pub fn write_gshort(&mut self, mut value: u16) -> &mut Self {
        if value > 28767 {
            value = 28767;
        }
        let mut first = value >> 7;
        if first > 223 {
            first = 223;
        }
        let second = value - (first << 7);
        self.write_byte((first as u8).wrapping_add(32));
        self.write_byte((second as u8).wrapping_add(32));
        self
    }
    pub fn WriteGShort(&mut self, value: u16) -> &mut Self {
        self.write_gshort(value)
    }
    pub fn write_gint(&mut self, mut value: u32) -> &mut Self {
        if value > 3_682_399 {
            value = 3_682_399;
        }
        let mut first = value >> 14;
        if first > 223 {
            first = 223;
        }
        value -= first << 14;
        let mut second = value >> 7;
        if second > 223 {
            second = 223;
        }
        let third = value - (second << 7);
        self.write_byte((first as u8).wrapping_add(32));
        self.write_byte((second as u8).wrapping_add(32));
        self.write_byte((third as u8).wrapping_add(32));
        self
    }
    pub fn WriteGInt(&mut self, value: u32) -> &mut Self {
        self.write_gint(value)
    }
    pub fn write_gint4(&mut self, mut value: u32) -> &mut Self {
        if value > 471_347_295 {
            value = 471_347_295;
        }
        let mut first = value >> 21;
        if first > 223 {
            first = 223;
        }
        value -= first << 21;
        let mut second = value >> 14;
        if second > 223 {
            second = 223;
        }
        value -= second << 14;
        let mut third = value >> 7;
        if third > 223 {
            third = 223;
        }
        let fourth = value - (third << 7);
        self.write_byte(first as u8 + 32);
        self.write_byte(second as u8 + 32);
        self.write_byte(third as u8 + 32);
        self.write_byte(fourth as u8 + 32);
        self
    }
    pub fn WriteGInt4(&mut self, value: u32) -> &mut Self {
        self.write_gint4(value)
    }
    pub fn write_gint5(&mut self, mut value: u64) -> &mut Self {
        if value > 0xffff_ffff {
            value = 0xffff_ffff;
        }
        let mut first = value >> 28;
        if first > 15 {
            first = 15;
        }
        value -= first << 28;
        let mut second = value >> 21;
        if second > 223 {
            second = 223;
        }
        value -= second << 21;
        let mut third = value >> 14;
        if third > 223 {
            third = 223;
        }
        value -= third << 14;
        let mut fourth = value >> 7;
        if fourth > 223 {
            fourth = 223;
        }
        let fifth = value - (fourth << 7);
        self.write_byte(first as u8 + 32);
        self.write_byte(second as u8 + 32);
        self.write_byte(third as u8 + 32);
        self.write_byte(fourth as u8 + 32);
        self.write_byte(fifth as u8 + 32);
        self
    }
    pub fn WriteGInt5(&mut self, value: u64) -> &mut Self {
        self.write_gint5(value)
    }
    pub fn write_gstring(&mut self, value: &str) -> &mut Self {
        self.write_gint(value.len() as u32);
        self.write(value.as_bytes())
    }
    pub fn WriteGString(&mut self, value: &str) -> &mut Self {
        self.write_gstring(value)
    }
    pub fn write_string8(&mut self, value: &str) -> &mut Self {
        self.write_byte(value.len() as u8);
        self.write(value.as_bytes())
    }
    pub fn WriteString8(&mut self, value: &str) -> &mut Self {
        self.write_string8(value)
    }
    pub fn write_string8_encoded(&mut self, value: &str) -> &mut Self {
        self.write_gchar(value.len() as u8);
        self.write(value.as_bytes())
    }
    pub fn WriteString8Encoded(&mut self, value: &str) -> &mut Self {
        self.write_string8_encoded(value)
    }
    pub fn write_string(&mut self, value: &str) -> &mut Self {
        self.write(value.as_bytes());
        self.write_byte(0)
    }
    pub fn WriteString(&mut self, value: &str) -> &mut Self {
        self.write_string(value)
    }

    pub fn read_byte(&mut self) -> u8 {
        if self.read >= self.data.len() {
            return 0;
        }
        let result = self.data[self.read];
        self.read += 1;
        result
    }
    pub fn ReadByte(&mut self) -> u8 {
        self.read_byte()
    }
    pub fn read_char(&mut self) -> i8 {
        self.read_byte() as i8
    }
    pub fn ReadChar(&mut self) -> i8 {
        self.read_char()
    }
    pub fn read_short(&mut self) -> i16 {
        if self.read + 2 > self.data.len() {
            return 0;
        }
        let value = i16::from_be_bytes([self.data[self.read], self.data[self.read + 1]]);
        self.read += 2;
        value
    }
    pub fn ReadShort(&mut self) -> i16 {
        self.read_short()
    }
    pub fn read_int(&mut self) -> i32 {
        if self.read + 4 > self.data.len() {
            return 0;
        }
        let value = i32::from_be_bytes(self.data[self.read..self.read + 4].try_into().unwrap());
        self.read += 4;
        value
    }
    pub fn ReadInt(&mut self) -> i32 {
        self.read_int()
    }
    pub fn read_int3(&mut self) -> i32 {
        if self.read + 3 > self.data.len() {
            return 0;
        }
        let value = ((self.data[self.read] as i32) << 16)
            | ((self.data[self.read + 1] as i32) << 8)
            | self.data[self.read + 2] as i32;
        self.read += 3;
        value
    }
    pub fn ReadInt3(&mut self) -> i32 {
        self.read_int3()
    }
    pub fn read_gbyte(&mut self) -> u8 {
        self.read_byte()
    }
    pub fn ReadGByte(&mut self) -> u8 {
        self.read_gbyte()
    }
    pub fn read_gchar(&mut self) -> u8 {
        let value = self.read_byte();
        if value < 32 {
            0
        } else {
            value - 32
        }
    }
    pub fn ReadGChar(&mut self) -> u8 {
        self.read_gchar()
    }
    pub fn read_gshort(&mut self) -> u16 {
        if self.read + 2 > self.data.len() {
            return 0;
        }
        let first = self.read_gchar() as u16;
        let second = self.read_gchar() as u16;
        (first << 7) | second
    }
    pub fn ReadGShort(&mut self) -> u16 {
        self.read_gshort()
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
    pub fn ReadGInt(&mut self) -> u32 {
        self.read_gint()
    }
    pub fn read_gint4(&mut self) -> u32 {
        if self.read + 4 > self.data.len() {
            return 0;
        }
        let value = ((self.data[self.read] as u32) << 21)
            | ((self.data[self.read + 1] as u32) << 14)
            | ((self.data[self.read + 2] as u32) << 7)
            | self.data[self.read + 3] as u32;
        self.read += 4;
        value.wrapping_sub(0x0408_1020)
    }
    pub fn ReadGInt4(&mut self) -> u32 {
        self.read_gint4()
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
    pub fn ReadGInt5(&mut self) -> u64 {
        self.read_gint5()
    }
    pub fn read_gstring(&mut self) -> String {
        let len = self.read_gint() as usize;
        if self.read + len > self.data.len() {
            return String::new();
        }
        let start = self.read;
        self.read += len;
        String::from_utf8_lossy(&self.data[start..self.read]).into_owned()
    }
    pub fn ReadGString(&mut self) -> String {
        self.read_gstring()
    }
    pub fn read_gchar_string(&mut self) -> String {
        let len = (self.read_gchar() as usize).min(self.remaining());
        String::from_utf8_lossy(&self.read_bytes(len)).into_owned()
    }
    pub fn ReadGCharString(&mut self) -> String {
        self.read_gchar_string()
    }
    pub fn read_string(&mut self) -> String {
        let start = self.read;
        while self.read < self.data.len() && self.data[self.read] != 0 {
            self.read += 1;
        }
        let result = String::from_utf8_lossy(&self.data[start..self.read]).into_owned();
        if self.read < self.data.len() {
            self.read += 1;
        }
        result
    }
    pub fn ReadString(&mut self) -> String {
        self.read_string()
    }
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }
    pub fn Bytes(&self) -> &[u8] {
        self.bytes()
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn Len(&self) -> usize {
        self.len()
    }
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.read)
    }
    pub fn Remaining(&self) -> usize {
        self.remaining()
    }
    pub fn bytes_left(&self) -> usize {
        self.remaining()
    }
    pub fn BytesLeft(&self) -> usize {
        self.bytes_left()
    }
    pub fn read_bytes(&mut self, count: usize) -> Vec<u8> {
        let mut result = vec![0u8; count];
        for value in &mut result {
            *value = self.read_byte();
        }
        result
    }
    pub fn ReadBytes(&mut self, count: usize) -> Vec<u8> {
        self.read_bytes(count)
    }
    pub fn reset(&mut self) {
        self.read = 0;
    }
    pub fn Reset(&mut self) {
        self.reset()
    }
    pub fn clear(&mut self) {
        self.data.clear();
        self.read = 0;
        self.write = 0;
    }
    pub fn Clear(&mut self) {
        self.clear()
    }
    pub fn base64_encode(&mut self) -> &mut Self {
        self.data = general_purpose::STANDARD.encode(&self.data).into_bytes();
        self.read = 0;
        self.write = self.data.len();
        self
    }
    pub fn Base64Encode(&mut self) -> &mut Self {
        self.base64_encode()
    }
    pub fn base64_decode(&mut self) -> &mut Self {
        let Ok(decoded) = general_purpose::STANDARD.decode(&self.data) else {
            return self;
        };
        self.data = decoded;
        self.read = 0;
        self.write = self.data.len();
        self
    }
    pub fn Base64Decode(&mut self) -> &mut Self {
        self.base64_decode()
    }
}

#[derive(Clone, Debug)]
pub struct Encryption {
    pub key: u8,
    pub iterator: u32,
    pub limit: i32,
    pub gen: u32,
}

impl Default for Encryption {
    fn default() -> Self {
        Self::new()
    }
}

impl Encryption {
    pub const ITERATOR_START: [u32; 6] = ITERATOR_START;
    pub fn new() -> Self {
        Self {
            key: 0,
            limit: -1,
            gen: ENCRYPT_GEN_3,
            iterator: Self::ITERATOR_START[ENCRYPT_GEN_3 as usize],
        }
    }
    pub fn NewEncryption() -> Self {
        Self::new()
    }
    pub fn reset(&mut self, key: u8) {
        self.key = key;
        self.iterator = Self::ITERATOR_START[self.gen as usize];
        self.limit = -1;
    }
    pub fn Reset(&mut self, key: u8) {
        self.reset(key)
    }
    pub fn set_gen(&mut self, gen: u32) {
        // This preserves the reference's historical bounds check, including
        // its behavior when a caller supplies an already-invalid generation.
        if self.gen > 6 {
            self.gen = 6;
        } else {
            self.gen = gen;
        }
        self.iterator = Self::ITERATOR_START[self.gen as usize];
    }
    pub fn SetGen(&mut self, gen: u32) {
        self.set_gen(gen)
    }
    pub fn get_gen(&self) -> u32 {
        self.gen
    }
    pub fn GetGen(&self) -> u32 {
        self.get_gen()
    }
    pub fn set_limit(&mut self, limit: i32) {
        self.limit = limit;
    }
    pub fn SetLimit(&mut self, limit: i32) {
        self.set_limit(limit)
    }
    pub fn limit_from_type(&mut self, packet_type: u8) -> i32 {
        for (kind, limit) in [
            (COMPRESS_UNCOMPRESSED, 0x0c),
            (COMPRESS_ZLIB, 0x04),
            (COMPRESS_BZ2, 0x04),
        ] {
            if kind == packet_type {
                self.limit = limit;
                return 0;
            }
        }
        1
    }
    pub fn LimitFromType(&mut self, packet_type: u8) -> i32 {
        self.limit_from_type(packet_type)
    }
    pub fn decrypt(&mut self, data: &mut [u8]) {
        if data.is_empty() {
            return;
        }
        match self.gen {
            ENCRYPT_GEN_1 | ENCRYPT_GEN_2 | ENCRYPT_GEN_6 => {}
            ENCRYPT_GEN_3 => {
                self.iterator = self
                    .iterator
                    .wrapping_mul(0x0808_8405)
                    .wrapping_add(self.key as u32);
                let position = (self.iterator & 0xffff) as usize % data.len();
                data.copy_within(position + 1.., position);
            }
            ENCRYPT_GEN_4 | ENCRYPT_GEN_5 => {
                for (index, byte) in data.iter_mut().enumerate() {
                    if index % 4 == 0 {
                        if self.limit == 0 {
                            return;
                        }
                        self.iterator = self
                            .iterator
                            .wrapping_mul(0x0808_8405)
                            .wrapping_add(self.key as u32);
                        if self.limit > 0 {
                            self.limit -= 1;
                        }
                    }
                    *byte ^= self.iterator.to_le_bytes()[index % 4];
                }
            }
            _ => {}
        }
    }
    pub fn Decrypt(&mut self, data: &mut [u8]) {
        self.decrypt(data)
    }
    pub fn encrypt(&mut self, data: &[u8]) -> Vec<u8> {
        if data.is_empty() {
            return Vec::new();
        }
        let mut result = data.to_vec();
        match self.gen {
            ENCRYPT_GEN_1 | ENCRYPT_GEN_2 | ENCRYPT_GEN_6 => result,
            ENCRYPT_GEN_3 => {
                self.iterator = self
                    .iterator
                    .wrapping_mul(0x0808_8405)
                    .wrapping_add(self.key as u32);
                let position = (self.iterator & 0xffff) as usize % result.len();
                result.insert(position, b')');
                result
            }
            ENCRYPT_GEN_4 | ENCRYPT_GEN_5 => {
                for (index, byte) in result.iter_mut().enumerate() {
                    if index % 4 == 0 {
                        if self.limit == 0 {
                            return result;
                        }
                        self.iterator = self
                            .iterator
                            .wrapping_mul(0x0808_8405)
                            .wrapping_add(self.key as u32);
                        if self.limit > 0 {
                            self.limit -= 1;
                        }
                    }
                    *byte ^= self.iterator.to_le_bytes()[index % 4];
                }
                result
            }
            _ => result,
        }
    }
    pub fn Encrypt(&mut self, data: &[u8]) -> Vec<u8> {
        self.encrypt(data)
    }
}

pub trait SocketStub: Send + Sync {
    fn on_recv(&self) -> bool;
    fn on_send(&self) -> bool;
    fn on_register(&self) -> bool;
    fn on_unregister(&self);
    fn can_recv(&self) -> bool;
    fn can_send(&self) -> bool;
}

struct SocketEntry {
    id: usize,
    stream: Option<Arc<Mutex<TcpStream>>>,
    stub: Arc<dyn SocketStub>,
}

/// Small polling manager with deterministic interleaving semantics.
pub struct SocketManager {
    entries: Mutex<HashMap<usize, SocketEntry>>,
    next_id: Mutex<usize>,
    pub running: bool,
}

impl SocketManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
            running: false,
        })
    }
    pub fn NewSocketManager() -> Arc<Self> {
        Self::new()
    }
    pub fn register(
        &self,
        stream: Option<Arc<Mutex<TcpStream>>>,
        stub: Arc<dyn SocketStub>,
    ) -> usize {
        let mut next = self.next_id.lock().unwrap();
        let id = *next;
        *next += 1;
        self.entries.lock().unwrap().insert(
            id,
            SocketEntry {
                id,
                stream,
                stub: stub.clone(),
            },
        );
        let _ = stub.on_register();
        id
    }
    pub fn Register(
        &self,
        stream: Option<Arc<Mutex<TcpStream>>>,
        stub: Arc<dyn SocketStub>,
    ) -> usize {
        self.register(stream, stub)
    }
    pub fn unregister(&self, id: usize) {
        if let Some(entry) = self.entries.lock().unwrap().remove(&id) {
            entry.stub.on_unregister();
            if let Some(stream) = entry.stream {
                if let Ok(stream) = stream.lock() {
                    let _ = stream.shutdown(Shutdown::Both);
                }
            }
        }
    }
    pub fn Unregister(&self, id: usize) {
        self.unregister(id)
    }
    pub fn is_empty(&self) -> bool {
        self.entries.lock().unwrap().is_empty()
    }
    pub fn IsEmpty(&self) -> bool {
        self.is_empty()
    }
    pub fn update(&self, timeout: Duration) -> bool {
        let entries: Vec<(usize, Arc<dyn SocketStub>)> = self
            .entries
            .lock()
            .unwrap()
            .values()
            .map(|e| (e.id, e.stub.clone()))
            .collect();
        if entries.is_empty() {
            thread::sleep(timeout);
            return false;
        }
        let mut handled = false;
        let mut failed = Vec::new();
        for (id, stub) in entries {
            if stub.can_recv() {
                if !stub.on_recv() {
                    failed.push(id);
                    continue;
                }
                handled = true;
            }
            if stub.can_send() {
                if !stub.on_send() {
                    failed.push(id);
                    continue;
                }
                handled = true;
            }
        }
        for id in failed {
            self.unregister(id);
        }
        handled
    }
    pub fn Update(&self, timeout: Duration) -> bool {
        self.update(timeout)
    }
    pub fn cleanup(&self) {
        let entries: Vec<usize> = self.entries.lock().unwrap().keys().copied().collect();
        for id in entries {
            self.unregister(id);
        }
    }
    pub fn Cleanup(&self) {
        self.cleanup()
    }
}

pub fn zlib_decompress(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}
pub fn ZlibDecompress(data: &[u8]) -> io::Result<Vec<u8>> {
    zlib_decompress(data)
}
pub fn zlib_compress(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    encoder.finish()
}
pub fn ZlibCompress(data: &[u8]) -> io::Result<Vec<u8>> {
    zlib_compress(data)
}
pub fn bz2_compress(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = BzEncoder::new(Vec::new(), BzCompression::default());
    encoder.write_all(data)?;
    encoder.finish()
}
pub fn Bz2Compress(data: &[u8]) -> io::Result<Vec<u8>> {
    bz2_compress(data)
}
pub fn bz2_decompress(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut decoder = BzDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}
pub fn Bz2Decompress(data: &[u8]) -> io::Result<Vec<u8>> {
    bz2_decompress(data)
}

pub fn write_all(stream: &mut TcpStream, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        let count = stream.write(data)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "zero-byte socket write",
            ));
        }
        data = &data[count..];
    }
    Ok(())
}
