//! Minimal HTTP/1.1 adapter for the server API.
//!
//! The native game listener is intentionally shared with the API listener in
//! the reference implementation.  `serve_connection` consumes the already
//! sniffed/replayed stream and emits one response, while `handle_request` is a
//! pure request/response surface useful to embedders and protocol tests.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose, Engine as _};
use ring::hmac;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model::{all_local_rights, serverOptionsStaffContains, Account, Server};
use crate::protocol::PLPERM_NPCCONTROL;
use crate::websocket::ReplayStream;

const DEFAULT_JWT_SECRET: &str =
    "Preagonal.GameServer.Default.Jwt.Signing.Key.Change.This.In.AdminConfig";

#[derive(Clone, Debug, Default)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl HttpRequest {
    pub fn header(&self, name: &str) -> String {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl HttpResponse {
    pub fn new(status: u16, content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".to_string(), content_type.to_string())],
            body,
        }
    }
    pub fn text(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::new(status, "text/plain; charset=utf-8", body.into())
    }
    pub fn json(status: u16, value: &Value) -> Self {
        let mut body = serde_json::to_vec(value).unwrap_or_default();
        body.push(b'\n');
        Self::new(status, "application/json; charset=utf-8", body)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiTokenClaims {
    #[serde(rename = "sub")]
    pub account: String,
    #[serde(rename = "admin_rights")]
    pub admin_rights: i32,
    #[serde(rename = "folder_right")]
    pub folder_rights: Vec<String>,
    #[serde(rename = "exp")]
    pub expires: i64,
}

pub fn serve_connection(mut stream: ReplayStream, server: Arc<Server>) {
    let mut data = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];
    loop {
        let header_end = loop {
            if let Some(index) = find_bytes(&data, b"\r\n\r\n") {
                break index + 4;
            }
            if data.len() > 64 * 1024 {
                return;
            }
            match stream.read(&mut chunk) {
                Ok(0) => return,
                Ok(count) => data.extend_from_slice(&chunk[..count]),
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(_) => return,
            }
        };
        let (request, remaining) = match parse_request(
            &data[..header_end],
            data[header_end..].to_vec(),
            &mut stream,
        ) {
            Ok(value) => value,
            Err(_) => return,
        };
        data = remaining;
        let close = request_connection_should_close(&request);
        let suppress_body = request.method == "HEAD";
        let response = handle_request(&server, &request);
        if write_response(&mut stream, response, close, suppress_body).is_err() || close {
            return;
        }
    }
}

fn parse_request(
    header: &[u8],
    mut body: Vec<u8>,
    stream: &mut ReplayStream,
) -> io::Result<(HttpRequest, Vec<u8>)> {
    let text = String::from_utf8_lossy(header);
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap_or_default();
    let fields: Vec<_> = first.split_whitespace().collect();
    if fields.len() != 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP request line",
        ));
    }
    let (raw_path, query) = fields[1]
        .split_once('?')
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .unwrap_or_else(|| (fields[1].to_string(), String::new()));
    let path = percent_decode(&raw_path);
    let mut headers = Vec::new();
    let mut length = 0usize;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim().to_string();
            if key.eq_ignore_ascii_case("content-length") {
                length = value.parse().unwrap_or(0);
            }
            headers.push((key.to_string(), value));
        }
    }
    while body.len() < length {
        let mut chunk = vec![0u8; length - body.len()];
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..count]);
    }
    let remaining = if body.len() > length {
        body.split_off(length)
    } else {
        Vec::new()
    };
    body.truncate(length);
    Ok((
        HttpRequest {
            method: fields[0].to_string(),
            path,
            query,
            headers,
            body,
        },
        remaining,
    ))
}

fn request_connection_should_close(request: &HttpRequest) -> bool {
    request
        .header("Connection")
        .split(',')
        .any(|value| value.trim().eq_ignore_ascii_case("close"))
}

fn write_response(
    stream: &mut ReplayStream,
    mut response: HttpResponse,
    close: bool,
    suppress_body: bool,
) -> io::Result<()> {
    let body_length = response.body.len();
    let mut bytes = Vec::new();
    let reason = reason_phrase(response.status);
    bytes.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", response.status, reason).as_bytes());
    response
        .headers
        .push(("Access-Control-Allow-Origin".to_string(), "*".to_string()));
    response.headers.push((
        "Access-Control-Allow-Headers".to_string(),
        "Authorization, Content-Type".to_string(),
    ));
    response.headers.push((
        "Access-Control-Allow-Methods".to_string(),
        "DELETE, GET, HEAD, OPTIONS, POST, PUT".to_string(),
    ));
    if !response
        .headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("Content-Length"))
    {
        response
            .headers
            .push(("Content-Length".to_string(), body_length.to_string()));
    }
    if close {
        response
            .headers
            .push(("Connection".to_string(), "close".to_string()));
    }
    for (key, value) in response.headers {
        bytes.extend_from_slice(format!("{key}: {value}\r\n").as_bytes());
    }
    bytes.extend_from_slice(b"\r\n");
    if !suppress_body {
        bytes.extend_from_slice(&response.body);
    }
    stream.write_all(&bytes)
}

pub fn handle_request(server: &Arc<Server>, request: &HttpRequest) -> HttpResponse {
    if request.method == "OPTIONS" {
        return response_with_headers(HttpResponse {
            status: 204,
            headers: Vec::new(),
            body: Vec::new(),
        });
    }
    match request.path.as_str() {
        "/" => {
            if request.method == "GET" {
                response_with_headers(HttpResponse::new(
                    200,
                    "text/html; charset=utf-8",
                    server.server_message.read().unwrap().as_bytes().to_vec(),
                ))
            } else {
                method_not_allowed(&["GET"])
            }
        }
        "/api" | "/api/" => api_info_response(server, request),
        "/api/v1/login" => handle_login(server, request),
        "/api/v1/stats" => {
            if request.method == "GET" {
                response_with_headers(HttpResponse::json(
                    200,
                    &json!({"levels":server.levels.read().unwrap().len(),"players":server.get_player_count()}),
                ))
            } else {
                method_not_allowed(&["GET"])
            }
        }
        "/api/v1/scripts/stats" => {
            if request.method != "GET" {
                method_not_allowed(&["GET"])
            } else if let Err(response) = require_claims(server, request, PLPERM_NPCCONTROL) {
                response
            } else {
                response_with_headers(HttpResponse::json(
                    200,
                    &json!({"weapons":server.weapons.read().unwrap().len(),"classes":server.classes.read().unwrap().len(),"npcs":server.npcs.read().unwrap().len()}),
                ))
            }
        }
        "/api/v1/scripts/definitions" => handle_script_definitions(server, request),
        "/swagger" | "/swagger/" => {
            if !server.settings.get_bool("enableswagger", true) {
                http_not_found()
            } else {
                redirect("/swagger/index.html", &request.method)
            }
        }
        "/swagger/v1/swagger.json" => {
            if !server.settings.get_bool("enableswagger", true) {
                http_not_found()
            } else if request.method != "GET" {
                method_not_allowed(&["GET"])
            } else {
                response_with_headers(HttpResponse::json(200, &openapi_spec()))
            }
        }
        path if path.starts_with("/swagger/") => serve_asset(server, request, &path[9..]),
        path if path == "/api/v1/files" || path.starts_with("/api/v1/files/") => {
            handle_files(server, request, path)
        }
        path if path.starts_with("/api/") => api_info_response(server, request),
        _ => http_not_found(),
    }
}

fn api_info_response(server: &Arc<Server>, request: &HttpRequest) -> HttpResponse {
    if request.method == "GET" {
        response_with_headers(HttpResponse::json(
            200,
            &json!({"name":server.configured_name(),"version":"v1","openapi":"/swagger/v1/swagger.json","endpoints":["/api/v1/login","/api/v1/stats","/api/v1/scripts/definitions","/api/v1/scripts/stats","/api/v1/files"]}),
        ))
    } else {
        method_not_allowed(&["GET"])
    }
}

fn response_with_headers(response: HttpResponse) -> HttpResponse {
    response
}
fn method_not_allowed(methods: &[&str]) -> HttpResponse {
    let mut response = HttpResponse::json(405, &json!({"error":"method not allowed"}));
    response
        .headers
        .push(("Allow".to_string(), methods.join(", ")));
    response
}
fn unauthorized() -> HttpResponse {
    HttpResponse::json(401, &json!({"error":"missing bearer token"}))
}
fn unauthorized_message(message: &str) -> HttpResponse {
    HttpResponse::json(401, &json!({"error":message}))
}
fn forbidden(message: &str) -> HttpResponse {
    HttpResponse::json(403, &json!({"error":message}))
}
fn bad_request(message: &str) -> HttpResponse {
    HttpResponse::json(400, &json!({"error":message}))
}
fn not_found() -> HttpResponse {
    HttpResponse::json(404, &json!({"error":"file or directory not found"}))
}
fn http_not_found() -> HttpResponse {
    let mut response = HttpResponse::text(404, "404 page not found\n");
    response
        .headers
        .push(("X-Content-Type-Options".to_string(), "nosniff".to_string()));
    response
}
fn redirect(path: &str, method: &str) -> HttpResponse {
    let body = if method == "GET" {
        format!("<a href=\"{path}\">Temporary Redirect</a>.\n\n").into_bytes()
    } else {
        Vec::new()
    };
    let mut response = HttpResponse::new(307, "text/html; charset=utf-8", body);
    response
        .headers
        .push(("Location".to_string(), path.to_string()));
    response
}

fn handle_login(server: &Arc<Server>, request: &HttpRequest) -> HttpResponse {
    if request.method != "POST" {
        return method_not_allowed(&["POST"]);
    }
    #[derive(Deserialize)]
    struct LoginRequest {
        account: Option<String>,
        password: Option<String>,
    }
    let body = &request.body[..request.body.len().min(1 << 20)];
    let mut decoder = serde_json::Deserializer::from_slice(body);
    let value = match LoginRequest::deserialize(&mut decoder) {
        Ok(value) => value,
        Err(_) => return bad_request("invalid login request"),
    };
    let account_value = value.account.unwrap_or_default();
    let account = account_value.trim();
    let password = value.password.unwrap_or_default();
    if account.is_empty()
        || password.is_empty()
        || !serverOptionsStaffContains(&server.settings.get("staff"), account)
    {
        return HttpResponse::json(401, &json!({"error":"invalid credentials"}));
    }
    let (authenticated, message) = server.authenticate_api(account, &password);
    if !authenticated {
        if message.eq_ignore_ascii_case("listserver unavailable") {
            return HttpResponse::json(503, &json!({"error":message}));
        }
        return HttpResponse::json(
            401,
            &json!({"error":if message.is_empty() { "invalid credentials" } else { message.as_str() }}),
        );
    }
    let token = match issue_api_token(server, account) {
        Ok(value) => value,
        Err(error) => return HttpResponse::json(500, &json!({"error":error.to_string()})),
    };
    HttpResponse::new(200, "text/plain; charset=utf-8", token.into_bytes())
}

fn handle_script_definitions(server: &Arc<Server>, request: &HttpRequest) -> HttpResponse {
    if request.method != "GET" {
        return method_not_allowed(&["GET"]);
    }
    if let Err(response) = require_claims(server, request, PLPERM_NPCCONTROL) {
        return response;
    }
    let mut definitions = Vec::new();
    for weapon in server.weapons.read().unwrap().values() {
        definitions.push(json!({"name":weapon.name,"type":"weapon","hasBytecode":!weapon.bytecode.is_empty(),"bytecodeSize":weapon.bytecode.len()}));
    }
    for class_obj in server.classes.read().unwrap().values() {
        definitions.push(
            json!({"name":class_obj.name,"type":"class","hasBytecode":false,"bytecodeSize":0}),
        );
    }
    for npc in server.npcs.read().unwrap().values() {
        let snapshot = npc.snapshot();
        definitions.push(
            json!({"name":snapshot.npc_name,"type":"npc","hasBytecode":false,"bytecodeSize":0}),
        );
    }
    definitions.sort_by(|a, b| {
        (
            a["type"].as_str().unwrap_or_default(),
            a["name"].as_str().unwrap_or_default(),
        )
            .cmp(&(
                b["type"].as_str().unwrap_or_default(),
                b["name"].as_str().unwrap_or_default(),
            ))
    });
    response_with_headers(HttpResponse::json(200, &Value::Array(definitions)))
}

fn secret(server: &Arc<Server>) -> String {
    let value = server.admin_settings.get("jwtsecretkey");
    if value.trim().is_empty() {
        DEFAULT_JWT_SECRET.to_string()
    } else {
        value.trim().to_string()
    }
}
pub fn sign_api_token(server: &Arc<Server>, claims: &ApiTokenClaims) -> String {
    let header = general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload =
        general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap_or_default());
    let message = format!("{header}.{payload}");
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret(server).as_bytes());
    let signature = hmac::sign(&key, message.as_bytes());
    format!(
        "{message}.{}",
        general_purpose::URL_SAFE_NO_PAD.encode(signature.as_ref())
    )
}
pub fn issue_api_token(server: &Arc<Server>, account: &str) -> io::Result<String> {
    let mut data = Account::new();
    data.set_server(server);
    let _ = data.load_account(account, true);
    let mut rights = data.admin_rights;
    if serverOptionsStaffContains(&server.settings.get("staff"), account) {
        rights |= all_local_rights();
    }
    if rights == 0 {
        rights = all_local_rights();
    }
    let folder_rights = if data.folder_list.is_empty() {
        server.default_rc_folder_rights()
    } else {
        data.folder_list.clone()
    };
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        + 7 * 24 * 60 * 60;
    Ok(sign_api_token(
        server,
        &ApiTokenClaims {
            account: account.to_string(),
            admin_rights: rights,
            folder_rights,
            expires,
        },
    ))
}
pub fn parse_api_claims(server: &Arc<Server>, request: &HttpRequest) -> io::Result<ApiTokenClaims> {
    let value = request.header("Authorization").trim().to_string();
    const PREFIX: &str = "Bearer ";
    let Some(prefix) = value.get(..PREFIX.len()) else {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "missing bearer token",
        ));
    };
    if !prefix.eq_ignore_ascii_case(PREFIX) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "missing bearer token",
        ));
    }
    let token = value.get(PREFIX.len()..).unwrap_or_default();
    let parts: Vec<_> = token.trim().split('.').collect();
    if parts.len() != 3 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid bearer token",
        ));
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret(server).as_bytes());
    let signature = general_purpose::URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "invalid bearer token"))?;
    hmac::verify(
        &key,
        format!("{}.{}", parts[0], parts[1]).as_bytes(),
        &signature,
    )
    .map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "invalid bearer token"))?;
    let bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "invalid bearer token"))?;
    let claims: ApiTokenClaims = serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "expired bearer token"))?;
    if claims.account.is_empty()
        || claims.expires
            <= SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "expired bearer token",
        ));
    }
    Ok(claims)
}
fn issue_claims(
    server: &Arc<Server>,
    request: &HttpRequest,
    right: i32,
) -> Result<ApiTokenClaims, HttpResponse> {
    let claims = parse_api_claims(server, request)
        .map_err(|error| unauthorized_message(&error.to_string()))?;
    if right != 0 && claims.admin_rights & right == 0 {
        return Err(forbidden("administrator right required"));
    }
    Ok(claims)
}
fn require_claims(
    server: &Arc<Server>,
    request: &HttpRequest,
    right: i32,
) -> Result<ApiTokenClaims, HttpResponse> {
    issue_claims(server, request, right)
}

fn handle_files(server: &Arc<Server>, request: &HttpRequest, path: &str) -> HttpResponse {
    let claims = match require_claims(server, request, 0) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let raw = path.strip_prefix("/api/v1/files").unwrap_or_default();
    let Some((rel, full)) = api_file_path(&server.config.get_base_path(), raw) else {
        return bad_request("invalid path");
    };
    match request.method.as_str() {
        "GET" | "HEAD" => {
            let metadata = match fs::metadata(&full) {
                Ok(value) => value,
                Err(_) => return not_found(),
            };
            if !rel.is_empty()
                && claims.admin_rights & PLPERM_NPCCONTROL == 0
                && !file_right(&claims, &rel, false)
            {
                return forbidden("folder read right required");
            }
            if metadata.is_dir() {
                let mut entries = Vec::new();
                let read_dir = match fs::read_dir(&full) {
                    Ok(value) => value,
                    Err(error) => {
                        return HttpResponse::json(500, &json!({"error":error.to_string()}));
                    }
                };
                for entry in read_dir.flatten() {
                    let Ok(info) = entry.metadata() else { continue };
                    let child = if rel.is_empty() {
                        entry.file_name().to_string_lossy().to_string()
                    } else {
                        format!("{rel}/{}", entry.file_name().to_string_lossy())
                    };
                    if claims.admin_rights & PLPERM_NPCCONTROL == 0
                        && !file_right(&claims, &child, false)
                    {
                        continue;
                    }
                    let modified = format_system_time(info.modified().unwrap_or(UNIX_EPOCH));
                    let mut item = json!({"name":entry.file_name().to_string_lossy(),"path":child,"isDirectory":info.is_dir(),"modified":modified});
                    if !info.is_dir() {
                        item["size"] = json!(info.len());
                    }
                    entries.push(item);
                }
                entries.sort_by(|a, b| {
                    (
                        b["isDirectory"].as_bool().unwrap_or(false),
                        a["name"].as_str().unwrap_or_default().to_ascii_lowercase(),
                    )
                        .cmp(&(
                            a["isDirectory"].as_bool().unwrap_or(false),
                            b["name"].as_str().unwrap_or_default().to_ascii_lowercase(),
                        ))
                });
                response_with_headers(HttpResponse::json(200, &Value::Array(entries)))
            } else {
                let mut response = HttpResponse::new(
                    200,
                    "application/octet-stream",
                    if request.method == "HEAD" {
                        Vec::new()
                    } else {
                        match fs::read(&full) {
                            Ok(value) => value,
                            Err(error) => {
                                return HttpResponse::json(500, &json!({"error":error.to_string()}))
                            }
                        }
                    },
                );
                response.headers.push((
                    "Content-Disposition".to_string(),
                    format!(
                        "attachment; filename=\"{}\"",
                        full.file_name().unwrap_or_default().to_string_lossy()
                    ),
                ));
                response
                    .headers
                    .push(("Content-Length".to_string(), metadata.len().to_string()));
                response
            }
        }
        "PUT" => {
            if rel.is_empty()
                || (claims.admin_rights & PLPERM_NPCCONTROL == 0
                    && !file_right(&claims, &rel, true))
            {
                return forbidden("folder write right required");
            }
            write_upload(request, &full)
                .map(|_| no_content())
                .unwrap_or_else(|error| bad_request(&error))
        }
        "POST" => {
            if let Some(destination) = query_value(&request.query, "destination") {
                if !destination.trim().is_empty()
                    && (claims.admin_rights & PLPERM_NPCCONTROL != 0
                        || (file_right(&claims, &rel, true)
                            && file_right(&claims, destination.trim(), true)))
                {
                    let Some((_, destination_full)) =
                        api_file_path(&server.config.get_base_path(), destination.trim())
                    else {
                        return bad_request("invalid path");
                    };
                    if let Some(parent) = destination_full.parent() {
                        if let Err(error) = fs::create_dir_all(parent) {
                            return HttpResponse::json(500, &json!({"error":error.to_string()}));
                        }
                    }
                    return fs::rename(&full, &destination_full)
                        .map(|_| no_content())
                        .unwrap_or_else(|error| bad_request(&error.to_string()));
                }
                if !destination.trim().is_empty() {
                    return forbidden("folder write right required");
                }
            }
            if rel.is_empty()
                || (claims.admin_rights & PLPERM_NPCCONTROL == 0
                    && !file_right(&claims, &rel, true))
            {
                return forbidden("folder write right required");
            }
            write_upload(request, &full)
                .map(|_| no_content())
                .unwrap_or_else(|error| bad_request(&error))
        }
        "DELETE" => {
            if rel.is_empty()
                || (claims.admin_rights & PLPERM_NPCCONTROL == 0
                    && !file_right(&claims, &rel, true))
            {
                return forbidden("folder write right required");
            }
            let result = match fs::metadata(&full) {
                Ok(info) if info.is_dir() => fs::remove_dir(&full),
                Ok(_) => fs::remove_file(&full),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return not_found(),
                Err(error) => Err(error),
            };
            result
                .map(|_| no_content())
                .unwrap_or_else(|error| bad_request(&error.to_string()))
        }
        _ => method_not_allowed(&["DELETE", "GET", "HEAD", "POST", "PUT"]),
    }
}

fn no_content() -> HttpResponse {
    HttpResponse {
        status: 204,
        headers: Vec::new(),
        body: Vec::new(),
    }
}

fn api_file_path(base: &Path, raw: &str) -> Option<(String, PathBuf)> {
    let mut value = raw.replace('\\', "/");
    while value.starts_with('/') {
        value.remove(0);
    }
    if value == "." {
        value.clear();
    }
    let mut components: Vec<&str> = Vec::new();
    for component in value.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            value => components.push(value),
        }
    }
    let rel = components.join("/");
    let base = if base.is_absolute() {
        base.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(base)
    };
    Some((rel.clone(), base.join(&rel)))
}
fn file_right(claims: &ApiTokenClaims, rel: &str, write: bool) -> bool {
    claims.folder_rights.iter().any(|entry| {
        let entry = entry.trim();
        let Some(index) = entry.find(char::is_whitespace) else {
            return false;
        };
        let rights = entry[..index].to_ascii_lowercase();
        let pattern = entry[index..].trim().replace('\\', "/");
        if !rights.contains(if write { 'w' } else { 'r' }) {
            return false;
        }
        wildcard_path(&pattern, rel)
            || (pattern.ends_with("/*")
                && rel.starts_with(pattern.strip_suffix('*').unwrap_or(&pattern)))
    })
}
fn wildcard_path(pattern: &str, value: &str) -> bool {
    let mut p = pattern.split('/');
    let mut v = value.split('/');
    loop {
        match (p.next(), v.next()) {
            (None, None) => return true,
            (Some(pattern), Some(value)) if wildcard_segment(pattern, value) => {}
            _ => return false,
        }
    }
}
fn wildcard_segment(pattern: &str, value: &str) -> bool {
    fn matches(
        pattern: &[u8],
        value: &[u8],
        p: usize,
        v: usize,
        memo: &mut HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(result) = memo.get(&(p, v)) {
            return *result;
        }
        let result = if p == pattern.len() {
            v == value.len()
        } else {
            match pattern[p] {
                b'*' => {
                    matches(pattern, value, p + 1, v, memo)
                        || (v < value.len() && matches(pattern, value, p, v + 1, memo))
                }
                b'?' => v < value.len() && matches(pattern, value, p + 1, v + 1, memo),
                b'\\' if p + 1 < pattern.len() => {
                    v < value.len()
                        && pattern[p + 1] == value[v]
                        && matches(pattern, value, p + 2, v + 1, memo)
                }
                b'[' => {
                    let mut index = p + 1;
                    let mut negate = false;
                    if index < pattern.len() && (pattern[index] == b'^' || pattern[index] == b'!') {
                        negate = true;
                        index += 1;
                    }
                    let start = index;
                    let mut matched = false;
                    while index < pattern.len() && pattern[index] != b']' {
                        let left = pattern[index];
                        if index + 2 < pattern.len()
                            && pattern[index + 1] == b'-'
                            && pattern[index + 2] != b']'
                        {
                            if v < value.len() && left <= value[v] && value[v] <= pattern[index + 2]
                            {
                                matched = true;
                            }
                            index += 3;
                        } else {
                            if v < value.len() && left == value[v] {
                                matched = true;
                            }
                            index += 1;
                        }
                    }
                    if index == start || index >= pattern.len() {
                        v < value.len()
                            && pattern[p] == value[v]
                            && matches(pattern, value, p + 1, v + 1, memo)
                    } else {
                        v < value.len()
                            && (if negate { !matched } else { matched })
                            && matches(pattern, value, index + 1, v + 1, memo)
                    }
                }
                literal => {
                    v < value.len()
                        && literal == value[v]
                        && matches(pattern, value, p + 1, v + 1, memo)
                }
            }
        };
        memo.insert((p, v), result);
        result
    }
    matches(
        pattern.as_bytes(),
        value.as_bytes(),
        0,
        0,
        &mut HashMap::new(),
    )
}

fn write_upload(request: &HttpRequest, full: &Path) -> Result<(), String> {
    const MAX_UPLOAD_SIZE: usize = 64 << 20;
    let data = if request
        .header("Content-Type")
        .to_ascii_lowercase()
        .starts_with("multipart/")
    {
        parse_multipart_upload(&request.body, &request.header("Content-Type"))?
    } else {
        request.body[..request.body.len().min(MAX_UPLOAD_SIZE)].to_vec()
    };
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(full, data).map_err(|error| error.to_string())
}

fn parse_multipart_upload(body: &[u8], content_type: &str) -> Result<Vec<u8>, String> {
    let boundary = content_type
        .split(';')
        .find_map(|part| part.trim().strip_prefix("boundary="))
        .map(|value| value.trim_matches('"').as_bytes().to_vec())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "multipart request requires a boundary".to_string())?;
    let marker = [b"--".as_slice(), boundary.as_slice()].concat();
    let mut cursor = 0usize;
    while let Some(offset) = find_bytes(&body[cursor..], &marker) {
        let start = cursor + offset + marker.len();
        if body.get(start..start + 2) == Some(b"--") {
            break;
        }
        let mut part_start = start;
        if body.get(part_start..part_start + 2) == Some(b"\r\n") {
            part_start += 2;
        }
        let Some(header_end) = find_bytes(&body[part_start..], b"\r\n\r\n") else {
            return Err("invalid multipart request".to_string());
        };
        let header_end = part_start + header_end;
        let headers = String::from_utf8_lossy(&body[part_start..header_end]);
        let Some(data_start) = header_end.checked_add(4) else {
            return Err("invalid multipart request".to_string());
        };
        let Some(data_end_rel) = find_bytes(
            &body[data_start..],
            &[b'\r', b'\n', b'-', b'-']
                .iter()
                .copied()
                .chain(boundary.iter().copied())
                .collect::<Vec<_>>(),
        ) else {
            return Err("invalid multipart request".to_string());
        };
        let data_end = data_start + data_end_rel;
        let is_upload = headers.lines().any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("content-disposition:")
                && (lower.contains("name=\"file\"") || lower.contains("name=\"upload\""))
        });
        if is_upload {
            return Ok(body[data_start..data_end].to_vec());
        }
        cursor = data_end + 2;
    }
    Err("multipart request requires a file field".to_string())
}

fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        if percent_decode(key).eq_ignore_ascii_case(name) {
            Some(percent_decode(value))
        } else {
            None
        }
    })
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            if let (Some(high), Some(low)) = (high, low) {
                output.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        output.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn format_system_time(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = duration.as_secs() as i64;
    let nanos = duration.subsec_nanos();
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_date_from_days(days);
    let base = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        day_seconds / 3_600,
        (day_seconds / 60) % 60,
        day_seconds % 60
    );
    if nanos == 0 {
        format!("{base}Z")
    } else {
        let mut fraction = format!("{nanos:09}");
        while fraction.ends_with('0') {
            fraction.pop();
        }
        format!("{base}.{fraction}Z")
    }
}

fn civil_date_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}

fn serve_asset(server: &Arc<Server>, request: &HttpRequest, asset: &str) -> HttpResponse {
    if !server.settings.get_bool("enableswagger", true) {
        return http_not_found();
    }
    if request.method != "GET" && request.method != "HEAD" {
        return method_not_allowed(&["GET"]);
    }
    let asset = asset.replace('\\', "/");
    if asset.contains("..") {
        return http_not_found();
    }
    let Some(data) = embedded_swagger_asset(&asset) else {
        return http_not_found();
    };
    let content_type = if asset.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if asset.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if asset.ends_with(".css") {
        "text/css; charset=utf-8"
    } else {
        "application/octet-stream"
    };
    let body = if request.method == "HEAD" {
        Vec::new()
    } else {
        data.to_vec()
    };
    let mut response = HttpResponse::new(200, content_type, body);
    response
        .headers
        .push(("Content-Length".to_string(), data.len().to_string()));
    response
}

fn embedded_swagger_asset(asset: &str) -> Option<&'static [u8]> {
    match asset {
        "SWAGGER-UI-LICENSE" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/SWAGGER-UI-LICENSE"
        ))),
        "SWAGGER-UI-NOTICE" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/SWAGGER-UI-NOTICE"
        ))),
        "favicon-16x16.png" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/favicon-16x16.png"
        ))),
        "favicon-32x32.png" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/favicon-32x32.png"
        ))),
        "index.css" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/index.css"
        ))),
        "index.html" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/index.html"
        ))),
        "index.js" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/index.js"
        ))),
        "oauth2-redirect.html" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/oauth2-redirect.html"
        ))),
        "oauth2-redirect.js" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/oauth2-redirect.js"
        ))),
        "swagger-ui-bundle.js" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/swagger-ui-bundle.js"
        ))),
        "swagger-ui-standalone-preset.js" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/swagger-ui-standalone-preset.js"
        ))),
        "swagger-ui.css" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/swagger/swagger-ui.css"
        ))),
        _ => None,
    }
}

fn openapi_spec() -> Value {
    let security = json!([{"BearerAuth": []}]);
    let file_read = json!({
        "security": security.clone(),
        "summary": "List or download server content",
        "responses": {"200": {"description": "File or directory entries"}}
    });
    let file_write = json!({
        "security": security.clone(),
        "summary": "Upload a file",
        "responses": {"204": {"description": "File written"}}
    });
    let file_post = json!({
        "security": security.clone(),
        "summary": "Upload a multipart file",
        "responses": {"204": {"description": "File written"}}
    });
    let file_delete = json!({
        "security": security.clone(),
        "summary": "Delete a file or directory",
        "responses": {"204": {"description": "File deleted"}}
    });
    let file_path_parameter = json!({
        "name": "path",
        "in": "path",
        "required": true,
        "schema": {"type": "string"}
    });
    json!({
        "openapi": "3.0.3",
        "info": {"title": "Preagonal GameServer API", "version": "v1"},
        "servers": [{"url": "/"}],
        "components": {
            "securitySchemes": {
                "BearerAuth": {"type": "http", "scheme": "bearer", "bearerFormat": "JWT"}
            },
            "schemas": {
                "LoginRequest": {
                    "type": "object",
                    "required": ["account", "password"],
                    "properties": {
                        "account": {"type": "string"},
                        "password": {"type": "string", "format": "password"}
                    }
                }
            }
        },
        "paths": {
            "/api/v1/login": {
                "post": {
                    "summary": "Authenticate a staff account",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {"$ref": "#/components/schemas/LoginRequest"}
                            }
                        }
                    },
                    "responses": {
                        "200": {"description": "JWT bearer token"},
                        "401": {"description": "Authentication failed"}
                    }
                }
            },
            "/api/v1/stats": {
                "get": {
                    "summary": "Get server statistics",
                    "responses": {"200": {"description": "Current level and player counts"}}
                }
            },
            "/api/v1/scripts/definitions": {
                "get": {
                    "security": security.clone(),
                    "summary": "List loaded script definitions",
                    "responses": {"200": {"description": "Loaded weapons, classes, and NPCs"}}
                }
            },
            "/api/v1/scripts/stats": {
                "get": {
                    "security": security.clone(),
                    "summary": "Get loaded script counts",
                    "responses": {"200": {"description": "Loaded script counts"}}
                }
            },
            "/api/v1/files": {
                "get": file_read.clone(),
                "put": file_write.clone(),
                "post": file_post.clone(),
                "delete": file_delete.clone()
            },
            "/api/v1/files/{path}": {
                "parameters": [file_path_parameter],
                "get": file_read,
                "put": file_write,
                "post": file_post,
                "delete": file_delete
            }
        }
    })
}
fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        307 => "Temporary Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    }
}
fn find_bytes(value: &[u8], needle: &[u8]) -> Option<usize> {
    value
        .windows(needle.len())
        .position(|window| window == needle)
}
