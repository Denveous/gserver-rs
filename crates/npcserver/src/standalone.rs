use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

use crate::runtime::{Logger, configured_nickname};
use crate::settings::Settings;

const STANDALONE_VERSION: &str = "G3D0311C";

pub struct Standalone {
    pub settings: Settings,
    logger: Logger,
    listener: Mutex<Option<TcpListener>>,
    game_conn: Mutex<Option<TcpStream>>,
}

impl Standalone {
    pub fn new(settings: Settings, logger: Option<Logger>) -> Self {
        let logger = logger.unwrap_or_else(|| Arc::new(|_: String| {}));
        Self {
            settings,
            logger,
            listener: Mutex::new(None),
            game_conn: Mutex::new(None),
        }
    }

    pub fn listen_address(&self) -> String {
        let mut host = self.settings.get("npcserver_ip");
        if host.is_empty() || host.eq_ignore_ascii_case("auto") || host == "*" {
            host = "0.0.0.0".to_string();
        }
        join_host_port(&host, self.listener_port())
    }

    pub fn game_server_address(&self) -> String {
        let mut host = self.settings.get("gserver_ip");
        if host.is_empty() || host.eq_ignore_ascii_case("auto") || host == "*" {
            host = "127.0.0.1".to_string();
        }
        let port = valid_port(self.settings.get_int("gserver_port", 14877), 14877);
        join_host_port(&host, port)
    }

    pub fn listener_port(&self) -> i32 {
        valid_port(self.settings.get_int("npcserver_port", 14899), 14899)
    }

    #[allow(non_snake_case)]
    pub fn ListenAddress(&self) -> String {
        self.listen_address()
    }

    #[allow(non_snake_case)]
    pub fn GameServerAddress(&self) -> String {
        self.game_server_address()
    }

    /// Run until cancel becomes true. An atomic flag lets callers use any
    /// signal or runtime integration without coupling this crate to a
    /// particular async runtime.
    pub fn run_with_cancel(&self, cancel: &AtomicBool) -> io::Result<()> {
        let listener = TcpListener::bind(self.listen_address())?;
        // The listener is closed by the cancellation path. The accept loop
        // uses a non-blocking clone and observes the same flag so
        // it can exit without leaving a thread blocked in accept().
        listener.set_nonblocking(true)?;
        *self.listener.lock().expect("listener mutex poisoned") = Some(listener.try_clone()?);
        (self.logger)(format!("listening on {}", listener.local_addr()?));
        let accept_listener = listener.try_clone()?;
        let game_cancel = Arc::new(AtomicBool::new(false));
        let accept_cancel = Arc::clone(&game_cancel);
        let control_settings = self.settings.clone();
        let control_logger = Arc::clone(&self.logger);
        thread::spawn(move || {
            accept_loop(
                accept_cancel,
                accept_listener,
                control_settings,
                control_logger,
            )
        });
        let monitor_stop = Arc::new(AtomicBool::new(false));
        let monitor_stop_for_thread = Arc::clone(&monitor_stop);
        let game_cancel_for_thread = Arc::clone(&game_cancel);
        let result = thread::scope(|scope| {
            let monitor = scope.spawn(move || {
                while !cancel.load(Ordering::Acquire)
                    && !monitor_stop_for_thread.load(Ordering::Acquire)
                {
                    thread::sleep(Duration::from_millis(25));
                }
                if cancel.load(Ordering::Acquire) {
                    game_cancel_for_thread.store(true, Ordering::Release);
                    self.close_listener();
                    self.close_game_connection();
                }
            });
            let result = self.game_loop(cancel);
            game_cancel.store(true, Ordering::Release);
            monitor_stop.store(true, Ordering::Release);
            self.close_listener();
            self.close_game_connection();
            let _ = monitor.join();
            result
        });
        result
    }

    /// Convenience entry point matching the standalone process behavior.
    /// It runs until the process is terminated by its host.
    pub fn run(&self) -> io::Result<()> {
        let cancel = AtomicBool::new(false);
        self.run_with_cancel(&cancel)
    }

    fn game_loop(&self, cancel: &AtomicBool) -> io::Result<()> {
        loop {
            if cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            let address = self.game_server_address();
            let stream = match connect_timeout(&address, Duration::from_secs(5)) {
                Ok(value) => value,
                Err(error) => {
                    (self.logger)(format!("GameServer connection failed: {error}"));
                    if !wait_cancel(cancel, Duration::from_secs(2)) {
                        return Ok(());
                    }
                    continue;
                }
            };
            stream.set_nodelay(true).ok();
            self.set_game_connection(&stream);
            (self.logger)(format!("connected to GameServer at {address}"));
            let result = self.run_game_connection(cancel, &stream);
            let _ = stream.shutdown(Shutdown::Both);
            self.clear_game_connection(&stream);
            if cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            if let Err(error) = result {
                (self.logger)(format!("GameServer connection closed: {error}"));
            }
            if !wait_cancel(cancel, Duration::from_secs(2)) {
                return Ok(());
            }
        }
    }

    fn run_game_connection(&self, cancel: &AtomicBool, stream: &TcpStream) -> io::Result<()> {
        let mut writer = stream.try_clone()?;
        write_all(
            &mut writer,
            &standalone_login_frame(&self.settings.game_server_account(), self.listener_port()),
        )?;
        loop {
            if cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            let frame = read_legacy_frame(&mut &*stream)?;
            let payload = zlib_decompress(&frame)?;
            self.handle_game_payload(stream, &payload)?;
        }
    }

    fn handle_game_payload(&self, stream: &TcpStream, payload: &[u8]) -> io::Result<()> {
        if payload.is_empty() {
            return Ok(());
        }
        let packet_id = i32::from(payload[0]) - 32;
        if packet_id < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid GameServer packet id {}", payload[0]),
            ));
        }
        (self.logger)(format!(
            "GameServer packet id={packet_id} bytes={}",
            payload.len()
        ));
        if packet_id == 0 {
            let mut writer = stream.try_clone()?;
            write_all(
                &mut writer,
                &standalone_nickname_frame(&self.settings.get("nickname")),
            )?;
        }
        Ok(())
    }

    fn set_game_connection(&self, stream: &TcpStream) {
        *self
            .game_conn
            .lock()
            .expect("game connection mutex poisoned") = stream.try_clone().ok();
    }

    fn clear_game_connection(&self, stream: &TcpStream) {
        let mut connection = self
            .game_conn
            .lock()
            .expect("game connection mutex poisoned");
        if let Some(existing) = connection.as_ref() {
            if same_connection(existing, stream) {
                *connection = None;
            }
        }
    }

    fn close_game_connection(&self) {
        if let Some(stream) = self
            .game_conn
            .lock()
            .expect("game connection mutex poisoned")
            .take()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    fn close_listener(&self) {
        if let Some(listener) = self
            .listener
            .lock()
            .expect("listener mutex poisoned")
            .take()
        {
            drop(listener);
        }
    }
}

fn accept_loop(cancel: Arc<AtomicBool>, listener: TcpListener, settings: Settings, logger: Logger) {
    while !cancel.load(Ordering::Acquire) {
        let connection = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(error) => {
                if cancel.load(Ordering::Acquire) {
                    return;
                }
                (logger)(format!("control listener stopped: {error}"));
                return;
            }
        };
        let peer = connection
            .peer_addr()
            .map(|x| x.to_string())
            .unwrap_or_default();
        (logger)(format!("accepted control connection from {peer}"));
        let child_cancel = Arc::clone(&cancel);
        let child_logger = Arc::clone(&logger);
        let child_settings = settings.clone();
        thread::spawn(move || {
            proxy_control(child_cancel, connection, child_settings, child_logger)
        });
    }
}

fn proxy_control(cancel: Arc<AtomicBool>, client: TcpStream, settings: Settings, logger: Logger) {
    let peer = client
        .peer_addr()
        .map(|x| x.to_string())
        .unwrap_or_default();
    let game_address = {
        let mut host = settings.get("gserver_ip");
        if host.is_empty() || host.eq_ignore_ascii_case("auto") || host == "*" {
            host = "127.0.0.1".to_string();
        }
        join_host_port(
            &host,
            valid_port(settings.get_int("gserver_port", 14877), 14877),
        )
    };
    let game = match connect_timeout(&game_address, Duration::from_secs(5)) {
        Ok(value) => value,
        Err(error) => {
            (logger)(format!(
                "control connection from {peer} could not reach GameServer: {error}"
            ));
            return;
        }
    };
    let mut to_game = game.try_clone().ok();
    let mut from_game = client.try_clone().ok();
    let client_close = client.try_clone().ok();
    let game_close = game.try_clone().ok();
    let mut client_read = client;
    let mut game_read = game;
    client_read
        .set_read_timeout(Some(Duration::from_millis(100)))
        .ok();
    game_read
        .set_read_timeout(Some(Duration::from_millis(100)))
        .ok();
    let proxy_stop = Arc::new(AtomicBool::new(false));
    let first_stop = Arc::clone(&proxy_stop);
    let first_cancel = Arc::clone(&cancel);
    let first = thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        while !first_stop.load(Ordering::Acquire) && !first_cancel.load(Ordering::Acquire) {
            match client_read.read(&mut buffer) {
                Ok(0) => break,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    continue;
                }
                Err(_) => break,
                Ok(count) => {
                    if to_game
                        .as_mut()
                        .is_none_or(|x| x.write_all(&buffer[..count]).is_err())
                    {
                        break;
                    }
                }
            }
        }
        first_stop.store(true, Ordering::Release);
    });
    let second_stop = Arc::clone(&proxy_stop);
    let second_cancel = Arc::clone(&cancel);
    let second = thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        while !second_stop.load(Ordering::Acquire) && !second_cancel.load(Ordering::Acquire) {
            match game_read.read(&mut buffer) {
                Ok(0) => break,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    continue;
                }
                Err(_) => break,
                Ok(count) => {
                    if from_game
                        .as_mut()
                        .is_none_or(|x| x.write_all(&buffer[..count]).is_err())
                    {
                        break;
                    }
                }
            }
        }
        second_stop.store(true, Ordering::Release);
    });
    while !cancel.load(Ordering::Acquire) && !proxy_stop.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(10));
    }
    proxy_stop.store(true, Ordering::Release);
    if let Some(stream) = client_close {
        let _ = stream.shutdown(Shutdown::Both);
    }
    if let Some(stream) = game_close {
        let _ = stream.shutdown(Shutdown::Both);
    }
    let _ = first.join();
    let _ = second.join();
}

fn connect_timeout(address: &str, timeout: Duration) -> io::Result<TcpStream> {
    let mut addresses = address.to_socket_addrs()?;
    let mut last = None;
    while let Some(address) = addresses.next() {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(value) => return Ok(value),
            Err(error) => last = Some(error),
        }
    }
    Err(last
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "no socket address")))
}

fn same_connection(left: &TcpStream, right: &TcpStream) -> bool {
    left.local_addr().ok() == right.local_addr().ok()
        && left.peer_addr().ok() == right.peer_addr().ok()
}

pub fn standalone_login_frame(account: &str, listener_port: i32) -> Vec<u8> {
    let mut bytes = account.trim().as_bytes().to_vec();
    if bytes.len() > 223 {
        bytes.truncate(223);
    }
    if bytes.is_empty() {
        bytes = b"npcserver".to_vec();
    }
    let mut raw = Vec::new();
    write_gchar(&mut raw, 2);
    raw.extend_from_slice(STANDALONE_VERSION.as_bytes());
    write_gchar(&mut raw, bytes.len() as u8);
    raw.extend_from_slice(&bytes);
    write_gchar(&mut raw, 0);
    raw.push(0);
    raw.push((listener_port >> 8) as u8);
    raw.push(listener_port as u8);
    legacy_frame(&zlib_compress(&raw))
}

pub fn standalone_nickname_frame(nickname: &str) -> Vec<u8> {
    let nickname = configured_nickname(nickname);
    let mut bytes = nickname.as_bytes().to_vec();
    if bytes.len() > 223 {
        bytes.truncate(223);
    }
    let mut packet = vec![34, 32, bytes.len() as u8 + 32];
    packet.extend_from_slice(&bytes);
    legacy_frame(&zlib_compress(&packet))
}

pub fn legacy_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 2);
    frame.push((payload.len() >> 8) as u8);
    frame.push(payload.len() as u8);
    frame.extend_from_slice(payload);
    frame
}

pub fn read_legacy_frame<R: Read>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut header = [0u8; 2];
    reader.read_exact(&mut header)?;
    let length = usize::from(u16::from_be_bytes(header));
    if length == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "empty legacy frame",
        ));
    }
    let mut frame = vec![0u8; length];
    reader.read_exact(&mut frame)?;
    Ok(frame)
}

pub fn write_all<W: Write>(writer: &mut W, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        let count = writer.write(data)?;
        if count == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "short write"));
        }
        data = &data[count..];
    }
    Ok(())
}

pub fn zlib_compress(data: &[u8]) -> Vec<u8> {
    let mut writer = ZlibEncoder::new(Vec::new(), Compression::default());
    let _ = writer.write_all(data);
    writer.finish().unwrap_or_default()
}

pub fn zlib_decompress(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut reader = ZlibDecoder::new(data);
    let mut result = Vec::new();
    reader.read_to_end(&mut result)?;
    Ok(result)
}

fn write_gchar(out: &mut Vec<u8>, mut value: u8) {
    if value > 223 {
        value = 223;
    }
    out.push(value + 32);
}

pub fn wait_cancel(cancel: &AtomicBool, duration: Duration) -> bool {
    let mut elapsed = Duration::ZERO;
    while elapsed < duration {
        if cancel.load(Ordering::Acquire) {
            return false;
        }
        let step = Duration::from_millis(25).min(duration - elapsed);
        thread::sleep(step);
        elapsed += step;
    }
    true
}

fn valid_port(value: i32, fallback: i32) -> i32 {
    if !(1..=65535).contains(&value) {
        fallback
    } else {
        value
    }
}

fn join_host_port(host: &str, port: i32) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[allow(non_snake_case)]
pub fn NewStandalone(settings: Settings, logger: Option<Logger>) -> Standalone {
    Standalone::new(settings, logger)
}
