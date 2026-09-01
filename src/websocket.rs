use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, SystemTime};

use base64::{engine::general_purpose, Engine as _};
use ring::digest;

pub const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
pub const WEBSOCKET_BINARY_OPCODE: u8 = 0x2;
pub const WEBSOCKET_CONTINUATION_OPCODE: u8 = 0x0;
pub const WEBSOCKET_CLOSE_OPCODE: u8 = 0x8;
pub const WEBSOCKET_PING_OPCODE: u8 = 0x9;
pub const WEBSOCKET_PONG_OPCODE: u8 = 0xA;
pub const WEBSOCKET_MAX_HANDSHAKE_SIZE: usize = 64 * 1024;
pub const WEBSOCKET_MAX_FRAME_PAYLOAD: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebSocketFrame {
    pub fin: bool,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub struct ReplayStream {
    stream: TcpStream,
    prefix: Vec<u8>,
}

impl ReplayStream {
    pub fn new(stream: TcpStream, prefix: Vec<u8>) -> Self {
        Self { stream, prefix }
    }
    pub fn peer_addr(&self) -> io::Result<std::net::SocketAddr> {
        self.stream.peer_addr()
    }
    pub fn local_addr(&self) -> io::Result<std::net::SocketAddr> {
        self.stream.local_addr()
    }
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.set_read_timeout(timeout)
    }
    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.set_write_timeout(timeout)
    }
    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        self.stream.shutdown(how)
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            stream: self.stream.try_clone()?,
            prefix: self.prefix.clone(),
        })
    }
}

impl Read for ReplayStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if !self.prefix.is_empty() {
            let count = output.len().min(self.prefix.len());
            output[..count].copy_from_slice(&self.prefix[..count]);
            self.prefix.drain(..count);
            return Ok(count);
        }
        self.stream.read(output)
    }
}

impl Write for ReplayStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.stream.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

pub fn parse_websocket_frame(data: &[u8]) -> io::Result<(WebSocketFrame, usize, bool)> {
    if data.len() < 2 {
        return Ok((empty_frame(), 0, false));
    }
    let first = data[0];
    let second = data[1];
    if first & 0x70 != 0 {
        return Err(invalid("WebSocket extensions are not supported"));
    }
    if second & 0x80 == 0 {
        return Err(invalid("client WebSocket frame is not masked"));
    }
    let length_code = (second & 0x7f) as usize;
    let header_len = match length_code {
        126 => 4,
        127 => 10,
        _ => 2,
    };
    if data.len() < header_len {
        return Ok((empty_frame(), 0, false));
    }
    let payload_len = match length_code {
        126 => u16::from_be_bytes([data[2], data[3]]) as u64,
        127 => {
            let value = u64::from_be_bytes(data[2..10].try_into().unwrap());
            if value >> 63 != 0 {
                return Err(invalid("invalid WebSocket payload length"));
            }
            value
        }
        value => value as u64,
    };
    if payload_len > WEBSOCKET_MAX_FRAME_PAYLOAD as u64 {
        return Err(invalid(&format!(
            "WebSocket payload exceeds {WEBSOCKET_MAX_FRAME_PAYLOAD} bytes"
        )));
    }
    if first & 0x0f >= 8 && (first & 0x80 == 0 || payload_len > 125) {
        return Err(invalid("invalid WebSocket control frame"));
    }
    let total = header_len + 4 + payload_len as usize;
    if data.len() < total {
        return Ok((empty_frame(), 0, false));
    }
    let mask = &data[header_len..header_len + 4];
    let start = header_len + 4;
    let mut payload = data[start..total].to_vec();
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    Ok((
        WebSocketFrame {
            fin: first & 0x80 != 0,
            opcode: first & 0x0f,
            payload,
        },
        total,
        true,
    ))
}

pub fn parseWebSocketFrame(data: &[u8]) -> io::Result<(WebSocketFrame, usize, bool)> {
    parse_websocket_frame(data)
}

pub fn make_websocket_frame(opcode: u8, payload: &[u8]) -> io::Result<Vec<u8>> {
    if opcode >= 8 && payload.len() > 125 {
        return Err(invalid("WebSocket control payload exceeds 125 bytes"));
    }
    if payload.len() > WEBSOCKET_MAX_FRAME_PAYLOAD {
        return Err(invalid(&format!(
            "WebSocket payload exceeds {WEBSOCKET_MAX_FRAME_PAYLOAD} bytes"
        )));
    }
    let header_len = if payload.len() <= 125 {
        2
    } else if payload.len() <= 0xffff {
        4
    } else {
        10
    };
    let mut frame = vec![0u8; header_len + payload.len()];
    frame[0] = 0x80 | (opcode & 0x0f);
    match header_len {
        2 => frame[1] = payload.len() as u8,
        4 => {
            frame[1] = 126;
            frame[2..4].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        _ => {
            frame[1] = 127;
            frame[2..10].copy_from_slice(&(payload.len() as u64).to_be_bytes());
        }
    }
    frame[header_len..].copy_from_slice(payload);
    Ok(frame)
}

pub fn makeWebSocketFrame(opcode: u8, payload: &[u8]) -> io::Result<Vec<u8>> {
    make_websocket_frame(opcode, payload)
}

pub fn websocket_accept(key: &str) -> String {
    let mut input = key.as_bytes().to_vec();
    input.extend_from_slice(WEBSOCKET_GUID.as_bytes());
    let digest = digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, &input);
    general_purpose::STANDARD.encode(digest.as_ref())
}

pub fn websocket_header_value(header: &[u8], name: &str) -> String {
    for line in header.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some(index) = line.iter().position(|byte| *byte == b':') {
            let key = String::from_utf8_lossy(&line[..index]);
            if key.trim().eq_ignore_ascii_case(name) {
                return String::from_utf8_lossy(&line[index + 1..])
                    .trim()
                    .to_string();
            }
        }
    }
    String::new()
}

pub fn websocket_header_has_token(header: &[u8], name: &str, token: &str) -> bool {
    websocket_header_value(header, name)
        .split(',')
        .any(|value| value.trim().eq_ignore_ascii_case(token))
}

pub fn is_websocket_request_prefix(data: &[u8]) -> bool {
    let prefix = b"GET ";
    !data.is_empty()
        && (data.starts_with(prefix) || (data.len() < prefix.len() && prefix.starts_with(data)))
}

pub const API_SNIFF_LIMIT: usize = 64 * 1024;

/// Reads just enough of a connection to distinguish HTTP/API traffic from a
/// native game stream while retaining every byte for the consumer.
pub fn sniff_game_connection(mut stream: TcpStream) -> io::Result<(ReplayStream, bool)> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut prefix = Vec::with_capacity(1024);
    let mut first = [0u8; 1];
    stream.read_exact(&mut first)?;
    prefix.push(first[0]);
    if !(b'A'..=b'Z').contains(&first[0]) {
        stream.set_read_timeout(None)?;
        return Ok((ReplayStream::new(stream, prefix), false));
    }
    loop {
        if prefix.len() >= API_SNIFF_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP sniff limit exceeded",
            ));
        }
        if let Some(end) = find_bytes(&prefix, b"\r\n") {
            let request_line = String::from_utf8_lossy(&prefix[..end]).into_owned();
            let fields: Vec<&str> = request_line.split_whitespace().collect();
            let valid =
                fields.len() == 3 && is_http_method(fields[0]) && fields[2].starts_with("HTTP/");
            if !valid {
                stream.set_read_timeout(None)?;
                return Ok((ReplayStream::new(stream, prefix), false));
            }
            break;
        }
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte)?;
        prefix.push(byte[0]);
    }
    while find_bytes(&prefix, b"\r\n\r\n").is_none() {
        if prefix.len() >= API_SNIFF_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP sniff limit exceeded",
            ));
        }
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte)?;
        prefix.push(byte[0]);
    }
    stream.set_read_timeout(None)?;
    let is_http = !has_websocket_upgrade(&prefix);
    Ok((ReplayStream::new(stream, prefix), is_http))
}

pub fn has_websocket_upgrade(header: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(header).to_ascii_lowercase();
    lower.contains("\r\nupgrade: websocket") || lower.contains("\nupgrade: websocket")
}

fn is_http_method(value: &str) -> bool {
    matches!(
        value,
        "CONNECT" | "DELETE" | "GET" | "HEAD" | "OPTIONS" | "PATCH" | "POST" | "PUT" | "TRACE"
    )
}
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
fn empty_frame() -> WebSocketFrame {
    WebSocketFrame {
        fin: false,
        opcode: 0,
        payload: Vec::new(),
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

pub fn write_raw(stream: &mut ReplayStream, data: &[u8]) -> io::Result<()> {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut remaining = data;
    let result = loop {
        if remaining.is_empty() {
            break Ok(());
        }
        let count = match stream.write(remaining) {
            Ok(count) => count,
            Err(error) => break Err(error),
        };
        if count == 0 {
            break Err(io::Error::new(io::ErrorKind::WriteZero, "short write"));
        }
        remaining = &remaining[count..];
    };
    let _ = stream.set_write_timeout(None);
    result
}

pub fn system_time_unix() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
