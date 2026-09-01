use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use serde_json::Value;

use crate::vm::{AnyMap, SocketAction, SocketContext};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SocketUpdate {
    pub name: String,
    pub id: String,
    pub address: String,
    pub port: i32,
    pub ip_address: String,
    pub data: String,
    pub buffer: String,
    pub package_delimiter: String,
    pub is_connected: bool,
    pub state: AnyMap,
    pub joined_classes: Vec<String>,
    pub parent_name: String,
    pub parent_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct SocketScript {
    pub script_type: String,
    pub script_name: String,
    pub event_name: String,
    pub script: String,
    pub player_context: HashMap<String, String>,
    pub npc_id: u32,
    pub revision: i64,
    pub this: AnyMap,
}

#[derive(Clone, Debug, Default)]
pub struct SocketEvent {
    pub base: SocketScript,
    pub name: String,
    pub id: String,
    pub event: String,
    pub socket: SocketContext,
    pub argument: Option<SocketContext>,
    pub params: Vec<String>,
}

pub type SocketFire = Arc<dyn Fn(SocketEvent) + Send + Sync + 'static>;

struct Socket {
    key: String,
    name: String,
    id: String,
    port: i32,
    conn: Option<Arc<Mutex<TcpStream>>>,
    listener: Option<TcpListener>,
    package_delimiter: String,
    buffer: String,
    address: String,
    ip_address: String,
    state: AnyMap,
    joined_classes: Vec<String>,
    parent_name: String,
    parent_id: String,
    result: SocketScript,
    closed: AtomicBool,
}

struct ManagerInner {
    sockets: Mutex<HashMap<String, Arc<Mutex<Socket>>>>,
    fire: Option<SocketFire>,
    event_queues: Mutex<HashMap<String, Vec<SocketEvent>>>,
    event_processing: Mutex<HashMap<String, bool>>,
}

pub struct SocketManager {
    inner: Arc<ManagerInner>,
}

impl SocketManager {
    pub fn new<F>(fire: Option<F>) -> Self
    where
        F: Fn(SocketEvent) + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(ManagerInner {
                sockets: Mutex::new(HashMap::new()),
                fire: fire.map(|value| Arc::new(value) as SocketFire),
                event_queues: Mutex::new(HashMap::new()),
                event_processing: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn without_callback() -> Self {
        Self::new::<fn(SocketEvent)>(None)
    }

    pub fn apply(&self, result: SocketScript, updates: &[SocketUpdate], actions: &[SocketAction]) {
        for update in updates {
            self.update(update);
        }
        for action in actions {
            match action.action.to_ascii_lowercase().as_str() {
                "join" => self.join(&result, action),
                "bind" => self.bind(&result, action),
                "connect" => self.connect(&result, action),
                "close" => self.close(&result, action),
                "send" => self.send(&result, action),
                "trigger" => self.trigger(&result, action),
                _ => {}
            }
        }
    }

    #[allow(non_snake_case)]
    pub fn Apply(
        &self,
        result: SocketScript,
        updates: Vec<SocketUpdate>,
        actions: Vec<SocketAction>,
    ) {
        self.apply(result, &updates, &actions)
    }

    pub fn prepare_bind(
        &self,
        result: &SocketScript,
        action: &SocketAction,
    ) -> Result<SocketContext, String> {
        if action.name.is_empty() {
            return Err("socket name is required".to_string());
        }
        if !(0..=u16::MAX as i32).contains(&action.port) {
            return Err(format!("invalid port {}", action.port));
        }
        let key = self.key(result, &action.name, &action.id);
        self.close_key(&key);
        let listener =
            TcpListener::bind(("0.0.0.0", action.port as u16)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let actual_port = listener
            .local_addr()
            .map(|x| x.port() as i32)
            .unwrap_or(action.port);
        let socket = Socket {
            key: key.clone(),
            name: action.name.clone(),
            id: action.id.clone(),
            port: actual_port,
            listener: Some(listener),
            conn: None,
            package_delimiter: action.package_delimiter.clone(),
            buffer: String::new(),
            address: String::new(),
            ip_address: String::new(),
            state: action.state.clone(),
            joined_classes: action.joined_classes.clone(),
            parent_name: String::new(),
            parent_id: String::new(),
            result: result.clone(),
            closed: AtomicBool::new(false),
        };
        let socket = Arc::new(Mutex::new(socket));
        self.inner
            .sockets
            .lock()
            .expect("socket mutex poisoned")
            .insert(key, Arc::clone(&socket));
        Ok(self.context(&socket))
    }

    #[allow(non_snake_case)]
    pub fn PrepareBind(
        &self,
        result: &SocketScript,
        action: &SocketAction,
    ) -> Result<SocketContext, String> {
        self.prepare_bind(result, action)
    }

    pub fn close_all(&self) {
        let sockets = {
            let mut values = self.inner.sockets.lock().expect("socket mutex poisoned");
            let result = values.values().cloned().collect::<Vec<_>>();
            values.clear();
            result
        };
        for socket in sockets {
            let (connection, listener) = {
                let mut value = socket.lock().expect("socket mutex poisoned");
                value.closed.store(true, Ordering::Release);
                (value.conn.take(), value.listener.take())
            };
            if let Some(connection) = connection {
                if let Ok(stream) = connection.lock() {
                    let _ = stream.shutdown(Shutdown::Both);
                }
            }
            drop(listener);
        }
    }

    #[allow(non_snake_case)]
    pub fn CloseAll(&self) {
        self.close_all()
    }

    fn bind(&self, result: &SocketScript, action: &SocketAction) {
        if action.name.is_empty() {
            return;
        }
        if action.prepared {
            if let Some(socket) = self.find(result, &action.name, &action.id) {
                let has_listener = socket
                    .lock()
                    .expect("socket mutex poisoned")
                    .listener
                    .is_some();
                if has_listener {
                    self.activate_bind(socket, result, action);
                    return;
                }
            }
        }
        if self.prepare_bind(result, action).is_err() {
            self.fire(
                result.clone(),
                &action.name,
                &action.id,
                "onBindFailed",
                socket_state(
                    &action.name,
                    &action.id,
                    "",
                    "",
                    action.port,
                    &action.package_delimiter,
                    "",
                    "",
                    false,
                    &action.state,
                    &action.joined_classes,
                    "",
                    "",
                ),
                None,
                &[],
            );
            return;
        }
        if let Some(socket) = self.find(result, &action.name, &action.id) {
            self.activate_bind(socket, result, action);
        }
    }

    fn activate_bind(
        &self,
        socket: Arc<Mutex<Socket>>,
        result: &SocketScript,
        action: &SocketAction,
    ) {
        let listener = {
            let mut value = socket.lock().expect("socket mutex poisoned");
            value.package_delimiter = action.package_delimiter.clone();
            value.state = action.state.clone();
            value.joined_classes = action.joined_classes.clone();
            value.result = result.clone();
            value.listener.as_ref().and_then(|x| x.try_clone().ok())
        };
        self.fire(
            result.clone(),
            &action.name,
            &action.id,
            "onBind",
            self.context(&socket),
            None,
            &[],
        );
        let Some(listener) = listener else {
            return;
        };
        let manager = Arc::clone(&self.inner);
        thread::spawn(move || accept_loop(manager, socket, listener));
    }

    fn connect(&self, result: &SocketScript, action: &SocketAction) {
        if action.name.is_empty() || action.address.is_empty() || action.port <= 0 {
            return;
        }
        let key = self.key(result, &action.name, &action.id);
        self.close_key(&key);
        let address = join_host_port(&action.address, action.port);
        let stream = match TcpStream::connect(address) {
            Ok(value) => value,
            Err(_) => {
                self.fire(
                    result.clone(),
                    &action.name,
                    &action.id,
                    "onConnectFailed",
                    socket_state(
                        &action.name,
                        &action.id,
                        &action.address,
                        &action.address,
                        action.port,
                        &action.package_delimiter,
                        "",
                        "",
                        false,
                        &action.state,
                        &action.joined_classes,
                        "",
                        "",
                    ),
                    None,
                    &[],
                );
                return;
            }
        };
        let socket = Arc::new(Mutex::new(Socket {
            key: key.clone(),
            name: action.name.clone(),
            id: action.id.clone(),
            port: action.port,
            conn: Some(Arc::new(Mutex::new(stream))),
            listener: None,
            package_delimiter: action.package_delimiter.clone(),
            buffer: String::new(),
            address: action.address.clone(),
            ip_address: String::new(),
            state: action.state.clone(),
            joined_classes: action.joined_classes.clone(),
            parent_name: String::new(),
            parent_id: String::new(),
            result: result.clone(),
            closed: AtomicBool::new(false),
        }));
        self.inner
            .sockets
            .lock()
            .expect("socket mutex poisoned")
            .insert(key, Arc::clone(&socket));
        self.fire(
            result.clone(),
            &action.name,
            &action.id,
            "onConnect",
            self.context(&socket),
            None,
            &[],
        );
        let manager = Arc::clone(&self.inner);
        thread::spawn(move || {
            let stream = {
                socket
                    .lock()
                    .expect("socket mutex poisoned")
                    .conn
                    .as_ref()
                    .and_then(|x| x.lock().ok().and_then(|y| y.try_clone().ok()))
            };
            if let Some(stream) = stream {
                read_loop(manager, socket, stream, None);
            }
        });
    }

    fn join(&self, result: &SocketScript, action: &SocketAction) {
        let Some(socket) = self.ensure(result, &action.name, &action.id) else {
            return;
        };
        {
            let mut value = socket.lock().expect("socket mutex poisoned");
            value.state = action.state.clone();
            value.joined_classes = action.joined_classes.clone();
        }
        self.fire(
            result.clone(),
            &action.name,
            &action.id,
            "onCreated",
            self.context(&socket),
            None,
            &[],
        );
    }

    fn trigger(&self, result: &SocketScript, action: &SocketAction) {
        let Some(socket) = self.ensure(result, &action.name, &action.id) else {
            return;
        };
        {
            let mut value = socket.lock().expect("socket mutex poisoned");
            for (key, item) in &action.state {
                if !item.is_null() {
                    value.state.insert(key.clone(), item.clone());
                }
            }
            if !action.joined_classes.is_empty() {
                value.joined_classes = action.joined_classes.clone();
            }
        }
        let mut event = action.event.clone();
        if !event.is_empty() && !event.to_ascii_lowercase().starts_with("on") {
            let mut chars = event.chars();
            event = chars
                .next()
                .map(|x| format!("on{}{}", x.to_ascii_uppercase(), chars.collect::<String>()))
                .unwrap_or_default();
        }
        self.fire(
            result.clone(),
            &action.name,
            &action.id,
            &event,
            self.context(&socket),
            None,
            &action.params,
        );
    }

    fn send(&self, result: &SocketScript, action: &SocketAction) {
        let Some(socket) = self.find(result, &action.name, &action.id) else {
            return;
        };
        let stream = socket.lock().expect("socket mutex poisoned").conn.clone();
        if let Some(stream) = stream {
            let _ = stream
                .lock()
                .expect("socket stream mutex poisoned")
                .write(action.data.as_bytes());
        }
    }

    fn close(&self, result: &SocketScript, action: &SocketAction) {
        if action.id.is_empty() {
            self.close_key(&self.key(result, &action.name, ""));
            return;
        }
        if let Some(socket) = self.find(result, &action.name, &action.id) {
            self.close_socket(&socket);
        }
    }

    fn update(&self, update: &SocketUpdate) {
        let sockets = self.inner.sockets.lock().expect("socket mutex poisoned");
        for socket in sockets.values() {
            let mut value = socket.lock().expect("socket mutex poisoned");
            if value.name == update.name && value.id == update.id {
                value.package_delimiter = update.package_delimiter.clone();
                value.address = update.address.clone();
                value.ip_address = update.ip_address.clone();
                value.buffer = if update.package_delimiter.is_empty() {
                    update.data.clone()
                } else {
                    update.buffer.clone()
                };
                value.state = update.state.clone();
                value.joined_classes = update.joined_classes.clone();
            }
        }
        propagate_referenced_socket_states_locked(
            &sockets,
            &Value::Object(update.state.clone().into_iter().collect()),
            &mut HashMap::new(),
        );
    }

    fn close_key(&self, key: &str) {
        let socket = self
            .inner
            .sockets
            .lock()
            .expect("socket mutex poisoned")
            .remove(key);
        if let Some(socket) = socket {
            self.close_socket(&socket);
        }
    }

    fn close_socket(&self, socket: &Arc<Mutex<Socket>>) {
        let mut children = Vec::new();
        {
            let sockets = self.inner.sockets.lock().expect("socket mutex poisoned");
            let value = socket.lock().expect("socket mutex poisoned");
            if value.closed.swap(true, Ordering::AcqRel) {
                return;
            }
            let key = value.key.clone();
            drop(value);
            drop(sockets);
            self.inner
                .sockets
                .lock()
                .expect("socket mutex poisoned")
                .remove(&key);
        }
        {
            let sockets = self.inner.sockets.lock().expect("socket mutex poisoned");
            let parent = socket.lock().expect("socket mutex poisoned");
            for candidate in sockets.values() {
                if Arc::ptr_eq(candidate, socket) {
                    continue;
                }
                let candidate_value = candidate.lock().expect("socket mutex poisoned");
                if candidate_value.closed.load(Ordering::Acquire) {
                    continue;
                }
                if (candidate_value.parent_name == parent.name
                    && candidate_value.parent_id == parent.id)
                    || socket_state_has_parent(&candidate_value.state, &parent.name, &parent.id, 0)
                {
                    children.push(Arc::clone(candidate));
                }
            }
        }
        let (connection, listener) = {
            let mut value = socket.lock().expect("socket mutex poisoned");
            (value.conn.take(), value.listener.take())
        };
        if let Some(connection) = connection {
            if let Ok(stream) = connection.lock() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
        drop(listener);
        for child in children {
            self.close_socket(&child);
        }
    }

    fn find(&self, result: &SocketScript, name: &str, id: &str) -> Option<Arc<Mutex<Socket>>> {
        let sockets = self.inner.sockets.lock().expect("socket mutex poisoned");
        if id.is_empty() {
            return sockets.get(&self.key(result, name, id)).cloned();
        }
        sockets
            .values()
            .find(|socket| {
                let value = socket.lock().expect("socket mutex poisoned");
                value.name == name && value.id == id
            })
            .cloned()
    }

    fn ensure(&self, result: &SocketScript, name: &str, id: &str) -> Option<Arc<Mutex<Socket>>> {
        if name.is_empty() {
            return None;
        }
        let key = self.key(result, name, id);
        let mut sockets = self.inner.sockets.lock().expect("socket mutex poisoned");
        if let Some(socket) = sockets.get(&key) {
            return Some(Arc::clone(socket));
        }
        let socket = Arc::new(Mutex::new(Socket {
            key: key.clone(),
            name: name.to_string(),
            id: id.to_string(),
            port: 0,
            conn: None,
            listener: None,
            package_delimiter: String::new(),
            buffer: String::new(),
            address: String::new(),
            ip_address: String::new(),
            state: AnyMap::new(),
            joined_classes: Vec::new(),
            parent_name: String::new(),
            parent_id: String::new(),
            result: result.clone(),
            closed: AtomicBool::new(false),
        }));
        sockets.insert(key, Arc::clone(&socket));
        Some(socket)
    }

    fn key(&self, result: &SocketScript, name: &str, id: &str) -> String {
        format!(
            "{}:{}:{}:{}",
            result.script_type, result.script_name, name, id
        )
    }

    fn context(&self, socket: &Arc<Mutex<Socket>>) -> SocketContext {
        let value = socket.lock().expect("socket mutex poisoned");
        SocketContext {
            name: value.name.clone(),
            id: value.id.clone(),
            address: value.address.clone(),
            ip_address: value.ip_address.clone(),
            port: value.port,
            package_delimiter: value.package_delimiter.clone(),
            buffer: value.buffer.clone(),
            is_connected: !value.closed.load(Ordering::Acquire),
            state: value.state.clone(),
            joined_classes: value.joined_classes.clone(),
            parent_name: value.parent_name.clone(),
            parent_id: value.parent_id.clone(),
            ..SocketContext::default()
        }
    }

    fn fire(
        &self,
        base: SocketScript,
        name: &str,
        id: &str,
        event: &str,
        socket: SocketContext,
        argument: Option<SocketContext>,
        params: &[String],
    ) {
        fire_inner(&self.inner, base, name, id, event, socket, argument, params);
    }
}

fn join_host_port(host: &str, port: i32) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn accept_loop(inner: Arc<ManagerInner>, server_socket: Arc<Mutex<Socket>>, listener: TcpListener) {
    loop {
        let closed = server_socket
            .lock()
            .expect("socket mutex poisoned")
            .closed
            .load(Ordering::Acquire);
        if closed {
            return;
        }
        let connection = match listener.accept() {
            Ok((connection, _)) => connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            Err(_) => return,
        };
        let peer = connection.peer_addr().ok();
        let local = connection.local_addr().ok();
        let id = peer.map(|x| x.to_string()).unwrap_or_default();
        let host = peer.map(|x| x.ip().to_string()).unwrap_or_default();
        let local_host = local.map(|x| x.ip().to_string()).unwrap_or_default();
        let (name, port, delimiter, result, parent_id) = {
            let value = server_socket.lock().expect("socket mutex poisoned");
            (
                value.name.clone(),
                value.port,
                value.package_delimiter.clone(),
                value.result.clone(),
                value.id.clone(),
            )
        };
        let key = format!(
            "{}:{}:{}:{}",
            result.script_type, result.script_name, name, id
        );
        let client = Arc::new(Mutex::new(Socket {
            key: key.clone(),
            name: name.clone(),
            id: id.clone(),
            port,
            conn: Some(Arc::new(Mutex::new(connection))),
            listener: None,
            package_delimiter: delimiter,
            buffer: String::new(),
            address: local_host,
            ip_address: host,
            state: AnyMap::new(),
            joined_classes: Vec::new(),
            parent_name: name.clone(),
            parent_id: parent_id.clone(),
            result: result.clone(),
            closed: AtomicBool::new(false),
        }));
        inner
            .sockets
            .lock()
            .expect("socket mutex poisoned")
            .insert(key, Arc::clone(&client));
        let (sender, receiver) = mpsc::channel();
        let read_socket = Arc::clone(&client);
        let read_inner = Arc::clone(&inner);
        let stream = client
            .lock()
            .expect("socket mutex poisoned")
            .conn
            .as_ref()
            .and_then(|x| x.lock().ok().and_then(|y| y.try_clone().ok()));
        if let Some(stream) = stream {
            thread::spawn(move || read_loop(read_inner, read_socket, stream, Some(receiver)));
        }
        let manager = SocketManager {
            inner: Arc::clone(&inner),
        };
        let argument = manager.context(&client);
        manager.fire(
            result.clone(),
            &name,
            &parent_id,
            "onNewClient",
            manager.context(&server_socket),
            Some(argument),
            &[],
        );
        let _ = sender.send(());
    }
}

fn read_loop(
    inner: Arc<ManagerInner>,
    socket: Arc<Mutex<Socket>>,
    mut stream: TcpStream,
    mut ready: Option<mpsc::Receiver<()>>,
) {
    let mut buffer = [0u8; 4096];
    loop {
        let count = match stream.read(&mut buffer) {
            Ok(value) if value > 0 => value,
            _ => {
                let manager = SocketManager {
                    inner: Arc::clone(&inner),
                };
                manager.close_socket(&socket);
                let (result, name, id) = {
                    let value = socket.lock().expect("socket mutex poisoned");
                    (value.result.clone(), value.name.clone(), value.id.clone())
                };
                manager.fire(
                    result,
                    &name,
                    &id,
                    "onClose",
                    manager.context(&socket),
                    None,
                    &[],
                );
                return;
            }
        };
        let chunk = String::from_utf8_lossy(&buffer[..count]).to_string();
        let (delimiter, accumulated) = {
            let mut value = socket.lock().expect("socket mutex poisoned");
            value.buffer.push_str(&chunk);
            (value.package_delimiter.clone(), value.buffer.clone())
        };
        // Wait for the accept callback only for the first packet. Consuming
        // the receiver here is important: retaining
        // it would make every subsequent read wait for a channel that has
        // already been closed.
        if let Some(receiver) = ready.take() {
            let _ = receiver.recv();
        }
        let manager = SocketManager {
            inner: Arc::clone(&inner),
        };
        if delimiter.is_empty() {
            let mut state = manager.context(&socket);
            state.data = accumulated.clone();
            state.buffer = accumulated;
            let (result, name, id) = socket_identity(&socket);
            manager.fire(result, &name, &id, "onReceiveData", state, None, &[chunk]);
            continue;
        }
        loop {
            let packet = {
                let mut value = socket.lock().expect("socket mutex poisoned");
                let Some(index) = value.buffer.find(&delimiter) else {
                    break;
                };
                let packet = value.buffer[..index].to_string();
                value.buffer = value.buffer[index + delimiter.len()..].to_string();
                (packet, value.buffer.clone())
            };
            let mut state = manager.context(&socket);
            state.data = packet.0.clone();
            state.buffer = packet.1;
            let (result, name, id) = socket_identity(&socket);
            manager.fire(
                result,
                &name,
                &id,
                "onReceiveDataPackage",
                state,
                None,
                &[packet.0],
            );
        }
    }
}

fn socket_identity(socket: &Arc<Mutex<Socket>>) -> (SocketScript, String, String) {
    let value = socket.lock().expect("socket mutex poisoned");
    (value.result.clone(), value.name.clone(), value.id.clone())
}

fn fire_inner(
    inner: &Arc<ManagerInner>,
    base: SocketScript,
    name: &str,
    id: &str,
    event: &str,
    socket: SocketContext,
    argument: Option<SocketContext>,
    params: &[String],
) {
    let Some(callback) = inner.fire.clone() else {
        return;
    };
    let key = format!("{}\0{}", base.script_type, base.script_name);
    let item = SocketEvent {
        base,
        name: name.to_string(),
        id: id.to_string(),
        event: event.to_string(),
        socket,
        argument,
        params: params.to_vec(),
    };
    {
        let mut queues = inner
            .event_queues
            .lock()
            .expect("event queue mutex poisoned");
        queues.entry(key.clone()).or_default().push(item);
        let mut processing = inner
            .event_processing
            .lock()
            .expect("event processing mutex poisoned");
        if processing.get(&key).copied().unwrap_or(false) {
            return;
        }
        processing.insert(key.clone(), true);
    }
    loop {
        let mut next = {
            let mut queues = inner
                .event_queues
                .lock()
                .expect("event queue mutex poisoned");
            let queue = queues.entry(key.clone()).or_default();
            if queue.is_empty() {
                queues.remove(&key);
                inner
                    .event_processing
                    .lock()
                    .expect("event processing mutex poisoned")
                    .remove(&key);
                return;
            }
            queue.remove(0)
        };
        let manager = SocketManager {
            inner: Arc::clone(inner),
        };
        if let Some(socket) = manager.find(&next.base, &next.name, &next.id) {
            let current = manager.context(&socket);
            let data = next.socket.data.clone();
            let buffer = next.socket.buffer.clone();
            next.socket = SocketContext {
                data,
                buffer,
                ..current
            };
        }
        if let Some(argument) = next.argument.as_mut() {
            if let Some(socket) = manager.find(&next.base, &argument.name, &argument.id) {
                let current = manager.context(&socket);
                let data = argument.data.clone();
                let buffer = argument.buffer.clone();
                *argument = SocketContext {
                    data,
                    buffer,
                    ..current
                };
            }
        }
        callback(next);
    }
}

fn socket_state(
    name: &str,
    id: &str,
    address: &str,
    ip: &str,
    port: i32,
    delimiter: &str,
    data: &str,
    buffer: &str,
    connected: bool,
    state: &AnyMap,
    classes: &[String],
    parent_name: &str,
    parent_id: &str,
) -> SocketContext {
    SocketContext {
        name: name.to_string(),
        id: id.to_string(),
        address: address.to_string(),
        ip_address: ip.to_string(),
        port,
        package_delimiter: delimiter.to_string(),
        data: data.to_string(),
        buffer: buffer.to_string(),
        is_connected: connected,
        state: state.clone(),
        joined_classes: classes.to_vec(),
        parent_name: parent_name.to_string(),
        parent_id: parent_id.to_string(),
    }
}

pub fn copy_any_map(values: &AnyMap) -> AnyMap {
    values.clone()
}

fn socket_state_has_parent(value: &AnyMap, name: &str, id: &str, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    for (key, child) in value {
        if key.eq_ignore_ascii_case("parent") || key.eq_ignore_ascii_case("parentSocket") {
            if socket_state_references(child, name, id, depth + 1) {
                return true;
            }
        }
        if let Value::Object(map) = child {
            let map = map
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<AnyMap>();
            if socket_state_has_parent(&map, name, id, depth + 1) {
                return true;
            }
        } else if let Value::Array(values) = child {
            for item in values {
                if let Value::Object(map) = item {
                    let map = map
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<AnyMap>();
                    if socket_state_has_parent(&map, name, id, depth + 1) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn socket_state_references(value: &Value, name: &str, id: &str, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match value {
        Value::Object(map) => {
            if map
                .get("__tsocket_ref")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return map.get("name").and_then(Value::as_str).unwrap_or("") == name
                    && map.get("id").and_then(Value::as_str).unwrap_or("") == id;
            }
            map.values()
                .any(|child| socket_state_references(child, name, id, depth + 1))
        }
        Value::Array(values) => values
            .iter()
            .any(|child| socket_state_references(child, name, id, depth + 1)),
        _ => false,
    }
}

fn propagate_referenced_socket_states_locked(
    sockets: &HashMap<String, Arc<Mutex<Socket>>>,
    value: &Value,
    seen: &mut HashMap<String, bool>,
) {
    match value {
        Value::Object(map) => {
            if map
                .get("__tsocket_ref")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let name = map.get("name").and_then(Value::as_str).unwrap_or("");
                let id = map.get("id").and_then(Value::as_str).unwrap_or("");
                let key = format!("{name}\0{id}");
                if !seen.contains_key(&key) {
                    seen.insert(key, true);
                    if let Some(Value::Object(state)) = map.get("state") {
                        for socket in sockets.values() {
                            let mut socket = socket.lock().expect("socket mutex poisoned");
                            if socket.name == name && socket.id == id {
                                for (key, value) in state {
                                    if !value.is_null() {
                                        socket.state.insert(key.clone(), value.clone());
                                    }
                                }
                                if let Some(delimiter) =
                                    map.get("packagedelimiter").and_then(Value::as_str)
                                {
                                    socket.package_delimiter = delimiter.to_string();
                                }
                            }
                        }
                        let state = Value::Object(state.clone());
                        propagate_referenced_socket_states_locked(sockets, &state, seen);
                    }
                }
            }
            for child in map.values() {
                propagate_referenced_socket_states_locked(sockets, child, seen);
            }
        }
        Value::Array(values) => {
            for child in values {
                propagate_referenced_socket_states_locked(sockets, child, seen);
            }
        }
        _ => {}
    }
}

#[allow(non_snake_case)]
pub fn NewSocketManager<F>(fire: Option<F>) -> SocketManager
where
    F: Fn(SocketEvent) + Send + Sync + 'static,
{
    SocketManager::new(fire)
}

#[allow(non_snake_case)]
pub fn CopyAnyMap(values: &AnyMap) -> AnyMap {
    copy_any_map(values)
}
