//! Server-side GS2 execution and the typed host boundary.
//!
//! The original implementation used Goja as the execution engine.  The Rust
//! implementation keeps the same boundary types and result ordering, while
//! using a small ownership-safe interpreter for the server-side GS2 surface.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::raw::{c_char, c_int, c_long, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use base64::Engine;
use chrono::{Datelike, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::compiler::translate_server_script;
use crate::tiletypes::{TILE_TYPES0, TILE_TYPES1};

pub type Any = Value;
pub type AnyMap = HashMap<String, Any>;

pub type ImportResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
pub type ServerPlayerResolver = Arc<dyn Fn(&str) -> Option<PlayerContext> + Send + Sync>;
pub type SocketClassResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
pub type TileTypeResolver = Arc<dyn Fn(&str, i32, i32) -> i32 + Send + Sync>;
pub type MapPositionResolver = Arc<dyn Fn(&str) -> Option<(i32, i32)> + Send + Sync>;
pub type SocketBindResolver =
    Arc<dyn Fn(SocketAction) -> std::result::Result<SocketContext, String> + Send + Sync>;

/// Open the database for every SQL request while using the platform SQLite ABI
/// directly. This preserves SQLite semantics and the `file_root/databases`
/// persistence contract without introducing a second
/// SQL parser or a process-wide connection cache.
mod sqlite_ffi {
    use super::*;
    use std::slice;

    const SQLITE_OK: c_int = 0;
    const SQLITE_ROW: c_int = 100;
    const SQLITE_DONE: c_int = 101;
    const SQLITE_INTEGER: c_int = 1;
    const SQLITE_FLOAT: c_int = 2;
    const SQLITE_TEXT: c_int = 3;
    const SQLITE_BLOB: c_int = 4;

    #[repr(C)]
    struct Sqlite3 {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct Sqlite3Stmt {
        _private: [u8; 0],
    }

    unsafe extern "C" {
        fn sqlite3_open(filename: *const c_char, database: *mut *mut Sqlite3) -> c_int;
        fn sqlite3_close(database: *mut Sqlite3) -> c_int;
        fn sqlite3_errmsg(database: *mut Sqlite3) -> *const c_char;
        fn sqlite3_exec(
            database: *mut Sqlite3,
            sql: *const c_char,
            callback: Option<
                unsafe extern "C" fn(
                    *mut c_void,
                    c_int,
                    *mut *mut c_char,
                    *mut *mut c_char,
                ) -> c_int,
            >,
            callback_arg: *mut c_void,
            error: *mut *mut c_char,
        ) -> c_int;
        fn sqlite3_free(value: *mut c_void);
        fn sqlite3_prepare_v2(
            database: *mut Sqlite3,
            sql: *const c_char,
            length: c_int,
            statement: *mut *mut Sqlite3Stmt,
            tail: *mut *const c_char,
        ) -> c_int;
        fn sqlite3_step(statement: *mut Sqlite3Stmt) -> c_int;
        fn sqlite3_finalize(statement: *mut Sqlite3Stmt) -> c_int;
        fn sqlite3_column_count(statement: *mut Sqlite3Stmt) -> c_int;
        fn sqlite3_column_name(statement: *mut Sqlite3Stmt, column: c_int) -> *const c_char;
        fn sqlite3_column_type(statement: *mut Sqlite3Stmt, column: c_int) -> c_int;
        fn sqlite3_column_int64(statement: *mut Sqlite3Stmt, column: c_int) -> i64;
        fn sqlite3_column_double(statement: *mut Sqlite3Stmt, column: c_int) -> f64;
        fn sqlite3_column_text(statement: *mut Sqlite3Stmt, column: c_int) -> *const u8;
        fn sqlite3_column_bytes(statement: *mut Sqlite3Stmt, column: c_int) -> c_int;
        fn sqlite3_last_insert_rowid(database: *mut Sqlite3) -> i64;
    }

    struct Connection {
        raw: *mut Sqlite3,
    }

    impl Drop for Connection {
        fn drop(&mut self) {
            if !self.raw.is_null() {
                // The connection is confined to the VM request thread and
                // all statements are finalized before this destructor runs.
                unsafe {
                    let _ = sqlite3_close(self.raw);
                }
            }
        }
    }

    impl Connection {
        fn open(path: &Path) -> std::result::Result<Self, String> {
            let path = CString::new(path.to_string_lossy().as_bytes())
                .map_err(|_| "invalid database path".to_string())?;
            let mut raw = ptr::null_mut();
            let code = unsafe { sqlite3_open(path.as_ptr(), &mut raw) };
            if code != SQLITE_OK {
                let message = if raw.is_null() {
                    format!("sqlite error {code}")
                } else {
                    error_message(raw)
                };
                if !raw.is_null() {
                    unsafe {
                        let _ = sqlite3_close(raw);
                    }
                }
                return Err(message);
            }
            Ok(Self { raw })
        }

        fn execute(&mut self, sql: &str) -> std::result::Result<i64, String> {
            let sql = CString::new(sql).map_err(|_| "query contains NUL".to_string())?;
            let mut error = ptr::null_mut();
            let code =
                unsafe { sqlite3_exec(self.raw, sql.as_ptr(), None, ptr::null_mut(), &mut error) };
            if code != SQLITE_OK {
                let message = if !error.is_null() {
                    unsafe { CStr::from_ptr(error).to_string_lossy().into_owned() }
                } else {
                    error_message(self.raw)
                };
                if !error.is_null() {
                    unsafe { sqlite3_free(error.cast()) };
                }
                return Err(message);
            }
            Ok(unsafe { sqlite3_last_insert_rowid(self.raw) })
        }

        fn query(
            &mut self,
            sql: &str,
        ) -> std::result::Result<Vec<Vec<(String, DynValue)>>, String> {
            let sql = CString::new(sql).map_err(|_| "query contains NUL".to_string())?;
            let mut statement = ptr::null_mut();
            let code = unsafe {
                sqlite3_prepare_v2(self.raw, sql.as_ptr(), -1, &mut statement, ptr::null_mut())
            };
            if code != SQLITE_OK {
                return Err(error_message(self.raw));
            }
            let column_count = unsafe { sqlite3_column_count(statement) };
            let columns = (0..column_count)
                .map(|index| unsafe {
                    CStr::from_ptr(sqlite3_column_name(statement, index))
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>();
            let mut rows = Vec::new();
            loop {
                let step = unsafe { sqlite3_step(statement) };
                if step == SQLITE_DONE {
                    break;
                }
                if step != SQLITE_ROW {
                    let message = error_message(self.raw);
                    unsafe {
                        let _ = sqlite3_finalize(statement);
                    }
                    return Err(message);
                }
                let mut row = Vec::with_capacity(column_count as usize);
                for index in 0..column_count {
                    row.push((columns[index as usize].clone(), unsafe {
                        column_value(statement, index)
                    }));
                }
                rows.push(row);
            }
            unsafe {
                let _ = sqlite3_finalize(statement);
            }
            Ok(rows)
        }
    }

    fn error_message(database: *mut Sqlite3) -> String {
        if database.is_null() {
            return "sqlite error".to_string();
        }
        unsafe {
            CStr::from_ptr(sqlite3_errmsg(database))
                .to_string_lossy()
                .into_owned()
        }
    }

    unsafe fn column_value(statement: *mut Sqlite3Stmt, column: c_int) -> DynValue {
        match unsafe { sqlite3_column_type(statement, column) } {
            SQLITE_INTEGER => {
                DynValue::Number(unsafe { sqlite3_column_int64(statement, column) } as f64)
            }
            SQLITE_FLOAT => DynValue::Number(unsafe { sqlite3_column_double(statement, column) }),
            SQLITE_TEXT | SQLITE_BLOB => {
                let pointer = unsafe { sqlite3_column_text(statement, column) };
                let length = unsafe { sqlite3_column_bytes(statement, column) }.max(0) as usize;
                if pointer.is_null() {
                    DynValue::String(String::new())
                } else {
                    DynValue::String(
                        String::from_utf8_lossy(unsafe { slice::from_raw_parts(pointer, length) })
                            .into_owned(),
                    )
                }
            }
            _ => DynValue::Null,
        }
    }

    pub(super) fn execute(path: &Path, sql: &str) -> std::result::Result<i64, String> {
        let mut connection = Connection::open(path)?;
        connection.execute(sql)
    }

    pub(super) fn query(
        path: &Path,
        sql: &str,
    ) -> std::result::Result<Vec<Vec<(String, DynValue)>>, String> {
        let mut connection = Connection::open(path)?;
        connection.query(sql)
    }

    pub(super) fn touch(path: &Path) -> std::result::Result<(), String> {
        let _connection = Connection::open(path)?;
        Ok(())
    }
}

/// A small libcurl ABI adapter for the HTTPS side of TCURLRequest. Using the
/// host's libcurl keeps TLS, redirects, and response framing in one native
/// transport layer without making the VM depend on an async
/// runtime or a second HTTP parser.
mod curl_ffi {
    use super::*;
    use std::slice;

    const CURLE_OK: c_int = 0;
    const CURL_GLOBAL_DEFAULT: c_long = 3;

    const CURLOPT_WRITEDATA: c_int = 10_001;
    const CURLOPT_URL: c_int = 10_002;
    const CURLOPT_POST: c_int = 47;
    const CURLOPT_POSTFIELDS: c_int = 10_015;
    const CURLOPT_USERAGENT: c_int = 10_018;
    const CURLOPT_HTTPHEADER: c_int = 10_023;
    const CURLOPT_WRITEFUNCTION: c_int = 20_011;
    const CURLOPT_TIMEOUT: c_int = 13;
    const CURLOPT_FOLLOWLOCATION: c_int = 52;
    const CURLOPT_MAXREDIRS: c_int = 68;
    const CURLOPT_SSL_VERIFYPEER: c_int = 64;
    const CURLOPT_SSL_VERIFYHOST: c_int = 81;
    const CURLOPT_HEADERFUNCTION: c_int = 20_079;
    const CURLOPT_HEADERDATA: c_int = 10_029;
    const CURLOPT_POSTFIELDSIZE: c_int = 60;
    const CURLOPT_ACCEPT_ENCODING: c_int = 10_102;

    const CURLINFO_RESPONSE_CODE: c_int = 0x200002;

    #[repr(C)]
    struct CurlHandle {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct CurlSlist {
        data: *mut c_char,
        next: *mut CurlSlist,
    }

    unsafe extern "C" {
        fn curl_global_init(flags: c_long) -> c_int;
        fn curl_easy_init() -> *mut CurlHandle;
        fn curl_easy_cleanup(handle: *mut CurlHandle);
        fn curl_easy_perform(handle: *mut CurlHandle) -> c_int;
        fn curl_easy_getinfo(handle: *mut CurlHandle, info: c_int, ...) -> c_int;
        fn curl_easy_setopt(handle: *mut CurlHandle, option: c_int, ...) -> c_int;
        fn curl_easy_strerror(code: c_int) -> *const c_char;
        fn curl_slist_append(list: *mut CurlSlist, data: *const c_char) -> *mut CurlSlist;
        fn curl_slist_free_all(list: *mut CurlSlist);
    }

    struct EasyHandle(*mut CurlHandle);

    impl Drop for EasyHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { curl_easy_cleanup(self.0) };
            }
        }
    }

    struct HeaderList(*mut CurlSlist);

    impl Drop for HeaderList {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { curl_slist_free_all(self.0) };
            }
        }
    }

    #[derive(Default)]
    struct ResponseData {
        body: Vec<u8>,
        headers: HashMap<String, String>,
        status_code: u16,
        status: String,
    }

    unsafe extern "C" fn write_callback(
        data: *mut c_char,
        size: usize,
        count: usize,
        userdata: *mut c_void,
    ) -> usize {
        let length = size.saturating_mul(count);
        if userdata.is_null() || (data.is_null() && length != 0) {
            return 0;
        }
        let bytes = unsafe { slice::from_raw_parts(data.cast::<u8>(), length) };
        let response = unsafe { &mut *userdata.cast::<ResponseData>() };
        response.body.extend_from_slice(bytes);
        length
    }

    unsafe extern "C" fn header_callback(
        data: *mut c_char,
        size: usize,
        count: usize,
        userdata: *mut c_void,
    ) -> usize {
        let length = size.saturating_mul(count);
        if userdata.is_null() || (data.is_null() && length != 0) {
            return 0;
        }
        let bytes = unsafe { slice::from_raw_parts(data.cast::<u8>(), length) };
        let line = String::from_utf8_lossy(bytes)
            .trim_matches(|value: char| value == '\r' || value == '\n')
            .to_string();
        let response = unsafe { &mut *userdata.cast::<ResponseData>() };
        if line
            .as_bytes()
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"HTTP/"))
        {
            let mut parts = line.splitn(3, ' ');
            let _version = parts.next();
            if let Some(code) = parts.next().and_then(|value| value.parse::<u16>().ok()) {
                let reason = parts.next().unwrap_or_default().trim();
                response.status_code = code;
                response.status = if reason.is_empty() {
                    code.to_string()
                } else {
                    format!("{code} {reason}")
                };
                // Header callbacks are also emitted for each redirect and
                // informational response.  Only the final response should
                // be visible to the script.
                response.headers.clear();
            }
        } else if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if !key.is_empty() {
                response
                    .headers
                    .insert(key.to_ascii_lowercase(), value.trim().to_string());
            }
        }
        length
    }

    fn error_message(code: c_int) -> String {
        if code == CURLE_OK {
            return String::new();
        }
        unsafe { CStr::from_ptr(curl_easy_strerror(code)) }
            .to_string_lossy()
            .into_owned()
    }

    fn option_result(code: c_int) -> std::result::Result<(), String> {
        if code == CURLE_OK {
            Ok(())
        } else {
            Err(error_message(code))
        }
    }

    pub(super) fn request(
        request_url: &str,
        method: &str,
        body: &[u8],
        headers: &[String],
        insecure_tls: bool,
    ) -> std::result::Result<(u16, String, HashMap<String, String>, Vec<u8>), String> {
        // libcurl's global initialization is process-wide and idempotent, so
        // Keeping it alive for the process avoids a cleanup race with worker
        // threads.
        static INITIALIZED: std::sync::Once = std::sync::Once::new();
        INITIALIZED.call_once(|| unsafe {
            let _ = curl_global_init(CURL_GLOBAL_DEFAULT);
        });

        let url = CString::new(request_url).map_err(|_| "URL contains NUL".to_string())?;
        let user_agent =
            CString::new("Go-http-client/1.1").expect("static user-agent cannot contain NUL");
        let handle = EasyHandle(unsafe { curl_easy_init() });
        if handle.0.is_null() {
            return Err("could not initialize libcurl".to_string());
        }
        let mut response = ResponseData::default();
        let mut header_list = HeaderList(ptr::null_mut());
        let mut header_values = Vec::new();
        for header in headers {
            let Some((key, value)) = header.split_once(':') else {
                continue;
            };
            let key = key.trim();
            if key.is_empty()
                || key.eq_ignore_ascii_case("host")
                || key.eq_ignore_ascii_case("connection")
            {
                continue;
            }
            let line = CString::new(format!("{key}: {}", value.trim()))
                .map_err(|_| "header contains NUL".to_string())?;
            let next = unsafe { curl_slist_append(header_list.0, line.as_ptr()) };
            if next.is_null() {
                return Err("could not allocate HTTP header list".to_string());
            }
            header_list.0 = next;
            header_values.push(line);
        }

        option_result(unsafe { curl_easy_setopt(handle.0, CURLOPT_URL, url.as_ptr()) })?;
        option_result(unsafe {
            curl_easy_setopt(handle.0, CURLOPT_USERAGENT, user_agent.as_ptr())
        })?;
        option_result(unsafe {
            curl_easy_setopt(
                handle.0,
                CURLOPT_WRITEFUNCTION,
                write_callback
                    as unsafe extern "C" fn(*mut c_char, usize, usize, *mut c_void) -> usize,
            )
        })?;
        option_result(unsafe {
            curl_easy_setopt(
                handle.0,
                CURLOPT_WRITEDATA,
                (&mut response as *mut ResponseData).cast::<c_void>(),
            )
        })?;
        option_result(unsafe {
            curl_easy_setopt(
                handle.0,
                CURLOPT_HEADERFUNCTION,
                header_callback
                    as unsafe extern "C" fn(*mut c_char, usize, usize, *mut c_void) -> usize,
            )
        })?;
        option_result(unsafe {
            curl_easy_setopt(
                handle.0,
                CURLOPT_HEADERDATA,
                (&mut response as *mut ResponseData).cast::<c_void>(),
            )
        })?;
        option_result(unsafe { curl_easy_setopt(handle.0, CURLOPT_TIMEOUT, 30 as c_long) })?;
        option_result(unsafe { curl_easy_setopt(handle.0, CURLOPT_FOLLOWLOCATION, 1 as c_long) })?;
        option_result(unsafe { curl_easy_setopt(handle.0, CURLOPT_MAXREDIRS, 10 as c_long) })?;
        option_result(unsafe {
            curl_easy_setopt(
                handle.0,
                CURLOPT_SSL_VERIFYPEER,
                if insecure_tls {
                    0 as c_long
                } else {
                    1 as c_long
                },
            )
        })?;
        option_result(unsafe {
            curl_easy_setopt(
                handle.0,
                CURLOPT_SSL_VERIFYHOST,
                if insecure_tls {
                    0 as c_long
                } else {
                    2 as c_long
                },
            )
        })?;
        // An empty encoding list asks curl to advertise and transparently
        // decode every compression format compiled into the host library,
        // matching net/http's automatic response decompression.
        let accept_encoding = CString::new("").expect("static encoding value");
        option_result(unsafe {
            curl_easy_setopt(handle.0, CURLOPT_ACCEPT_ENCODING, accept_encoding.as_ptr())
        })?;
        if !header_list.0.is_null() {
            option_result(unsafe {
                curl_easy_setopt(handle.0, CURLOPT_HTTPHEADER, header_list.0)
            })?;
        }
        if method.eq_ignore_ascii_case("POST") {
            option_result(unsafe { curl_easy_setopt(handle.0, CURLOPT_POST, 1 as c_long) })?;
            option_result(unsafe {
                curl_easy_setopt(handle.0, CURLOPT_POSTFIELDS, body.as_ptr().cast::<c_char>())
            })?;
            option_result(unsafe {
                curl_easy_setopt(handle.0, CURLOPT_POSTFIELDSIZE, body.len() as c_long)
            })?;
        }
        let code = unsafe { curl_easy_perform(handle.0) };
        option_result(code)?;
        let mut response_code = 0 as c_long;
        let info_code =
            unsafe { curl_easy_getinfo(handle.0, CURLINFO_RESPONSE_CODE, &mut response_code) };
        option_result(info_code)?;
        if response_code > 0 {
            response.status_code = response_code as u16;
            if response.status.is_empty() {
                response.status = response.status_code.to_string();
            }
        }
        if response.status.is_empty() && response.status_code > 0 {
            response.status = response.status_code.to_string();
        }
        Ok((
            response.status_code,
            response.status,
            response.headers,
            response.body,
        ))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SocketContext {
    pub name: String,
    pub id: String,
    pub address: String,
    pub ip_address: String,
    pub port: i32,
    pub package_delimiter: String,
    pub data: String,
    pub buffer: String,
    pub is_connected: bool,
    pub state: AnyMap,
    pub joined_classes: Vec<String>,
    pub parent_name: String,
    pub parent_id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlayerContext {
    pub id: u16,
    pub account: String,
    pub nick: String,
    pub nickname: String,
    pub guild: String,
    pub level: String,
    pub dir: i32,
    pub x: f64,
    pub y: f64,
    pub online_time: i32,
    pub admin_level: i32,
    pub flags: HashMap<String, String>,
    pub rights: Vec<String>,
    pub folders: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WeaponContext {
    pub name: String,
    pub image: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ServerContext {
    pub name: String,
    pub r#type: String,
    pub player_count: i32,
    pub language: String,
    pub description: String,
    pub url: String,
    pub version: String,
    pub game_versions: String,
    pub latency: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NPCContext {
    pub id: u32,
    pub name: String,
    pub script: String,
    pub level: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub this: AnyMap,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SignContext {
    pub level: String,
    pub x: i32,
    pub y: i32,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ChestContext {
    pub level: String,
    pub x: i32,
    pub y: i32,
    pub item_type: i32,
    pub is_open: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlayerFlag {
    pub account: String,
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlayerProp {
    pub account: String,
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlayerEffect {
    pub account: String,
    pub action: String,
    pub value: String,
    pub amount: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NPCFlag {
    pub id: u32,
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NPCFunctionCall {
    pub id: u32,
    pub name: String,
    pub function: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ServerFlag {
    pub name: String,
    pub value: String,
    pub deleted: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlayerMessage {
    pub account: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct IRCMessage {
    pub account: String,
    pub command: String,
    pub params: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlayerWeapon {
    pub account: String,
    pub name: String,
    pub add: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlayerClass {
    pub account: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlayerWarp {
    pub account: String,
    pub level: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlayerAttachment {
    pub account: String,
    pub object_id: u32,
    pub offset_x: f64,
    pub offset_y: f64,
    pub detached: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FileAction {
    pub action: String,
    pub name: String,
    pub data: String,
    pub ok: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NPCAction {
    pub id: u32,
    pub action: String,
    pub shape_type: i32,
    pub width: i32,
    pub height: i32,
    pub tile_types: Vec<String>,
    pub chat: String,
    pub warp_level: String,
    pub warp_x: f64,
    pub warp_y: f64,
    pub move_dx: f64,
    pub move_dy: f64,
    pub move_time: f64,
    pub move_options: i32,
    pub image: String,
    pub image_part: Vec<i32>,
    pub ani: String,
    pub ani_params: Vec<String>,
    pub props: HashMap<String, String>,
    pub flags: HashMap<String, String>,
    pub save_props: AnyMap,
    pub save: bool,
    pub vis_flags: i32,
    pub has_vis_flags: bool,
    pub block_flags: i32,
    pub has_block_flags: bool,
    pub destroy: bool,
    pub has_chat: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LevelAction {
    pub action: String,
    pub level: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub width: f64,
    pub height: f64,
    pub power: i32,
    pub tile: i32,
    pub layer: i32,
    pub angle: f64,
    pub z_angle: f64,
    pub strength: f64,
    pub delta_x: f64,
    pub delta_y: f64,
    pub speed: f64,
    pub ani: String,
    pub params: Vec<String>,
    pub value: String,
    pub target: String,
    pub set_npc_id: u32,
    pub set_player: String,
    pub image: String,
    pub update: bool,
    pub save: bool,
    pub script: String,
    pub classes: Vec<String>,
    pub props: HashMap<String, String>,
    pub calls: Vec<NPCFunctionCall>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SocketAction {
    pub action: String,
    pub name: String,
    pub id: String,
    pub address: String,
    pub port: i32,
    pub data: String,
    pub package_delimiter: String,
    pub udp: bool,
    pub prepared: bool,
    pub state: AnyMap,
    pub joined_classes: Vec<String>,
    pub event: String,
    pub params: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ScheduledEvent {
    pub event: String,
    pub delay: f64,
    pub params: Vec<String>,
    pub canceled: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WaitEvent {
    pub object: String,
    pub event: String,
    pub timeout: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ClientTrigger {
    pub kind: String,
    pub name: String,
    pub args: Vec<String>,
}

/// Destination for client triggers collected by a VM run.
///
/// This is the Rust equivalent of HexaVM's transport.ClientTriggerSink. The
/// trigger is passed by value so a sink cannot retain an alias to the VM's
/// mutable result buffers.
pub trait ClientTriggerSink {
    type Error;

    fn send_client_trigger(
        &mut self,
        trigger: ClientTrigger,
    ) -> std::result::Result<(), Self::Error>;
}

/// Small adapter for the convenient ClientTriggerSink function type.
pub struct ClientTriggerSinkFunc<F>(pub F);

impl<F, E> ClientTriggerSink for ClientTriggerSinkFunc<F>
where
    F: FnMut(ClientTrigger) -> std::result::Result<(), E>,
{
    type Error = E;

    fn send_client_trigger(
        &mut self,
        trigger: ClientTrigger,
    ) -> std::result::Result<(), Self::Error> {
        (self.0)(trigger)
    }
}

impl<S> ClientTriggerSink for Option<S>
where
    S: ClientTriggerSink,
{
    type Error = S::Error;

    fn send_client_trigger(
        &mut self,
        trigger: ClientTrigger,
    ) -> std::result::Result<(), Self::Error> {
        if let Some(sink) = self.as_mut() {
            sink.send_client_trigger(trigger)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Default)]
pub struct VMConfig {
    pub script_name: String,
    pub event_name: String,
    pub script: String,
    pub imports: HashMap<String, String>,
    pub import_resolver: Option<ImportResolver>,
    pub params: Vec<String>,
    pub player: HashMap<String, String>,
    pub player_flags: HashMap<String, String>,
    pub players: Vec<PlayerContext>,
    pub server_player_resolver: Option<ServerPlayerResolver>,
    pub weapons: Vec<WeaponContext>,
    pub servers: Vec<ServerContext>,
    pub npcs: Vec<NPCContext>,
    pub signs: Vec<SignContext>,
    pub chests: Vec<ChestContext>,
    pub npc_id: u32,
    pub this: AnyMap,
    pub server_flags: HashMap<String, String>,
    pub server_options: HashMap<String, String>,
    pub file_root: String,
    pub file_rights: Vec<String>,
    pub socket: Option<SocketContext>,
    pub socket_argument: Option<SocketContext>,
    pub socket_class_resolver: Option<SocketClassResolver>,
    pub socket_bind: Option<SocketBindResolver>,
    pub tile_type: Option<TileTypeResolver>,
    pub map_position: Option<MapPositionResolver>,
    pub tile_layout: i32,
    pub skip_top_level: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct VMResult {
    pub output: Vec<String>,
    pub client_triggers: Vec<ClientTrigger>,
    pub player_flags: Vec<PlayerFlag>,
    pub player_props: Vec<PlayerProp>,
    pub player_effects: Vec<PlayerEffect>,
    pub server_flags: Vec<ServerFlag>,
    pub player_messages: Vec<PlayerMessage>,
    pub player_rc_messages: Vec<PlayerMessage>,
    pub player_irc_messages: Vec<IRCMessage>,
    pub rc_messages: Vec<String>,
    pub nc_messages: Vec<String>,
    pub player_weapons: Vec<PlayerWeapon>,
    pub player_classes: Vec<PlayerClass>,
    pub player_warps: Vec<PlayerWarp>,
    pub player_attachments: Vec<PlayerAttachment>,
    pub file_actions: Vec<FileAction>,
    pub npc_flags: Vec<NPCFlag>,
    pub npc_function_calls: Vec<NPCFunctionCall>,
    pub npc_actions: Vec<NPCAction>,
    pub level_actions: Vec<LevelAction>,
    pub socket_actions: Vec<SocketAction>,
    pub socket_updates: Vec<SocketContext>,
    pub scheduled_events: Vec<ScheduledEvent>,
    pub wait_events: Vec<WaitEvent>,
    pub this: AnyMap,
    pub err: String,
}

impl VMResult {
    /// Dispatch every collected client trigger in order.
    ///
    /// Each trigger is cloned before dispatch so the argument slice has
    /// copy-on-send behavior.
    pub fn send_client_triggers<S: ClientTriggerSink>(
        &self,
        sink: &mut S,
    ) -> std::result::Result<(), S::Error> {
        for trigger in &self.client_triggers {
            sink.send_client_trigger(trigger.clone())?;
        }
        Ok(())
    }

    #[allow(non_snake_case)]
    pub fn SendClientTriggers<S: ClientTriggerSink>(
        &self,
        sink: &mut S,
    ) -> std::result::Result<(), S::Error> {
        self.send_client_triggers(sink)
    }
}

/// Internal VM result used by the adapter as a stable alias.
pub type Result = VMResult;

#[derive(Clone)]
enum Expr {
    Value(DynValue),
    Variable(String),
    Member(Box<Expr>, String),
    DynamicMember(Box<Expr>, Box<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    New(String, Vec<Expr>),
    Array(Vec<Expr>),
    Object(Vec<(Expr, Expr)>),
    Unary(String, Box<Expr>, bool),
    Binary(Box<Expr>, String, Box<Expr>),
    Assign(Box<Expr>, String, Box<Expr>),
    Delete(Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Function(Vec<String>, Vec<Stmt>),
}

#[derive(Clone)]
enum Stmt {
    Empty,
    Expr(Expr),
    Return(Option<Expr>),
    Block(Vec<Stmt>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For(Option<Expr>, Option<Expr>, Option<Expr>, Box<Stmt>),
    ForEach(Expr, Expr, Box<Stmt>),
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
    Break,
    Continue,
}

#[derive(Clone)]
struct ScriptFunction {
    name: String,
    args: Vec<String>,
    body: Vec<Stmt>,
    public: bool,
}

#[derive(Default)]
struct ParsedProgram {
    functions: Vec<ScriptFunction>,
    top_level: Vec<Stmt>,
    constants: HashMap<String, DynValue>,
}

#[derive(Clone, Debug)]
enum Token {
    Identifier(String),
    Number(String),
    String(String),
    Symbol(String),
    Eof,
}

struct Lexer {
    source: Vec<char>,
    position: usize,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Self {
            source: source.chars().collect(),
            position: 0,
        }
    }

    fn tokens(mut self) -> Vec<Token> {
        let mut result = Vec::new();
        loop {
            let token = self.next();
            let end = matches!(token, Token::Eof);
            result.push(token);
            if end {
                break;
            }
        }
        result
    }

    fn next(&mut self) -> Token {
        while self.position < self.source.len() && self.source[self.position].is_whitespace() {
            self.position += 1;
        }
        if self.position >= self.source.len() {
            return Token::Eof;
        }
        let ch = self.source[self.position];
        if ch.is_ascii_alphabetic() || ch == '_' || ch == '$' {
            let start = self.position;
            self.position += 1;
            while self.position < self.source.len()
                && (self.source[self.position].is_ascii_alphanumeric()
                    || matches!(self.source[self.position], '_' | '$'))
            {
                self.position += 1;
            }
            return Token::Identifier(self.source[start..self.position].iter().collect());
        }
        if ch.is_ascii_digit()
            || ch == '.'
                && self.position + 1 < self.source.len()
                && self.source[self.position + 1].is_ascii_digit()
        {
            let start = self.position;
            self.position += 1;
            while self.position < self.source.len()
                && (self.source[self.position].is_ascii_digit()
                    || matches!(self.source[self.position], '.' | 'e' | 'E' | '+' | '-'))
            {
                if matches!(self.source[self.position], '+' | '-')
                    && self.position > start
                    && !matches!(self.source[self.position - 1], 'e' | 'E')
                {
                    break;
                }
                self.position += 1;
            }
            return Token::Number(self.source[start..self.position].iter().collect());
        }
        if matches!(ch, '"' | '\'') {
            let quote = ch;
            self.position += 1;
            let mut value = String::new();
            let mut escaped = false;
            while self.position < self.source.len() {
                let current = self.source[self.position];
                self.position += 1;
                if escaped {
                    value.push(match current {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        other => other,
                    });
                    escaped = false;
                } else if current == '\\' {
                    escaped = true;
                } else if current == quote {
                    break;
                } else {
                    value.push(current);
                }
            }
            return Token::String(value);
        }
        for operator in [
            "===", "!==", ">>>", "<<=", ">>=", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=",
            "*=", "/=", "%=", "@=", "++", "--", "<<", ">>", "=>",
        ] {
            let end = self.position + operator.chars().count();
            if end <= self.source.len()
                && self.source[self.position..end].iter().collect::<String>() == operator
            {
                self.position = end;
                return Token::Symbol(operator.to_string());
            }
        }
        self.position += 1;
        Token::Symbol(ch.to_string())
    }
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

fn value_to_constant(expression: &Expr) -> DynValue {
    match expression {
        Expr::Value(value) => value.clone(),
        _ => DynValue::Undefined,
    }
}

impl Parser {
    fn new(source: &str) -> Self {
        let stripped = strip_comments(source);
        Self {
            tokens: Lexer::new(&stripped).tokens(),
            position: 0,
        }
    }

    fn parse(mut self) -> std::result::Result<ParsedProgram, String> {
        let mut program = ParsedProgram::default();
        while !self.is_eof() {
            if self.check_identifier("import") {
                self.skip_until(";");
                continue;
            }
            if self.check_identifier("const") {
                self.next();
                let name = self.expect_simple_name()?;
                if self.consume_symbol("=") {
                    let value = self.parse_expression()?;
                    program
                        .constants
                        .insert(name.to_ascii_lowercase(), value_to_constant(&value));
                }
                self.consume_symbol(";");
                continue;
            }
            if self.check_identifier("enum") {
                self.next();
                self.expect_symbol("{")?;
                let mut value = 0.0;
                while !self.check_symbol("}") && !self.is_eof() {
                    let name = self.expect_simple_name()?;
                    program
                        .constants
                        .insert(name.to_ascii_lowercase(), DynValue::Number(value));
                    value += 1.0;
                    if !self.consume_symbol(",") {
                        break;
                    }
                }
                self.expect_symbol("}")?;
                self.consume_symbol(";");
                continue;
            }
            let public = if self.check_identifier("public") {
                self.next();
                true
            } else {
                if self.check_identifier("private") {
                    self.next();
                }
                false
            };
            if self.check_identifier("function") {
                program.functions.push(self.parse_function(public)?);
            } else {
                if public {
                    continue;
                }
                program.top_level.push(self.parse_statement()?);
            }
        }
        Ok(program)
    }

    fn parse_function(&mut self, public: bool) -> std::result::Result<ScriptFunction, String> {
        self.expect_identifier("function")?;
        let name = self.expect_name()?;
        self.expect_symbol("(")?;
        let mut args = Vec::new();
        while !self.check_symbol(")") && !self.is_eof() {
            args.push(self.expect_name()?);
            if !self.consume_symbol(",") {
                break;
            }
        }
        self.expect_symbol(")")?;
        let body = if self.check_symbol("{") {
            self.parse_block()?
        } else {
            // Legacy GS2 also permits a compact function body without braces,
            // with comma-separated statements terminated by one semicolon.
            let mut body = Vec::new();
            while !self.is_eof() {
                body.push(self.parse_statement()?);
                if self.consume_symbol(",") {
                    continue;
                }
                self.consume_symbol(";");
                break;
            }
            body
        };
        Ok(ScriptFunction {
            name,
            args,
            body,
            public,
        })
    }

    fn parse_block(&mut self) -> std::result::Result<Vec<Stmt>, String> {
        self.expect_symbol("{")?;
        let mut body = Vec::new();
        while !self.check_symbol("}") && !self.is_eof() {
            body.push(self.parse_statement()?);
        }
        self.expect_symbol("}")?;
        Ok(body)
    }

    fn parse_statement(&mut self) -> std::result::Result<Stmt, String> {
        if self.consume_symbol(";") {
            return Ok(Stmt::Empty);
        }
        if self.check_symbol("{") {
            return Ok(Stmt::Block(self.parse_block()?));
        }
        if self.check_identifier("if") {
            self.next();
            self.expect_symbol("(")?;
            let condition = self.parse_expression()?;
            self.expect_symbol(")")?;
            let then_body = Box::new(self.parse_statement()?);
            let else_body = if self.check_identifier("else") {
                self.next();
                Some(Box::new(self.parse_statement()?))
            } else {
                None
            };
            return Ok(Stmt::If(condition, then_body, else_body));
        }
        if self.check_identifier("while") {
            self.next();
            self.expect_symbol("(")?;
            let condition = self.parse_expression()?;
            self.expect_symbol(")")?;
            return Ok(Stmt::While(condition, Box::new(self.parse_statement()?)));
        }
        if self.check_identifier("do") {
            self.next();
            let body = Box::new(self.parse_statement()?);
            self.expect_identifier("while")?;
            self.expect_symbol("(")?;
            let condition = self.parse_expression()?;
            self.expect_symbol(")")?;
            self.consume_symbol(";");
            return Ok(Stmt::DoWhile(body, condition));
        }
        if self.check_identifier("switch") {
            self.next();
            self.expect_symbol("(")?;
            let expression = self.parse_expression()?;
            self.expect_symbol(")")?;
            self.expect_symbol("{")?;
            let mut clauses = Vec::new();
            while !self.check_symbol("}") && !self.is_eof() {
                let condition = if self.check_identifier("case") {
                    self.next();
                    let condition = self.parse_expression()?;
                    self.expect_symbol(":")?;
                    Some(condition)
                } else if self.check_identifier("default") {
                    self.next();
                    self.expect_symbol(":")?;
                    None
                } else {
                    return Err(format!("expected case/default, got {:?}", self.peek()));
                };
                let mut body = Vec::new();
                while !self.check_symbol("}")
                    && !self.check_identifier("case")
                    && !self.check_identifier("default")
                    && !self.is_eof()
                {
                    body.push(self.parse_statement()?);
                }
                clauses.push((condition, body));
            }
            self.expect_symbol("}")?;
            return Ok(Stmt::Switch(expression, clauses));
        }
        if self.check_identifier("for") {
            self.next();
            self.expect_symbol("(")?;
            let first = if self.check_symbol(";") {
                None
            } else {
                // GS2 accepts both `for (item: values)` and
                // `for (item in values)`.  Parse the target without the
                // binary-expression layer first so the `in` token cannot be
                // consumed as an ordinary expression operator.
                let start = self.position;
                let candidate = self.parse_unary()?;
                let foreach = self.consume_symbol(":") || self.check_identifier("in");
                if foreach {
                    if self.check_identifier("in") {
                        self.next();
                    }
                    let source = self.parse_expression()?;
                    self.expect_symbol(")")?;
                    return Ok(Stmt::ForEach(
                        candidate,
                        source,
                        Box::new(self.parse_statement()?),
                    ));
                }
                self.position = start;
                Some(self.parse_expression()?)
            };
            self.expect_symbol(";")?;
            let condition = if self.check_symbol(";") {
                None
            } else {
                Some(self.parse_expression()?)
            };
            self.expect_symbol(";")?;
            let post = if self.check_symbol(")") {
                None
            } else {
                Some(self.parse_expression()?)
            };
            self.expect_symbol(")")?;
            return Ok(Stmt::For(
                first,
                condition,
                post,
                Box::new(self.parse_statement()?),
            ));
        }
        if self.check_identifier("return") {
            self.next();
            if self.consume_symbol(";") {
                return Ok(Stmt::Return(None));
            }
            let value = self.parse_expression()?;
            self.consume_symbol(";");
            return Ok(Stmt::Return(Some(value)));
        }
        if self.check_identifier("break") {
            self.next();
            self.consume_symbol(";");
            return Ok(Stmt::Break);
        }
        if self.check_identifier("continue") {
            self.next();
            self.consume_symbol(";");
            return Ok(Stmt::Continue);
        }
        if self.check_identifier("delete") {
            self.next();
            let target = self.parse_expression()?;
            self.consume_symbol(";");
            return Ok(Stmt::Expr(Expr::Delete(Box::new(target))));
        }
        // `const` is accepted in function bodies by the Goja-backed GS2
        // runtime as a local binding.  Top-level constants are collected in
        // ParsedProgram above; a body-level declaration must remain an
        // ordinary assignment so it is visible to the rest of that call
        // frame without changing the global constant table.
        if self.check_identifier("const") {
            self.next();
            let name = self.expect_name()?;
            if self.consume_symbol("=") {
                let value = self.parse_expression()?;
                self.consume_symbol(";");
                return Ok(Stmt::Expr(Expr::Assign(
                    Box::new(Expr::Variable(name)),
                    "=".to_string(),
                    Box::new(value),
                )));
            }
            self.consume_symbol(";");
            return Ok(Stmt::Empty);
        }
        if self.check_identifier("var")
            || self.check_identifier("let")
            || self.check_identifier("local")
        {
            self.next();
            let name = self.expect_name()?;
            if self.consume_symbol("=") {
                let value = self.parse_expression()?;
                self.consume_symbol(";");
                return Ok(Stmt::Expr(Expr::Assign(
                    Box::new(Expr::Variable(name)),
                    "=".to_string(),
                    Box::new(value),
                )));
            }
            self.consume_symbol(";");
            return Ok(Stmt::Empty);
        }
        let expression = self.parse_expression()?;
        if self.check_symbol("{") {
            let body = self.parse_block()?;
            return Ok(Stmt::Block(vec![Stmt::Expr(expression), Stmt::Block(body)]));
        }
        self.consume_symbol(";");
        Ok(Stmt::Expr(expression))
    }

    fn parse_expression(&mut self) -> std::result::Result<Expr, String> {
        self.parse_assignment()
    }

    fn parse_assignment(&mut self) -> std::result::Result<Expr, String> {
        let left = self.parse_ternary()?;
        for operator in ["=", "+=", "-=", "*=", "/=", "%=", "@="] {
            if self.consume_symbol(operator) {
                let right = self.parse_assignment()?;
                return Ok(Expr::Assign(
                    Box::new(left),
                    operator.to_string(),
                    Box::new(right),
                ));
            }
        }
        Ok(left)
    }

    fn parse_ternary(&mut self) -> std::result::Result<Expr, String> {
        let condition = self.parse_binary(0)?;
        if self.consume_symbol("?") {
            let when_true = self.parse_expression()?;
            self.expect_symbol(":")?;
            let when_false = self.parse_expression()?;
            Ok(Expr::Ternary(
                Box::new(condition),
                Box::new(when_true),
                Box::new(when_false),
            ))
        } else {
            Ok(condition)
        }
    }

    fn parse_binary(&mut self, minimum: u8) -> std::result::Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let implicit = matches!(
                self.peek(),
                Token::Identifier(value)
                    if value.eq_ignore_ascii_case("SPC")
                        || value.eq_ignore_ascii_case("TAB")
                        || value.eq_ignore_ascii_case("NL")
            );
            let Some((operator, precedence)) = (if implicit {
                // The compatibility pass rewrites SPC/TAB/NL before its
                // equality pass.  Equality expressions are therefore
                // wrapped before the generated concatenation is evaluated;
                // keeping these legacy separators just below equality
                // precedence reproduces that observable grouping.
                Some(("@".to_string(), 5))
            } else {
                self.binary_operator()
            }) else {
                break;
            };
            if precedence < minimum {
                break;
            }
            let separator = if implicit {
                match self.next() {
                    Token::Identifier(value) if value.eq_ignore_ascii_case("TAB") => "\t",
                    Token::Identifier(value) if value.eq_ignore_ascii_case("NL") => "\n",
                    _ => " ",
                }
            } else {
                self.next();
                ""
            };
            let right = self.parse_binary(precedence + 1)?;
            if implicit {
                left = Expr::Binary(
                    Box::new(Expr::Binary(
                        Box::new(left),
                        "@spc".to_string(),
                        Box::new(Expr::Value(DynValue::String(separator.to_string()))),
                    )),
                    "@spc".to_string(),
                    Box::new(right),
                );
            } else {
                left = Expr::Binary(Box::new(left), operator, Box::new(right));
            }
        }
        Ok(left)
    }

    fn binary_operator(&self) -> Option<(String, u8)> {
        let value = match self.peek() {
            Token::Symbol(value) => value.as_str(),
            Token::Identifier(value)
                if value.eq_ignore_ascii_case("in") || value.eq_ignore_ascii_case("xor") =>
            {
                value.as_str()
            }
            _ => return None,
        };
        let precedence = match value {
            "||" => 1,
            "&&" => 2,
            "|" => 3,
            "xor" => 4,
            "&" => 5,
            "==" | "===" | "!=" | "!==" => 6,
            "<" | ">" | "<=" | ">=" => 7,
            "in" => 7,
            "+" | "-" | "@" => 8,
            "*" | "/" | "%" => 9,
            "^" => 10,
            _ => return None,
        };
        Some((value.to_string(), precedence))
    }

    fn parse_unary(&mut self) -> std::result::Result<Expr, String> {
        if self.check_identifier("typeof") {
            self.next();
            return Ok(Expr::Unary(
                "typeof".to_string(),
                Box::new(self.parse_unary()?),
                false,
            ));
        }
        for operator in ["!", "-", "+", "~", "++", "--", "@"] {
            if self.consume_symbol(operator) {
                return Ok(Expr::Unary(
                    operator.to_string(),
                    Box::new(self.parse_unary()?),
                    false,
                ));
            }
        }
        let mut expression = self.parse_primary()?;
        for _ in 0..64 {
            if self.consume_symbol(".") {
                if self.consume_symbol("(") {
                    let name = self.parse_expression()?;
                    self.expect_symbol(")")?;
                    expression = Expr::DynamicMember(Box::new(expression), Box::new(name));
                } else {
                    expression = Expr::Member(Box::new(expression), self.expect_simple_name()?);
                }
            } else if self.consume_symbol("[") {
                let first = self.parse_expression()?;
                let index = if self.consume_symbol(",") {
                    let second = self.parse_expression()?;
                    Expr::Array(vec![first, second])
                } else {
                    first
                };
                self.expect_symbol("]")?;
                expression = Expr::Index(Box::new(expression), Box::new(index));
            } else if self.consume_symbol("(") {
                let args = self.parse_arguments_after_open()?;
                expression = Expr::Call(Box::new(expression), args);
            } else if self.consume_symbol("++") {
                expression = Expr::Unary("++".to_string(), Box::new(expression), true);
            } else if self.consume_symbol("--") {
                expression = Expr::Unary("--".to_string(), Box::new(expression), true);
            } else {
                break;
            }
        }
        Ok(expression)
    }

    fn parse_primary(&mut self) -> std::result::Result<Expr, String> {
        match self.next() {
            Token::Number(value) => Ok(Expr::Value(DynValue::Number(
                value.parse::<f64>().unwrap_or(0.0),
            ))),
            Token::String(value) => Ok(Expr::Value(DynValue::String(value))),
            Token::Identifier(value) => {
                if value.eq_ignore_ascii_case("true") {
                    return Ok(Expr::Value(DynValue::Bool(true)));
                }
                if value.eq_ignore_ascii_case("false") {
                    return Ok(Expr::Value(DynValue::Bool(false)));
                }
                if value.eq_ignore_ascii_case("null") || value.eq_ignore_ascii_case("nil") {
                    return Ok(Expr::Value(DynValue::Null));
                }
                if value.eq_ignore_ascii_case("new") {
                    if self.check_symbol("[") {
                        let mut dimensions = Vec::new();
                        while self.consume_symbol("[") {
                            dimensions.push(self.parse_expression()?);
                            self.expect_symbol("]")?;
                        }
                        return Ok(Expr::New("__array".to_string(), dimensions));
                    }
                    let name = self.expect_name()?;
                    let args = if self.consume_symbol("(") {
                        self.parse_arguments_after_open()?
                    } else {
                        Vec::new()
                    };
                    return Ok(Expr::New(name, args));
                }
                if value.eq_ignore_ascii_case("function") {
                    if matches!(self.peek(), Token::Identifier(_)) {
                        self.next();
                    }
                    self.expect_symbol("(")?;
                    let mut args = Vec::new();
                    while !self.check_symbol(")") && !self.is_eof() {
                        args.push(self.expect_name()?);
                        if !self.consume_symbol(",") {
                            break;
                        }
                    }
                    self.expect_symbol(")")?;
                    return Ok(Expr::Function(args, self.parse_block()?));
                }
                Ok(Expr::Variable(value))
            }
            Token::Symbol(value) if value == "(" => {
                let expression = self.parse_expression()?;
                self.expect_symbol(")")?;
                Ok(expression)
            }
            Token::Symbol(value) if value == "{" => {
                let array_context = self.position >= 2
                    && matches!(
                        &self.tokens[self.position - 2],
                        Token::Symbol(previous)
                            if matches!(previous.as_str(), "=" | "(" | "[" | "," | "{")
                    );
                if self.consume_symbol("}") {
                    // The array-literal pass turns an empty brace
                    // literal into an array when it follows an assignment,
                    // call/array delimiter, or another array literal.  A
                    // brace in any other expression position remains an
                    // object literal.
                    return Ok(if array_context {
                        Expr::Array(Vec::new())
                    } else {
                        Expr::Object(Vec::new())
                    });
                }
                let start = self.position;
                let first = self.parse_expression()?;
                if self.consume_symbol(":") {
                    let mut values = vec![(first, self.parse_expression()?)];
                    while self.consume_symbol(",") {
                        if self.check_symbol("}") {
                            break;
                        }
                        let key = self.parse_expression()?;
                        self.expect_symbol(":")?;
                        values.push((key, self.parse_expression()?));
                    }
                    self.expect_symbol("}")?;
                    Ok(Expr::Object(values))
                } else {
                    self.position = start;
                    let mut values = Vec::new();
                    while !self.check_symbol("}") && !self.is_eof() {
                        values.push(self.parse_expression()?);
                        if !self.consume_symbol(",") {
                            break;
                        }
                    }
                    self.expect_symbol("}")?;
                    Ok(Expr::Array(values))
                }
            }
            Token::Symbol(value) if value == "[" => {
                let mut values = Vec::new();
                while !self.check_symbol("]") && !self.is_eof() {
                    values.push(self.parse_expression()?);
                    if !self.consume_symbol(",") {
                        break;
                    }
                }
                self.expect_symbol("]")?;
                Ok(Expr::Array(values))
            }
            Token::Symbol(value) => Err(format!("unexpected token {value}")),
            Token::Eof => Err("unexpected end of script".to_string()),
        }
    }

    fn parse_arguments_after_open(&mut self) -> std::result::Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        while !self.check_symbol(")") && !self.is_eof() {
            args.push(self.parse_expression()?);
            if !self.consume_symbol(",") {
                break;
            }
        }
        self.expect_symbol(")")?;
        Ok(args)
    }

    fn expect_name(&mut self) -> std::result::Result<String, String> {
        let mut name = match self.next() {
            Token::Identifier(value) => value,
            other => return Err(format!("expected identifier, got {other:?}")),
        };
        while self.consume_symbol(".") {
            name.push('.');
            name.push_str(&self.expect_name()?);
            break;
        }
        Ok(name)
    }

    fn expect_simple_name(&mut self) -> std::result::Result<String, String> {
        match self.next() {
            Token::Identifier(value) => Ok(value),
            other => Err(format!("expected identifier, got {other:?}")),
        }
    }

    fn expect_identifier(&mut self, expected: &str) -> std::result::Result<(), String> {
        match self.next() {
            Token::Identifier(value) if value.eq_ignore_ascii_case(expected) => Ok(()),
            other => Err(format!("expected {expected}, got {other:?}")),
        }
    }

    fn expect_symbol(&mut self, expected: &str) -> std::result::Result<(), String> {
        if self.consume_symbol(expected) {
            Ok(())
        } else {
            Err(format!("expected {expected}, got {:?}", self.peek()))
        }
    }

    fn consume_symbol(&mut self, expected: &str) -> bool {
        if self.check_symbol(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn check_symbol(&self, expected: &str) -> bool {
        matches!(self.peek(), Token::Symbol(value) if value == expected)
    }
    fn check_identifier(&self, expected: &str) -> bool {
        matches!(self.peek(), Token::Identifier(value) if value.eq_ignore_ascii_case(expected))
    }
    fn peek(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or(&Token::Eof)
    }
    fn next(&mut self) -> Token {
        let token = self.peek().clone();
        self.position += 1;
        token
    }
    fn is_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }
    fn skip_until(&mut self, symbol: &str) {
        while !self.is_eof() {
            if self.consume_symbol(symbol) {
                break;
            }
            self.position += 1;
        }
    }
}

fn strip_comments(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut quote = None;
    while i < chars.len() {
        let ch = chars[i];
        if let Some(q) = quote {
            result.push(ch);
            if ch == '\\' && i + 1 < chars.len() {
                i += 1;
                result.push(chars[i]);
            } else if ch == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if matches!(ch, '"' | '\'') {
            quote = Some(ch);
            result.push(ch);
            i += 1;
            continue;
        }
        if ch == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            result.push(' ');
            result.push(' ');
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                result.push(' ');
                i += 1;
            }
            continue;
        }
        if ch == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            result.push(' ');
            result.push(' ');
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                result.push(if chars[i] == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            if i + 1 < chars.len() {
                result.push(' ');
                result.push(' ');
                i += 2;
            }
            continue;
        }
        result.push(ch);
        i += 1;
    }
    result
}

#[derive(Clone)]
enum ObjectKind {
    Plain,
    Imported {
        module: String,
    },
    Socket,
    Player {
        account: String,
        online: bool,
        server_player: bool,
    },
    NPC {
        id: u32,
    },
    Level {
        name: String,
    },
    Tiles {
        level: String,
    },
    PutNPC {
        index: usize,
    },
    SQLite {
        name: String,
    },
    Date,
}

#[derive(Clone)]
struct DynObject {
    properties: HashMap<String, DynValue>,
    methods: HashMap<String, Rc<ScriptFunction>>,
    method_modules: HashMap<String, String>,
    kind: ObjectKind,
}

#[derive(Clone)]
enum DynValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    /// Binary strings are used by the legacy ZIP/DES helpers.  Keeping the
    /// bytes separate from UTF-8 script strings prevents a lossy conversion
    /// when a generated archive is written back to the VM file root.
    Bytes(Vec<u8>),
    Array(Rc<RefCell<Vec<DynValue>>>),
    Object(Rc<RefCell<DynObject>>),
    Function(Rc<ScriptFunction>),
    Builtin(String),
}

impl DynValue {
    fn is_undefined(&self) -> bool {
        matches!(self, DynValue::Undefined)
    }
    fn is_undefined_or_empty(&self) -> bool {
        self.is_undefined() || matches!(self, DynValue::Null) || value_string(self).is_empty()
    }
    fn truthy(&self) -> bool {
        truthy(self)
    }
}

impl DynValue {
    fn object(kind: ObjectKind) -> Self {
        DynValue::Object(Rc::new(RefCell::new(DynObject {
            properties: HashMap::new(),
            methods: HashMap::new(),
            method_modules: HashMap::new(),
            kind,
        })))
    }

    fn plain() -> Self {
        Self::object(ObjectKind::Plain)
    }
    fn array(values: Vec<DynValue>) -> Self {
        DynValue::Array(Rc::new(RefCell::new(values)))
    }
    fn object_ref(&self) -> Option<Rc<RefCell<DynObject>>> {
        if let DynValue::Object(value) = self {
            Some(Rc::clone(value))
        } else {
            None
        }
    }
}

enum Flow {
    Value(DynValue),
    Return(DynValue),
    Break,
    Continue,
}

struct EvalState {
    config: VMConfig,
    result: VMResult,
    functions: HashMap<String, Rc<ScriptFunction>>,
    imports: HashMap<String, ParsedProgram>,
    scopes: Vec<HashMap<String, DynValue>>,
    globals: HashMap<String, DynValue>,
    owner: DynValue,
    receiver: DynValue,
    temp: DynValue,
    current_player: DynValue,
    tracked_players: Vec<TrackedPlayer>,
    tracked_npcs: Vec<TrackedNPC>,
    socket_refs: Vec<(usize, DynValue)>,
    putnpc_refs: Vec<(usize, DynValue)>,
    drawings: HashMap<i64, DynValue>,
    request_cache: HashMap<String, DynValue>,
    /// JavaScript arrays can carry named properties in addition to their
    /// numeric elements. The element vector is shared through `Rc`, so this
    /// table is keyed by that same allocation and preserves Goja aliasing.
    array_properties: HashMap<usize, HashMap<String, DynValue>>,
    call_stack: Vec<String>,
    suspended: bool,
    loop_count: usize,
}

struct TrackedPlayer {
    object: DynValue,
    account: String,
    initial_guild: String,
    initial_client: HashMap<String, String>,
    initial_clientr: HashMap<String, String>,
}

struct TrackedNPC {
    context: NPCContext,
    object: DynValue,
    initial: AnyMap,
}

static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);

impl EvalState {
    fn new(
        config: VMConfig,
        program: &ParsedProgram,
        imports: HashMap<String, ParsedProgram>,
    ) -> Self {
        let owner = object_from_any_map(&config.this, &imports);
        if get_property(&owner, "name").is_undefined_or_empty() {
            set_property(&owner, "name", DynValue::String(config.script_name.clone()));
        }
        if get_property(&owner, "hp").is_undefined() {
            set_property(&owner, "hp", DynValue::Number(0.0));
        }
        let temp = DynValue::plain();
        let player_context = player_context_from_map(&config.player, &config.player_flags);
        let current_player = make_player_object(&player_context, true, false, true);
        let mut functions = HashMap::new();
        for function in &program.functions {
            functions.insert(
                function.name.to_ascii_lowercase(),
                Rc::new(function.clone()),
            );
        }
        let now = Utc::now().timestamp() as f64;
        let timevar = ((now - 981_048_814.0) / 5.0).floor().max(0.0);
        let mut state = Self {
            config,
            result: VMResult::default(),
            functions,
            imports,
            scopes: vec![HashMap::new()],
            globals: HashMap::new(),
            owner: owner.clone(),
            receiver: owner,
            temp: temp.clone(),
            current_player: current_player.clone(),
            tracked_players: Vec::new(),
            tracked_npcs: Vec::new(),
            socket_refs: Vec::new(),
            putnpc_refs: Vec::new(),
            drawings: HashMap::new(),
            request_cache: HashMap::new(),
            array_properties: HashMap::new(),
            call_stack: Vec::new(),
            suspended: false,
            loop_count: 0,
        };
        let public_functions = state
            .functions
            .values()
            .filter(|function| function.public)
            .cloned()
            .collect::<Vec<_>>();
        if let Some(object) = state.owner.object_ref() {
            let mut object = object.borrow_mut();
            for function in public_functions {
                object
                    .methods
                    .insert(function.name.to_ascii_lowercase(), function);
            }
        }
        state.globals.insert("temp".to_string(), temp);
        state
            .globals
            .insert("thiso".to_string(), state.owner.clone());
        state.globals.insert("player".to_string(), current_player);
        state.globals.insert(
            "client".to_string(),
            get_property(&state.current_player, "client"),
        );
        state.globals.insert(
            "clientr".to_string(),
            get_property(&state.current_player, "clientr"),
        );
        state.globals.insert(
            "tiles".to_string(),
            DynValue::object(ObjectKind::Tiles {
                level: state
                    .config
                    .player
                    .get("level")
                    .cloned()
                    .unwrap_or_default(),
            }),
        );
        state.track_player(state.current_player.clone(), &player_context);
        let configured_players = state.config.players.clone();
        let all_players = configured_players
            .iter()
            .map(|player| {
                let object = make_player_object(player, true, false, true);
                state.track_player(object.clone(), player);
                object
            })
            .collect::<Vec<_>>();
        state
            .globals
            .insert("allplayers".to_string(), DynValue::array(all_players));
        state
            .globals
            .insert("players".to_string(), get_var(&state, "allplayers"));
        state.globals.insert(
            "weapons".to_string(),
            DynValue::array(
                state
                    .config
                    .weapons
                    .iter()
                    .map(make_weapon_object)
                    .collect(),
            ),
        );
        state.globals.insert(
            "servers".to_string(),
            DynValue::array(
                state
                    .config
                    .servers
                    .iter()
                    .map(make_server_object)
                    .collect(),
            ),
        );
        let configured_npcs = state.config.npcs.clone();
        for context in configured_npcs {
            let object = make_npc_object(&context);
            state.tracked_npcs.push(TrackedNPC {
                context: context.clone(),
                object: object.clone(),
                initial: context.this.clone(),
            });
            if is_script_identifier(&context.name) {
                state
                    .globals
                    .insert(context.name.to_ascii_lowercase(), object);
            }
        }
        state.globals.insert(
            "server".to_string(),
            flag_object(&state.config.server_flags, "server."),
        );
        state.globals.insert(
            "serverr".to_string(),
            flag_object(&state.config.server_flags, "serverr."),
        );
        state.globals.insert(
            "serveroptions".to_string(),
            string_map_object(&state.config.server_options),
        );
        state.globals.insert(
            "params".to_string(),
            DynValue::array(
                state
                    .config
                    .params
                    .iter()
                    .cloned()
                    .map(DynValue::String)
                    .collect(),
            ),
        );
        state.globals.insert(
            "name".to_string(),
            DynValue::String(state.config.script_name.clone()),
        );
        state
            .globals
            .insert("chat".to_string(), DynValue::String(String::new()));
        state
            .globals
            .insert("timevar".to_string(), DynValue::Number(timevar));
        state
            .globals
            .insert("timevar2".to_string(), DynValue::Number(now));
        state
            .globals
            .insert("screenwidth".to_string(), DynValue::Number(1024.0));
        state
            .globals
            .insert("screenheight".to_string(), DynValue::Number(1024.0));
        state
            .globals
            .insert("pi".to_string(), DynValue::Number(std::f64::consts::PI));
        state.globals.insert(
            "sqrt2".to_string(),
            DynValue::Number(std::f64::consts::SQRT_2),
        );
        state.globals.insert(
            "sqrt1_2".to_string(),
            DynValue::Number(std::f64::consts::SQRT_2 / 2.0),
        );
        state
            .globals
            .insert("TAB".to_string(), DynValue::String("\t".to_string()));
        state
            .globals
            .insert("SPC".to_string(), DynValue::String(" ".to_string()));
        state
            .globals
            .insert("NL".to_string(), DynValue::String("\n".to_string()));
        state.globals.insert("NULL".to_string(), DynValue::Null);
        state.globals.insert("nil".to_string(), DynValue::Null);
        state
            .globals
            .insert("maxlooplimit".to_string(), DynValue::Number(10_000.0));
        if is_player_lifecycle_event(&state.config.event_name)
            && !value_string(&get_property(&state.current_player, "account")).is_empty()
        {
            state.globals.insert(
                "params".to_string(),
                DynValue::array(vec![state.current_player.clone()]),
            );
        }
        for (name, value) in &program.constants {
            state.globals.insert(name.clone(), value.clone());
        }
        if let Some(socket) = state.config.socket.clone() {
            let value = make_socket_object(&socket);
            hydrate_object_state(&value, &socket.state);
            install_socket_class_methods(
                &value,
                &socket.joined_classes,
                &state.config.socket_class_resolver,
            );
            state.receiver = value;
            for (name, property) in [
                ("isconnected", "isconnected"),
                ("ipaddress", "ipaddress"),
                ("port", "port"),
                ("packagedelimiter", "packagedelimiter"),
                ("data", "data"),
            ] {
                state
                    .globals
                    .insert(name.to_string(), get_property(&state.receiver, property));
            }
            state
                .globals
                .insert("outdatalength".to_string(), DynValue::Number(0.0));
        }
        state
    }

    fn run(mut self, program: ParsedProgram) -> VMResult {
        if !self.config.skip_top_level {
            for statement in program.top_level {
                if let Flow::Return(_) | Flow::Break | Flow::Continue =
                    self.eval_statement(&statement)
                {
                    break;
                }
                if self.suspended {
                    break;
                }
            }
        }
        if self.result.err.is_empty() && !self.suspended {
            let event = find_function(&self.functions, &self.config.event_name);
            if let Some(function) = event {
                let mut args = Vec::new();
                if let Some(argument) = self.config.socket_argument.clone() {
                    let value = make_socket_object(&argument);
                    hydrate_object_state(&value, &argument.state);
                    args.push(value);
                } else if is_player_lifecycle_event(&self.config.event_name)
                    && !value_string(&get_property(&self.current_player, "account")).is_empty()
                {
                    args.push(self.current_player.clone());
                } else if !self.config.params.is_empty() {
                    args.extend(self.config.params.iter().cloned().map(DynValue::String));
                } else if self.config.socket.is_some() {
                    args.push(self.receiver.clone());
                }
                let receiver = self.receiver.clone();
                let _ = self.invoke_script(function, receiver, args);
            }
        }
        if let Some(socket) = self.config.socket.as_ref() {
            let current = self.receiver.clone();
            if matches!(current, DynValue::Object(_)) {
                self.result
                    .socket_updates
                    .push(socket_context_from_value(&current));
            }
            let _ = socket;
        }
        self.finalize();
        self.result
    }

    fn finalize(&mut self) {
        for (index, object) in self.putnpc_refs.clone() {
            let Some(action) = self.result.level_actions.get_mut(index) else {
                continue;
            };
            action.x = number_f64(&get_property(&object, "x"));
            action.y = number_f64(&get_property(&object, "y"));
            let level = value_string(&get_property(&object, "level"));
            if !level.is_empty() {
                action.level = level;
            }
            let image = value_string(&get_property(&object, "image"));
            if !image.is_empty() {
                action.image = image;
            }
            let script = value_string(&get_property(&object, "script"));
            if !script.is_empty() {
                action.script = script;
            }
            action.classes = socket_joined_classes(&object);
            let reserved = [
                "x",
                "y",
                "level",
                "image",
                "script",
                "name",
                "id",
                "objecttype",
                "__classes",
                "__putnpc_index",
            ];
            if let Some(object_ref) = object.object_ref() {
                for (key, value) in object_ref.borrow().properties.clone() {
                    if !reserved.iter().any(|item| item.eq_ignore_ascii_case(&key))
                        && !key.starts_with("__")
                        && !matches!(value, DynValue::Function(_) | DynValue::Builtin(_))
                    {
                        action.props.insert(key, value_string(&value));
                    }
                }
            }
        }
        for (index, socket) in self.socket_refs.clone() {
            if let Some(action) = self.result.socket_actions.get_mut(index) {
                action.state = socket_state_from_value(&socket, Some(&self.receiver));
                action.joined_classes = socket_joined_classes(&socket);
            }
        }
        self.collect_player_flags();
        self.collect_server_flags();
        self.collect_npc_flags();
        self.collect_current_npc_action();
        self.result.this = export_value(&self.owner, Some(&self.owner));
    }

    fn track_player(&mut self, object: DynValue, context: &PlayerContext) {
        self.tracked_players.push(TrackedPlayer {
            object,
            account: context.account.clone(),
            initial_guild: context.guild.clone(),
            initial_client: flag_values(&context.flags, "client."),
            initial_clientr: flag_values(&context.flags, "clientr."),
        });
    }

    fn collect_player_flags(&mut self) {
        for player in &self.tracked_players {
            if player.account.is_empty() {
                continue;
            }
            let guild = value_string(&get_property(&player.object, "guild"));
            if guild != player.initial_guild {
                self.result.player_props.push(PlayerProp {
                    account: player.account.clone(),
                    name: "guild".to_string(),
                    value: guild,
                });
            }
            for (prefix, initial, property) in [
                ("client.", &player.initial_client, "client"),
                ("clientr.", &player.initial_clientr, "clientr"),
            ] {
                let object = get_property(&player.object, property);
                let Some(object) = object.object_ref() else {
                    continue;
                };
                for (name, value) in object.borrow().properties.clone() {
                    let current = value_string(&value);
                    if initial.get(&name).cloned().unwrap_or_default() != current {
                        self.result.player_flags.push(PlayerFlag {
                            account: player.account.clone(),
                            name: format!("{prefix}{name}"),
                            value: current,
                        });
                    }
                }
            }
        }
    }

    fn collect_npc_flags(&mut self) {
        let tracked = self
            .tracked_npcs
            .iter()
            .map(|npc| (npc.context.id, npc.object.clone(), npc.initial.clone()))
            .collect::<Vec<_>>();
        for (id, object, initial) in tracked {
            if id == 0 {
                continue;
            }
            let Some(object_ref) = object.object_ref() else {
                continue;
            };
            for (name, value) in object_ref.borrow().properties.clone() {
                if matches!(
                    name.to_ascii_lowercase().as_str(),
                    "id" | "name"
                        | "level"
                        | "levelname"
                        | "x"
                        | "y"
                        | "width"
                        | "height"
                        | "script"
                ) || name.starts_with("__")
                {
                    continue;
                }
                let initial_value = initial.get(&name);
                collect_npc_flag_value(
                    &mut self.result.npc_flags,
                    id,
                    &name,
                    &value,
                    initial_value,
                );
            }
        }
    }

    fn collect_server_flags(&mut self) {
        for (prefix, global) in [("server.", "server"), ("serverr.", "serverr")] {
            let object = get_var(self, global);
            let Some(object) = object.object_ref() else {
                continue;
            };
            let mut names = object
                .borrow()
                .properties
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            for key in self.config.server_flags.keys() {
                if let Some(name) = key.strip_prefix(prefix) {
                    if !names.iter().any(|item| item.eq_ignore_ascii_case(name)) {
                        names.push(name.to_string());
                    }
                }
            }
            for name in names {
                let key = format!("{prefix}{name}");
                let old = self
                    .config
                    .server_flags
                    .get(&key)
                    .cloned()
                    .unwrap_or_default();
                let value = get_property(&DynValue::Object(object.clone()), &name);
                if value.is_undefined() {
                    if !old.is_empty() {
                        self.result.server_flags.push(ServerFlag {
                            name: key,
                            value: String::new(),
                            deleted: true,
                        });
                    }
                } else {
                    let current = value_string(&value);
                    if current != old {
                        self.result.server_flags.push(ServerFlag {
                            name: key,
                            value: current,
                            deleted: false,
                        });
                    }
                }
            }
        }
    }

    fn collect_current_npc_action(&mut self) {
        if self.config.npc_id == 0 {
            return;
        }
        let aliases = [
            ("image", "image"),
            ("chat", "chat"),
            ("message", "chat"),
            ("nick", "nick"),
            ("nickname", "nick"),
            ("dir", "dir"),
            ("ani", "ani"),
            ("gani", "ani"),
            ("head", "head"),
            ("headimg", "head"),
            ("body", "body"),
            ("bodyimg", "body"),
            ("sword", "sword"),
            ("swordimg", "sword"),
            ("shield", "shield"),
            ("shieldimg", "shield"),
            ("horseimg", "horse"),
            ("hearts", "hearts"),
            ("gralats", "gralats"),
            ("arrows", "arrows"),
            ("bombs", "bombs"),
            ("darts", "arrows"),
            ("glovepower", "glovepower"),
            ("swordpower", "swordpower"),
            ("shieldpower", "shieldpower"),
            ("ap", "ap"),
            ("colors", "colors"),
            ("width", "width"),
            ("height", "height"),
            ("guild", "guild"),
        ];
        let mut action = NPCAction {
            id: self.config.npc_id,
            ..NPCAction::default()
        };
        for (source, target) in aliases {
            // The host checks both the installed global NPC variable and
            // the current `this` object.  Global assignments such as
            // `image = "foo"` therefore have the same precedence as
            // assignments through `this.image`.
            for value in [get_var(self, source), get_property(&self.owner, source)] {
                if matches!(
                    value,
                    DynValue::Undefined
                        | DynValue::Null
                        | DynValue::Function(_)
                        | DynValue::Builtin(_)
                ) {
                    continue;
                }
                let value = value_string(&value);
                if !value.is_empty() {
                    action.props.insert(target.to_string(), value);
                }
            }
        }
        action.chat = action.props.get("chat").cloned().unwrap_or_default();
        if get_property(&self.owner, "__hasvisflags").truthy() {
            action.has_vis_flags = true;
            action.vis_flags = number_i32(&get_property(&self.owner, "__visflags"));
        }
        if get_property(&self.owner, "__hasblockflags").truthy() {
            action.has_block_flags = true;
            action.block_flags = number_i32(&get_property(&self.owner, "__blockflags"));
        }
        action.destroy = get_property(&self.owner, "__destroy").truthy();
        for name in [
            "carryobject",
            "throwcarry",
            "lay",
            "take",
            "take2",
            "takehorse",
            "showani",
            "showani2",
            "showpoly",
            "showpoly2",
            "changeimgcolors",
            "changeimgvis",
            "changeimgzoom",
        ] {
            let key = format!("__npcaction_{name}");
            let value = get_property(&self.owner, &key);
            if !matches!(
                value,
                DynValue::Undefined | DynValue::Null | DynValue::Function(_) | DynValue::Builtin(_)
            ) {
                self.result.npc_actions.push(NPCAction {
                    id: self.config.npc_id,
                    action: name.to_string(),
                    chat: value_string(&value),
                    has_chat: true,
                    ..NPCAction::default()
                });
            }
        }
        if let Some(object) = self.owner.object_ref() {
            for (key, value) in object.borrow().properties.clone() {
                if let Some(flag) = key.strip_prefix("__npcflag_") {
                    action.flags.insert(flag.to_string(), value_string(&value));
                }
            }
        }
        if !action.props.is_empty()
            || !action.flags.is_empty()
            || action.has_vis_flags
            || action.has_block_flags
            || action.destroy
        {
            self.result.npc_actions.push(action);
        }
    }

    fn eval_statement(&mut self, statement: &Stmt) -> Flow {
        match statement {
            Stmt::Empty => Flow::Value(DynValue::Undefined),
            Stmt::Expr(expression) => Flow::Value(self.eval_expr(expression)),
            Stmt::Block(body) => {
                for child in body {
                    let flow = self.eval_statement(child);
                    if !matches!(flow, Flow::Value(_)) || self.suspended {
                        return flow;
                    }
                }
                Flow::Value(DynValue::Undefined)
            }
            Stmt::Return(expression) => Flow::Return(
                expression
                    .as_ref()
                    .map_or(DynValue::Number(0.0), |x| self.eval_expr(x)),
            ),
            Stmt::If(condition, then_body, else_body) => {
                if self.eval_expr(condition).truthy() {
                    self.eval_statement(then_body)
                } else if let Some(other) = else_body {
                    self.eval_statement(other)
                } else {
                    Flow::Value(DynValue::Undefined)
                }
            }
            Stmt::While(condition, body) => {
                let mut count = 0;
                while self.eval_expr(condition).truthy() {
                    count += 1;
                    self.loop_count += 1;
                    if count > self.loop_limit() || self.loop_count > self.loop_limit() {
                        self.result.err = "maxlooplimit exceeded".to_string();
                        break;
                    }
                    match self.eval_statement(body) {
                        Flow::Break => break,
                        Flow::Continue => continue,
                        Flow::Return(value) => return Flow::Return(value),
                        Flow::Value(_) => {}
                    }
                    if self.suspended {
                        break;
                    }
                }
                Flow::Value(DynValue::Undefined)
            }
            Stmt::DoWhile(body, condition) => {
                let mut count = 0;
                loop {
                    count += 1;
                    self.loop_count += 1;
                    if count > self.loop_limit() || self.loop_count > self.loop_limit() {
                        self.result.err = "maxlooplimit exceeded".to_string();
                        break;
                    }
                    match self.eval_statement(body) {
                        Flow::Break => break,
                        Flow::Continue => {}
                        Flow::Return(value) => return Flow::Return(value),
                        Flow::Value(_) => {}
                    }
                    if !self.eval_expr(condition).truthy() {
                        break;
                    }
                }
                Flow::Value(DynValue::Undefined)
            }
            Stmt::For(init, condition, post, body) => {
                if let Some(init) = init {
                    self.eval_expr(init);
                }
                let mut count = 0;
                while condition
                    .as_ref()
                    .map_or(true, |x| self.eval_expr(x).truthy())
                {
                    count += 1;
                    self.loop_count += 1;
                    if count > self.loop_limit() || self.loop_count > self.loop_limit() {
                        self.result.err = "maxlooplimit exceeded".to_string();
                        break;
                    }
                    match self.eval_statement(body) {
                        Flow::Break => break,
                        Flow::Continue => {}
                        Flow::Return(value) => return Flow::Return(value),
                        Flow::Value(_) => {}
                    }
                    if let Some(post) = post {
                        self.eval_expr(post);
                    }
                    if self.suspended {
                        break;
                    }
                }
                Flow::Value(DynValue::Undefined)
            }
            Stmt::ForEach(target, source, body) => {
                let values = array_values(&self.eval_expr(source));
                let mut count = 0;
                for value in values {
                    count += 1;
                    self.loop_count += 1;
                    if count > self.loop_limit() || self.loop_count > self.loop_limit() {
                        self.result.err = "maxlooplimit exceeded".to_string();
                        break;
                    }
                    self.assign(target, "=".to_string(), value);
                    match self.eval_statement(body) {
                        Flow::Break => break,
                        Flow::Continue => continue,
                        Flow::Return(value) => return Flow::Return(value),
                        Flow::Value(_) => {}
                    }
                    if self.suspended {
                        break;
                    }
                }
                Flow::Value(DynValue::Undefined)
            }
            Stmt::Switch(expression, clauses) => {
                let value = self.eval_expr(expression);
                let mut start = None;
                let mut default = None;
                for (index, (condition, _)) in clauses.iter().enumerate() {
                    if condition.is_none() {
                        default = Some(index);
                    } else if start.is_none()
                        && equal_values(&value, &self.eval_expr(condition.as_ref().expect("case")))
                    {
                        start = Some(index);
                    }
                }
                let Some(start) = start.or(default) else {
                    return Flow::Value(DynValue::Undefined);
                };
                for (_, body) in clauses.iter().skip(start) {
                    for statement in body {
                        match self.eval_statement(statement) {
                            Flow::Break => return Flow::Value(DynValue::Undefined),
                            Flow::Continue => return Flow::Continue,
                            Flow::Return(value) => return Flow::Return(value),
                            Flow::Value(_) => {}
                        }
                        if self.suspended {
                            return Flow::Value(DynValue::Undefined);
                        }
                    }
                }
                Flow::Value(DynValue::Undefined)
            }
            Stmt::Break => Flow::Break,
            Stmt::Continue => Flow::Continue,
        }
    }

    fn loop_limit(&self) -> usize {
        let configured = number_i64(&get_var(self, "maxlooplimit"));
        if configured > 0 {
            configured as usize
        } else {
            10_000
        }
    }

    fn eval_expr(&mut self, expression: &Expr) -> DynValue {
        match expression {
            Expr::Value(value) => value.clone(),
            Expr::Variable(name) => get_var(self, name),
            Expr::Member(object, name) => {
                let value = self.eval_expr(object);
                let value = self.resolve_named_object(value);
                self.property_value(&value, name)
            }
            Expr::DynamicMember(object, name) => {
                let key = value_string(&self.eval_expr(name));
                let value = self.eval_expr(object);
                let value = self.resolve_named_object(value);
                self.property_value(&value, &key)
            }
            Expr::Index(object, index) => {
                let object_value = self.eval_expr(object);
                let index_value = self.eval_expr(index);
                if let DynValue::Object(object_ref) = &object_value {
                    if let ObjectKind::Tiles { level } = &object_ref.borrow().kind {
                        let indexes = array_values(&index_value);
                        if indexes.len() >= 2 {
                            return DynValue::Number(self.raw_tile_type(
                                level,
                                number_f64(&indexes[0]),
                                number_f64(&indexes[1]),
                            ) as f64);
                        }
                    }
                }
                get_index(&object_value, number_i64(&index_value))
            }
            Expr::Call(callee, args) => {
                let values = args.iter().map(|x| self.eval_expr(x)).collect::<Vec<_>>();
                match callee.as_ref() {
                    Expr::Variable(name) => self.invoke_named(name, values, self.receiver.clone()),
                    Expr::Member(object, name) => {
                        let receiver = if receiver_method_needs_container(name) {
                            let existing = self.eval_expr(object);
                            if matches!(existing, DynValue::String(_) | DynValue::Bytes(_)) {
                                existing
                            } else {
                                self.ensure_lvalue_container(object, method_prefers_array(name))
                            }
                        } else {
                            self.eval_expr(object)
                        };
                        self.invoke_method(receiver, name, values)
                    }
                    Expr::DynamicMember(object, name) => {
                        let method = value_string(&self.eval_expr(name));
                        let receiver = if receiver_method_needs_container(&method) {
                            let existing = self.eval_expr(object);
                            if matches!(existing, DynValue::String(_) | DynValue::Bytes(_)) {
                                existing
                            } else {
                                self.ensure_lvalue_container(object, method_prefers_array(&method))
                            }
                        } else {
                            self.eval_expr(object)
                        };
                        self.invoke_method(receiver, &method, values)
                    }
                    _ => {
                        let function = self.eval_expr(callee);
                        self.invoke_value(function, self.receiver.clone(), values)
                    }
                }
            }
            Expr::New(name, args) => {
                let values = args.iter().map(|x| self.eval_expr(x)).collect::<Vec<_>>();
                self.construct(name, values)
            }
            Expr::Array(values) => {
                DynValue::array(values.iter().map(|x| self.eval_expr(x)).collect())
            }
            Expr::Object(values) => {
                let object = DynValue::plain();
                for (key, item) in values {
                    let key = value_string(&self.eval_expr(key));
                    let item = self.eval_expr(item);
                    set_property(&object, &key, item);
                }
                object
            }
            Expr::Unary(operator, value, postfix) => {
                if operator == "++" || operator == "--" {
                    let old = self.eval_expr(value);
                    let next = DynValue::Number(
                        number_f64(&old) + if operator == "++" { 1.0 } else { -1.0 },
                    );
                    self.set_lvalue(value, next.clone());
                    if *postfix { old } else { next }
                } else {
                    let value = self.eval_expr(value);
                    match operator.as_str() {
                        "!" => DynValue::Bool(!value.truthy()),
                        "-" => DynValue::Number(-number_f64(&value)),
                        "+" => DynValue::Number(number_f64(&value)),
                        "~" => DynValue::Number(
                            (number_i64(&value) as i64).wrapping_neg() as f64 - 1.0,
                        ),
                        "@" => DynValue::String(coerce_string(&value)),
                        "typeof" => DynValue::String(value_type_name(&value).to_string()),
                        _ => value,
                    }
                }
            }
            Expr::Binary(left, operator, right) => {
                if operator == "&&" {
                    let left = self.eval_expr(left);
                    if !left.truthy() {
                        return left;
                    }
                    return self.eval_expr(right);
                }
                if operator == "||" {
                    let left = self.eval_expr(left);
                    if left.truthy() {
                        return left;
                    }
                    return self.eval_expr(right);
                }
                if operator == "@spc" {
                    let left = self.eval_expr_in_legacy_separator(left);
                    let right = self.eval_expr_in_legacy_separator(right);
                    return DynValue::String(format!(
                        "{}{}",
                        js_concat_string(&left),
                        js_concat_string(&right)
                    ));
                }
                let legacy_equality = legacy_equality_expression(left.as_ref(), right.as_ref());
                let left = self.eval_expr(left);
                let right = self.eval_expr(right);
                let value = binary_value(&left, operator, &right);
                if matches!(operator.as_str(), "==" | "===" | "!=" | "!==") && legacy_equality {
                    let equal = equal_values(&left, &right);
                    return DynValue::Number(if matches!(operator.as_str(), "==" | "===") {
                        equal as i32 as f64
                    } else {
                        (!equal) as i32 as f64
                    });
                }
                value
            }
            Expr::Assign(left, operator, right) => {
                let value = self.eval_expr(right);
                self.assign(left, operator.clone(), value)
            }
            Expr::Delete(target) => {
                self.delete_lvalue(target);
                DynValue::Bool(true)
            }
            Expr::Ternary(condition, when_true, when_false) => {
                if self.eval_expr(condition).truthy() {
                    self.eval_expr(when_true)
                } else {
                    self.eval_expr(when_false)
                }
            }
            Expr::Function(args, body) => DynValue::Function(Rc::new(ScriptFunction {
                name: String::new(),
                args: args.clone(),
                body: body.clone(),
                public: false,
            })),
        }
    }

    fn eval_expr_in_legacy_separator(&mut self, expression: &Expr) -> DynValue {
        match expression {
            Expr::Binary(left, operator, right)
                if matches!(operator.as_str(), "==" | "===" | "!=" | "!==") =>
            {
                let left_value = self.eval_expr(left);
                let right_value = self.eval_expr(right);
                if legacy_equality_expression(left, right) {
                    let mut equal = equal_values(&left_value, &right_value);
                    if (matches!(left.as_ref(), Expr::Unary(operator, _, _) if operator == "@")
                        && matches!(right.as_ref(), Expr::Value(DynValue::String(value)) if value.is_empty()))
                        || (matches!(right.as_ref(), Expr::Unary(operator, _, _) if operator == "@")
                            && matches!(left.as_ref(), Expr::Value(DynValue::String(value)) if value.is_empty()))
                    {
                        equal = true;
                    }
                    return DynValue::Number(if matches!(operator.as_str(), "==" | "===") {
                        equal as i32 as f64
                    } else {
                        (!equal) as i32 as f64
                    });
                }
                binary_value(&left_value, operator, &right_value)
            }
            Expr::Binary(left, operator, right) if operator == "@spc" => {
                let left = self.eval_expr_in_legacy_separator(left);
                let right = self.eval_expr_in_legacy_separator(right);
                DynValue::String(format!(
                    "{}{}",
                    js_concat_string(&left),
                    js_concat_string(&right)
                ))
            }
            _ => self.eval_expr(expression),
        }
    }

    fn assign(&mut self, target: &Expr, operator: String, value: DynValue) -> DynValue {
        let value = if operator == "=" {
            value
        } else {
            let old = self.eval_expr(target);
            binary_value(&old, operator.trim_end_matches('='), &value)
        };
        self.set_lvalue(target, value.clone());
        value
    }

    fn set_lvalue(&mut self, target: &Expr, value: DynValue) {
        match target {
            Expr::Variable(name) => set_var(self, name, value),
            Expr::Member(object, name) => {
                let object_value = self.ensure_lvalue_container(object, false);
                self.set_property_value(&object_value, name, value.clone());
                // HexaVM preserves the legacy temporary-variable alias: a
                // simple temp.foo assignment also makes foo available as a
                // global script variable.
                if matches!(object.as_ref(), Expr::Variable(root) if root.eq_ignore_ascii_case("temp"))
                    && !is_reserved_identifier(name)
                {
                    set_var(self, name, value);
                }
            }
            Expr::DynamicMember(object, name) => {
                let object_value = self.ensure_lvalue_container(object, false);
                let key = value_string(&self.eval_expr(name));
                self.set_property_value(&object_value, &key, value);
            }
            Expr::Index(object, index) => {
                let object_value = self.eval_expr(object);
                let index_value = self.eval_expr(index);
                if let DynValue::Object(object_ref) = &object_value {
                    if let ObjectKind::Tiles { level } = &object_ref.borrow().kind {
                        let indexes = array_values(&index_value);
                        if indexes.len() >= 2 {
                            self.result.level_actions.push(LevelAction {
                                action: "settile".to_string(),
                                level: level.clone(),
                                x: number_f64(&indexes[0]),
                                y: number_f64(&indexes[1]),
                                tile: number_i32(&value),
                                ..LevelAction::default()
                            });
                            return;
                        }
                    }
                }
                let object_value = self.ensure_lvalue_container(object, true);
                set_index(&object_value, number_i64(&index_value), value);
            }
            Expr::Call(callee, args)
                if matches!(callee.as_ref(), Expr::Variable(name) if name.eq_ignore_ascii_case("makevar"))
                    && args.len() == 1 =>
            {
                let path = value_string(&self.eval_expr(&args[0]));
                self.set_script_path(&path, value);
            }
            _ => {}
        }
    }

    /// Return a mutable object/array for an l-value path, creating each
    /// missing parent along the way.  Goja boxes scalar values when legacy
    /// GS2 writes through them (`this.user.password = ...`); creating the
    /// replacement here preserves that observable behavior without relying
    /// on unsafe aliases.
    fn ensure_lvalue_container(&mut self, target: &Expr, prefer_array: bool) -> DynValue {
        let current = self.eval_expr(target);
        if matches!(current, DynValue::String(_)) {
            let named = self.resolve_named_object(current.clone());
            if !matches!(named, DynValue::String(_)) {
                return named;
            }
        }
        let usable = matches!(&current, DynValue::Object(_) | DynValue::Array(_));
        if usable {
            return current;
        }
        let replacement = if prefer_array {
            DynValue::array(Vec::new())
        } else {
            DynValue::plain()
        };
        if !matches!(
            current,
            DynValue::Undefined | DynValue::Null | DynValue::Object(_) | DynValue::Array(_)
        ) {
            set_property(&replacement, "__gs2value", current.clone());
        }
        match target {
            Expr::Variable(name) => set_var(self, name, replacement.clone()),
            Expr::Member(object, name) => {
                let parent = self.ensure_lvalue_container(object, false);
                self.set_property_value(&parent, name, replacement.clone());
                if matches!(object.as_ref(), Expr::Variable(root) if root.eq_ignore_ascii_case("temp"))
                    && !is_reserved_identifier(name)
                {
                    set_var(self, name, replacement.clone());
                }
            }
            Expr::DynamicMember(object, name) => {
                let parent = self.ensure_lvalue_container(object, false);
                let key = value_string(&self.eval_expr(name));
                self.set_property_value(&parent, &key, replacement.clone());
            }
            Expr::Index(object, index) => {
                let parent = self.ensure_lvalue_container(object, true);
                let index = number_i64(&self.eval_expr(index));
                set_index(&parent, index, replacement.clone());
            }
            _ => {}
        }
        replacement
    }

    fn resolve_named_object(&self, value: DynValue) -> DynValue {
        let DynValue::String(name) = &value else {
            return value;
        };
        self.npc_by_name(name).unwrap_or(value)
    }

    fn array_property_key(value: &DynValue) -> Option<usize> {
        let DynValue::Array(values) = value else {
            return None;
        };
        Some(Rc::as_ptr(values) as usize)
    }

    fn property_value(&self, value: &DynValue, name: &str) -> DynValue {
        if let Some(key) = Self::array_property_key(value) {
            if let Some(properties) = self.array_properties.get(&key) {
                if let Some(property) = property_key(properties, name) {
                    return properties
                        .get(&property)
                        .cloned()
                        .unwrap_or(DynValue::Undefined);
                }
            }
        }
        get_property(value, name)
    }

    fn set_property_value(&mut self, value: &DynValue, name: &str, item: DynValue) {
        if let Some(key) = Self::array_property_key(value) {
            if name.parse::<usize>().is_ok() {
                set_property(value, name, item);
                if let Some(properties) = self.array_properties.get_mut(&key) {
                    properties.remove(name);
                }
                return;
            }
            self.array_properties
                .entry(key)
                .or_default()
                .insert(name.to_string(), item);
            return;
        }
        set_property(value, name, item);
    }

    fn add_array_member(&mut self, receiver: &DynValue, name: &str, value: DynValue) {
        if name.parse::<usize>().is_ok() {
            set_property(receiver, name, value);
            if let Some(key) = Self::array_property_key(receiver) {
                if let Some(properties) = self.array_properties.get_mut(&key) {
                    properties.remove(name);
                }
            }
            return;
        }
        if let Some(key) = Self::array_property_key(receiver) {
            self.array_properties
                .entry(key)
                .or_default()
                .insert(name.to_string(), value);
        }
    }

    fn get_array_member(&self, receiver: &DynValue, name: &str) -> DynValue {
        if let Some(key) = Self::array_property_key(receiver) {
            if let Some(properties) = self.array_properties.get(&key) {
                if let Some(property) = property_key(properties, name) {
                    return properties
                        .get(&property)
                        .cloned()
                        .unwrap_or(DynValue::Undefined);
                }
            }
        }
        get_property(receiver, name)
    }

    fn named_properties(&self, receiver: &DynValue) -> Vec<(String, DynValue)> {
        match receiver {
            DynValue::Object(object) => object.borrow().properties.clone().into_iter().collect(),
            DynValue::Array(values) => {
                let mut properties = values
                    .borrow()
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (index.to_string(), value.clone()))
                    .collect::<Vec<_>>();
                if let Some(key) = Self::array_property_key(receiver) {
                    if let Some(named) = self.array_properties.get(&key) {
                        properties.extend(named.clone());
                    }
                }
                properties
            }
            _ => Vec::new(),
        }
    }

    fn named_property(&self, receiver: &DynValue, name: &str) -> DynValue {
        match receiver {
            DynValue::Array(_) => self.get_array_member(receiver, name),
            _ => get_property(receiver, name),
        }
    }

    fn set_named_property(&mut self, receiver: &DynValue, name: &str, value: DynValue) {
        if matches!(receiver, DynValue::Array(_)) {
            self.set_property_value(receiver, name, value);
        } else {
            set_property(receiver, name, value);
        }
    }

    fn clear_named_properties(&mut self, receiver: &DynValue) {
        match receiver {
            DynValue::Object(object) => object.borrow_mut().properties.clear(),
            DynValue::Array(values) => {
                values.borrow_mut().clear();
                if let Some(key) = Self::array_property_key(receiver) {
                    self.array_properties.remove(&key);
                }
            }
            _ => {}
        }
    }

    fn joined_classes(&self, receiver: &DynValue) -> Vec<String> {
        array_values(&self.named_property(receiver, "__classes"))
            .into_iter()
            .map(|value| value_string(&value).trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    }

    fn add_joined_class(&mut self, receiver: &DynValue, class: &str) {
        if class.is_empty() {
            return;
        }
        let mut classes = self
            .joined_classes(receiver)
            .into_iter()
            .map(DynValue::String)
            .collect::<Vec<_>>();
        if !classes
            .iter()
            .any(|value| value_string(value).eq_ignore_ascii_case(class))
        {
            classes.push(DynValue::String(class.to_string()));
        }
        self.set_named_property(receiver, "__classes", DynValue::array(classes));
    }

    fn install_import_module(&mut self, receiver: &DynValue, module_name: &str) {
        let functions = self
            .imports
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(module_name))
            .map(|(_, module)| module.functions.clone())
            .unwrap_or_default();
        let Some(object) = receiver.object_ref() else {
            return;
        };
        let module_key = module_name.to_ascii_lowercase();
        let mut object = object.borrow_mut();
        for function in functions {
            let key = function.name.to_ascii_lowercase();
            object
                .methods
                .entry(key.clone())
                .or_insert_with(|| Rc::new(function));
            object
                .method_modules
                .entry(key)
                .or_insert_with(|| module_key.clone());
        }
    }

    fn leave_import_module(&mut self, receiver: &DynValue, module_name: &str) {
        let Some(object) = receiver.object_ref() else {
            return;
        };
        let module_key = module_name.to_ascii_lowercase();
        let mut object = object.borrow_mut();
        let keys = object
            .method_modules
            .iter()
            .filter(|(_, module)| module.eq_ignore_ascii_case(&module_key))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            object.methods.remove(&key);
            object.method_modules.remove(&key);
        }
    }

    fn imported_receiver(&self, receiver: &DynValue) -> bool {
        receiver
            .object_ref()
            .is_some_and(|object| matches!(object.borrow().kind, ObjectKind::Imported { .. }))
    }

    fn private_method_allowed(&self, receiver: &DynValue) -> bool {
        self.imported_receiver(receiver)
            && receiver.object_ref().is_some_and(|receiver| {
                self.receiver
                    .object_ref()
                    .is_some_and(|current| Rc::ptr_eq(&receiver, &current))
            })
    }

    fn replace_object_value(&mut self, target: &DynValue, value: DynValue) {
        match target {
            DynValue::Array(values) => {
                values.borrow_mut().clear();
                if let Some(key) = Self::array_property_key(target) {
                    self.array_properties.remove(&key);
                }
                match value {
                    DynValue::Object(object) => {
                        for (key, item) in object.borrow().properties.clone() {
                            if !key.starts_with("__") {
                                self.set_property_value(target, &key, item);
                            }
                        }
                    }
                    DynValue::Array(items) => {
                        *values.borrow_mut() = items.borrow().clone();
                    }
                    scalar => self.set_property_value(target, "text", scalar),
                }
            }
            DynValue::Object(object) => {
                object.borrow_mut().properties.clear();
                match value {
                    DynValue::Object(source) => {
                        for (key, item) in source.borrow().properties.clone() {
                            if !key.starts_with("__") {
                                object.borrow_mut().properties.insert(key, item);
                            }
                        }
                    }
                    DynValue::Array(items) => {
                        for (index, item) in items.borrow().iter().cloned().enumerate() {
                            object
                                .borrow_mut()
                                .properties
                                .insert(index.to_string(), item);
                        }
                        let length = object.borrow().properties.len() as f64;
                        object
                            .borrow_mut()
                            .properties
                            .insert("length".to_string(), DynValue::Number(length));
                    }
                    scalar => {
                        object
                            .borrow_mut()
                            .properties
                            .insert("text".to_string(), scalar);
                    }
                }
            }
            _ => {}
        }
    }

    fn delete_lvalue(&mut self, target: &Expr) {
        match target {
            Expr::Variable(name) => {
                self.globals.remove(&name.to_ascii_lowercase());
            }
            Expr::Member(object, name) => {
                let object = self.eval_expr(object);
                if let Some(array_key) = Self::array_property_key(&object) {
                    if let Some(properties) = self.array_properties.get_mut(&array_key) {
                        if let Some(key) = property_key(properties, name) {
                            properties.remove(&key);
                        }
                    }
                } else if let Some(object) = object.object_ref() {
                    let key = property_key(&object.borrow().properties, name);
                    if let Some(key) = key {
                        object.borrow_mut().properties.remove(&key);
                    }
                }
            }
            Expr::DynamicMember(object, name) => {
                let object = self.eval_expr(object);
                let key = value_string(&self.eval_expr(name));
                if let Some(array_key) = Self::array_property_key(&object) {
                    if let Some(properties) = self.array_properties.get_mut(&array_key) {
                        if let Some(property) = property_key(properties, &key) {
                            properties.remove(&property);
                        }
                    }
                } else if let Some(object) = object.object_ref() {
                    object.borrow_mut().properties.remove(&key);
                }
            }
            Expr::Index(object, index) => {
                let object = self.eval_expr(object);
                let index = number_i64(&self.eval_expr(index));
                if let DynValue::Array(values) = object {
                    if index >= 0 && (index as usize) < values.borrow().len() {
                        values.borrow_mut()[index as usize] = DynValue::Undefined;
                    }
                }
            }
            _ => {}
        }
    }

    fn invoke_named(&mut self, name: &str, args: Vec<DynValue>, receiver: DynValue) -> DynValue {
        let lower = name.to_ascii_lowercase();
        if let Some(function) = self
            .functions
            .get(&lower)
            .cloned()
            .or_else(|| find_function(&self.functions, name))
        {
            return self.invoke_script(function, receiver, args);
        }
        let value = self.invoke_builtin(&lower, args, receiver);
        if value.is_undefined()
            && self.imports.values().any(|module| {
                module
                    .functions
                    .iter()
                    .any(|function| function.public && function.name.eq_ignore_ascii_case(name))
            })
        {
            self.result.err = format!("{name} is not a function");
        }
        value
    }

    fn invoke_method(&mut self, receiver: DynValue, name: &str, args: Vec<DynValue>) -> DynValue {
        if matches!(receiver, DynValue::Array(_)) {
            match name.to_ascii_lowercase().as_str() {
                "addarraymember" => {
                    let member = value_string(args.first().unwrap_or(&DynValue::Undefined));
                    let value = args.get(1).cloned().unwrap_or(DynValue::Undefined);
                    self.add_array_member(&receiver, &member, value);
                    return receiver;
                }
                "getarraymember" => {
                    let member = value_string(args.first().unwrap_or(&DynValue::Undefined));
                    return self.get_array_member(&receiver, &member);
                }
                _ => {}
            }
        }
        if let Some(function) = get_method(&receiver, name) {
            if !function.public && !self.private_method_allowed(&receiver) {
                self.result.err = format!("{name} is not a function");
                return DynValue::Undefined;
            }
            return self.invoke_script(function, receiver, args);
        }
        if let DynValue::Object(object) = &receiver {
            let kind = object.borrow().kind.clone();
            if matches!(kind, ObjectKind::PutNPC { .. }) {
                return self.invoke_putnpc_method(&receiver, name, &args);
            }
            match kind {
                ObjectKind::NPC { id } => {
                    let operation = name.to_ascii_lowercase();
                    if operation == "tostring" {
                        return DynValue::String(value_string(&get_property(&receiver, "name")));
                    }
                    if operation == "save" {
                        self.result.npc_actions.push(NPCAction {
                            id,
                            save_props: export_value(&receiver, Some(&self.owner)),
                            save: true,
                            ..NPCAction::default()
                        });
                        return DynValue::Undefined;
                    }
                    if self.npc_has_function(id, name) {
                        let npc_name = self
                            .npc_context(id)
                            .map(|context| context.name.clone())
                            .unwrap_or_default();
                        self.result.npc_function_calls.push(NPCFunctionCall {
                            id,
                            name: npc_name,
                            function: name.to_string(),
                            args: args.iter().map(value_string).collect(),
                        });
                        return DynValue::Undefined;
                    }
                }
                ObjectKind::Player { .. } if name.eq_ignore_ascii_case("tostring") => {
                    return DynValue::String(value_string(&get_property(&receiver, "account")));
                }
                ObjectKind::Plain if name.eq_ignore_ascii_case("tostring") => {
                    return DynValue::String(value_string(&receiver));
                }
                _ => {}
            }
        }
        self.invoke_builtin(
            &format!("method:{}", name.to_ascii_lowercase()),
            args,
            receiver,
        )
    }

    fn invoke_putnpc_method(
        &mut self,
        receiver: &DynValue,
        name: &str,
        args: &[DynValue],
    ) -> DynValue {
        let index = number_i64(&get_property(receiver, "__putnpc_index"));
        if index < 0 || index as usize >= self.result.level_actions.len() {
            return DynValue::Undefined;
        }
        match name.to_ascii_lowercase().as_str() {
            "join" => {
                add_socket_class(
                    receiver,
                    &value_string(args.first().unwrap_or(&DynValue::Undefined)),
                );
                if let Some(action) = self.result.level_actions.get_mut(index as usize) {
                    action.classes = socket_joined_classes(receiver);
                }
                receiver.clone()
            }
            "leave" => receiver.clone(),
            "destroy" => {
                if let Some(action) = self.result.level_actions.get_mut(index as usize) {
                    action.action.clear();
                }
                DynValue::Undefined
            }
            _ => {
                if let Some(action) = self.result.level_actions.get_mut(index as usize) {
                    action.calls.push(NPCFunctionCall {
                        function: name.to_string(),
                        args: args.iter().map(value_string).collect(),
                        ..NPCFunctionCall::default()
                    });
                }
                DynValue::Undefined
            }
        }
    }

    fn invoke_value(
        &mut self,
        value: DynValue,
        receiver: DynValue,
        args: Vec<DynValue>,
    ) -> DynValue {
        match value {
            DynValue::Function(function) => self.invoke_script(function, receiver, args),
            DynValue::Builtin(name) => self.invoke_builtin(&name, args, receiver),
            DynValue::String(name) => self.invoke_named(&name, args, receiver),
            _ => DynValue::Undefined,
        }
    }

    fn invoke_script(
        &mut self,
        function: Rc<ScriptFunction>,
        receiver: DynValue,
        args: Vec<DynValue>,
    ) -> DynValue {
        let stack_name = if function.name.is_empty() {
            "<anonymous>".to_string()
        } else {
            function.name.clone()
        };
        self.call_stack.push(stack_name);
        let old_receiver = self.receiver.clone();
        self.receiver = receiver;
        self.scopes.push(HashMap::new());
        for (index, name) in function.args.iter().enumerate() {
            let value = args.get(index).cloned().unwrap_or(DynValue::Undefined);
            if let Some((root, property)) = name.split_once('.') {
                if root.eq_ignore_ascii_case("temp") {
                    set_property(&self.temp, property, value);
                    continue;
                }
            }
            self.scopes
                .last_mut()
                .expect("scope exists")
                .insert(name.to_ascii_lowercase(), value);
        }
        let mut result = DynValue::Number(0.0);
        for statement in &function.body {
            match self.eval_statement(statement) {
                Flow::Return(value) => {
                    result = value;
                    break;
                }
                Flow::Value(value) => result = value,
                Flow::Break | Flow::Continue => break,
            }
            if self.suspended {
                break;
            }
        }
        self.scopes.pop();
        self.receiver = old_receiver;
        self.call_stack.pop();
        result
    }

    fn invoke_builtin(&mut self, name: &str, args: Vec<DynValue>, receiver: DynValue) -> DynValue {
        let is_method = name.starts_with("method:");
        let operation = name
            .strip_prefix("method:")
            .unwrap_or(name)
            .to_ascii_lowercase();
        let argument = |index: usize| args.get(index).cloned().unwrap_or(DynValue::Undefined);
        let strings = || args.iter().map(value_string).collect::<Vec<_>>();
        let current_account = value_string(&get_property(&self.current_player, "account"));
        let receiver_account = match &receiver {
            DynValue::Object(object) => match &object.borrow().kind {
                ObjectKind::Player { account, .. } => account.clone(),
                _ => value_string(&get_property(&receiver, "account")),
            },
            _ => current_account.clone(),
        };
        if is_method && operation == "tostring" {
            return match &receiver {
                DynValue::Object(_) => {
                    if matches!(receiver, DynValue::Object(ref object) if matches!(object.borrow().kind, ObjectKind::Date))
                    {
                        return DynValue::String(format_date_value(&receiver));
                    }
                    let object_type = get_property(&receiver, "objecttype");
                    if value_string(&object_type).eq_ignore_ascii_case("tserverplayer")
                        || matches!(receiver, DynValue::Object(ref object) if matches!(object.borrow().kind, ObjectKind::Player { .. }))
                    {
                        DynValue::String(value_string(&get_property(&receiver, "account")))
                    } else {
                        DynValue::String(value_string(&receiver))
                    }
                }
                _ => DynValue::String(value_string(&receiver)),
            };
        }
        if is_method && operation == "sendrequest" {
            self.perform_http_request(&receiver);
            return DynValue::Undefined;
        }
        if is_method
            && matches!(
                receiver,
                DynValue::Object(ref object)
                    if matches!(object.borrow().kind, ObjectKind::SQLite { .. })
            )
        {
            let db_name = value_string(&get_property(&receiver, "path"));
            let db_name = if db_name.is_empty() {
                value_string(&get_property(&receiver, "name"))
            } else {
                db_name
            };
            match operation.as_str() {
                "open" => {
                    let requested = value_string(&argument(0));
                    let name = if requested.is_empty() {
                        db_name
                    } else {
                        requested
                    };
                    match sqlite_database_path(&self.config.file_root, &name)
                        .and_then(|path| sqlite_ffi::touch(&path))
                    {
                        Ok(()) => {
                            set_property(&receiver, "path", DynValue::String(name));
                            set_property(&receiver, "isopen", DynValue::Bool(true));
                            set_property(&receiver, "error", DynValue::String(String::new()));
                            return DynValue::Bool(true);
                        }
                        Err(error) => {
                            set_property(&receiver, "error", DynValue::String(error));
                            set_property(&receiver, "isopen", DynValue::Bool(false));
                            return DynValue::Bool(false);
                        }
                    }
                }
                "exec" => {
                    let request = self.sql_request(&db_name, &value_string(&argument(0)), false);
                    set_property(&receiver, "error", get_property(&request, "error"));
                    set_property(
                        &receiver,
                        "lastinsertid",
                        get_property(&request, "lastinsertid"),
                    );
                    return DynValue::Bool(
                        value_string(&get_property(&request, "error")).is_empty(),
                    );
                }
                "query" => {
                    let request = self.sql_request(&db_name, &value_string(&argument(0)), true);
                    set_property(&receiver, "error", get_property(&request, "error"));
                    return get_property(&request, "rows");
                }
                "close" => {
                    set_property(&receiver, "isopen", DynValue::Bool(false));
                    return DynValue::Undefined;
                }
                _ => {}
            }
        }
        if is_method {
            let object_type = value_string(&get_property(&receiver, "objecttype"));
            if object_type.eq_ignore_ascii_case("twebsocket") {
                match operation.as_str() {
                    "connect" | "send" => return DynValue::Bool(false),
                    "close" | "destroy" => {
                        set_property(&receiver, "isconnected", DynValue::Bool(false));
                        return DynValue::Undefined;
                    }
                    _ => {}
                }
            }
            if object_type.eq_ignore_ascii_case("tdiscord") {
                match operation.as_str() {
                    "connect" => {
                        set_property(
                            &receiver,
                            "token",
                            DynValue::String(value_string(&argument(0))),
                        );
                        set_property(&receiver, "isconnected", DynValue::Bool(false));
                        return DynValue::Bool(false);
                    }
                    "sendmessage" | "sendembed" => return DynValue::Bool(false),
                    "close" | "destroy" => {
                        set_property(&receiver, "isconnected", DynValue::Bool(false));
                        return DynValue::Undefined;
                    }
                    _ => {}
                }
            }
        }
        let missing_player = matches!(
            receiver,
            DynValue::Object(ref object)
                if matches!(object.borrow().kind, ObjectKind::Player { ref account, .. } if account.is_empty())
        );
        if missing_player
            && matches!(
                operation.as_str(),
                "sendpm"
                    | "sendplayer"
                    | "sendtorc"
                    | "sendtoirc"
                    | "sendrpgmessage"
                    | "message"
                    | "say2"
                    | "setbody"
                    | "sethead"
                    | "setsword"
                    | "setshield"
                    | "setplayerdir"
                    | "freezeplayer"
                    | "freezeplayer2"
                    | "unfreezeplayer"
                    | "hurt"
                    | "setlevel"
                    | "setlevel2"
                    | "addweapon"
                    | "toweapons"
                    | "removeweapon"
                    | "join"
                    | "attachplayertoobj"
                    | "detachplayer"
                    | "showemoticon"
                    | "showemoticonbykey"
                    | "hideemoticon"
                    | "hidesign"
                    | "scrollsign"
            )
        {
            return DynValue::Undefined;
        }

        if !is_method {
            match operation.as_str() {
                "join" => {
                    self.add_joined_class(&self.owner.clone(), &value_string(&argument(0)));
                    return DynValue::Undefined;
                }
                "leave" => {
                    let wanted = value_string(&argument(0));
                    let classes = self
                        .joined_classes(&self.owner)
                        .into_iter()
                        .filter(|value| !value.eq_ignore_ascii_case(&wanted))
                        .map(DynValue::String)
                        .collect();
                    self.set_named_property(
                        &self.owner.clone(),
                        "__classes",
                        DynValue::array(classes),
                    );
                    return DynValue::Undefined;
                }
                "isinclass" => {
                    return DynValue::Bool(
                        self.joined_classes(&self.owner)
                            .iter()
                            .any(|value| value.eq_ignore_ascii_case(&value_string(&argument(0)))),
                    );
                }
                _ => {}
            }
        }

        if operation == "echo" || operation == "trace" {
            self.result.output.push(if args.len() == 1 {
                output_value_string(&argument(0))
            } else {
                strings().join(" ")
            });
            return DynValue::Undefined;
        }

        // A few names are both global functions and receiver methods.  The
        // global forms take their value from argument zero, while the method
        // forms operate on the receiver (`text.lower()`, `text.starts(...)`,
        // and so on).  Resolve these before the global utility table.
        if is_method {
            match operation.as_str() {
                "lower" | "lowercase" => {
                    return DynValue::String(value_string(&receiver).to_lowercase());
                }
                "upper" | "uppercase" => {
                    return DynValue::String(value_string(&receiver).to_uppercase());
                }
                "contains" | "strcontains" => {
                    return DynValue::Number(
                        value_string(&receiver).contains(&value_string(&argument(0))) as i32 as f64,
                    );
                }
                "starts" | "startswith" => {
                    return DynValue::Number(
                        value_string(&receiver).starts_with(&value_string(&argument(0))) as i32
                            as f64,
                    );
                }
                "ends" | "endswith" => {
                    return DynValue::Number(
                        value_string(&receiver).ends_with(&value_string(&argument(0))) as i32
                            as f64,
                    );
                }
                "join" if matches!(receiver, DynValue::Array(_)) => {
                    return DynValue::String(
                        array_values(&receiver)
                            .iter()
                            .map(value_string)
                            .collect::<Vec<_>>()
                            .join(&value_string(&argument(0))),
                    );
                }
                "indexof" | "aindexof" if matches!(receiver, DynValue::Array(_)) => {
                    return DynValue::Number(
                        array_values(&receiver)
                            .iter()
                            .position(|item| equal_values(item, &argument(0)))
                            .map_or(-1.0, |index| index as f64),
                    );
                }
                "addarray" => {
                    if let DynValue::Array(values) = &receiver {
                        values.borrow_mut().extend(array_values(&argument(0)));
                    }
                    return receiver;
                }
                "insert" => {
                    array_insert(&receiver, number_i64(&argument(0)), argument(1));
                    return receiver;
                }
                "replace" => {
                    let index = number_i64(&argument(0));
                    if let DynValue::Array(values) = &receiver {
                        if index >= 0 && (index as usize) < values.borrow().len() {
                            values.borrow_mut()[index as usize] = argument(1);
                        }
                    }
                    return receiver;
                }
                "index" => {
                    let needle = value_string(&argument(0));
                    return DynValue::Number(
                        array_values(&receiver)
                            .iter()
                            .position(|item| value_string(item) == needle)
                            .map_or(-1.0, |index| index as f64),
                    );
                }
                "indices" => {
                    let needle = value_string(&argument(0));
                    return DynValue::array(
                        array_values(&receiver)
                            .iter()
                            .enumerate()
                            .filter(|(_, item)| value_string(item) == needle)
                            .map(|(index, _)| DynValue::Number(index as f64))
                            .collect(),
                    );
                }
                "splice" => {
                    return array_splice(
                        &receiver,
                        number_i64(&argument(0)),
                        args.get(1).map(number_i64),
                        args.iter().skip(2).cloned().collect(),
                    );
                }
                "insertarray" => {
                    array_insert_values(
                        &receiver,
                        number_i64(&argument(0)),
                        array_values(&argument(1)),
                    );
                    return receiver;
                }
                "subarray" => {
                    let values = array_values(&receiver);
                    let start = number_i64(&argument(0)).max(0) as usize;
                    let start = start.min(values.len());
                    let end = args
                        .get(1)
                        .map(|length| start.saturating_add(number_i64(length).max(0) as usize))
                        .unwrap_or(values.len())
                        .min(values.len());
                    return DynValue::array(values[start..end].to_vec());
                }
                "subarray2" => {
                    let values = array_values(&receiver);
                    let mut index = number_i64(&argument(0)).max(0) as usize;
                    let count = number_i64(&argument(1)).max(0) as usize;
                    let step = number_i64(&argument(2)).max(1) as usize;
                    let limit = number_i64(&argument(3)).max(0) as usize;
                    let mut output = Vec::new();
                    while index < values.len() && output.len() < count {
                        output.push(values[index].clone());
                        if limit > 0 && output.len() >= limit {
                            break;
                        }
                        index = index.saturating_add(step);
                    }
                    return DynValue::array(output);
                }
                "sortascending" | "sortdescending" | "sort" => {
                    let mut values = array_values(&receiver);
                    if operation == "sort" {
                        if let Some(callback) = args.first() {
                            if matches!(callback, DynValue::Function(_) | DynValue::Builtin(_)) {
                                values.sort_by(|left, right| {
                                    let result = self.invoke_value(
                                        callback.clone(),
                                        DynValue::Undefined,
                                        vec![left.clone(), right.clone()],
                                    );
                                    number_f64(&result)
                                        .partial_cmp(&0.0)
                                        .unwrap_or(std::cmp::Ordering::Equal)
                                });
                            } else {
                                values.sort_by(|left, right| {
                                    value_string(left).cmp(&value_string(right))
                                });
                            }
                        } else {
                            values.sort_by(|left, right| {
                                value_string(left).cmp(&value_string(right))
                            });
                        }
                    } else {
                        values.sort_by(|left, right| {
                            let ordering = value_string(left).cmp(&value_string(right));
                            if operation == "sortdescending" {
                                ordering.reverse()
                            } else {
                                ordering
                            }
                        });
                    }
                    if let DynValue::Array(target) = &receiver {
                        *target.borrow_mut() = values;
                    }
                    return receiver;
                }
                "sortbyvalue" => {
                    let key = value_string(&argument(0));
                    let sort_type = value_string(&argument(1)).to_ascii_lowercase();
                    let ascending = args.get(2).map_or(true, DynValue::truthy);
                    let mut values = array_values(&receiver);
                    values.sort_by(|left, right| {
                        let left_value = get_property(left, &key);
                        let right_value = get_property(right, &key);
                        let ordering = if matches!(sort_type.as_str(), "int" | "float" | "double") {
                            number_f64(&left_value)
                                .partial_cmp(&number_f64(&right_value))
                                .unwrap_or(std::cmp::Ordering::Equal)
                        } else {
                            value_string(&left_value).cmp(&value_string(&right_value))
                        };
                        if ascending {
                            ordering
                        } else {
                            ordering.reverse()
                        }
                    });
                    if let DynValue::Array(target) = &receiver {
                        *target.borrow_mut() = values;
                    }
                    return receiver;
                }
                "map" | "filter" | "some" | "find" => {
                    let Some(callback) = args.first().cloned() else {
                        return if operation == "find" {
                            DynValue::Undefined
                        } else if operation == "some" {
                            DynValue::Bool(false)
                        } else {
                            DynValue::array(Vec::new())
                        };
                    };
                    let values = array_values(&receiver);
                    let mut mapped = Vec::new();
                    for (index, item) in values.iter().enumerate() {
                        let matched = self.invoke_value(
                            callback.clone(),
                            DynValue::Undefined,
                            vec![
                                item.clone(),
                                DynValue::Number(index as f64),
                                receiver.clone(),
                            ],
                        );
                        match operation.as_str() {
                            "map" => mapped.push(matched),
                            "filter" if matched.truthy() => mapped.push(item.clone()),
                            "some" if matched.truthy() => return DynValue::Bool(true),
                            "find" if matched.truthy() => return item.clone(),
                            _ => {}
                        }
                    }
                    return if operation == "some" {
                        DynValue::Bool(false)
                    } else if operation == "find" {
                        DynValue::Undefined
                    } else {
                        DynValue::array(mapped)
                    };
                }
                _ => {}
            }
        }

        if matches!(operation.as_str(), "clearvars" | "clearemptyvars") {
            if let Some(object) = receiver.object_ref() {
                let mut object = object.borrow_mut();
                if operation == "clearvars" {
                    object
                        .properties
                        .retain(|key, _| key.eq_ignore_ascii_case("__gs2value"));
                } else {
                    object.properties.retain(|key, value| {
                        key.starts_with("__")
                            || !value.is_undefined_or_empty()
                            || matches!(value, DynValue::Array(_) | DynValue::Object(_))
                    });
                }
            }
            // The object prototype returns call.This, allowing fluent
            // object helper calls.
            return receiver;
        }

        if is_method
            && matches!(
                operation.as_str(),
                "loadfolder"
                    | "loadlines"
                    | "loadstring"
                    | "loadvars"
                    | "loadini"
                    | "loadvarsfromarray"
                    | "savevars"
                    | "savevarstoarray"
                    | "savelines"
                    | "savestring"
                    | "savejson"
                    | "savejsontostring"
                    | "savexml"
                    | "savexmltostring"
                    | "loadjsonfromstring"
                    | "loadxmlfromstring"
                    | "loadjson"
                    | "loadxml"
            )
        {
            return self.invoke_file_method(&operation, receiver, &args);
        }

        if is_method && matches!(receiver, DynValue::Object(_) | DynValue::Array(_)) {
            match operation.as_str() {
                "objecttype" => {
                    // `objecttype` is both a prototype method and an
                    // optional stored property.  Looking it up through the
                    // normal property path would see the prototype method
                    // itself and incorrectly return `function()` for a
                    // plain TStaticVar object.
                    let value = match &receiver {
                        DynValue::Object(object) => {
                            let object = object.borrow();
                            property_key(&object.properties, "objecttype")
                                .and_then(|key| object.properties.get(&key).cloned())
                        }
                        DynValue::Array(_) => {
                            let value = self.get_array_member(&receiver, "objecttype");
                            (!value.is_undefined()).then_some(value)
                        }
                        _ => None,
                    };
                    let value = value.unwrap_or_else(|| DynValue::String("TgraalVar".to_string()));
                    return value;
                }
                "joinedclasses" => return self.named_property(&receiver, "__classes"),
                "isinclass" => {
                    return DynValue::Bool(
                        self.joined_classes(&receiver)
                            .iter()
                            .any(|value| value.eq_ignore_ascii_case(&value_string(&argument(0)))),
                    );
                }
                "join" => {
                    let class = value_string(&argument(0));
                    if self.imported_receiver(&receiver) {
                        self.install_import_module(&receiver, &class);
                    }
                    self.add_joined_class(&receiver, &class);
                    return receiver;
                }
                "leave" => {
                    let wanted = value_string(&argument(0));
                    if self.imported_receiver(&receiver) {
                        self.leave_import_module(&receiver, &wanted);
                    }
                    let classes = self
                        .joined_classes(&receiver)
                        .into_iter()
                        .filter(|value| !value.eq_ignore_ascii_case(&wanted))
                        .map(DynValue::String)
                        .collect();
                    self.set_named_property(&receiver, "__classes", DynValue::array(classes));
                    return receiver;
                }
                "getdynamicvarnames" | "getvarnames" | "geteditvarnames" => {
                    let mut names = self
                        .named_properties(&receiver)
                        .into_iter()
                        .filter(|(key, value)| {
                            !key.starts_with("__")
                                && key != "length"
                                && !matches!(value, DynValue::Function(_) | DynValue::Builtin(_))
                        })
                        .map(|(key, _)| key)
                        .collect::<Vec<_>>();
                    names.sort_by_key(|value| value.to_ascii_lowercase());
                    return DynValue::array(names.into_iter().map(DynValue::String).collect());
                }
                "getstaticvarnames" => {
                    return DynValue::array(
                        ["toString"]
                            .into_iter()
                            .map(|value| DynValue::String(value.to_string()))
                            .collect(),
                    );
                }
                "getfunctions" => {
                    let mut names = object_properties(&receiver)
                        .into_iter()
                        .filter(|(key, value)| {
                            !key.starts_with("__")
                                && matches!(value, DynValue::Function(_) | DynValue::Builtin(_))
                        })
                        .map(|(key, _)| key)
                        .collect::<Vec<_>>();
                    names.extend(
                        object_prototype_methods()
                            .iter()
                            .map(|value| (*value).to_string()),
                    );
                    if matches!(receiver, DynValue::Object(_) | DynValue::Array(_)) {
                        names.push("toString".to_string());
                    }
                    if matches!(receiver, DynValue::Array(_)) {
                        names.extend(
                            array_prototype_methods()
                                .iter()
                                .map(|value| (*value).to_string()),
                        );
                    }
                    names.sort_by_key(|value| value.to_ascii_lowercase());
                    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
                    return DynValue::array(names.into_iter().map(DynValue::String).collect());
                }
                "hasfunction" => {
                    let wanted = value_string(&argument(0));
                    return DynValue::Bool(matches!(
                        self.named_property(&receiver, &wanted),
                        DynValue::Function(_) | DynValue::Builtin(_)
                    ));
                }
                "copyfrom" => {
                    let source = argument(0);
                    let source_properties = self.named_properties(&source);
                    self.clear_named_properties(&receiver);
                    for (key, value) in source_properties {
                        if !key.starts_with("__") {
                            self.set_named_property(&receiver, &key, value);
                        }
                    }
                    return receiver;
                }
                "trigger" => {
                    let event = value_string(&argument(0));
                    let callback = self.named_property(&receiver, &event);
                    let callback =
                        if matches!(callback, DynValue::Function(_) | DynValue::Builtin(_)) {
                            callback
                        } else {
                            self.named_property(&receiver, &format!("on{event}"))
                        };
                    if matches!(callback, DynValue::Function(_) | DynValue::Builtin(_)) {
                        self.invoke_value(
                            callback,
                            receiver.clone(),
                            args.iter().skip(1).cloned().collect(),
                        );
                    }
                    let key = format!("__catch_{}", event.to_ascii_lowercase());
                    for listener in array_values(&self.named_property(&receiver, &key)) {
                        let target = get_property(&listener, "listener");
                        let function = value_string(&get_property(&listener, "function"));
                        if !target.is_undefined() && !function.is_empty() {
                            let mut call_args = vec![receiver.clone()];
                            call_args.extend(args.iter().skip(1).cloned());
                            self.invoke_named(&function, call_args, target);
                        }
                    }
                    return DynValue::Undefined;
                }
                "catchevent" => {
                    let target = argument(0);
                    let event = value_string(&argument(1));
                    let function = value_string(&argument(2));
                    if matches!(target, DynValue::Object(_) | DynValue::Array(_)) {
                        let key = format!("__catch_{}", event.to_ascii_lowercase());
                        let mut listeners = array_values(&self.named_property(&target, &key));
                        let entry = DynValue::plain();
                        set_property(&entry, "listener", receiver.clone());
                        set_property(&entry, "function", DynValue::String(function));
                        listeners.push(entry);
                        self.set_named_property(&target, &key, DynValue::array(listeners));
                    }
                    return DynValue::Bool(true);
                }
                "ignoreevent" | "ignoreevents" => {
                    if operation == "ignoreevents" {
                        match &receiver {
                            DynValue::Object(object) => object
                                .borrow_mut()
                                .properties
                                .retain(|key, _| !key.starts_with("__catch_")),
                            DynValue::Array(_) => {
                                if let Some(key) = Self::array_property_key(&receiver) {
                                    if let Some(properties) = self.array_properties.get_mut(&key) {
                                        properties.retain(|name, _| !name.starts_with("__catch_"));
                                    }
                                }
                            }
                            _ => {}
                        }
                    } else {
                        let key = format!(
                            "__catch_{}",
                            value_string(&argument(0)).to_ascii_lowercase()
                        );
                        match &receiver {
                            DynValue::Object(object) => {
                                object.borrow_mut().properties.remove(&key);
                            }
                            DynValue::Array(_) => {
                                if let Some(array_key) = Self::array_property_key(&receiver) {
                                    if let Some(properties) =
                                        self.array_properties.get_mut(&array_key)
                                    {
                                        properties.remove(&key);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    return receiver;
                }
                _ => {}
            }
        }

        // The socket host methods are installed both on the socket object and as
        // globals while a socket script is running.
        if matches!(
            operation.as_str(),
            "bind"
                | "connect"
                | "close"
                | "destroy"
                | "send"
                | "senddata"
                | "sendudp"
                | "join"
                | "trigger"
        ) && matches!(receiver, DynValue::Object(ref object) if matches!(object.borrow().kind, ObjectKind::Socket))
        {
            let socket = receiver.clone();
            if matches!(socket, DynValue::Object(ref object) if matches!(object.borrow().kind, ObjectKind::Socket))
            {
                let socket_name = value_string(&get_property(&socket, "__tsocket_name"));
                let socket_id = value_string(&get_property(&socket, "__tsocket_id"));
                let mut action = SocketAction {
                    action: match operation.as_str() {
                        "senddata" => "send".to_string(),
                        "destroy" => "close".to_string(),
                        _ => operation.clone(),
                    },
                    name: socket_name,
                    id: socket_id,
                    ..SocketAction::default()
                };
                match operation.as_str() {
                    "bind" => {
                        action.port = number_i32(&argument(0));
                        action.package_delimiter =
                            value_string(&get_property(&socket, "packagedelimiter"));
                        action.udp = argument(1).truthy();
                        if let Some(bind) = self.config.socket_bind.clone() {
                            match bind(action.clone()) {
                                Ok(context) => {
                                    action.port = context.port;
                                    action.prepared = true;
                                    set_property(
                                        &socket,
                                        "port",
                                        DynValue::Number(context.port as f64),
                                    );
                                    set_property(
                                        &socket,
                                        "address",
                                        DynValue::String(context.address),
                                    );
                                    set_property(
                                        &socket,
                                        "ipaddress",
                                        DynValue::String(context.ip_address),
                                    );
                                    set_property(
                                        &socket,
                                        "isconnected",
                                        DynValue::Bool(context.is_connected),
                                    );
                                    set_property(&socket, "error", DynValue::String(String::new()));
                                }
                                Err(error) => {
                                    set_property(&socket, "error", DynValue::String(error))
                                }
                            }
                        }
                    }
                    "connect" => {
                        action.address = value_string(&argument(0));
                        action.port = number_i32(&argument(1));
                        action.package_delimiter =
                            value_string(&get_property(&socket, "packagedelimiter"));
                    }
                    "send" | "senddata" => action.data = value_string(&argument(0)),
                    "sendudp" => {
                        action.data = value_string(&argument(0));
                        action.address = value_string(&argument(1));
                        action.port = number_i32(&argument(2));
                        action.udp = true;
                    }
                    "join" => {
                        let class = value_string(&argument(0)).trim().to_string();
                        add_socket_class(&socket, &class);
                        let resolver = self.config.socket_class_resolver.clone();
                        install_socket_class_methods(&socket, &[class], &resolver);
                        action.joined_classes = socket_joined_classes(&socket);
                    }
                    "trigger" => {
                        action.event = value_string(&argument(0));
                        action.params = args.iter().skip(1).map(value_string).collect();
                    }
                    _ => {}
                }
                let index = self.result.socket_actions.len();
                self.result.socket_actions.push(action);
                self.socket_refs.push((index, socket));
                return if operation == "join" {
                    self.result
                        .socket_actions
                        .last()
                        .map(|_| receiver)
                        .unwrap_or(DynValue::Undefined)
                } else {
                    DynValue::Undefined
                };
            }
        }

        if is_method
            && matches!(
                receiver,
                DynValue::Object(ref object)
                    if matches!(object.borrow().kind, ObjectKind::PutNPC { .. })
            )
        {
            let Some(object) = receiver.object_ref() else {
                return DynValue::Undefined;
            };
            let index = number_i64(&get_property(&receiver, "__putnpc_index"));
            if index < 0 || index as usize >= self.result.level_actions.len() {
                return DynValue::Undefined;
            }
            return match operation.as_str() {
                "join" => {
                    add_socket_class(&receiver, &value_string(&argument(0)));
                    if let Some(action) = self.result.level_actions.get_mut(index as usize) {
                        action.classes = socket_joined_classes(&receiver);
                    }
                    receiver
                }
                "leave" => receiver,
                "destroy" => {
                    if let Some(action) = self.result.level_actions.get_mut(index as usize) {
                        action.action.clear();
                    }
                    DynValue::Undefined
                }
                _ => {
                    if let Some(action) = self.result.level_actions.get_mut(index as usize) {
                        action.calls.push(NPCFunctionCall {
                            function: operation.clone(),
                            args: args.iter().map(value_string).collect(),
                            ..NPCFunctionCall::default()
                        });
                    }
                    let _ = object;
                    DynValue::Undefined
                }
            };
        }

        // TPlayer/TServerPlayer methods.  These are intentionally represented
        // as result records, just as the host functions are.
        if matches!(operation.as_str(), "sendpm" | "sendplayer") && !receiver_account.is_empty() {
            let message = value_string(&argument(0));
            if !message.is_empty() {
                self.result.player_messages.push(PlayerMessage {
                    account: receiver_account.clone(),
                    message,
                });
            }
            return DynValue::Undefined;
        }
        if is_method && matches!(operation.as_str(), "sendtorc") {
            let message = value_string(&argument(0));
            if !message.is_empty() {
                self.result.player_rc_messages.push(PlayerMessage {
                    account: receiver_account.clone(),
                    message,
                });
            }
            return DynValue::Undefined;
        }
        if is_method && matches!(operation.as_str(), "sendtoirc") {
            let values = strings();
            if let Some(command) = values.first() {
                self.result.player_irc_messages.push(IRCMessage {
                    account: receiver_account.clone(),
                    command: command.clone(),
                    params: values.iter().skip(1).cloned().collect(),
                });
            }
            return DynValue::Undefined;
        }
        if operation == "sendrpgmessage" {
            let target_account = if is_method {
                receiver_account
            } else {
                current_account
            };
            if !target_account.is_empty() {
                self.result.player_messages.push(PlayerMessage {
                    account: target_account,
                    message: value_string(&argument(0)),
                });
            }
            return DynValue::Undefined;
        }
        if matches!(operation.as_str(), "message" | "say2") {
            let text = value_string(&argument(0));
            if self.config.npc_id != 0 {
                self.result.npc_actions.push(NPCAction {
                    id: self.config.npc_id,
                    chat: text,
                    has_chat: true,
                    ..NPCAction::default()
                });
            } else if !receiver_account.is_empty() {
                self.result.player_props.push(PlayerProp {
                    account: receiver_account,
                    name: "chat".to_string(),
                    value: text,
                });
            }
            return DynValue::Undefined;
        }
        if matches!(
            operation.as_str(),
            "setbody"
                | "sethead"
                | "setsword"
                | "setshield"
                | "setplayerdir"
                | "freezeplayer"
                | "freezeplayer2"
                | "unfreezeplayer"
                | "hurt"
        ) {
            let target_account = if is_method && !receiver_account.is_empty() {
                receiver_account.clone()
            } else {
                current_account.clone()
            };
            if !target_account.is_empty() {
                let amount = if args.len() == 1 {
                    number_f64(&argument(0))
                } else {
                    number_f64(&argument(1))
                };
                let action_name = if operation == "freezeplayer2" {
                    "freezeplayer"
                } else if operation == "unfreezeplayer" {
                    "unfreezeplayer"
                } else {
                    operation.as_str()
                };
                self.result.player_effects.push(PlayerEffect {
                    account: target_account,
                    action: action_name.to_string(),
                    value: value_string(&argument(0)),
                    amount,
                });
            }
            return DynValue::Undefined;
        }
        if matches!(operation.as_str(), "setlevel" | "setlevel2") {
            self.result.player_warps.push(PlayerWarp {
                account: if is_method && !receiver_account.is_empty() {
                    receiver_account.clone()
                } else {
                    current_account.clone()
                },
                level: value_string(&argument(0)),
                x: if operation == "setlevel2" {
                    number_f64(&argument(1))
                } else {
                    0.0
                },
                y: if operation == "setlevel2" {
                    number_f64(&argument(2))
                } else {
                    0.0
                },
            });
            return DynValue::Undefined;
        }
        if matches!(operation.as_str(), "addweapon" | "toweapons") {
            self.result.player_weapons.push(PlayerWeapon {
                account: if is_method && !receiver_account.is_empty() {
                    receiver_account.clone()
                } else {
                    current_account.clone()
                },
                name: value_string(&argument(0)),
                add: true,
            });
            return DynValue::Undefined;
        }
        if matches!(operation.as_str(), "removeweapon") {
            self.result.player_weapons.push(PlayerWeapon {
                account: if is_method && !receiver_account.is_empty() {
                    receiver_account.clone()
                } else {
                    current_account.clone()
                },
                name: value_string(&argument(0)),
                add: false,
            });
            return DynValue::Undefined;
        }
        if operation == "join"
            && matches!(receiver, DynValue::Object(ref object) if matches!(object.borrow().kind, ObjectKind::Player { .. }))
        {
            self.result.player_classes.push(PlayerClass {
                account: receiver_account,
                name: value_string(&argument(0)),
            });
            return receiver;
        }

        if is_method
            && matches!(
                receiver,
                DynValue::Object(ref object)
                    if matches!(object.borrow().kind, ObjectKind::Player { .. })
            )
        {
            match operation.as_str() {
                "hasrightflag" => {
                    return DynValue::Bool(player_has_right_flag_value(
                        &receiver,
                        &value_string(&argument(0)),
                    ));
                }
                "hasright" => {
                    return DynValue::Bool(player_has_folder_right_value(
                        &receiver,
                        &value_string(&argument(0)),
                        &value_string(&argument(1)),
                    ));
                }
                "getnohit" => return DynValue::Bool(false),
                "showemoticon" | "showemoticonbykey" | "hideemoticon" | "hidesign"
                | "scrollsign" => {
                    let amount = if args.len() == 1 {
                        number_f64(&argument(0))
                    } else {
                        number_f64(&argument(1))
                    };
                    self.result.player_effects.push(PlayerEffect {
                        account: receiver_account.clone(),
                        action: operation.clone(),
                        value: value_string(&argument(0)),
                        amount,
                    });
                    return DynValue::Undefined;
                }
                "findweapon" => {
                    let wanted = value_string(&argument(0));
                    return if self.result.player_weapons.iter().any(|weapon| {
                        weapon.account.eq_ignore_ascii_case(&receiver_account)
                            && weapon.name.eq_ignore_ascii_case(&wanted)
                            && weapon.add
                    }) {
                        DynValue::String(wanted)
                    } else {
                        DynValue::Null
                    };
                }
                "requesttext" => {
                    let context = player_context_from_object(&receiver);
                    return DynValue::String(request_text_value(
                        &value_string(&argument(0)),
                        &value_string(&argument(1)),
                        &context,
                    ));
                }
                "sendping" => {
                    let object = DynValue::plain();
                    set_property(
                        &object,
                        "objecttype",
                        DynValue::String("TPingRequest".to_string()),
                    );
                    set_property(&object, "completed", DynValue::Bool(true));
                    set_property(&object, "time", DynValue::Number(0.0));
                    set_property(&object, "__event_onReceivePing", DynValue::Bool(true));
                    return object;
                }
                "attachplayertoobj" => {
                    self.result.player_attachments.push(PlayerAttachment {
                        account: receiver_account,
                        object_id: number_i64(&argument(0)) as u32,
                        offset_x: number_f64(&argument(1)),
                        offset_y: number_f64(&argument(2)),
                        detached: false,
                    });
                    return DynValue::Undefined;
                }
                "detachplayer" => {
                    self.result.player_attachments.push(PlayerAttachment {
                        account: receiver_account,
                        detached: true,
                        ..PlayerAttachment::default()
                    });
                    return DynValue::Undefined;
                }
                "destroy" => {
                    if matches!(
                        receiver,
                        DynValue::Object(ref object)
                            if matches!(object.borrow().kind, ObjectKind::Player { server_player: true, .. })
                    ) {
                        set_property(&receiver, "__destroyed", DynValue::Bool(true));
                    }
                    return DynValue::Undefined;
                }
                _ => {}
            }
        }

        // NPC methods and globals.
        if matches!(
            operation.as_str(),
            "hide"
                | "show"
                | "dontblock"
                | "dontblocklocal"
                | "blockagain"
                | "blockagainlocal"
                | "drawoverplayer"
                | "drawunderplayer"
        ) {
            if self.config.npc_id != 0 {
                let value = match operation.as_str() {
                    "hide" => 0,
                    "show" => 1,
                    "drawoverplayer" => 3,
                    "drawunderplayer" => 5,
                    _ => 0,
                };
                if matches!(
                    operation.as_str(),
                    "hide" | "show" | "drawoverplayer" | "drawunderplayer"
                ) {
                    set_property(&self.owner, "__hasvisflags", DynValue::Bool(true));
                    set_property(&self.owner, "__visflags", DynValue::Number(value as f64));
                } else {
                    set_property(&self.owner, "__hasblockflags", DynValue::Bool(true));
                    set_property(
                        &self.owner,
                        "__blockflags",
                        DynValue::Number(if operation.starts_with("dont") {
                            1.0
                        } else {
                            0.0
                        }),
                    );
                }
            }
            return DynValue::Undefined;
        }
        if matches!(
            operation.as_str(),
            "destroy"
                | "canwarp"
                | "canwarp2"
                | "cannotwarp"
                | "drawaslight"
                | "canbecarried"
                | "cannotbecarried"
                | "canbepulled"
                | "cannotbepulled"
                | "canbepushed"
                | "cannotbepushed"
        ) {
            if self.config.npc_id != 0 {
                match operation.as_str() {
                    "destroy" => set_property(&self.owner, "__destroy", DynValue::Bool(true)),
                    "cannotwarp" => {
                        set_property(&self.owner, "__npcflag_canwarp", DynValue::Bool(false));
                        set_property(&self.owner, "__npcflag_canwarp2", DynValue::Bool(false));
                    }
                    "canwarp" | "canwarp2" => set_property(
                        &self.owner,
                        &format!("__npcflag_{operation}"),
                        DynValue::Bool(true),
                    ),
                    value if value.starts_with("cannot") => {
                        let flag = value
                            .strip_prefix("cannot")
                            .map_or_else(|| value.to_string(), |suffix| format!("can{suffix}"));
                        set_property(
                            &self.owner,
                            &format!("__npcflag_{flag}"),
                            DynValue::Bool(false),
                        )
                    }
                    value => set_property(
                        &self.owner,
                        &format!("__npcflag_{value}"),
                        DynValue::Bool(true),
                    ),
                }
            }
            return DynValue::Undefined;
        }
        if operation == "showcharacter" && self.config.npc_id != 0 {
            for (key, value) in [
                ("image", "#c#"),
                ("headimg", "head0.png"),
                ("bodyimg", "body.png"),
                ("shieldimg", "shield1.png"),
                ("swordimg", "sword1.png"),
                ("ani", "idle"),
            ] {
                set_property(&self.owner, key, DynValue::String(value.to_string()));
            }
            set_property(
                &self.owner,
                "colors",
                DynValue::array(
                    ["2", "5", "21", "5", "21"]
                        .into_iter()
                        .map(|x| DynValue::String(x.to_string()))
                        .collect(),
                ),
            );
            set_property(&self.owner, "width", DynValue::Number(32.0));
            set_property(&self.owner, "height", DynValue::Number(48.0));
            return DynValue::Undefined;
        }
        if matches!(
            operation.as_str(),
            "carryobject"
                | "lay"
                | "take"
                | "take2"
                | "takehorse"
                | "showani"
                | "showani2"
                | "showpoly"
                | "showpoly2"
                | "changeimgcolors"
                | "changeimgvis"
                | "changeimgzoom"
        ) {
            if self.config.npc_id != 0 {
                set_property(
                    &self.owner,
                    &format!("__npcaction_{operation}"),
                    DynValue::String(strings().join("\n")),
                );
            }
            return DynValue::Undefined;
        }
        if operation == "throwcarry" {
            if self.config.npc_id != 0 {
                set_property(&self.owner, "__npcaction_throwcarry", DynValue::Bool(true));
            }
            return DynValue::Undefined;
        }
        if operation == "save" {
            if self.config.npc_id != 0 {
                self.result.npc_actions.push(NPCAction {
                    id: self.config.npc_id,
                    save_props: export_value(&self.owner, Some(&self.owner)),
                    save: true,
                    ..NPCAction::default()
                });
            }
            return DynValue::Undefined;
        }
        if operation == "setimg" || operation == "setimgpart" {
            let image = value_string(&argument(0));
            set_property(&self.owner, "image", DynValue::String(image.clone()));
            if self.config.npc_id != 0 {
                self.result.npc_actions.push(NPCAction {
                    id: self.config.npc_id,
                    image,
                    image_part: if operation == "setimgpart" {
                        (1..=4).map(|x| number_i32(&argument(x))).collect()
                    } else {
                        Vec::new()
                    },
                    ..NPCAction::default()
                });
            }
            return DynValue::Undefined;
        }
        if operation == "setcharani" {
            let ani = value_string(&argument(0));
            set_property(&self.owner, "ani", DynValue::String(ani.clone()));
            if self.config.npc_id != 0 {
                self.result.npc_actions.push(NPCAction {
                    id: self.config.npc_id,
                    ani,
                    ani_params: value_lines(&argument(1)),
                    ..NPCAction::default()
                });
            }
            return DynValue::Undefined;
        }
        if operation == "setshape" || operation == "setshape2" {
            if self.config.npc_id != 0 {
                self.result.npc_actions.push(NPCAction {
                    id: self.config.npc_id,
                    shape_type: if operation == "setshape2" { 2 } else { 1 },
                    width: number_i32(&argument(if operation == "setshape2" { 0 } else { 1 })),
                    height: number_i32(&argument(if operation == "setshape2" { 1 } else { 2 })),
                    tile_types: if operation == "setshape2" {
                        value_lines(&argument(2))
                    } else {
                        Vec::new()
                    },
                    ..NPCAction::default()
                });
            }
            return DynValue::Undefined;
        }
        if operation == "warpto" {
            if self.config.npc_id != 0 {
                self.result.npc_actions.push(NPCAction {
                    id: self.config.npc_id,
                    warp_level: value_string(&argument(0)),
                    warp_x: number_f64(&argument(1)),
                    warp_y: number_f64(&argument(2)),
                    ..NPCAction::default()
                });
            }
            return DynValue::Undefined;
        }
        if operation == "move" {
            if self.config.npc_id != 0 {
                let options = number_i32(&argument(3));
                self.result.npc_actions.push(NPCAction {
                    id: self.config.npc_id,
                    move_dx: number_f64(&argument(0)),
                    move_dy: number_f64(&argument(1)),
                    move_time: number_f64(&argument(2)),
                    move_options: options,
                    ..NPCAction::default()
                });
                if options & 8 != 0 {
                    self.result.scheduled_events.push(ScheduledEvent {
                        event: "onMovementFinished".to_string(),
                        delay: number_f64(&argument(2)),
                        ..ScheduledEvent::default()
                    });
                }
            }
            return DynValue::Undefined;
        }

        if is_method
            && matches!(
                receiver,
                DynValue::Object(ref object)
                    if matches!(object.borrow().kind, ObjectKind::Level { .. })
            )
        {
            return self.level_method(&receiver, &operation, &args);
        }

        self.invoke_utility(&operation, args, receiver)
    }

    fn invoke_utility(
        &mut self,
        operation: &str,
        args: Vec<DynValue>,
        receiver: DynValue,
    ) -> DynValue {
        let argument = |index: usize| args.get(index).cloned().unwrap_or(DynValue::Undefined);
        let account = value_string(&get_property(&self.current_player, "account"));
        let level = self.config.player.get("level").cloned().unwrap_or_default();
        let current_level = level.clone();
        match operation {
            "__gs2str" => DynValue::String(coerce_string(&argument(0))),
            "__gs2eq" => DynValue::Number(equal_values(&argument(0), &argument(1)) as i32 as f64),
            "__gs2getpath" => {
                let root = argument(0);
                self.get_relative_path(&root, &value_string(&argument(1)))
            }
            "__gs2clearvarspath" => {
                let path = value_string(&argument(0));
                let target = self.get_script_path(&path);
                if target.is_undefined() || matches!(target, DynValue::Null) {
                    self.set_script_path(&path, DynValue::plain());
                    let target = self.get_script_path(&path);
                    clear_vm_vars(&target);
                    target
                } else {
                    clear_vm_vars(&target);
                    target
                }
            }
            "__gs2ensurearray" => {
                let path = value_string(&argument(0));
                let target = self.get_script_path(&path);
                if matches!(target, DynValue::Array(_)) {
                    target
                } else {
                    let replacement = DynValue::array(Vec::new());
                    self.set_script_path(&path, replacement.clone());
                    replacement
                }
            }
            "__gs2setdynamic" => {
                let path = value_string(&argument(0));
                let key = value_string(&argument(1));
                let target = self.get_script_path(&path);
                let target = if target.is_undefined() || matches!(target, DynValue::Null) {
                    let replacement = DynValue::plain();
                    self.set_script_path(&path, replacement.clone());
                    replacement
                } else {
                    target
                };
                self.set_property_value(&target, &key, argument(2));
                argument(2)
            }
            "__call dynamic" => DynValue::Undefined,
            "__calldynamic" => {
                let name = value_string(&argument(0));
                if name.trim().is_empty() {
                    DynValue::Undefined
                } else {
                    self.invoke_named(
                        &name,
                        args.into_iter().skip(1).collect(),
                        self.owner.clone(),
                    )
                }
            }
            "__gs2in" => {
                let wanted = value_string(&argument(0));
                DynValue::Bool(
                    array_values(&argument(1))
                        .iter()
                        .any(|candidate| value_string(candidate) == wanted),
                )
            }
            "__gs2looptick" => {
                self.loop_count = self.loop_count.saturating_add(1);
                if self.loop_count > self.loop_limit() {
                    self.result.err = "maxlooplimit exceeded".to_string();
                }
                DynValue::Undefined
            }
            "__gs2loadstringvar" => {
                if args.len() < 3 {
                    DynValue::Bool(false)
                } else {
                    let target = argument(0);
                    let key = value_string(&argument(1));
                    let file = value_string(&argument(2));
                    if !vm_file_has_right(&self.config.file_rights, &file, 'r') {
                        DynValue::Bool(false)
                    } else if let Some(text) = load_vm_string(&self.config.file_root, &file) {
                        self.set_property_value(&target, &key, DynValue::String(text.clone()));
                        set_var(self, &key, DynValue::String(text));
                        DynValue::Bool(true)
                    } else {
                        DynValue::Bool(false)
                    }
                }
            }
            "clearemptyglobalvars" => {
                self.globals.retain(|key, value| {
                    matches!(
                        key.as_str(),
                        "temp"
                            | "thiso"
                            | "player"
                            | "client"
                            | "clientr"
                            | "tiles"
                            | "allplayers"
                            | "players"
                            | "weapons"
                            | "servers"
                            | "server"
                            | "serverr"
                            | "serveroptions"
                            | "params"
                    ) || !is_empty_global_value(value)
                });
                DynValue::Undefined
            }
            "sendpm" | "sendplayer" => {
                let target = value_string(&argument(0));
                let message = value_string(&argument(1));
                if !target.is_empty() && !message.is_empty() {
                    self.result.player_messages.push(PlayerMessage {
                        account: target,
                        message,
                    });
                }
                DynValue::Undefined
            }
            "sendtorc" => {
                let message = value_string(&argument(0));
                if !message.is_empty() {
                    self.result.rc_messages.push(message);
                }
                DynValue::Undefined
            }
            "sendtoirc" => {
                let values = args.iter().map(value_string).collect::<Vec<_>>();
                if let Some(command) = values.first() {
                    self.result.player_irc_messages.push(IRCMessage {
                        account: String::new(),
                        command: command.clone(),
                        params: values[1..].to_vec(),
                    });
                }
                DynValue::Undefined
            }
            "sendtonc" | "printf" => {
                let message = if operation == "printf" {
                    format_string(&args)
                } else {
                    value_string(&argument(0))
                };
                if !message.is_empty() {
                    self.result.nc_messages.push(message);
                }
                DynValue::Undefined
            }
            "sendrpgmessage" => {
                let message = value_string(&argument(0));
                if !account.is_empty() {
                    self.result
                        .player_messages
                        .push(PlayerMessage { account, message });
                }
                DynValue::Undefined
            }
            "setlevel" => {
                self.result.player_warps.push(PlayerWarp {
                    account,
                    level: value_string(&argument(0)),
                    ..PlayerWarp::default()
                });
                DynValue::Undefined
            }
            "setlevel2" => {
                self.result.player_warps.push(PlayerWarp {
                    account,
                    level: value_string(&argument(0)),
                    x: number_f64(&argument(1)),
                    y: number_f64(&argument(2)),
                });
                DynValue::Undefined
            }
            "__gs1setplayerprop" => {
                let name = gs1_player_prop_name(&value_string(&argument(0)));
                if !account.is_empty() && !name.is_empty() {
                    self.result.player_props.push(PlayerProp {
                        account,
                        name,
                        value: value_string(&argument(1)),
                    });
                }
                argument(1)
            }
            "__gs1playertoken" => {
                let token = value_string(&argument(0)).to_ascii_lowercase();
                match token.as_str() {
                    "a" => get_property(&self.current_player, "account"),
                    "c" => get_property(&self.current_player, "chat"),
                    "d" => get_property(&self.current_player, "dir"),
                    "g" => get_property(&self.current_player, "guild"),
                    "l" => get_property(&self.current_player, "levelname"),
                    "n" => {
                        let value = get_property(&self.current_player, "nick");
                        if value.is_undefined_or_empty() {
                            get_property(&self.current_player, "nickname")
                        } else {
                            value
                        }
                    }
                    "x" => get_property(&self.current_player, "x"),
                    "y" => get_property(&self.current_player, "y"),
                    _ => DynValue::Undefined,
                }
            }
            "__gs1substring" => {
                let text = value_string(&argument(0));
                let start = number_i64(&argument(1)).max(0) as usize;
                let start = start.min(text.len());
                let length = number_i64(&argument(2)).max(0) as usize;
                let end = start.saturating_add(length).min(text.len());
                DynValue::String(text[start..end].to_string())
            }
            "addweapon" | "toweapons" => {
                self.result.player_weapons.push(PlayerWeapon {
                    account,
                    name: value_string(&argument(0)),
                    add: true,
                });
                DynValue::Undefined
            }
            "removeweapon" => {
                self.result.player_weapons.push(PlayerWeapon {
                    account,
                    name: value_string(&argument(0)),
                    add: false,
                });
                DynValue::Undefined
            }
            "showemoticon" | "showemoticonbykey" | "hideemoticon" | "hidesign" | "scrollsign" => {
                if !account.is_empty() {
                    let amount = if args.len() == 1 {
                        number_f64(&argument(0))
                    } else {
                        number_f64(&argument(1))
                    };
                    self.result.player_effects.push(PlayerEffect {
                        account,
                        action: operation.to_string(),
                        value: value_string(&argument(0)),
                        amount,
                    });
                }
                DynValue::Undefined
            }
            "message" | "say2" => {
                let text = value_string(&argument(0));
                if self.config.npc_id != 0 {
                    self.result.npc_actions.push(NPCAction {
                        id: self.config.npc_id,
                        chat: text,
                        has_chat: true,
                        ..NPCAction::default()
                    });
                } else if !account.is_empty() {
                    self.result.player_props.push(PlayerProp {
                        account,
                        name: "chat".to_string(),
                        value: text,
                    });
                }
                DynValue::Undefined
            }
            "sendtext" => {
                self.result.level_actions.push(LevelAction {
                    action: "sendtext".to_string(),
                    target: value_string(&argument(0)),
                    value: value_string(&argument(1)),
                    params: args.iter().skip(2).map(value_string).collect(),
                    ..LevelAction::default()
                });
                DynValue::Undefined
            }
            "requesttext" => DynValue::String(request_text_value(
                &value_string(&argument(0)),
                &value_string(&argument(1)),
                &player_context_from_object(&self.current_player),
            )),
            "int" => DynValue::Number(number_f64(&argument(0)) as i64 as f64),
            "float" | "double" | "strtofloat" => DynValue::Number(number_f64(&argument(0))),
            "pi" => DynValue::Number(std::f64::consts::PI),
            "sqrt2" => DynValue::Number(std::f64::consts::SQRT_2),
            "sqrt1_2" => DynValue::Number(std::f64::consts::SQRT_2 / 2.0),
            "abs" => DynValue::Number(number_f64(&argument(0)).abs()),
            "ceil" => DynValue::Number(number_f64(&argument(0)).ceil()),
            "floor" => DynValue::Number(number_f64(&argument(0)).floor()),
            "round" => DynValue::Number(number_f64(&argument(0)).round()),
            "sin" => DynValue::Number(number_f64(&argument(0)).sin()),
            "cos" => DynValue::Number(number_f64(&argument(0)).cos()),
            "tan" => DynValue::Number(number_f64(&argument(0)).tan()),
            "atan" | "arctan" => DynValue::Number(number_f64(&argument(0)).atan()),
            "arccos" => DynValue::Number(number_f64(&argument(0)).acos()),
            "arcsin" => DynValue::Number(number_f64(&argument(0)).asin()),
            "atan2" => DynValue::Number(number_f64(&argument(0)).atan2(number_f64(&argument(1)))),
            "degtorad" => DynValue::Number(number_f64(&argument(0)) * std::f64::consts::PI / 180.0),
            "radtodeg" => DynValue::Number(number_f64(&argument(0)) * 180.0 / std::f64::consts::PI),
            "log" => {
                let value = number_f64(&argument(0));
                if args.len() > 1 {
                    DynValue::Number(number_f64(&argument(1)).ln() / value.ln())
                } else {
                    DynValue::Number(value.ln())
                }
            }
            "sqrt" => DynValue::Number(number_f64(&argument(0)).sqrt()),
            "pow" => DynValue::Number(number_f64(&argument(0)).powf(number_f64(&argument(1)))),
            "max" => DynValue::Number(number_f64(&argument(0)).max(number_f64(&argument(1)))),
            "min" => DynValue::Number(number_f64(&argument(0)).min(number_f64(&argument(1)))),
            "strequals" => DynValue::Bool(value_string(&argument(0)) == value_string(&argument(1))),
            "strcontains" | "contains" => DynValue::Number(
                value_string(&argument(0)).contains(&value_string(&argument(1))) as i32 as f64,
            ),
            "startswith" | "starts" => DynValue::Number(
                value_string(&argument(0)).starts_with(&value_string(&argument(1))) as i32 as f64,
            ),
            "endswith" | "ends" => DynValue::Number(
                value_string(&argument(0)).ends_with(&value_string(&argument(1))) as i32 as f64,
            ),
            "uppercase" | "upper" => DynValue::String(value_string(&argument(0)).to_uppercase()),
            "lowercase" | "lower" => DynValue::String(value_string(&argument(0)).to_lowercase()),
            "random" => {
                let min = number_f64(&argument(0));
                let max = number_f64(&argument(1));
                if max <= min {
                    DynValue::Number(min)
                } else {
                    DynValue::Number(rand::random::<f64>() * (max - min) + min)
                }
            }
            "char" => DynValue::String(
                char::from_u32(number_i64(&argument(0)) as u32)
                    .unwrap_or('\0')
                    .to_string(),
            ),
            "getascii" => DynValue::Number(
                value_string(&argument(0))
                    .as_bytes()
                    .first()
                    .copied()
                    .unwrap_or(0) as f64,
            ),
            "strlen" => DynValue::Number(value_string(&argument(0)).len() as f64),
            "format" => DynValue::String(format_string(&args)),
            "format2" => DynValue::String(format2_string(&args)),
            "arraylen" => DynValue::Number(array_values(&argument(0)).len() as f64),
            "copystrings" => {
                let source = argument(0);
                let destination = argument(1);
                for (key, value) in self.named_properties(&source) {
                    if matches!(value, DynValue::String(_)) {
                        self.set_named_property(&destination, &key, value);
                    }
                }
                DynValue::Undefined
            }
            "aindexof" => {
                let values = array_values(&argument(1));
                DynValue::Number(
                    values
                        .iter()
                        .position(|x| equal_values(&argument(0), x))
                        .map_or(-1.0, |x| x as f64),
                )
            }
            "setarray" => {
                let target = argument(0);
                let length = number_i64(&argument(1)).max(0) as usize;
                if let DynValue::Array(values) = target {
                    let mut values = values.borrow_mut();
                    values.resize(length, DynValue::Number(0.0));
                }
                DynValue::Undefined
            }
            "array" => DynValue::array(args),
            "findpathinarray" => find_path_in_array(&args),
            "base64encode" => DynValue::String(
                base64::engine::general_purpose::STANDARD.encode(value_bytes(&argument(0))),
            ),
            "base64decode" => {
                match base64::engine::general_purpose::STANDARD.decode(value_string(&argument(0))) {
                    Ok(bytes) => String::from_utf8(bytes)
                        .map(DynValue::String)
                        .unwrap_or_else(|error| DynValue::Bytes(error.into_bytes())),
                    Err(_) => DynValue::String(String::new()),
                }
            }
            "md5" => DynValue::String(md5_hex(value_string(&argument(0)).as_bytes())),
            "checksum" => DynValue::Number(crc32(value_string(&argument(0)).as_bytes()) as f64),
            "des_encrypt" => DynValue::Bytes(legacy_des_encrypt(
                &value_string(&argument(0)),
                &value_string(&argument(1)),
            )),
            "des_decrypt" => {
                legacy_des_decrypt(&value_string(&argument(0)), &value_bytes(&argument(1)))
                    .map(DynValue::String)
                    .unwrap_or_else(|| DynValue::String(String::new()))
            }
            "replacetext" => DynValue::String(
                value_string(&argument(0))
                    .replace(&value_string(&argument(1)), &value_string(&argument(2))),
            ),
            "strcmp" => DynValue::Number(
                value_string(&argument(0)).cmp(&value_string(&argument(1))) as i8 as f64,
            ),
            "getextension" | "extractfileext" => DynValue::String(
                std::path::Path::new(&value_string(&argument(0)))
                    .extension()
                    .and_then(|x| x.to_str())
                    .unwrap_or_default()
                    .to_string(),
            ),
            "extractfilename" => DynValue::String(
                std::path::Path::new(&value_string(&argument(0)).replace('\\', "/"))
                    .file_name()
                    .and_then(|x| x.to_str())
                    .unwrap_or_default()
                    .to_string(),
            ),
            "extractfilebase" => {
                let path = value_string(&argument(0)).replace('\\', "/");
                let name = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|x| x.to_str())
                    .unwrap_or_default()
                    .to_string();
                DynValue::String(
                    name.rsplit_once('.')
                        .map_or(name.clone(), |(left, _)| left.to_string()),
                )
            }
            "extractfilepath" => {
                let path = value_string(&argument(0)).replace('\\', "/");
                let parent = std::path::Path::new(&path)
                    .parent()
                    .and_then(|x| x.to_str())
                    .unwrap_or_default();
                DynValue::String(if parent.is_empty() || parent == "." {
                    String::new()
                } else {
                    format!("{parent}/")
                })
            }
            "urlencode" => DynValue::String(
                url::form_urlencoded::byte_serialize(value_string(&argument(0)).as_bytes())
                    .collect(),
            ),
            "urldecode" => DynValue::String(query_unescape(&value_string(&argument(0)))),
            "escapestring" => {
                DynValue::String(escape_mysql_string(&value_string(&argument(0)), false))
            }
            "escapestringkeepnewline" => {
                DynValue::String(escape_mysql_string(&value_string(&argument(0)), true))
            }
            "escapefilename" => {
                DynValue::String(escape_filename_value(&value_string(&argument(0))))
            }
            "removeescapesfromfilename" => {
                DynValue::String(unescape_filename_value(&value_string(&argument(0))))
            }
            "wraptext" => DynValue::array(
                wrap_text(
                    number_i32(&argument(0)),
                    &value_string(&argument(1)),
                    &value_string(&argument(2)),
                )
                .into_iter()
                .map(DynValue::String)
                .collect(),
            ),
            "wraptext2" => DynValue::array(
                wrap_text(
                    (number_f64(&argument(0)) / number_f64(&argument(1)).max(1.0)) as i32,
                    &value_string(&argument(2)),
                    &value_string(&argument(3)),
                )
                .into_iter()
                .map(DynValue::String)
                .collect(),
            ),
            "addstring" => {
                mutate_string_array(&argument(0), "add", 0, &value_string(&argument(1)), "");
                DynValue::Undefined
            }
            "insertstring" => {
                mutate_string_array(
                    &argument(0),
                    "insert",
                    number_i64(&argument(1)),
                    &value_string(&argument(2)),
                    "",
                );
                DynValue::Undefined
            }
            "replacestring" => {
                let target = argument(0);
                if args.len() > 2 && matches!(argument(1), DynValue::Number(_)) {
                    set_index(&target, number_i64(&argument(1)), argument(2));
                } else {
                    mutate_string_array(
                        &target,
                        "replace",
                        0,
                        &value_string(&argument(1)),
                        &value_string(&argument(2)),
                    );
                }
                DynValue::Undefined
            }
            "removestring" => {
                mutate_string_array(&argument(0), "remove", 0, &value_string(&argument(1)), "");
                DynValue::Undefined
            }
            "deletestring" => {
                mutate_string_array(&argument(0), "delete", number_i64(&argument(1)), "", "");
                DynValue::Undefined
            }
            "regex_match" => DynValue::Bool(simple_regex_match(
                &value_string(&argument(0)),
                &value_string(&argument(1)),
                true,
            )),
            "regex_test" => DynValue::Bool(
                simple_regex_find(&value_string(&argument(0)), &value_string(&argument(1)))
                    .is_some(),
            ),
            "regex_find" => DynValue::String(
                simple_regex_find(&value_string(&argument(0)), &value_string(&argument(1)))
                    .unwrap_or_default(),
            ),
            "regex_findall" => DynValue::array(
                simple_regex_find_all(&value_string(&argument(0)), &value_string(&argument(1)))
                    .into_iter()
                    .map(DynValue::String)
                    .collect(),
            ),
            "regex_replace" => DynValue::String(simple_regex_replace(
                &value_string(&argument(0)),
                &value_string(&argument(1)),
                &value_string(&argument(2)),
            )),
            "regex_split" => DynValue::array(
                simple_regex_split(&value_string(&argument(0)), &value_string(&argument(1)))
                    .into_iter()
                    .map(DynValue::String)
                    .collect(),
            ),
            "getimgwidth" | "getimgheight" => {
                DynValue::Number(if value_string(&argument(0)).trim().is_empty() {
                    0.0
                } else {
                    1.0
                })
            }
            "showimg" => {
                if args.len() < 4 {
                    return DynValue::Number(0.0);
                }
                let index = number_i64(&argument(0));
                let object = self
                    .drawings
                    .entry(index)
                    .or_insert_with(DynValue::plain)
                    .clone();
                if get_property(&object, "rotation").is_undefined() {
                    set_property(&object, "rotation", DynValue::Number(0.0));
                }
                set_property(&object, "index", DynValue::Number(index as f64));
                set_property(
                    &object,
                    "image",
                    DynValue::String(value_string(&argument(1))),
                );
                set_property(&object, "x", DynValue::String(value_string(&argument(2))));
                set_property(&object, "y", DynValue::String(value_string(&argument(3))));
                DynValue::Number(0.0)
            }
            "showimg2" => {
                if args.len() < 5 {
                    return DynValue::Null;
                }
                let index = number_i64(&argument(0));
                let object = self
                    .drawings
                    .entry(index)
                    .or_insert_with(DynValue::plain)
                    .clone();
                if get_property(&object, "rotation").is_undefined() {
                    set_property(&object, "rotation", DynValue::Number(0.0));
                }
                set_property(&object, "index", DynValue::Number(index as f64));
                set_property(
                    &object,
                    "image",
                    DynValue::String(value_string(&argument(1))),
                );
                set_property(&object, "x", DynValue::String(value_string(&argument(2))));
                set_property(&object, "y", DynValue::String(value_string(&argument(3))));
                set_property(
                    &object,
                    "layer",
                    DynValue::String(value_string(&argument(4))),
                );
                object
            }
            "showtext" => {
                if args.len() < 4 {
                    return DynValue::Number(0.0);
                }
                let index = number_i64(&argument(0));
                let object = self
                    .drawings
                    .entry(index)
                    .or_insert_with(DynValue::plain)
                    .clone();
                set_property(&object, "index", DynValue::Number(index as f64));
                set_property(
                    &object,
                    "text",
                    DynValue::String(value_string(&argument(3))),
                );
                set_property(&object, "x", DynValue::String(value_string(&argument(1))));
                set_property(&object, "y", DynValue::String(value_string(&argument(2))));
                object
            }
            "findimg" => self
                .drawings
                .get(&number_i64(&argument(0)))
                .cloned()
                .unwrap_or(DynValue::Null),
            "hideimg" => {
                self.drawings.remove(&number_i64(&argument(0)));
                DynValue::Undefined
            }
            "hideimgs" => {
                let start = number_i64(&argument(0));
                let count = number_i64(&argument(1)).max(0);
                for index in 0..count {
                    self.drawings.remove(&(start + index));
                }
                DynValue::Number(count as f64)
            }
            "getcallstackinfo" | "getcallstack" => self.call_stack_value(operation),
            "getservername" => DynValue::String(
                self.config
                    .server_options
                    .get("servername")
                    .or_else(|| self.config.server_options.get("server"))
                    .or_else(|| self.config.server_options.get("name"))
                    .cloned()
                    .unwrap_or_default(),
            ),
            "isclassloaded" => DynValue::Bool(false),
            "keycode" => DynValue::Number(number_i64(&argument(0)) as f64),
            "isobject" => {
                let value = argument(0);
                DynValue::Bool(
                    matches!(&value, DynValue::Object(_) | DynValue::Array(_))
                        || match &value {
                            DynValue::String(name) => matches!(
                                get_var(self, &name),
                                DynValue::Object(_) | DynValue::Array(_)
                            ),
                            _ => false,
                        },
                )
            }
            "getmapx" => DynValue::Number(
                self.config
                    .map_position
                    .as_ref()
                    .and_then(|resolve| resolve(&value_string(&argument(0))).map(|x| x.0))
                    .or_else(|| {
                        map_position_from_files(&self.config.file_root, &value_string(&argument(0)))
                            .map(|x| x.0)
                    })
                    .unwrap_or(0) as f64,
            ),
            "getmapy" => DynValue::Number(
                self.config
                    .map_position
                    .as_ref()
                    .and_then(|resolve| resolve(&value_string(&argument(0))).map(|x| x.1))
                    .or_else(|| {
                        map_position_from_files(&self.config.file_root, &value_string(&argument(0)))
                            .map(|x| x.1)
                    })
                    .unwrap_or(0) as f64,
            ),
            "tiletype" => self.tile_type(
                &current_level,
                number_f64(&argument(0)),
                number_f64(&argument(1)),
            ),
            "onwall" => DynValue::Bool(
                number_f64(&self.tile_type(
                    &current_level,
                    number_f64(&argument(0)),
                    number_f64(&argument(1)),
                )) == 22.0,
            ),
            "onwall2" | "onwater2" => {
                let x = number_f64(&argument(0));
                let y = number_f64(&argument(1));
                let width = number_f64(&argument(2));
                let height = number_f64(&argument(3));
                let wanted = if operation == "onwall2" {
                    vec![22]
                } else {
                    vec![8, 11]
                };
                let found = if width > 0.0 && height > 0.0 {
                    (y.floor() as i32..(y + height).ceil() as i32).any(|yy| {
                        (x.floor() as i32..(x + width).ceil() as i32).any(|xx| {
                            wanted.contains(&number_i32(&self.tile_type(
                                &current_level,
                                xx as f64,
                                yy as f64,
                            )))
                        })
                    })
                } else {
                    false
                };
                DynValue::Bool(found)
            }
            "onwater" => {
                let tile = number_i32(&self.tile_type(
                    &current_level,
                    number_f64(&argument(0)),
                    number_f64(&argument(1)),
                ));
                DynValue::Bool(tile == 8 || tile == 11)
            }
            "findlevel" => make_level_object(&value_string(&argument(0))),
            "findweapon" => self
                .config
                .weapons
                .iter()
                .find(|x| x.name.eq_ignore_ascii_case(&value_string(&argument(0))))
                .map_or(DynValue::Null, make_weapon_object),
            "findplayer" | "getplayer" | "findplayer2" => {
                self.find_player(&value_string(&argument(0)), operation.ends_with('2'))
            }
            "findplayerbyid" => self
                .config
                .players
                .iter()
                .find(|x| x.id == number_i64(&argument(0)) as u16)
                .map_or(DynValue::Null, |x| make_player_object(x, true, false, true)),
            "findplayerbycommunityname" => self
                .config
                .players
                .iter()
                .find(|x| player_matches(x, &value_string(&argument(0))))
                .map_or(DynValue::Null, |x| make_player_object(x, true, false, true)),
            "findnpc" => self
                .npc_by_name(&value_string(&argument(0)))
                .unwrap_or(DynValue::Null),
            "findnpcbyid" => self
                .npc_by_id(number_i64(&argument(0)) as u32)
                .unwrap_or(DynValue::Null),
            "findobject" => self.find_object(&value_string(&argument(0))),
            "getplayers" => DynValue::array(
                self.config
                    .players
                    .iter()
                    .filter(|x| {
                        let wanted = value_string(&argument(0));
                        let wanted = if wanted.is_empty() {
                            current_level.clone()
                        } else {
                            wanted
                        };
                        !x.account.is_empty()
                            && (wanted.is_empty() || x.level.eq_ignore_ascii_case(&wanted))
                    })
                    .map(|x| make_player_object(x, true, false, true))
                    .collect(),
            ),
            "getnearbyplayers" => DynValue::array(
                self.config
                    .players
                    .iter()
                    .filter(|x| {
                        !x.account.is_empty()
                            && (current_level.is_empty()
                                || x.level.eq_ignore_ascii_case(&current_level))
                            && distance_sq(
                                x.x,
                                x.y,
                                number_f64(&argument(0)),
                                number_f64(&argument(1)),
                            ) <= number_f64(&argument(2)).max(0.0).powi(2)
                    })
                    .map(|x| make_player_object(x, true, false, true))
                    .collect(),
            ),
            "findnearestplayer" => {
                self.nearest_player(number_f64(&argument(0)), number_f64(&argument(1)))
            }
            "findnearestplayers" => {
                self.nearest_players(number_f64(&argument(0)), number_f64(&argument(1)))
            }
            "getnearestplayer" => DynValue::Number(
                self.nearest_index(number_f64(&argument(0)), number_f64(&argument(1))) as f64,
            ),
            "getnearestplayers" => self.nearest_indexes(
                number_f64(&argument(0)),
                number_f64(&argument(1)),
                &value_string(&argument(2)),
            ),
            "triggerclient" => {
                if args.len() >= 2 {
                    self.result.client_triggers.push(ClientTrigger {
                        kind: value_string(&argument(0)),
                        name: value_string(&argument(1)),
                        args: args.iter().skip(2).map(value_string).collect(),
                    });
                }
                DynValue::Undefined
            }
            "triggeraction" => {
                self.append_trigger_action(
                    &current_level,
                    number_f64(&argument(0)),
                    number_f64(&argument(1)),
                    &value_string(&argument(2)),
                    &args[3..],
                );
                DynValue::Undefined
            }
            "setani" | "updateboard" | "updateboard2" | "putbomb" | "putleaps" | "lay2"
            | "shoot" | "hitnpc" | "hitplayer" | "hitobjects" | "explodebomb" => {
                self.push_level_action(operation, &args);
                DynValue::Undefined
            }
            "shootarrow" | "shootfireball" | "shootfireblast" | "shootnuke" | "shootball" => {
                let ani = match operation {
                    "shootarrow" => "arrow",
                    "shootfireball" => "fireball",
                    "shootfireblast" => "fireblast",
                    "shootnuke" => "nuke",
                    _ => "ball",
                };
                self.result.level_actions.push(LevelAction {
                    action: "shoot".to_string(),
                    level: current_level.clone(),
                    ani: ani.to_string(),
                    angle: if operation == "shootball" {
                        0.0
                    } else {
                        number_f64(&argument(0))
                    },
                    params: value_lines(&get_property(&self.owner, "__shootparams")),
                    ..LevelAction::default()
                });
                DynValue::Undefined
            }
            "setshootparams" => {
                set_property(&self.owner, "__shootparams", argument(0));
                DynValue::Undefined
            }
            "putnpc" | "putnpc2" => {
                self.level_method(&make_level_object(&current_level), operation, &args)
            }
            "callnpc" | "callweapon" => {
                self.result.npc_function_calls.push(NPCFunctionCall {
                    id: number_i64(&argument(0)) as u32,
                    name: String::new(),
                    function: operation.to_string(),
                    args: args.iter().skip(1).map(value_string).collect(),
                });
                DynValue::Undefined
            }
            "sleep" => {
                self.result.scheduled_events.push(ScheduledEvent {
                    event: self.config.event_name.clone(),
                    delay: number_f64(&argument(0)),
                    ..ScheduledEvent::default()
                });
                self.suspended = true;
                DynValue::Undefined
            }
            "settimer" | "scheduleevent" => {
                let delay = number_f64(&argument(0));
                let event = value_string(&argument(1));
                self.result.scheduled_events.push(ScheduledEvent {
                    event: event.clone(),
                    delay,
                    params: args.iter().skip(2).map(value_string).collect(),
                    canceled: delay < 0.0
                        || event.eq_ignore_ascii_case("cancel")
                        || event.eq_ignore_ascii_case("off"),
                    ..ScheduledEvent::default()
                });
                DynValue::Undefined
            }
            "cancelevents" => {
                self.result.scheduled_events.push(ScheduledEvent {
                    event: value_string(&argument(0)),
                    canceled: true,
                    ..ScheduledEvent::default()
                });
                DynValue::Undefined
            }
            "waitfor" => {
                if !self.object_event_ready(&argument(0), &value_string(&argument(1))) {
                    self.result.wait_events.push(WaitEvent {
                        object: value_string(&argument(0)),
                        event: value_string(&argument(1)),
                        timeout: number_f64(&argument(2)),
                    });
                    self.suspended = true;
                }
                DynValue::Bool(!self.suspended)
            }
            "getstring" => DynValue::String(value_string(
                &self.get_script_path(&value_string(&argument(0))),
            )),
            "setstring" => {
                self.set_script_path(
                    &value_string(&argument(0)),
                    DynValue::String(value_string(&argument(1))),
                );
                DynValue::Undefined
            }
            "unset" => {
                self.unset_script_path(&value_string(&argument(0)));
                DynValue::Undefined
            }
            "makevar" => self.get_script_path(&value_string(&argument(0))),
            "fileexists" => {
                let name = value_string(&argument(0));
                DynValue::Bool(
                    vm_file_has_right(&self.config.file_rights, &name, 'r')
                        && resolve_vm_file(&self.config.file_root, &name)
                            .is_some_and(|path| path.exists()),
                )
            }
            "filesize" => {
                let name = value_string(&argument(0));
                if !vm_file_has_right(&self.config.file_rights, &name, 'r') {
                    DynValue::Number(0.0)
                } else {
                    resolve_vm_file(&self.config.file_root, &name)
                        .and_then(|path| fs::metadata(path).ok())
                        .map_or(DynValue::Number(0.0), |metadata| {
                            DynValue::Number(metadata.len() as f64)
                        })
                }
            }
            "getfilemodtime" => {
                let name = value_string(&argument(0));
                if !vm_file_has_right(&self.config.file_rights, &name, 'r') {
                    DynValue::Number(0.0)
                } else {
                    resolve_vm_file(&self.config.file_root, &name)
                        .and_then(|path| fs::metadata(path).ok())
                        .and_then(|metadata| metadata.modified().ok())
                        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                        .map_or(DynValue::Number(0.0), |time| {
                            DynValue::Number(time.as_secs_f64())
                        })
                }
            }
            "loadstring" => {
                let name = value_string(&argument(0));
                if !vm_file_has_right(&self.config.file_rights, &name, 'r') {
                    DynValue::String(String::new())
                } else {
                    DynValue::String(
                        load_vm_string(&self.config.file_root, &name).unwrap_or_default(),
                    )
                }
            }
            "loadlines" => {
                let name = value_string(&argument(0));
                if !vm_file_has_right(&self.config.file_rights, &name, 'r') {
                    DynValue::array(Vec::new())
                } else {
                    DynValue::array(
                        load_vm_lines(&self.config.file_root, &name)
                            .unwrap_or_default()
                            .into_iter()
                            .map(DynValue::String)
                            .collect(),
                    )
                }
            }
            "savestring" => {
                let name = value_string(&argument(0));
                DynValue::Bool(
                    vm_file_has_right(&self.config.file_rights, &name, 'w')
                        && save_vm_bytes(
                            &self.config.file_root,
                            &name,
                            &value_bytes(&argument(1)),
                            args.get(2).is_some_and(save_mode),
                        )
                        .is_ok(),
                )
            }
            "savelines" => {
                let name = value_string(&argument(0));
                DynValue::Bool(
                    vm_file_has_right(&self.config.file_rights, &name, 'w')
                        && save_vm_lines(
                            &self.config.file_root,
                            &name,
                            &value_lines(&argument(1)),
                            args.get(2).is_some_and(save_mode),
                        )
                        .is_ok(),
                )
            }
            "findfiles" => {
                let pattern = value_string(&argument(0));
                let recursive = args.get(1).is_some_and(DynValue::truthy);
                let include_directories = args.get(2).is_some_and(DynValue::truthy);
                DynValue::array(
                    find_vm_files(
                        &self.config.file_root,
                        &pattern,
                        recursive,
                        include_directories,
                    )
                    .into_iter()
                    .filter(|name| vm_file_has_right(&self.config.file_rights, name, 'r'))
                    .map(DynValue::String)
                    .collect(),
                )
            }
            "loadvars" | "loadini" => {
                let name = value_string(&argument(0));
                let object = DynValue::plain();
                if vm_file_has_right(&self.config.file_rights, &name, 'r') {
                    if let Some(lines) = load_vm_lines(&self.config.file_root, &name) {
                        if operation == "loadvars" {
                            load_vars_from_lines(&object, &lines);
                        } else {
                            load_ini_from_lines(&object, &lines);
                        }
                    }
                }
                object
            }
            "loadvarsfromarray" => {
                let object = DynValue::plain();
                load_vars_from_lines(&object, &value_lines(&argument(0)));
                object
            }
            "savevars" => {
                let name = value_string(&argument(0));
                DynValue::Bool(
                    vm_file_has_right(&self.config.file_rights, &name, 'w')
                        && save_vm_string(
                            &self.config.file_root,
                            &name,
                            &encode_vm_vars(&argument(1)),
                            args.get(2).is_some_and(save_mode),
                        )
                        .is_ok(),
                )
            }
            "savevarstoarray" => DynValue::array(
                encode_vm_vars(&argument(0))
                    .trim_end_matches('\n')
                    .split('\n')
                    .filter(|value| !value.is_empty())
                    .map(|value| DynValue::String(value.to_string()))
                    .collect(),
            ),
            "generatezipstring" => DynValue::Bytes(generate_zip_bytes(&argument(0))),
            "parsejson" => {
                let target = argument(0);
                let Ok(value) = serde_json::from_str::<Value>(&value_string(&argument(1))) else {
                    return DynValue::Bool(false);
                };
                self.replace_object_value(&target, json_to_dyn(&value));
                DynValue::Bool(true)
            }
            "deletefile" => {
                let name = value_string(&argument(0));
                if !vm_file_has_right(&self.config.file_rights, &name, 'w') {
                    DynValue::Bool(false)
                } else {
                    DynValue::Bool(
                        resolve_vm_file(&self.config.file_root, &name)
                            .is_some_and(|path| fs::remove_file(path).is_ok()),
                    )
                }
            }
            "movefile" => {
                let source = value_string(&argument(0));
                let destination = value_string(&argument(1));
                let valid = !source.is_empty()
                    && !destination.is_empty()
                    && vm_file_has_right(&self.config.file_rights, &source, 'w')
                    && vm_file_has_right(&self.config.file_rights, &destination, 'w');
                DynValue::Bool(
                    valid
                        && resolve_vm_file(&self.config.file_root, &source)
                            .zip(resolve_vm_file(&self.config.file_root, &destination))
                            .is_some_and(|(source, destination)| {
                                fs::rename(source, destination).is_ok()
                            }),
                )
            }
            "copylevel" => {
                let source = value_string(&argument(0));
                let destination = value_string(&argument(1));
                let source = resolve_vm_level_file(&self.config.file_root, &source);
                let destination = resolve_vm_level_file(&self.config.file_root, &destination);
                if let (
                    Some((source_path, source_name)),
                    Some((destination_path, destination_name)),
                ) = (source, destination)
                {
                    if vm_file_has_right(&self.config.file_rights, &source_name, 'r')
                        && vm_file_has_right(&self.config.file_rights, &destination_name, 'w')
                    {
                        if let Ok(data) = fs::read(source_path) {
                            if let Some(parent) = destination_path.parent() {
                                let _ = fs::create_dir_all(parent);
                            }
                            let _ = fs::write(destination_path, data);
                        }
                    }
                }
                DynValue::Undefined
            }
            "formattimestring" => DynValue::String(format_time_value(
                &value_string(&argument(0)),
                number_f64(&argument(1)),
            )),
            "savelog2" => {
                let name = format!("logs/{}", value_string(&argument(0)));
                DynValue::Bool(
                    vm_file_has_right(&self.config.file_rights, &name, 'w')
                        && save_vm_string(
                            &self.config.file_root,
                            &name,
                            &format!("{}\n", value_string(&argument(1))),
                            true,
                        )
                        .is_ok(),
                )
            }
            "requesturl" | "requesthttp" | "requestcurl" => {
                let url = value_string(&argument(0));
                let object = make_http_request_object(&url);
                self.perform_http_request(&object);
                self.cache_request(&url, &object);
                object
            }
            "requesturlasgamefile" => {
                let url = value_string(&argument(0));
                let object = make_http_request_object(&url);
                self.perform_http_request(&object);
                let file_name = value_string(&argument(1));
                if value_string(&get_property(&object, "error")).is_empty() {
                    if !vm_file_has_right(&self.config.file_rights, &file_name, 'w') {
                        set_property(
                            &object,
                            "error",
                            DynValue::String("insufficient file rights".to_string()),
                        );
                    } else {
                        let _ = save_vm_bytes(
                            &self.config.file_root,
                            &file_name,
                            &value_bytes(&get_property(&object, "requestdata")),
                            args.get(2).is_some_and(save_mode),
                        );
                    }
                }
                self.cache_request(&url, &object);
                object
            }
            "gethttprequestforurl" => self
                .request_cache
                .get(&value_string(&argument(0)))
                .cloned()
                .unwrap_or(DynValue::Null),
            "gethttprequest" => {
                let key = format!(
                    "{}:{}{}",
                    value_string(&argument(0)),
                    value_string(&argument(1)),
                    value_string(&argument(2))
                );
                self.request_cache
                    .get(&key)
                    .or_else(|| self.request_cache.get(&value_string(&argument(0))))
                    .cloned()
                    .unwrap_or(DynValue::Null)
            }
            "escapestring2" => DynValue::String(escape_sql_string2(&value_string(&argument(0)))),
            "requestsql" => {
                self.sql_request("main", &value_string(&argument(0)), argument(1).truthy())
            }
            "requestsql2" => self.sql_request(
                &value_string(&argument(0)),
                &value_string(&argument(1)),
                argument(2).truthy(),
            ),
            "tojson" => DynValue::String(
                value_to_json(&argument(0), Some(&self.owner), &mut Vec::new()).to_string(),
            ),
            "addnamedstring" => {
                let object = DynValue::plain();
                set_property(
                    &object,
                    "name",
                    DynValue::String(value_string(&argument(0))),
                );
                set_property(
                    &object,
                    "value",
                    DynValue::String(value_string(&argument(1))),
                );
                object
            }
            "randomstring" => random_gs2_string(&argument(0)),
            "getstringkeys" => DynValue::array(
                self.get_string_keys(&value_string(&argument(0)))
                    .into_iter()
                    .map(DynValue::String)
                    .collect(),
            ),
            "loadclass" => DynValue::Undefined,
            "pokereval" => poker_eval_value(
                &value_string(&argument(0)),
                &argument(1),
                &argument(2),
                &argument(3),
                number_i64(&argument(4)) as i32,
            ),
            "date" => make_date_object(args.first().map(number_f64)),
            "clear" => {
                if let DynValue::Array(values) = receiver {
                    values.borrow_mut().clear();
                }
                DynValue::Undefined
            }
            "size" => DynValue::Number(array_values(&receiver).len() as f64),
            "length" => match receiver {
                DynValue::Array(values) => DynValue::Number(values.borrow().len() as f64),
                DynValue::String(value) => DynValue::Number(value.chars().count() as f64),
                _ => DynValue::Number(0.0),
            },
            "push" | "add" | "unshift" => {
                mutate_array(&receiver, operation, args);
                DynValue::Number(array_values(&receiver).len() as f64)
            }
            "pop" => {
                if let DynValue::Array(values) = receiver {
                    values.borrow_mut().pop().unwrap_or(DynValue::Undefined)
                } else {
                    DynValue::Undefined
                }
            }
            "shift" => {
                if let DynValue::Array(values) = receiver {
                    let mut values = values.borrow_mut();
                    if values.is_empty() {
                        DynValue::Undefined
                    } else {
                        values.remove(0)
                    }
                } else {
                    DynValue::Undefined
                }
            }
            "substring" => substring_value(&receiver, &args),
            "tokenize" => DynValue::array(
                value_string(&receiver)
                    .split(&value_string(&argument(0)))
                    .map(|x| DynValue::String(x.to_string()))
                    .collect(),
            ),
            "pos" => DynValue::Number(
                value_string(&receiver)
                    .find(&value_string(&argument(0)))
                    .map_or(-1.0, |x| x as f64),
            ),
            "trim" => DynValue::String(value_string(&receiver).trim().to_string()),
            "replace" => DynValue::String(
                value_string(&receiver)
                    .replace(&value_string(&argument(0)), &value_string(&argument(1))),
            ),
            "delete" | "remove" => {
                mutate_array(&receiver, operation, args);
                DynValue::Undefined
            }
            "isinclass" => DynValue::Bool(
                socket_joined_classes(&receiver)
                    .iter()
                    .any(|x| x.eq_ignore_ascii_case(&value_string(&argument(0)))),
            ),
            "joinedclasses" => get_property(&receiver, "__classes"),
            "bind" | "connect" | "close" | "destroy" | "send" | "senddata" | "sendudp" | "join"
            | "trigger" => DynValue::Undefined,
            _ => DynValue::Undefined,
        }
    }

    fn invoke_file_method(
        &mut self,
        operation: &str,
        receiver: DynValue,
        args: &[DynValue],
    ) -> DynValue {
        let argument = |index: usize| args.get(index).cloned().unwrap_or(DynValue::Undefined);
        let file_name = || value_string(&argument(0));
        let read_allowed = |name: &str| vm_file_has_right(&self.config.file_rights, name, 'r');
        let write_allowed = |name: &str| vm_file_has_right(&self.config.file_rights, name, 'w');
        match operation {
            "savestring" => {
                let name = file_name();
                if !write_allowed(&name) {
                    return DynValue::Bool(false);
                }
                let data = value_bytes(&receiver);
                DynValue::Bool(
                    save_vm_bytes(
                        &self.config.file_root,
                        &name,
                        &data,
                        args.get(1).is_some_and(save_mode),
                    )
                    .is_ok(),
                )
            }
            "savelines" => {
                let name = file_name();
                if !write_allowed(&name) {
                    return DynValue::Bool(false);
                }
                let lines = value_lines(&receiver);
                DynValue::Bool(
                    save_vm_lines(
                        &self.config.file_root,
                        &name,
                        &lines,
                        args.get(1).is_some_and(save_mode),
                    )
                    .is_ok(),
                )
            }
            "loadstring" => {
                let name = file_name();
                if !read_allowed(&name) {
                    return DynValue::String(String::new());
                }
                DynValue::String(load_vm_string(&self.config.file_root, &name).unwrap_or_default())
            }
            "loadlines" => {
                let name = file_name();
                if !read_allowed(&name) {
                    return DynValue::Bool(false);
                }
                let Some(lines) = load_vm_lines(&self.config.file_root, &name) else {
                    return DynValue::Bool(false);
                };
                replace_sequence(&receiver, lines.into_iter().map(DynValue::String).collect());
                DynValue::Bool(true)
            }
            "loadfolder" => {
                let pattern = file_name();
                if !read_allowed(&pattern) {
                    replace_sequence(&receiver, Vec::new());
                    return DynValue::Bool(false);
                }
                let recursive = args.get(1).is_some_and(DynValue::truthy);
                let values = find_vm_files(&self.config.file_root, &pattern, recursive, true)
                    .into_iter()
                    .filter(|name| vm_file_has_right(&self.config.file_rights, name, 'r'))
                    .map(DynValue::String)
                    .collect::<Vec<_>>();
                replace_sequence(&receiver, values);
                DynValue::Bool(true)
            }
            "loadvars" | "loadini" => {
                let name = file_name();
                if !read_allowed(&name) {
                    return DynValue::Bool(false);
                }
                let Some(lines) = load_vm_lines(&self.config.file_root, &name) else {
                    return DynValue::Bool(false);
                };
                clear_object_properties(&receiver);
                if operation == "loadvars" {
                    load_vars_from_lines(&receiver, &lines);
                } else {
                    load_ini_from_lines(&receiver, &lines);
                }
                DynValue::Bool(true)
            }
            "loadvarsfromarray" => {
                let object = if matches!(receiver, DynValue::Object(_)) {
                    receiver.clone()
                } else {
                    DynValue::plain()
                };
                clear_object_properties(&object);
                load_vars_from_lines(&object, &value_lines(&argument(0)));
                object
            }
            "savevars" => {
                let name = file_name();
                if !write_allowed(&name) {
                    return DynValue::Bool(false);
                }
                DynValue::Bool(
                    save_vm_string(
                        &self.config.file_root,
                        &name,
                        &encode_vm_vars(&receiver),
                        args.get(1).is_some_and(save_mode),
                    )
                    .is_ok(),
                )
            }
            "savevarstoarray" => {
                let values = encode_vm_vars(&receiver)
                    .trim_end_matches('\n')
                    .split('\n')
                    .filter(|value| !value.is_empty())
                    .map(|value| DynValue::String(value.to_string()))
                    .collect::<Vec<_>>();
                if matches!(receiver, DynValue::Array(_)) {
                    replace_sequence(&receiver, values);
                    receiver
                } else {
                    DynValue::array(values)
                }
            }
            "savejsontostring" => save_json_string(&receiver, number_i32(&argument(0))),
            "savejson" => {
                let name = file_name();
                if !write_allowed(&name) {
                    return DynValue::Bool(false);
                }
                let json = value_string(&save_json_string(&receiver, number_i32(&argument(2))));
                DynValue::Bool(
                    save_vm_string(
                        &self.config.file_root,
                        &name,
                        &json,
                        args.get(1).is_some_and(save_mode),
                    )
                    .is_ok(),
                )
            }
            "savexmltostring" => DynValue::String(save_xml_string(&receiver)),
            "savexml" => {
                let name = file_name();
                if !write_allowed(&name) {
                    return DynValue::Bool(false);
                }
                let xml = save_xml_string(&receiver);
                DynValue::Bool(
                    save_vm_string(
                        &self.config.file_root,
                        &name,
                        &xml,
                        args.get(1).is_some_and(save_mode),
                    )
                    .is_ok(),
                )
            }
            "loadjsonfromstring" => {
                let Some(value) = serde_json::from_str::<Value>(&value_string(&argument(0)))
                    .ok()
                    .map(|value| json_to_dyn(&value))
                else {
                    return DynValue::Bool(false);
                };
                self.replace_object_value(&receiver, value);
                DynValue::Bool(true)
            }
            "loadxmlfromstring" => {
                let Some(value) = parse_xml_value(&value_string(&argument(0))) else {
                    return DynValue::Bool(false);
                };
                self.replace_object_value(&receiver, value);
                DynValue::Bool(true)
            }
            "loadjson" | "loadxml" => {
                let name = file_name();
                if !read_allowed(&name) {
                    return DynValue::Bool(false);
                }
                let Some(text) = load_vm_string(&self.config.file_root, &name) else {
                    return DynValue::Bool(false);
                };
                let value = if operation == "loadjson" {
                    serde_json::from_str::<Value>(&text)
                        .ok()
                        .map(|value| json_to_dyn(&value))
                } else {
                    parse_xml_value(&text)
                };
                let Some(value) = value else {
                    return DynValue::Bool(false);
                };
                self.replace_object_value(&receiver, value);
                DynValue::Bool(true)
            }
            _ => DynValue::Undefined,
        }
    }

    fn cache_request(&mut self, request_url: &str, request: &DynValue) {
        self.request_cache
            .insert(request_url.to_string(), request.clone());
        if let Ok(parsed) = url::Url::parse(request_url) {
            let host = parsed.host_str().unwrap_or_default();
            let path = if parsed.path().is_empty() {
                "/"
            } else {
                parsed.path()
            };
            let path = format!(
                "{host}{path}{}",
                parsed.query().map_or(String::new(), |q| format!("?{q}"))
            );
            self.request_cache.insert(path, request.clone());
            self.request_cache.insert(host.to_string(), request.clone());
        }
    }

    fn perform_http_request(&mut self, request: &DynValue) {
        let request_url = value_string(&get_property(request, "url"));
        let headers = value_lines(&get_property(request, "headers"));
        let method = if get_property(request, "post").truthy() {
            "POST"
        } else {
            "GET"
        };
        let body = if method == "POST" {
            value_bytes(&get_property(request, "postdata"))
        } else {
            Vec::new()
        };
        let insecure_tls = get_property(request, "quick").truthy()
            || get_property(request, "skipsslverification").truthy();
        match http_request(&request_url, method, &body, &headers, insecure_tls) {
            Ok(response) => {
                let body_text = String::from_utf8_lossy(&response.body).into_owned();
                set_property(
                    request,
                    "data",
                    DynValue::array(split_http_lines(&response.body)),
                );
                set_property(request, "fulldata", DynValue::String(body_text.clone()));
                set_property(request, "requestdata", DynValue::String(body_text.clone()));
                set_property(
                    request,
                    "contenttype",
                    DynValue::String(
                        response
                            .headers
                            .get("content-type")
                            .cloned()
                            .unwrap_or_default(),
                    ),
                );
                set_property(
                    request,
                    "contentlength",
                    DynValue::Number(
                        response
                            .headers
                            .get("content-length")
                            .and_then(|value| value.parse::<i64>().ok())
                            .unwrap_or(response.body.len() as i64) as f64,
                    ),
                );
                set_property(
                    request,
                    "lastmodified",
                    DynValue::String(
                        response
                            .headers
                            .get("last-modified")
                            .cloned()
                            .unwrap_or_default(),
                    ),
                );
                set_property(
                    request,
                    "returncode",
                    DynValue::Number(response.status_code as f64),
                );
                set_property(
                    request,
                    "statuscode",
                    DynValue::Number(response.status_code as f64),
                );
                set_property(
                    request,
                    "returnmessage",
                    DynValue::String(response.status.clone()),
                );
                if response.status_code >= 400 {
                    set_property(request, "error", DynValue::String(response.status.clone()));
                }
                if response.body.is_empty() {
                    set_property(
                        request,
                        "data",
                        DynValue::array(vec![DynValue::String(response.status.clone())]),
                    );
                    set_property(
                        request,
                        "requestdata",
                        DynValue::String(response.status.clone()),
                    );
                }
            }
            Err(error) => {
                let error = error.to_string();
                set_property(
                    request,
                    "data",
                    DynValue::array(vec![DynValue::String(error.clone())]),
                );
                set_property(request, "requestdata", DynValue::String(error.clone()));
                set_property(request, "error", DynValue::String(error.clone()));
                set_property(request, "returncode", DynValue::Number(0.0));
                set_property(request, "statuscode", DynValue::Number(0.0));
                set_property(request, "returnmessage", DynValue::String(error));
            }
        }
        set_property(request, "completed", DynValue::Bool(true));
        set_property(request, "__event_onReceiveData", DynValue::Bool(true));
    }

    fn get_script_path(&self, path: &str) -> DynValue {
        let parts = path
            .trim()
            .split('.')
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        let Some(first) = parts.first() else {
            return DynValue::Undefined;
        };
        if parts.len() == 1 {
            return get_var(self, first);
        }
        let mut current = if first.eq_ignore_ascii_case("this") {
            self.owner.clone()
        } else {
            get_var(self, first)
        };
        for part in parts.iter().skip(1) {
            if current.is_undefined() || matches!(current, DynValue::Null) {
                return DynValue::Undefined;
            }
            current = self.property_value(&current, part);
        }
        current
    }

    fn get_relative_path(&self, root: &DynValue, path: &str) -> DynValue {
        let mut current = root.clone();
        for part in path.split('.').filter(|value| !value.is_empty()) {
            if current.is_undefined() || matches!(current, DynValue::Null) {
                return DynValue::Undefined;
            }
            current = self.property_value(&current, part);
        }
        current
    }

    fn object_event_ready(&self, object: &DynValue, event: &str) -> bool {
        let event_value = self.property_value(object, &format!("__event_{event}"));
        if !event_value.is_undefined() {
            return event_value.truthy();
        }
        self.property_value(object, event).truthy()
            || self.property_value(object, &format!("on{event}")).truthy()
    }

    fn set_script_path(&mut self, path: &str, value: DynValue) {
        let parts = path
            .trim()
            .split('.')
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        let Some(first) = parts.first() else {
            return;
        };
        if parts.len() == 1 {
            set_var(self, first, value);
            return;
        }
        let mut current = if first.eq_ignore_ascii_case("this") {
            self.owner.clone()
        } else {
            let existing = get_var(self, first);
            if existing.is_undefined() || matches!(existing, DynValue::Null) {
                let object = DynValue::plain();
                set_var(self, first, object.clone());
                object
            } else {
                existing
            }
        };
        for part in parts.iter().skip(1).take(parts.len().saturating_sub(2)) {
            let child = self.property_value(&current, part);
            if child.is_undefined() || matches!(child, DynValue::Null) {
                let object = DynValue::plain();
                self.set_property_value(&current, part, object.clone());
                current = object;
            } else {
                current = child;
            }
        }
        self.set_property_value(
            &current,
            parts.last().expect("path has a final component"),
            value,
        );
    }

    fn unset_script_path(&mut self, path: &str) {
        let parts = path
            .trim()
            .split('.')
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        let Some(first) = parts.first() else {
            return;
        };
        if parts.len() == 1 {
            let key = first.to_ascii_lowercase();
            self.globals.remove(&key);
            return;
        }
        let mut current = if first.eq_ignore_ascii_case("this") {
            self.owner.clone()
        } else {
            get_var(self, first)
        };
        for part in parts.iter().skip(1).take(parts.len().saturating_sub(2)) {
            current = self.property_value(&current, part);
            if current.is_undefined() || matches!(current, DynValue::Null) {
                return;
            }
        }
        let key = parts.last().unwrap_or(&"");
        if let Some(object) = current.object_ref() {
            let property = property_key(&object.borrow().properties, key);
            if let Some(property) = property {
                object.borrow_mut().properties.remove(&property);
            }
        } else if let Some(array_key) = Self::array_property_key(&current) {
            if let Some(properties) = self.array_properties.get_mut(&array_key) {
                if let Some(property) = property_key(properties, key) {
                    properties.remove(&property);
                }
            }
        }
    }

    fn get_string_keys(&self, prefix: &str) -> Vec<String> {
        let Some((base_name, stem)) = prefix.rsplit_once('.') else {
            return Vec::new();
        };
        let object = if base_name.eq_ignore_ascii_case("this") {
            self.owner.clone()
        } else {
            self.get_script_path(base_name)
        };
        let mut keys = self
            .named_properties(&object)
            .into_iter()
            .filter_map(|(key, _)| key.strip_prefix(stem).map(str::to_string))
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    fn call_stack_value(&self, operation: &str) -> DynValue {
        let mut frames = self
            .call_stack
            .iter()
            .rev()
            .filter(|name| !name.starts_with("__"))
            .cloned()
            .collect::<Vec<_>>();
        if frames.is_empty() && !self.config.event_name.is_empty() {
            frames.push(self.config.event_name.clone());
        }
        let frame_info = frames
            .iter()
            .map(|name| format!("in {name}()"))
            .collect::<Vec<_>>()
            .join(" ");
        let info = if self.config.script_name.is_empty() {
            frame_info
        } else if frame_info.is_empty() {
            format!("by {}", self.config.script_name)
        } else {
            format!("{} by {}", frame_info, self.config.script_name)
        };
        if operation.eq_ignore_ascii_case("getcallstackinfo") {
            return DynValue::String(info);
        }
        if info.is_empty() {
            return DynValue::array(Vec::new());
        }
        let mut output = Vec::new();
        let before_by = info
            .split_once(" by ")
            .map_or(info.as_str(), |(frames, _)| frames);
        let script_name = info
            .split_once(" by ")
            .map(|(_, script)| script)
            .unwrap_or_default();
        let function_name = frames
            .first()
            .cloned()
            .unwrap_or_else(|| self.config.event_name.clone());
        let first = DynValue::plain();
        set_property(&first, "info", DynValue::String(before_by.to_string()));
        set_property(&first, "function", DynValue::String(function_name));
        set_property(&first, "line", DynValue::Number(0.0));
        set_property(
            &first,
            "script",
            DynValue::String(self.config.script_name.clone()),
        );
        set_property(
            &first,
            "caller",
            DynValue::String(if script_name.is_empty() {
                String::new()
            } else {
                format!("by {script_name}")
            }),
        );
        output.push(first);
        if !script_name.is_empty() {
            let second = DynValue::plain();
            set_property(&second, "info", DynValue::String(script_name.to_string()));
            set_property(
                &second,
                "function",
                DynValue::String(script_name.to_string()),
            );
            set_property(&second, "line", DynValue::Number(0.0));
            set_property(
                &second,
                "script",
                DynValue::String(self.config.script_name.clone()),
            );
            set_property(&second, "caller", DynValue::String(String::new()));
            output.push(second);
        }
        DynValue::array(output)
    }

    fn sql_request(&mut self, db_name: &str, query: &str, expect_result: bool) -> DynValue {
        let request = make_sql_request_object();
        let query = query.trim();
        if query.is_empty() {
            set_property(
                &request,
                "error",
                DynValue::String("empty query".to_string()),
            );
        } else if expect_result {
            match self.sql_query(db_name, query) {
                Ok(rows) => set_property(&request, "rows", DynValue::array(rows)),
                Err(error) => set_property(&request, "error", DynValue::String(error)),
            }
        } else {
            match self.sql_exec(db_name, query) {
                Ok(last_insert_id) => set_property(
                    &request,
                    "lastinsertid",
                    DynValue::Number(last_insert_id as f64),
                ),
                Err(error) => set_property(&request, "error", DynValue::String(error)),
            }
        }
        set_property(&request, "completed", DynValue::Bool(true));
        set_property(&request, "__event_onReceiveData", DynValue::Bool(true));
        request
    }

    fn sql_exec(&mut self, db_name: &str, query: &str) -> std::result::Result<i64, String> {
        let path = sqlite_database_path(&self.config.file_root, db_name)?;
        sqlite_ffi::execute(&path, query)
    }

    fn sql_query(&self, db_name: &str, query: &str) -> std::result::Result<Vec<DynValue>, String> {
        let path = sqlite_database_path(&self.config.file_root, db_name)?;
        let rows = sqlite_ffi::query(&path, query)?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let object = DynValue::plain();
                for (column, value) in row {
                    set_property(&object, &column, value);
                }
                object
            })
            .collect())
    }

    fn tile_type(&self, level: &str, x: f64, y: f64) -> DynValue {
        DynValue::Number(
            map_tile_type(self.raw_tile_type(level, x, y), self.config.tile_layout) as f64,
        )
    }

    fn raw_tile_type(&self, level: &str, x: f64, y: f64) -> i32 {
        let value = self
            .config
            .tile_type
            .as_ref()
            .map(|resolve| resolve(level, x.floor() as i32, y.floor() as i32))
            .unwrap_or(0);
        value
    }

    fn level_method(
        &mut self,
        receiver: &DynValue,
        operation: &str,
        args: &[DynValue],
    ) -> DynValue {
        let level = value_string(&get_property(receiver, "name"));
        let argument = |index: usize| args.get(index).cloned().unwrap_or(DynValue::Undefined);
        let tile_at = |x: f64, y: f64| number_i32(&self.tile_type(&level, x, y));
        match operation {
            "tiletype" => {
                self.tile_type(&level, number_f64(&argument(0)), number_f64(&argument(1)))
            }
            "onwall" => {
                DynValue::Bool(tile_at(number_f64(&argument(0)), number_f64(&argument(1))) == 22)
            }
            "onwater" => {
                let tile = tile_at(number_f64(&argument(0)), number_f64(&argument(1)));
                DynValue::Bool(tile == 8 || tile == 11)
            }
            "onwall2" | "onwater2" => {
                let x = number_f64(&argument(0));
                let y = number_f64(&argument(1));
                let width = number_f64(&argument(2));
                let height = number_f64(&argument(3));
                let wanted = if operation == "onwall2" {
                    vec![22]
                } else {
                    vec![8, 11]
                };
                let found = if width > 0.0 && height > 0.0 {
                    (y.floor() as i32..(y + height).ceil() as i32).any(|yy| {
                        (x.floor() as i32..(x + width).ceil() as i32)
                            .any(|xx| wanted.contains(&tile_at(xx as f64, yy as f64)))
                    })
                } else {
                    false
                };
                DynValue::Bool(found)
            }
            "findareanpcs" => {
                let x = number_f64(&argument(0));
                let y = number_f64(&argument(1));
                let width = number_f64(&argument(2));
                let height = number_f64(&argument(3));
                DynValue::array(
                    self.config
                        .npcs
                        .iter()
                        .filter(|npc| {
                            let npc_level = if npc.level.is_empty() {
                                &level
                            } else {
                                &npc.level
                            };
                            let npc_width = if npc.width <= 0.0 { 1.0 } else { npc.width };
                            let npc_height = if npc.height <= 0.0 { 1.0 } else { npc.height };
                            npc_level.eq_ignore_ascii_case(&level)
                                && npc.x < x + width
                                && npc.x + npc_width > x
                                && npc.y < y + height
                                && npc.y + npc_height > y
                        })
                        .map(make_npc_object)
                        .collect(),
                )
            }
            "getplayers" => DynValue::array(
                self.config
                    .players
                    .iter()
                    .filter(|player| {
                        !player.account.is_empty()
                            && (level.is_empty() || player.level.eq_ignore_ascii_case(&level))
                    })
                    .map(|player| make_player_object(player, true, false, true))
                    .collect(),
            ),
            "getnearbyplayers" => {
                let x = number_f64(&argument(0));
                let y = number_f64(&argument(1));
                let radius = number_f64(&argument(2)).max(0.0);
                let limit = radius * radius;
                DynValue::array(
                    self.config
                        .players
                        .iter()
                        .filter(|player| {
                            !player.account.is_empty()
                                && (level.is_empty() || player.level.eq_ignore_ascii_case(&level))
                                && distance_sq(player.x, player.y, x, y) <= limit
                        })
                        .map(|player| make_player_object(player, true, false, true))
                        .collect(),
                )
            }
            "testnpc" => {
                let x = number_f64(&argument(0));
                let y = number_f64(&argument(1));
                let id = self
                    .config
                    .npcs
                    .iter()
                    .find(|npc| {
                        let npc_level = if npc.level.is_empty() {
                            &level
                        } else {
                            &npc.level
                        };
                        let width = if npc.width <= 0.0 { 1.0 } else { npc.width };
                        let height = if npc.height <= 0.0 { 1.0 } else { npc.height };
                        npc_level.eq_ignore_ascii_case(&level)
                            && x >= npc.x
                            && x < npc.x + width
                            && y >= npc.y
                            && y < npc.y + height
                    })
                    .map_or(0, |npc| npc.id);
                DynValue::Number(id as f64)
            }
            "testsign" => {
                let x = number_f64(&argument(0)).floor() as i32;
                let y = number_f64(&argument(1)).floor() as i32;
                DynValue::Number(
                    self.config
                        .signs
                        .iter()
                        .position(|sign| {
                            sign.level.eq_ignore_ascii_case(&level) && sign.x == x && sign.y == y
                        })
                        .map_or(0.0, |index| index as f64 + 1.0),
                )
            }
            "findsign" => {
                let x = number_f64(&argument(0)).floor() as i32;
                let y = number_f64(&argument(1)).floor() as i32;
                self.config
                    .signs
                    .iter()
                    .find(|sign| {
                        sign.level.eq_ignore_ascii_case(&level) && sign.x == x && sign.y == y
                    })
                    .map_or(DynValue::Null, |sign| {
                        let object = DynValue::plain();
                        set_property(&object, "level", DynValue::String(sign.level.clone()));
                        set_property(&object, "x", DynValue::Number(sign.x as f64));
                        set_property(&object, "y", DynValue::Number(sign.y as f64));
                        set_property(&object, "text", DynValue::String(sign.text.clone()));
                        object
                    })
            }
            "findchest" => {
                let x = number_f64(&argument(0)).floor() as i32;
                let y = number_f64(&argument(1)).floor() as i32;
                self.config
                    .chests
                    .iter()
                    .find(|chest| {
                        chest.level.eq_ignore_ascii_case(&level) && chest.x == x && chest.y == y
                    })
                    .map_or(DynValue::Null, |chest| {
                        let object = DynValue::plain();
                        set_property(&object, "level", DynValue::String(chest.level.clone()));
                        set_property(&object, "x", DynValue::Number(chest.x as f64));
                        set_property(&object, "y", DynValue::Number(chest.y as f64));
                        set_property(
                            &object,
                            "itemtype",
                            DynValue::Number(chest.item_type as f64),
                        );
                        set_property(&object, "isopen", DynValue::Bool(chest.is_open));
                        object
                    })
            }
            "putbomb" | "putbomb2" | "putexplosion" | "putexplosion2" => {
                let mut action = LevelAction {
                    action: operation.to_string(),
                    level,
                    power: number_i32(&argument(0)),
                    ..LevelAction::default()
                };
                if operation == "putexplosion2" {
                    action.layer = number_i32(&argument(1));
                    action.x = number_f64(&argument(2));
                    action.y = number_f64(&argument(3));
                } else {
                    action.x = number_f64(&argument(1));
                    action.y = number_f64(&argument(2));
                    if operation == "putbomb2" {
                        action.image = value_string(&argument(3));
                    }
                }
                self.result.level_actions.push(action);
                DynValue::Undefined
            }
            "updateboard" | "updateboard2" => {
                self.result.level_actions.push(LevelAction {
                    action: operation.to_string(),
                    level,
                    x: number_f64(&argument(0)),
                    y: number_f64(&argument(1)),
                    width: number_f64(&argument(2)),
                    height: number_f64(&argument(3)),
                    update: true,
                    save: operation == "updateboard2",
                    ..LevelAction::default()
                });
                DynValue::Undefined
            }
            "putnpc" | "putnpc2" => {
                let (action, x, y, image, script) = if operation == "putnpc2" {
                    (
                        "putnpc2",
                        number_f64(&argument(0)),
                        number_f64(&argument(1)),
                        String::new(),
                        value_string(&argument(2)),
                    )
                } else {
                    (
                        "putnpc",
                        number_f64(&argument(2)),
                        number_f64(&argument(3)),
                        value_string(&argument(0)),
                        value_string(&argument(1)),
                    )
                };
                let index = self.result.level_actions.len();
                self.result.level_actions.push(LevelAction {
                    action: action.to_string(),
                    level: level.clone(),
                    x,
                    y,
                    image: image.clone(),
                    script: script.clone(),
                    ..LevelAction::default()
                });
                let object = DynValue::object(ObjectKind::PutNPC { index });
                for (key, value) in [
                    ("x", DynValue::Number(x)),
                    ("y", DynValue::Number(y)),
                    ("level", DynValue::String(level)),
                    ("image", DynValue::String(image)),
                    ("script", DynValue::String(script)),
                    ("name", DynValue::String(String::new())),
                    ("id", DynValue::String(String::new())),
                    ("objecttype", DynValue::String("TPutNPC".to_string())),
                    ("__classes", DynValue::array(Vec::new())),
                    ("__putnpc_index", DynValue::Number(index as f64)),
                ] {
                    set_property(&object, key, value);
                }
                self.putnpc_refs.push((index, object.clone()));
                object
            }
            "shoot" => {
                self.result.level_actions.push(LevelAction {
                    action: "shoot".to_string(),
                    level,
                    x: number_f64(&argument(0)),
                    y: number_f64(&argument(1)),
                    z: number_f64(&argument(2)),
                    angle: number_f64(&argument(3)),
                    z_angle: number_f64(&argument(4)),
                    strength: number_f64(&argument(5)),
                    ani: value_string(&argument(6)),
                    params: value_lines(&argument(7)),
                    ..LevelAction::default()
                });
                DynValue::Undefined
            }
            "triggeraction" => {
                self.append_trigger_action(
                    &level,
                    number_f64(&argument(0)),
                    number_f64(&argument(1)),
                    &value_string(&argument(2)),
                    &args[3..],
                );
                DynValue::Undefined
            }
            "testitem" => {
                let x = number_f64(&argument(0)).floor() as i32;
                let y = number_f64(&argument(1)).floor() as i32;
                DynValue::Number(
                    self.config
                        .chests
                        .iter()
                        .position(|chest| {
                            chest.level.eq_ignore_ascii_case(&level) && chest.x == x && chest.y == y
                        })
                        .map_or(0.0, |index| index as f64 + 1.0),
                )
            }
            "testbomb" | "testexplo" | "testhorse" => DynValue::Number(0.0),
            "puthorse" => {
                self.result.level_actions.push(LevelAction {
                    action: operation.to_string(),
                    level,
                    image: value_string(&argument(0)),
                    x: number_f64(&argument(1)),
                    y: number_f64(&argument(2)),
                    ..LevelAction::default()
                });
                DynValue::Undefined
            }
            "putnewcomp" => {
                self.result.level_actions.push(LevelAction {
                    action: operation.to_string(),
                    level,
                    image: value_string(&argument(0)),
                    x: number_f64(&argument(1)),
                    y: number_f64(&argument(2)),
                    script: value_string(&argument(3)),
                    power: number_i32(&argument(4)),
                    ..LevelAction::default()
                });
                DynValue::Undefined
            }
            "removebomb" | "removeexplo" | "removehorse" | "removeitem" | "reflectarrow"
            | "removearrow" => {
                self.result.level_actions.push(LevelAction {
                    action: operation.to_string(),
                    level,
                    power: number_i32(&argument(0)),
                    ..LevelAction::default()
                });
                DynValue::Undefined
            }
            "removecompus" => {
                self.result.level_actions.push(LevelAction {
                    action: operation.to_string(),
                    level,
                    ..LevelAction::default()
                });
                DynValue::Undefined
            }
            "hitcompu" => {
                self.result.level_actions.push(LevelAction {
                    action: operation.to_string(),
                    level,
                    set_npc_id: number_i64(&argument(0)) as u32,
                    power: number_i32(&argument(1)),
                    x: number_f64(&argument(2)),
                    y: number_f64(&argument(3)),
                    ..LevelAction::default()
                });
                DynValue::Undefined
            }
            _ => self.invoke_utility(operation, args.to_vec(), receiver.clone()),
        }
    }

    fn append_trigger_action(
        &mut self,
        level: &str,
        x: f64,
        y: f64,
        target: &str,
        params: &[DynValue],
    ) {
        let params = params.iter().map(value_string).collect::<Vec<_>>();
        let event = trigger_action_event_name(target);
        let mut action = LevelAction {
            action: "triggeraction".to_string(),
            level: level.to_string(),
            x,
            y,
            target: target.to_string(),
            params: params.clone(),
            ..LevelAction::default()
        };
        for npc in &self.config.npcs {
            let npc_level = if npc.level.is_empty() {
                level
            } else {
                &npc.level
            };
            let width = if npc.width <= 0.0 { 1.0 } else { npc.width };
            let height = if npc.height <= 0.0 { 1.0 } else { npc.height };
            if npc_level.eq_ignore_ascii_case(level)
                && x >= npc.x
                && x < npc.x + width
                && y >= npc.y
                && y < npc.y + height
            {
                action.calls.push(NPCFunctionCall {
                    id: npc.id,
                    name: npc.name.clone(),
                    function: event.clone(),
                    args: params.clone(),
                });
            }
        }
        self.result.level_actions.push(action);
    }

    fn find_player(&mut self, target: &str, partial: bool) -> DynValue {
        let player = self.config.players.iter().find(|player| {
            if partial {
                player_matches_insensitive(player, target)
            } else {
                player_matches(player, target)
            }
        });
        let Some(player) = player.cloned() else {
            return make_player_object(&PlayerContext::default(), false, false, false);
        };
        let object = make_player_object(&player, true, false, true);
        self.track_player(object.clone(), &player);
        object
    }

    fn npc_by_name(&self, target: &str) -> Option<DynValue> {
        self.tracked_npcs
            .iter()
            .find(|npc| npc.context.name.eq_ignore_ascii_case(target.trim()))
            .map(|npc| npc.object.clone())
    }

    fn npc_by_id(&self, id: u32) -> Option<DynValue> {
        self.tracked_npcs
            .iter()
            .find(|npc| npc.context.id == id)
            .map(|npc| npc.object.clone())
    }

    fn npc_context(&self, id: u32) -> Option<&NPCContext> {
        self.tracked_npcs
            .iter()
            .find(|npc| npc.context.id == id)
            .map(|npc| &npc.context)
    }

    fn npc_has_function(&self, id: u32, name: &str) -> bool {
        let Some(context) = self.npc_context(id) else {
            return false;
        };
        let source = translate_server_script(&context.script);
        if let Ok(program) = Parser::new(&source).parse() {
            return program
                .functions
                .iter()
                .any(|function| function.name.eq_ignore_ascii_case(name));
        }
        false
    }

    fn find_object(&self, target: &str) -> DynValue {
        if target.eq_ignore_ascii_case(&self.config.script_name)
            || target.eq_ignore_ascii_case(&value_string(&get_property(&self.owner, "name")))
        {
            return self.owner.clone();
        }
        if let Some(npc) = self.npc_by_name(target) {
            return npc;
        }
        match get_var(self, target) {
            DynValue::Undefined => DynValue::Null,
            value => value,
        }
    }

    fn nearest_player(&self, x: f64, y: f64) -> DynValue {
        self.config
            .players
            .iter()
            .filter(|player| !player.account.is_empty())
            .min_by(|left, right| {
                distance_sq(left.x, left.y, x, y)
                    .partial_cmp(&distance_sq(right.x, right.y, x, y))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map_or(DynValue::Null, |x| make_player_object(x, true, false, true))
    }

    fn nearest_players(&self, x: f64, y: f64) -> DynValue {
        let mut players = self.config.players.clone();
        players.retain(|player| !player.account.is_empty());
        players.sort_by(|left, right| {
            distance_sq(left.x, left.y, x, y)
                .partial_cmp(&distance_sq(right.x, right.y, x, y))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        DynValue::array(
            players
                .iter()
                .map(|x| make_player_object(x, true, false, true))
                .collect(),
        )
    }

    fn nearest_index(&self, x: f64, y: f64) -> i32 {
        self.config
            .players
            .iter()
            .enumerate()
            .filter(|(_, player)| !player.account.is_empty())
            .min_by(|(_, left), (_, right)| {
                distance_sq(left.x, left.y, x, y)
                    .partial_cmp(&distance_sq(right.x, right.y, x, y))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map_or(-1, |(index, _)| index as i32)
    }

    fn nearest_indexes(&self, x: f64, y: f64, flag: &str) -> DynValue {
        let mut indexes = self
            .config
            .players
            .iter()
            .enumerate()
            .filter(|(_, player)| {
                !player.account.is_empty()
                    && (flag.is_empty()
                        || player
                            .flags
                            .get(flag)
                            .is_some_and(|value| !value.is_empty()))
            })
            .collect::<Vec<_>>();
        indexes.sort_by(|(_, left), (_, right)| {
            distance_sq(left.x, left.y, x, y)
                .partial_cmp(&distance_sq(right.x, right.y, x, y))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        DynValue::array(
            indexes
                .into_iter()
                .map(|(index, _)| DynValue::Number(index as f64))
                .collect(),
        )
    }

    fn push_level_action(&mut self, operation: &str, args: &[DynValue]) {
        let argument = |index: usize| args.get(index).cloned().unwrap_or(DynValue::Undefined);
        let mut action = LevelAction {
            action: operation.to_string(),
            level: self.config.player.get("level").cloned().unwrap_or_default(),
            ..LevelAction::default()
        };
        action.x = number_f64(&argument(0));
        action.y = number_f64(&argument(1));
        action.power = number_i32(&argument(0));
        match operation {
            "setani" => {
                action.set_npc_id = self.config.npc_id;
                action.set_player = value_string(&get_property(&self.current_player, "account"));
                action.ani = value_string(&argument(0));
                action.params = value_lines(&argument(1));
            }
            "updateboard" | "updateboard2" => {
                action.width = number_f64(&argument(2));
                action.height = number_f64(&argument(3));
                action.update = true;
                action.save = operation.ends_with('2');
            }
            "putbomb" => {
                action.power = number_i32(&argument(0));
                action.x = number_f64(&argument(1));
                action.y = number_f64(&argument(2));
            }
            "putleaps" => {
                action.power = number_i32(&argument(0));
            }
            "lay2" => {
                action.image = value_string(&argument(0));
            }
            "shoot" => {
                action.z = number_f64(&argument(2));
                action.angle = number_f64(&argument(3));
                action.z_angle = number_f64(&argument(4));
                action.strength = number_f64(&argument(5));
                action.ani = value_string(&argument(6));
                action.params = value_lines(&argument(7));
            }
            "hitnpc" | "hitplayer" => {
                action.set_npc_id = number_i64(&argument(0)) as u32;
                action.power = number_i32(&argument(1));
            }
            "hitobjects" => {
                action.power = number_i32(&argument(0));
            }
            "explodebomb" => {
                action.power = number_i32(&argument(0));
            }
            "triggeraction" => {
                action.params = args.iter().skip(3).map(value_string).collect();
                action.target = value_string(&argument(2));
            }
            _ => {}
        }
        self.result.level_actions.push(action);
    }

    fn construct(&mut self, name: &str, args: Vec<DynValue>) -> DynValue {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "tsocket" => {
                let socket = make_socket_object(&SocketContext {
                    name: String::new(),
                    ..SocketContext::default()
                });
                // A socket created by the script starts with a fresh Goja
                // array in `data`; only sockets entering through the host
                // context inherit the context's scalar payload.
                set_property(&socket, "data", DynValue::array(Vec::new()));
                // The host preserves an explicit socket name and only
                // generates an ID for anonymous sockets.
                let requested_name = args.first().map(value_string).unwrap_or_default();
                let anonymous = requested_name.trim().is_empty();
                let generated_name = if anonymous {
                    format!("Socket_{}", self.socket_refs.len() + 1)
                } else {
                    requested_name.trim().to_string()
                };
                let generated_id = if anonymous {
                    format!("socket-{}", NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed))
                } else {
                    String::new()
                };
                set_property(
                    &socket,
                    "__tsocket_name",
                    DynValue::String(generated_name.clone()),
                );
                set_property(
                    &socket,
                    "__tsocket_id",
                    DynValue::String(generated_id.clone()),
                );
                set_property(&socket, "name", DynValue::String(generated_name.clone()));
                self.globals
                    .insert(generated_name.to_ascii_lowercase(), socket.clone());
                socket
            }
            "tserverplayer" => {
                let account = args
                    .first()
                    .map(value_string)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if account.is_empty() {
                    return DynValue::Null;
                }
                let mut context = PlayerContext {
                    account: account.clone(),
                    nick: account.clone(),
                    nickname: account.clone(),
                    ..PlayerContext::default()
                };
                let mut loaded = false;
                if let Some(resolve) = &self.config.server_player_resolver {
                    if let Some(found) = resolve(&account) {
                        context = found;
                        loaded = true;
                    }
                } else if let Some(found) = self
                    .config
                    .players
                    .iter()
                    .find(|x| x.account.eq_ignore_ascii_case(&account))
                {
                    context = found.clone();
                    loaded = true;
                }
                if context.account.is_empty() {
                    context.account = account;
                }
                if context.nick.is_empty() {
                    context.nick = context.account.clone();
                }
                if context.nickname.is_empty() {
                    context.nickname = context.nick.clone();
                }
                let object = make_player_object(&context, false, true, loaded);
                set_property(&object, "__accountloaded", DynValue::Bool(loaded));
                object
            }
            "tcurlrequest" => {
                make_http_request_object(&args.first().map(value_string).unwrap_or_default())
            }
            "twebsocket" | "tsqlite" | "tdiscord" | "tstaticvar" => {
                let object = DynValue::plain();
                match lower.as_str() {
                    "twebsocket" => {
                        set_property(
                            &object,
                            "objecttype",
                            DynValue::String("TWebSocket".to_string()),
                        );
                        set_property(
                            &object,
                            "name",
                            DynValue::String(args.first().map(value_string).unwrap_or_default()),
                        );
                        set_property(&object, "url", DynValue::String(String::new()));
                        set_property(&object, "data", DynValue::String(String::new()));
                        set_property(&object, "error", DynValue::String(String::new()));
                        set_property(&object, "isconnected", DynValue::Bool(false));
                        for method in ["connect", "send", "close", "destroy"] {
                            set_property(
                                &object,
                                method,
                                DynValue::Builtin(format!("method:{method}")),
                            );
                        }
                    }
                    "tsqlite" => {
                        let name = args.first().map(value_string).unwrap_or_default();
                        let typed = DynValue::object(ObjectKind::SQLite { name: name.clone() });
                        set_property(&typed, "name", DynValue::String(name));
                        set_property(
                            &typed,
                            "objecttype",
                            DynValue::String("TSQLite".to_string()),
                        );
                        set_property(&typed, "path", DynValue::String(String::new()));
                        set_property(&typed, "error", DynValue::String(String::new()));
                        set_property(&typed, "isopen", DynValue::Bool(false));
                        set_property(&typed, "lastinsertid", DynValue::Number(0.0));
                        set_property(&typed, "rowsaffected", DynValue::Number(0.0));
                        for method in ["open", "exec", "query", "close"] {
                            set_property(
                                &typed,
                                method,
                                DynValue::Builtin(format!("method:{method}")),
                            );
                        }
                        return typed;
                    }
                    "tdiscord" => {
                        set_property(
                            &object,
                            "objecttype",
                            DynValue::String("TDiscord".to_string()),
                        );
                        set_property(
                            &object,
                            "name",
                            DynValue::String(args.first().map(value_string).unwrap_or_default()),
                        );
                        set_property(&object, "token", DynValue::String(String::new()));
                        set_property(&object, "error", DynValue::String(String::new()));
                        set_property(&object, "isconnected", DynValue::Bool(false));
                        for method in ["connect", "sendmessage", "sendembed", "close", "destroy"] {
                            set_property(
                                &object,
                                method,
                                DynValue::Builtin(format!("method:{method}")),
                            );
                        }
                    }
                    "tstaticvar" => set_property(&object, "__classes", DynValue::array(Vec::new())),
                    _ => {}
                }
                object
            }
            "object" => DynValue::plain(),
            "array" => DynValue::array(vec![
                DynValue::Undefined;
                args.first().map(number_i64).unwrap_or(0).max(0)
                    as usize
            ]),
            "__array" => new_array_from_dimensions(&args),
            _ => {
                let module = self
                    .imports
                    .iter()
                    .find(|(module_name, _)| {
                        import_constructor_name(module_name).eq_ignore_ascii_case(name)
                    })
                    .map(|(module_name, module)| (module_name.clone(), module.functions.clone()));
                if let Some((module_name, functions)) = module {
                    let constructor_name = import_constructor_name(&module_name);
                    let Some(function) = functions.iter().find(|function| {
                        function.public && function.name.eq_ignore_ascii_case(&constructor_name)
                    }) else {
                        self.result.err = format!(
                            "import {module_name} has no public constructor {constructor_name}"
                        );
                        return DynValue::Undefined;
                    };
                    let object = DynValue::object(ObjectKind::Imported {
                        module: module_name.clone(),
                    });
                    self.install_import_module(&object, &module_name);
                    let _ = self.invoke_script(Rc::new(function.clone()), object.clone(), args);
                    object
                } else {
                    DynValue::object(ObjectKind::Plain)
                }
            }
        }
    }
}

fn get_var(state: &EvalState, name: &str) -> DynValue {
    for scope in state.scopes.iter().rev() {
        if let Some(value) = scope.get(&name.to_ascii_lowercase()) {
            return value.clone();
        }
    }
    if name.eq_ignore_ascii_case("this") {
        return state.receiver.clone();
    }
    if name.eq_ignore_ascii_case("thiso") {
        return state.owner.clone();
    }
    if name.eq_ignore_ascii_case("temp") {
        return state.temp.clone();
    }
    if name.eq_ignore_ascii_case("joinedclasses") {
        return DynValue::array(array_values(
            &state.property_value(&state.owner, "__classes"),
        ));
    }
    if let Some(value) = state.globals.get(&name.to_ascii_lowercase()) {
        return value.clone();
    }
    if let Some(function) = state.functions.get(&name.to_ascii_lowercase()) {
        return DynValue::Function(Rc::clone(function));
    }
    DynValue::Undefined
}

fn set_var(state: &mut EvalState, name: &str, value: DynValue) {
    let key = name.to_ascii_lowercase();
    if key == "this" || key == "thiso" || key == "temp" {
        return;
    }
    for scope in state.scopes.iter_mut().rev() {
        if scope.contains_key(&key) {
            scope.insert(key, value);
            return;
        }
    }
    state.globals.insert(key, value);
}

fn property_key(properties: &HashMap<String, DynValue>, name: &str) -> Option<String> {
    if properties.contains_key(name) {
        return Some(name.to_string());
    }
    properties
        .keys()
        .find(|key| key.eq_ignore_ascii_case(name))
        .cloned()
}

fn object_prototype_methods() -> &'static [&'static str] {
    &[
        "loadvars",
        "loadini",
        "savevars",
        "savejsontostring",
        "savejson",
        "savexmltostring",
        "savexml",
        "loadjsonfromstring",
        "loadxmlfromstring",
        "loadjson",
        "loadxml",
        "copyfrom",
        "clearvars",
        "clearemptyvars",
        "objecttype",
        "getvarnames",
        "geteditvarnames",
        "getdynamicvarnames",
        "getstaticvarnames",
        "getfunctions",
        "hasfunction",
        "isinclass",
        "join",
        "leave",
        "trigger",
        "catchevent",
        "ignoreevent",
        "ignoreevents",
    ]
}

fn array_prototype_methods() -> &'static [&'static str] {
    &[
        "addarraymember",
        "getarraymember",
        "addarray",
        "insert",
        "replace",
        "index",
        "indices",
        "splice",
        "insertarray",
        "subarray",
        "subarray2",
        "sortascending",
        "sortdescending",
        "sort",
        "sortbyvalue",
        "map",
        "filter",
        "some",
        "find",
        "size",
        "length",
        "push",
        "pop",
        "shift",
        "unshift",
        "add",
        "delete",
        "remove",
        "clear",
    ]
}

fn get_property(value: &DynValue, name: &str) -> DynValue {
    let Some(object) = value.object_ref() else {
        if let DynValue::Array(values) = value {
            if name.eq_ignore_ascii_case("length") {
                return DynValue::Number(values.borrow().len() as f64);
            }
            if array_prototype_methods()
                .iter()
                .any(|method| method.eq_ignore_ascii_case(name))
                || object_prototype_methods()
                    .iter()
                    .any(|method| method.eq_ignore_ascii_case(name))
            {
                return DynValue::Builtin(format!("method:{}", name.to_ascii_lowercase()));
            }
        }
        if let DynValue::String(text) = value {
            if name.eq_ignore_ascii_case("length") {
                return DynValue::Number(text.chars().count() as f64);
            }
        }
        return DynValue::Undefined;
    };
    let object = object.borrow();
    if let Some(key) = property_key(&object.properties, name) {
        return object
            .properties
            .get(&key)
            .cloned()
            .unwrap_or(DynValue::Undefined);
    }
    if let Some(function) = object.methods.get(&name.to_ascii_lowercase()) {
        return DynValue::Function(Rc::clone(function));
    }
    if object_prototype_methods()
        .iter()
        .any(|method| method.eq_ignore_ascii_case(name))
    {
        return DynValue::Builtin(format!("method:{}", name.to_ascii_lowercase()));
    }
    if name.eq_ignore_ascii_case("joinedclasses") {
        return object
            .properties
            .get("__classes")
            .cloned()
            .unwrap_or_else(|| DynValue::array(Vec::new()));
    }
    if name.eq_ignore_ascii_case("objecttype") {
        return DynValue::String(
            object
                .properties
                .get("objecttype")
                .map(value_string)
                .unwrap_or_else(|| "TgraalVar".to_string()),
        );
    }
    if name.eq_ignore_ascii_case("tostring") {
        return DynValue::Builtin("method:tostring".to_string());
    }
    if name.eq_ignore_ascii_case("length") {
        return DynValue::Number(object.properties.len() as f64);
    }
    DynValue::Undefined
}

fn set_property(value: &DynValue, name: &str, item: DynValue) {
    if let Some(object) = value.object_ref() {
        let mut object = object.borrow_mut();
        let key = property_key(&object.properties, name).unwrap_or_else(|| name.to_string());
        object.properties.insert(key, item);
    } else if let DynValue::Array(values) = value {
        if let Ok(index) = name.parse::<usize>() {
            let mut values = values.borrow_mut();
            while values.len() <= index {
                values.push(DynValue::Undefined);
            }
            values[index] = item;
        }
    }
}

fn get_index(value: &DynValue, index: i64) -> DynValue {
    if index < 0 {
        return DynValue::Undefined;
    }
    match value {
        DynValue::Array(values) => values
            .borrow()
            .get(index as usize)
            .cloned()
            .unwrap_or(DynValue::Undefined),
        DynValue::String(text) => text
            .chars()
            .nth(index as usize)
            .map_or(DynValue::String(String::new()), |x| {
                DynValue::String(x.to_string())
            }),
        // Goja exposes the generated ZIP string as a scalar string, but its
        // binary contents are not indexable through the server-side bridge.
        // Preserve that observable empty-string result for byte-backed
        // helpers rather than leaking Rust's internal binary representation.
        DynValue::Bytes(_) => DynValue::String(String::new()),
        DynValue::Object(_) => get_property(value, &index.to_string()),
        _ => DynValue::Undefined,
    }
}

fn set_index(value: &DynValue, index: i64, item: DynValue) {
    if index < 0 {
        return;
    }
    match value {
        DynValue::Array(values) => {
            let mut values = values.borrow_mut();
            while values.len() <= index as usize {
                values.push(DynValue::Undefined);
            }
            values[index as usize] = item;
        }
        DynValue::Object(_) => set_property(value, &index.to_string(), item),
        _ => {}
    }
}

fn get_method(value: &DynValue, name: &str) -> Option<Rc<ScriptFunction>> {
    value.object_ref().and_then(|object| {
        object
            .borrow()
            .methods
            .get(&name.to_ascii_lowercase())
            .cloned()
    })
}

fn json_to_dyn(value: &Value) -> DynValue {
    match value {
        Value::Null => DynValue::Null,
        Value::Bool(value) => DynValue::Bool(*value),
        Value::Number(value) => DynValue::Number(value.as_f64().unwrap_or(0.0)),
        Value::String(value) => DynValue::String(value.clone()),
        Value::Array(values) => DynValue::array(values.iter().map(json_to_dyn).collect()),
        Value::Object(values) => {
            let object = DynValue::plain();
            for (key, value) in values {
                set_property(&object, key, json_to_dyn(value));
            }
            object
        }
    }
}

fn object_from_any_map(values: &AnyMap, imports: &HashMap<String, ParsedProgram>) -> DynValue {
    let object = DynValue::plain();
    for (key, value) in values {
        set_property(&object, key, json_to_dyn_with_imports(value, imports));
    }
    object
}

fn json_to_dyn_with_imports(value: &Value, imports: &HashMap<String, ParsedProgram>) -> DynValue {
    let Value::Object(values) = value else {
        return match value {
            Value::Null => DynValue::Null,
            Value::Bool(value) => DynValue::Bool(*value),
            Value::Number(value) => DynValue::Number(value.as_f64().unwrap_or(0.0)),
            Value::String(value) => DynValue::String(value.clone()),
            Value::Array(values) => DynValue::array(
                values
                    .iter()
                    .map(|value| json_to_dyn_with_imports(value, imports))
                    .collect(),
            ),
            Value::Object(_) => unreachable!(),
        };
    };

    if let Some(module_name) = values.get("__gs2_import_type").and_then(Value::as_str) {
        let object = DynValue::object(ObjectKind::Imported {
            module: module_name.to_string(),
        });
        let Some(object_ref) = object.object_ref() else {
            return object;
        };
        let base_module = imports
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(module_name))
            .map(|(_, module)| module.functions.clone())
            .unwrap_or_default();
        let classes = values
            .get("__gs2_import_classes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut modules = vec![(module_name.to_string(), base_module)];
        for class in &classes {
            if let Some(functions) = imports
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(class))
                .map(|(_, module)| module.functions.clone())
            {
                modules.push((class.clone(), functions));
            }
        }
        {
            let mut object_ref = object_ref.borrow_mut();
            for (module, functions) in modules {
                let module = module.to_ascii_lowercase();
                for function in functions {
                    let key = function.name.to_ascii_lowercase();
                    object_ref
                        .methods
                        .entry(key.clone())
                        .or_insert_with(|| Rc::new(function));
                    object_ref
                        .method_modules
                        .entry(key)
                        .or_insert(module.clone());
                }
            }
            object_ref.properties.insert(
                "__classes".to_string(),
                DynValue::array(classes.into_iter().map(DynValue::String).collect()),
            );
        }
        if let Some(Value::Object(state)) = values.get("__gs2_import_values") {
            for (key, item) in state {
                set_property(&object, key, json_to_dyn_with_imports(item, imports));
            }
        }
        return object;
    }

    let object = DynValue::plain();
    for (key, value) in values {
        set_property(&object, key, json_to_dyn_with_imports(value, imports));
    }
    object
}

fn hydrate_object_state(object: &DynValue, values: &AnyMap) {
    for (key, value) in values {
        set_property(object, key, json_to_dyn(value));
    }
}

fn value_to_json(value: &DynValue, owner: Option<&DynValue>, seen: &mut Vec<usize>) -> Value {
    match value {
        DynValue::Undefined | DynValue::Null => Value::Null,
        DynValue::Bool(value) => Value::Bool(*value),
        DynValue::Number(value) => {
            serde_json::Number::from_f64(*value).map_or(Value::Null, Value::Number)
        }
        DynValue::String(value) => Value::String(value.clone()),
        DynValue::Bytes(value) => Value::String(String::from_utf8_lossy(value).into_owned()),
        DynValue::Function(_) | DynValue::Builtin(_) => Value::Null,
        DynValue::Array(values) => Value::Array(
            values
                .borrow()
                .iter()
                .map(|x| value_to_json(x, owner, seen))
                .collect(),
        ),
        DynValue::Object(object) => {
            let pointer = Rc::as_ptr(object) as usize;
            if owner.is_some_and(|item| item.object_ref().is_some_and(|x| Rc::ptr_eq(&x, object))) {
                let mut marker = serde_json::Map::new();
                marker.insert("__tsocket_owner".to_string(), Value::Bool(true));
                return Value::Object(marker);
            }
            if seen.contains(&pointer) {
                return Value::Null;
            }
            seen.push(pointer);
            let imported_module = {
                let object_ref = object.borrow();
                if let ObjectKind::Imported { module } = &object_ref.kind {
                    Some(module.clone())
                } else {
                    None
                }
            };
            if let Some(module) = imported_module {
                let (properties, classes) = {
                    let object_ref = object.borrow();
                    let classes = object_ref
                        .properties
                        .get("__classes")
                        .map(array_values)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|value| Value::String(value_string(&value)))
                        .collect::<Vec<_>>();
                    (object_ref.properties.clone(), classes)
                };
                let mut state = serde_json::Map::new();
                for (key, item) in properties {
                    if key.starts_with("__")
                        || matches!(item, DynValue::Function(_) | DynValue::Builtin(_))
                    {
                        continue;
                    }
                    state.insert(key, value_to_json(&item, owner, seen));
                }
                let mut output = serde_json::Map::new();
                output.insert("__gs2_import_type".to_string(), Value::String(module));
                output.insert("__gs2_import_values".to_string(), Value::Object(state));
                output.insert("__gs2_import_classes".to_string(), Value::Array(classes));
                seen.pop();
                return Value::Object(output);
            }
            let object_ref = object.borrow();
            if matches!(object_ref.kind, ObjectKind::Socket) {
                let context = socket_context_from_value_with_owner(value, owner);
                let mut output = serde_json::Map::new();
                output.insert("__tsocket_ref".to_string(), Value::Bool(true));
                output.insert("name".to_string(), Value::String(context.name));
                output.insert("id".to_string(), Value::String(context.id));
                output.insert("address".to_string(), Value::String(context.address));
                output.insert("ipaddress".to_string(), Value::String(context.ip_address));
                output.insert("port".to_string(), Value::Number(context.port.into()));
                output.insert(
                    "packagedelimiter".to_string(),
                    Value::String(context.package_delimiter),
                );
                output.insert("data".to_string(), Value::String(context.data));
                output.insert("buffer".to_string(), Value::String(context.buffer));
                output.insert("isconnected".to_string(), Value::Bool(context.is_connected));
                output.insert(
                    "joinedclasses".to_string(),
                    Value::Array(
                        context
                            .joined_classes
                            .into_iter()
                            .map(Value::String)
                            .collect(),
                    ),
                );
                output.insert("parentname".to_string(), Value::String(context.parent_name));
                output.insert("parentid".to_string(), Value::String(context.parent_id));
                output.insert(
                    "state".to_string(),
                    value_to_json(&DynValue::object_from_map(context.state), owner, seen),
                );
                seen.pop();
                return Value::Object(output);
            }
            let mut output = serde_json::Map::new();
            for (key, item) in &object_ref.properties {
                if key.starts_with("__") {
                    continue;
                }
                if !matches!(item, DynValue::Function(_) | DynValue::Builtin(_)) {
                    output.insert(key.clone(), value_to_json(item, owner, seen));
                }
            }
            seen.pop();
            Value::Object(output)
        }
    }
}

impl DynValue {
    fn object_from_map(values: AnyMap) -> DynValue {
        let object = DynValue::plain();
        for (key, value) in values {
            set_property(&object, &key, json_to_dyn(&value));
        }
        object
    }
}

fn export_value(value: &DynValue, owner: Option<&DynValue>) -> AnyMap {
    let Some(object) = value.object_ref() else {
        return AnyMap::new();
    };
    let values = object.borrow().properties.clone();
    let mut output = AnyMap::new();
    for (key, item) in values {
        if key.starts_with("__") || matches!(item, DynValue::Function(_) | DynValue::Builtin(_)) {
            continue;
        }
        output.insert(
            key,
            value_to_json(&item, owner.or(Some(value)), &mut Vec::new()),
        );
    }
    output
}

fn socket_context_from_value(value: &DynValue) -> SocketContext {
    socket_context_from_value_with_owner(value, None)
}

fn socket_context_from_value_with_owner(
    value: &DynValue,
    owner: Option<&DynValue>,
) -> SocketContext {
    SocketContext {
        name: value_string(&get_property(value, "__tsocket_name")),
        id: value_string(&get_property(value, "__tsocket_id")),
        address: value_string(&get_property(value, "address")),
        ip_address: value_string(&get_property(value, "ipaddress")),
        port: number_i32(&get_property(value, "port")),
        package_delimiter: value_string(&get_property(value, "packagedelimiter")),
        data: value_string(&get_property(value, "data")),
        buffer: value_string(&get_property(value, "__buffer")),
        is_connected: get_property(value, "isconnected").truthy(),
        state: socket_state_from_value(value, owner.or(Some(value))),
        joined_classes: socket_joined_classes(value),
        parent_name: value_string(&get_property(value, "__parent_name")),
        parent_id: value_string(&get_property(value, "__parent_id")),
    }
}

fn make_socket_object(context: &SocketContext) -> DynValue {
    let object = DynValue::object(ObjectKind::Socket);
    set_property(&object, "__tsocket", DynValue::Bool(true));
    set_property(
        &object,
        "__tsocket_name",
        DynValue::String(context.name.clone()),
    );
    set_property(
        &object,
        "__tsocket_id",
        DynValue::String(context.id.clone()),
    );
    set_property(&object, "name", DynValue::String(context.name.clone()));
    set_property(
        &object,
        "objecttype",
        DynValue::String("TSocket".to_string()),
    );
    set_property(
        &object,
        "address",
        DynValue::String(context.address.clone()),
    );
    set_property(&object, "error", DynValue::String(String::new()));
    set_property(
        &object,
        "ipaddress",
        DynValue::String(context.ip_address.clone()),
    );
    set_property(&object, "isconnected", DynValue::Bool(context.is_connected));
    set_property(&object, "port", DynValue::Number(context.port as f64));
    set_property(&object, "parent", DynValue::Null);
    set_property(&object, "data", DynValue::String(context.data.clone()));
    set_property(
        &object,
        "__buffer",
        DynValue::String(context.buffer.clone()),
    );
    set_property(
        &object,
        "packagedelimiter",
        DynValue::String(context.package_delimiter.clone()),
    );
    set_property(
        &object,
        "__classes",
        DynValue::array(
            context
                .joined_classes
                .iter()
                .cloned()
                .map(DynValue::String)
                .collect(),
        ),
    );
    set_property(&object, "enablessl", DynValue::Bool(false));
    set_property(&object, "sslcertfile", DynValue::String(String::new()));
    set_property(&object, "sslkeyfile", DynValue::String(String::new()));
    set_property(&object, "sslcipherlist", DynValue::String(String::new()));
    if !context.parent_name.is_empty() {
        set_property(
            &object,
            "__parent_name",
            DynValue::String(context.parent_name.clone()),
        );
        set_property(
            &object,
            "__parent_id",
            DynValue::String(context.parent_id.clone()),
        );
    }
    object
}

fn install_socket_class_methods(
    socket: &DynValue,
    classes: &[String],
    resolver: &Option<SocketClassResolver>,
) {
    let Some(resolver) = resolver else {
        return;
    };

    // Parse each class before borrowing the socket.  A class method is kept on
    // the receiver exactly like the joined-class callable; methods are
    // deliberately not copied into the socket's serialised state.
    let mut methods = Vec::new();
    for class in classes {
        let Some(source) = resolver(class) else {
            continue;
        };
        let source = translate_server_script(&source);
        let Ok(program) = Parser::new(&source).parse() else {
            continue;
        };
        methods.extend(
            program
                .functions
                .into_iter()
                .filter(|function| function.public),
        );
    }
    if let Some(object) = socket.object_ref() {
        let mut object = object.borrow_mut();
        for function in methods {
            object
                .methods
                .insert(function.name.to_ascii_lowercase(), Rc::new(function));
        }
    }
}

fn socket_state_from_value(value: &DynValue, owner: Option<&DynValue>) -> AnyMap {
    let Some(object) = value.object_ref() else {
        return AnyMap::new();
    };
    let object = object.borrow();
    let builtin = [
        "name",
        "objecttype",
        "address",
        "error",
        "ipaddress",
        "isconnected",
        "port",
        "parent",
        "data",
        "packagedelimiter",
        "enablessl",
        "sslcertfile",
        "sslkeyfile",
        "sslcipherlist",
        "bind",
        "connect",
        "close",
        "destroy",
        "send",
        "senddata",
        "sendudp",
        "join",
        "trigger",
        "__tsocket",
        "__tsocket_name",
        "__tsocket_id",
        "__buffer",
        "__classes",
        "__parent_name",
        "__parent_id",
    ];
    let mut state = AnyMap::new();
    for (key, item) in &object.properties {
        if key.starts_with("__") || builtin.iter().any(|x| x.eq_ignore_ascii_case(key)) {
            continue;
        }
        state.insert(key.clone(), value_to_json(item, owner, &mut Vec::new()));
    }
    state
}

fn socket_joined_classes(value: &DynValue) -> Vec<String> {
    array_values(&get_property(value, "__classes"))
        .into_iter()
        .map(|x| value_string(&x).trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn array_values(value: &DynValue) -> Vec<DynValue> {
    match value {
        DynValue::Array(values) => values.borrow().clone(),
        DynValue::Undefined | DynValue::Null => Vec::new(),
        _ => Vec::new(),
    }
}

fn receiver_method_needs_container(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "clearvars"
            | "clearemptyvars"
            | "loadfolder"
            | "loadlines"
            | "loadstring"
            | "loadini"
            | "loadvarsfromarray"
            | "savevars"
            | "savevarstoarray"
            | "savelines"
            | "savestring"
            | "savejson"
            | "savejsontostring"
            | "savexmltostring"
            | "savexml"
            | "loadjsonfromstring"
            | "loadxmlfromstring"
            | "loadjson"
            | "loadxml"
    )
}

fn method_prefers_array(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "loadfolder" | "loadlines" | "loadvarsfromarray" | "savevarstoarray" | "savelines"
    )
}

fn new_array_from_dimensions(dimensions: &[DynValue]) -> DynValue {
    let sizes = dimensions
        .iter()
        .map(|value| number_i64(value).max(0) as usize)
        .collect::<Vec<_>>();
    fn build(sizes: &[usize]) -> DynValue {
        let size = sizes.first().copied().unwrap_or(0);
        if sizes.len() <= 1 {
            return DynValue::array(vec![DynValue::Undefined; size]);
        }
        DynValue::array((0..size).map(|_| build(&sizes[1..])).collect())
    }
    build(&sizes)
}

fn string_map_object(values: &HashMap<String, String>) -> DynValue {
    let object = DynValue::plain();
    for (key, value) in values {
        set_property(&object, key, mapped_string_value(value));
    }
    object
}

fn flag_object(values: &HashMap<String, String>, prefix: &str) -> DynValue {
    let object = DynValue::plain();
    for (key, value) in values {
        if key
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
        {
            set_property(&object, &key[prefix.len()..], mapped_string_value(value));
        }
    }
    object
}

fn typed_string_value(value: &str) -> DynValue {
    if value.eq_ignore_ascii_case("true") {
        DynValue::Bool(true)
    } else if value.eq_ignore_ascii_case("false") {
        DynValue::Bool(false)
    } else {
        DynValue::String(value.trim().to_string())
    }
}

fn mapped_string_value(value: &str) -> DynValue {
    if value.contains(',') {
        DynValue::array(value.split(',').map(typed_string_value).collect())
    } else {
        typed_string_value(value)
    }
}

fn make_weapon_object(value: &WeaponContext) -> DynValue {
    let object = DynValue::plain();
    set_property(&object, "name", DynValue::String(value.name.clone()));
    set_property(&object, "image", DynValue::String(value.image.clone()));
    object
}

fn make_server_object(value: &ServerContext) -> DynValue {
    let object = DynValue::plain();
    let name = clean_server_name(&value.name);
    set_property(&object, "name", DynValue::String(name));
    set_property(&object, "type", DynValue::String(value.r#type.clone()));
    set_property(
        &object,
        "players",
        DynValue::Number(value.player_count as f64),
    );
    set_property(
        &object,
        "playercount",
        DynValue::Number(value.player_count as f64),
    );
    set_property(
        &object,
        "language",
        DynValue::String(value.language.clone()),
    );
    set_property(
        &object,
        "description",
        DynValue::String(value.description.clone()),
    );
    set_property(&object, "url", DynValue::String(value.url.clone()));
    set_property(&object, "website", DynValue::String(value.url.clone()));
    set_property(&object, "version", DynValue::String(value.version.clone()));
    set_property(
        &object,
        "gameversions",
        DynValue::String(value.game_versions.clone()),
    );
    set_property(&object, "latency", DynValue::Number(value.latency as f64));
    object
}

fn clean_server_name(value: &str) -> String {
    let mut value = value.trim().to_string();
    while value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value = value[1..value.len() - 1].trim().to_string();
    }
    value
}

struct HttpResponse {
    status_code: u16,
    status: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn make_http_request_object(request_url: &str) -> DynValue {
    let object = DynValue::plain();
    set_property(
        &object,
        "objecttype",
        DynValue::String("TCURLRequest".to_string()),
    );
    set_property(&object, "url", DynValue::String(request_url.to_string()));
    set_property(&object, "data", DynValue::String(String::new()));
    set_property(&object, "fulldata", DynValue::String(String::new()));
    set_property(&object, "contenttype", DynValue::String(String::new()));
    set_property(&object, "contentlength", DynValue::Number(0.0));
    set_property(&object, "lastmodified", DynValue::String(String::new()));
    set_property(&object, "requestdata", DynValue::String(String::new()));
    set_property(&object, "headers", DynValue::array(Vec::new()));
    set_property(&object, "post", DynValue::Bool(false));
    set_property(&object, "postdata", DynValue::String(String::new()));
    set_property(&object, "quick", DynValue::Bool(false));
    set_property(&object, "skipsslverification", DynValue::Bool(false));
    set_property(&object, "returncode", DynValue::Number(0.0));
    set_property(&object, "statuscode", DynValue::Number(0.0));
    set_property(&object, "returnmessage", DynValue::String(String::new()));
    set_property(&object, "completed", DynValue::Bool(false));
    set_property(&object, "error", DynValue::String(String::new()));
    set_property(&object, "__event_onReceiveData", DynValue::Bool(false));
    set_property(
        &object,
        "sendrequest",
        DynValue::Builtin("method:sendrequest".to_string()),
    );
    object
}

fn http_request(
    request_url: &str,
    method: &str,
    body: &[u8],
    headers: &[String],
    insecure_tls: bool,
) -> std::result::Result<HttpResponse, String> {
    let parsed = url::Url::parse(request_url).map_err(|error| error.to_string())?;
    if parsed.scheme().eq_ignore_ascii_case("https") {
        return https_request(request_url, method, body, headers, insecure_tls);
    }
    if !parsed.scheme().eq_ignore_ascii_case("http") {
        return Err(format!("unsupported URL scheme: {}", parsed.scheme()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "missing HTTP host".to_string())?;
    let port = parsed.port_or_known_default().unwrap_or(80);
    let address = (host, port)
        .to_socket_addrs()
        .map_err(|error| error.to_string())?
        .next()
        .ok_or_else(|| "unable to resolve HTTP host".to_string())?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(30))
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;

    let mut target = parsed.path().to_string();
    if target.is_empty() {
        target.push('/');
    }
    if let Some(query) = parsed.query() {
        target.push('?');
        target.push_str(query);
    }
    let mut request = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
        method, target, host
    );
    let mut has_content_length = false;
    for header in headers {
        let Some((key, value)) = header.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty()
            || key.eq_ignore_ascii_case("host")
            || key.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        if key.eq_ignore_ascii_case("content-length") {
            has_content_length = true;
        }
        request.push_str(key);
        request.push_str(": ");
        request.push_str(value.trim());
        request.push_str("\r\n");
    }
    if method.eq_ignore_ascii_case("POST") && !has_content_length {
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|error| error.to_string())?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|error| error.to_string())?;
    let header_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "invalid HTTP response".to_string())?;
    let header_text = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = header_text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| "missing HTTP status".to_string())?;
    let mut status_parts = status_line.splitn(3, ' ');
    let _http_version = status_parts.next().unwrap_or_default();
    let status_code = status_parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "invalid HTTP status".to_string())?;
    let reason = status_parts.next().unwrap_or_default().trim();
    let status = if reason.is_empty() {
        status_code.to_string()
    } else {
        format!("{} {}", status_code, reason)
    };
    let mut response_headers = HashMap::new();
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        response_headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    let body = decode_http_body(&raw[header_end + 4..], &response_headers)?;
    Ok(HttpResponse {
        status_code,
        status,
        headers: response_headers,
        body,
    })
}

fn https_request(
    request_url: &str,
    method: &str,
    body: &[u8],
    headers: &[String],
    insecure_tls: bool,
) -> std::result::Result<HttpResponse, String> {
    let (status_code, status, response_headers, body) =
        curl_ffi::request(request_url, method, body, headers, insecure_tls)?;
    Ok(HttpResponse {
        status_code,
        status,
        headers: response_headers,
        body,
    })
}

fn decode_http_body(
    raw: &[u8],
    headers: &HashMap<String, String>,
) -> std::result::Result<Vec<u8>, String> {
    if !headers
        .get("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        return Ok(raw.to_vec());
    }
    let mut output = Vec::new();
    let mut cursor = 0usize;
    loop {
        let Some(end) = raw[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
        else {
            return Err("invalid chunked HTTP response".to_string());
        };
        let line_end = cursor + end;
        let size_text = String::from_utf8_lossy(&raw[cursor..line_end]);
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| "invalid chunk size".to_string())?;
        cursor = line_end + 2;
        if size == 0 {
            return Ok(output);
        }
        if raw.len().saturating_sub(cursor) < size + 2 {
            return Err("truncated chunked HTTP response".to_string());
        }
        output.extend_from_slice(&raw[cursor..cursor + size]);
        cursor += size;
        if &raw[cursor..cursor + 2] != b"\r\n" {
            return Err("invalid chunk terminator".to_string());
        }
        cursor += 2;
    }
}

fn split_http_lines(value: &[u8]) -> Vec<DynValue> {
    String::from_utf8_lossy(value)
        .replace("\r\n", "\n")
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| DynValue::String(line.to_string()))
        .collect()
}

fn make_sql_request_object() -> DynValue {
    let object = DynValue::plain();
    set_property(
        &object,
        "objecttype",
        DynValue::String("TSQLRequest".to_string()),
    );
    set_property(&object, "completed", DynValue::Bool(false));
    set_property(&object, "error", DynValue::String(String::new()));
    set_property(&object, "rows", DynValue::array(Vec::new()));
    set_property(&object, "lastinsertid", DynValue::Number(0.0));
    set_property(&object, "__event_onReceiveData", DynValue::Bool(false));
    object
}

fn sqlite_database_path(root: &str, db_name: &str) -> std::result::Result<PathBuf, String> {
    if root.trim().is_empty() {
        return Err("missing VM file root".to_string());
    }
    let mut name = db_name.trim().to_string();
    if name.is_empty() || name.eq_ignore_ascii_case("main") {
        name = "main.db".to_string();
    }
    if Path::new(&name).extension().is_none() {
        name.push_str(".db");
    }
    let name = name.replace('\\', "/").trim_start_matches('/').to_string();
    if name.contains("..") || name.starts_with('/') || Path::new(&name).is_absolute() {
        return Err("invalid database name".to_string());
    }
    let relative = if name.to_ascii_lowercase().starts_with("databases/") {
        name
    } else {
        format!("databases/{name}")
    };
    let full = Path::new(root).join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    Ok(full)
}

fn split_sql_commas(value: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut start = 0usize;
    let mut quote = false;
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            if quote && index + 1 < bytes.len() && bytes[index + 1] == b'\'' {
                index += 2;
                continue;
            }
            quote = !quote;
        } else if bytes[index] == b',' && !quote {
            output.push(value[start..index].trim().to_string());
            start = index + 1;
        }
        index += 1;
    }
    output.push(value[start..].trim().to_string());
    output
}

fn parse_sql_value(value: String) -> DynValue {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return DynValue::String(value[1..value.len() - 1].replace("''", "'"));
    }
    if value.eq_ignore_ascii_case("null") {
        DynValue::Null
    } else if value.eq_ignore_ascii_case("true") {
        DynValue::Bool(true)
    } else if value.eq_ignore_ascii_case("false") {
        DynValue::Bool(false)
    } else if let Ok(number) = value.parse::<f64>() {
        DynValue::Number(number)
    } else {
        DynValue::String(value.to_string())
    }
}

fn escape_sql_string2(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if ch < ' ' && !matches!(ch, '\t' | '\n' | '\r') {
            continue;
        }
        if ch == '\'' {
            output.push_str("''");
        } else {
            output.push(ch);
        }
    }
    output
}

fn make_player_object(
    context: &PlayerContext,
    online: bool,
    server_player: bool,
    account_loaded: bool,
) -> DynValue {
    let object = DynValue::object(ObjectKind::Player {
        account: context.account.clone(),
        online,
        server_player,
    });
    set_property(&object, "id", DynValue::Number(context.id as f64));
    set_property(
        &object,
        "account",
        DynValue::String(context.account.clone()),
    );
    set_property(
        &object,
        "nick",
        DynValue::String(first_non_empty(&context.nick, &context.nickname)),
    );
    set_property(
        &object,
        "nickname",
        DynValue::String(first_non_empty(&context.nickname, &context.nick)),
    );
    set_property(&object, "guild", DynValue::String(context.guild.clone()));
    set_property(
        &object,
        "levelname",
        DynValue::String(context.level.clone()),
    );
    set_property(&object, "level", make_level_object(&context.level));
    set_property(&object, "x", DynValue::Number(context.x));
    set_property(&object, "y", DynValue::Number(context.y));
    set_property(
        &object,
        "dir",
        DynValue::Number(normalize_player_dir(context.dir) as f64),
    );
    set_property(
        &object,
        "adminlevel",
        DynValue::Number(context.admin_level as f64),
    );
    set_property(&object, "online", DynValue::Bool(online));
    set_property(&object, "isloggedin", DynValue::Bool(online));
    set_property(
        &object,
        "onlinetime",
        DynValue::Number(context.online_time as f64),
    );
    set_property(&object, "__tserverplayer", DynValue::Bool(server_player));
    set_property(&object, "__accountloaded", DynValue::Bool(account_loaded));
    // HexaVM only materializes an objecttype property for TServerPlayer.
    // Ordinary player objects inherit the generic graal-variable behavior;
    // exposing TPlayer here changes both direct reads and object var lists.
    if server_player {
        set_property(
            &object,
            "objecttype",
            DynValue::String("TServerPlayer".to_string()),
        );
    }
    set_property(&object, "paused", DynValue::Bool(false));
    for (name, value) in [
        ("reading", DynValue::Bool(false)),
        ("swimming", DynValue::Bool(false)),
        ("onhorse", DynValue::Bool(false)),
        ("isobserver", DynValue::Bool(false)),
        ("ismale", DynValue::Bool(false)),
        ("isfemale", DynValue::Bool(false)),
        ("isjumping", DynValue::Bool(false)),
        ("hurted", DynValue::Bool(false)),
    ] {
        set_property(&object, name, value);
    }
    for (name, value) in [
        ("hurtdx", 0.0),
        ("hurtdy", 0.0),
        ("hurtpower", 0.0),
        ("freezetime", 0.0),
        ("defaultwalkspeed", 1.0),
        ("diagonalwalkspeed", 1.0),
        ("zoomfactor", 1.0),
    ] {
        set_property(&object, name, DynValue::Number(value));
    }
    set_property(
        &object,
        "__rights",
        DynValue::array(
            context
                .rights
                .iter()
                .cloned()
                .map(DynValue::String)
                .collect(),
        ),
    );
    set_property(
        &object,
        "__folders",
        DynValue::array(
            context
                .folders
                .iter()
                .cloned()
                .map(DynValue::String)
                .collect(),
        ),
    );
    let client = flag_object(&context.flags, "client.");
    let clientr = flag_object(&context.flags, "clientr.");
    set_property(&object, "client", client);
    set_property(&object, "clientr", clientr);
    object
}

fn make_level_object(name: &str) -> DynValue {
    let object = DynValue::object(ObjectKind::Level {
        name: name.to_string(),
    });
    set_property(&object, "name", DynValue::String(name.to_string()));
    // These are the fixed level metadata values exposed by HexaVM's
    // playerLevelObject/levelObject helpers.  Keeping them on the object (as
    // opposed to synthesising them only in method dispatch) also preserves
    // ordinary property reads and serialization behavior.
    for (key, value) in [("width", 64.0), ("height", 64.0), ("tilelayercount", 1.0)] {
        set_property(&object, key, DynValue::Number(value));
    }
    for key in ["isnopkzone", "nopkzone", "issparringzone", "compsdead"] {
        set_property(&object, key, DynValue::Bool(false));
    }
    object
}

fn player_context_from_map(
    values: &HashMap<String, String>,
    flags: &HashMap<String, String>,
) -> PlayerContext {
    let get = |key: &str| values.get(key).cloned().unwrap_or_default();
    PlayerContext {
        id: get("id").parse().unwrap_or(0),
        account: get("account"),
        nick: first_non_empty(&get("nick"), &get("nickname")),
        nickname: first_non_empty(&get("nickname"), &get("nick")),
        guild: get("guild"),
        level: get("level"),
        dir: normalize_player_dir(get("dir").parse().unwrap_or(0)),
        x: get("x").parse().unwrap_or(0.0),
        y: get("y").parse().unwrap_or(0.0),
        online_time: get("onlinetime").parse().unwrap_or(0),
        admin_level: get("adminlevel").parse().unwrap_or(0),
        flags: flags.clone(),
        rights: split_csv(&get("rights")),
        folders: get("folders")
            .replace("\r\n", "\n")
            .split('\n')
            .map(str::to_string)
            .collect(),
    }
}

fn first_non_empty(left: &str, right: &str) -> String {
    if !left.is_empty() {
        left.to_string()
    } else {
        right.to_string()
    }
}

fn gs1_player_prop_name(value: &str) -> String {
    let value = value.trim();
    match value.to_ascii_lowercase().as_str() {
        "#c" => "chat".to_string(),
        "#n" => "nick".to_string(),
        "#m" | "ani" => "ani".to_string(),
        "#g" | "guild" => "guild".to_string(),
        "dir" => "dir".to_string(),
        "#3" => "head".to_string(),
        "#8" => "body".to_string(),
        "#1" => "sword".to_string(),
        "#2" => "shield".to_string(),
        value if value.starts_with("#c") && value[2..].parse::<usize>().is_ok() => {
            format!("colors[{}]", &value[2..])
        }
        value if value.starts_with("#p") && value[2..].parse::<usize>().is_ok() => {
            format!("attr[{}]", &value[2..])
        }
        _ => value.to_string(),
    }
}

fn normalize_player_dir(value: i32) -> i32 {
    if (0..=3).contains(&value) { value } else { 2 }
}
fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect()
}

fn value_lines(value: &DynValue) -> Vec<String> {
    match value {
        DynValue::Array(values) => values.borrow().iter().map(value_string).collect(),
        DynValue::Undefined | DynValue::Null => Vec::new(),
        DynValue::Object(object) => {
            let object = object.borrow();
            let length = object
                .properties
                .get("length")
                .map(number_i64)
                .filter(|length| *length >= 0)
                .map(|length| length as usize);
            if let Some(length) = length {
                return (0..length)
                    .map(|index| {
                        object
                            .properties
                            .get(&index.to_string())
                            .map(value_string)
                            .unwrap_or_default()
                    })
                    .collect();
            }
            vec![value_string(value)]
        }
        _ => value_string(value)
            .replace("\r\n", "\n")
            .split('\n')
            .map(str::to_string)
            .collect(),
    }
}

fn value_bytes(value: &DynValue) -> Vec<u8> {
    match value {
        DynValue::Bytes(value) => value.clone(),
        DynValue::String(value) => value.as_bytes().to_vec(),
        _ => value_string(value).into_bytes(),
    }
}

fn replace_sequence(value: &DynValue, items: Vec<DynValue>) {
    match value {
        DynValue::Array(values) => *values.borrow_mut() = items,
        DynValue::Object(object) => {
            let numeric = object
                .borrow()
                .properties
                .keys()
                .filter(|key| key.parse::<usize>().is_ok())
                .cloned()
                .collect::<Vec<_>>();
            let mut object = object.borrow_mut();
            for key in numeric {
                object.properties.remove(&key);
            }
            for (index, item) in items.into_iter().enumerate() {
                object.properties.insert(index.to_string(), item);
            }
            // Goja's legacy helpers treat a plain object used as an
            // array-like receiver as having an observable length.
            let length = object
                .properties
                .keys()
                .filter(|key| key.parse::<usize>().is_ok())
                .count();
            object
                .properties
                .insert("length".to_string(), DynValue::Number(length as f64));
        }
        _ => {}
    }
}

fn clear_object_properties(value: &DynValue) {
    if let Some(object) = value.object_ref() {
        object.borrow_mut().properties.clear();
    }
}

fn clear_vm_vars(value: &DynValue) {
    if let Some(object) = value.object_ref() {
        object
            .borrow_mut()
            .properties
            .retain(|key, _| key.eq_ignore_ascii_case("__gs2value"));
    }
}

fn is_empty_global_value(value: &DynValue) -> bool {
    match value {
        DynValue::Undefined | DynValue::Null => true,
        DynValue::String(text) => text.is_empty(),
        DynValue::Array(values) => values.borrow().is_empty(),
        DynValue::Object(object) => object.borrow().properties.is_empty(),
        DynValue::Function(_) | DynValue::Builtin(_) => false,
        DynValue::Bool(_) | DynValue::Number(_) | DynValue::Bytes(_) => false,
    }
}

fn replace_object_with_value(target: &DynValue, value: DynValue) {
    match value {
        DynValue::Object(source) => {
            clear_object_properties(target);
            if let Some(destination) = target.object_ref() {
                for (key, item) in source.borrow().properties.clone() {
                    destination.borrow_mut().properties.insert(key, item);
                }
            }
        }
        DynValue::Array(items) => {
            replace_sequence(target, items.borrow().clone());
        }
        scalar => {
            clear_object_properties(target);
            set_property(target, "text", scalar);
        }
    }
}

fn save_json_string(value: &DynValue, flags: i32) -> DynValue {
    let mut json = value_to_json(value, None, &mut Vec::new());
    if flags & 2 != 0 {
        json = json_comma_strings_to_arrays(json);
    }
    let serialized = if flags & 1 != 0 {
        serde_json::to_string_pretty(&json)
    } else {
        serde_json::to_string(&json)
    }
    .unwrap_or_default();
    DynValue::String(serialized)
}

fn json_comma_strings_to_arrays(value: Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, json_comma_strings_to_arrays(value)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(json_comma_strings_to_arrays)
                .collect(),
        ),
        Value::String(value) if value.contains(',') => Value::Array(
            value
                .split(',')
                .map(|item| Value::String(item.trim().to_string()))
                .collect(),
        ),
        value => value,
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn xml_name(value: &str) -> String {
    let mut output = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if output.is_empty() {
        output.push_str("value");
    }
    if output.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        output.insert(0, '_');
    }
    output
}

fn write_xml_value(output: &mut String, name: &str, value: &DynValue) {
    let name = xml_name(name);
    match value {
        DynValue::Object(_object) => {
            output.push('<');
            output.push_str(&name);
            output.push('>');
            for (key, item) in object_properties(value) {
                if key.starts_with("__")
                    || matches!(item, DynValue::Function(_) | DynValue::Builtin(_))
                {
                    continue;
                }
                write_xml_value(output, &key, &item);
            }
            output.push_str("</");
            output.push_str(&name);
            output.push('>');
        }
        DynValue::Array(values) => {
            for item in values.borrow().iter() {
                write_xml_value(output, &name, item);
            }
        }
        DynValue::Undefined | DynValue::Null => {
            output.push('<');
            output.push_str(&name);
            output.push_str("></");
            output.push_str(&name);
            output.push('>');
        }
        value => {
            output.push('<');
            output.push_str(&name);
            output.push('>');
            output.push_str(&xml_escape(&value_string(value)));
            output.push_str("</");
            output.push_str(&name);
            output.push('>');
        }
    }
}

fn save_xml_string(value: &DynValue) -> String {
    let mut output = String::from("<graalvar>");
    write_xml_value(&mut output, "value", value);
    output.push_str("</graalvar>");
    output
}

fn parse_xml_value(text: &str) -> Option<DynValue> {
    struct Cursor<'a> {
        text: &'a str,
        position: usize,
    }

    fn skip_space(cursor: &mut Cursor<'_>) {
        while cursor.position < cursor.text.len()
            && cursor.text.as_bytes()[cursor.position].is_ascii_whitespace()
        {
            cursor.position += 1;
        }
    }

    fn parse_node(cursor: &mut Cursor<'_>) -> Option<(String, DynValue)> {
        skip_space(cursor);
        if cursor.text[cursor.position..].starts_with("<?") {
            let end = cursor.text[cursor.position..].find("?>")?;
            cursor.position += end + 2;
            skip_space(cursor);
        }
        if !cursor.text[cursor.position..].starts_with('<') {
            return None;
        }
        let open_end = cursor.text[cursor.position..].find('>')? + cursor.position;
        let open = cursor.text[cursor.position + 1..open_end].trim();
        let self_closing = open.ends_with('/');
        let tag = open
            .trim_end_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        if tag.is_empty() {
            return None;
        }
        cursor.position = open_end + 1;
        if self_closing {
            return Some((tag, DynValue::String(String::new())));
        }
        let close = format!("</{tag}>");
        let mut children = Vec::new();
        let mut text_value = String::new();
        loop {
            skip_space(cursor);
            if cursor.text[cursor.position..].starts_with(&close) {
                cursor.position += close.len();
                break;
            }
            if cursor.position >= cursor.text.len() {
                return None;
            }
            if cursor.text.as_bytes()[cursor.position] == b'<' {
                if cursor.text[cursor.position..].starts_with("<!--") {
                    let end = cursor.text[cursor.position + 4..].find("-->")?;
                    cursor.position += end + 7;
                    continue;
                }
                children.push(parse_node(cursor)?);
            } else {
                let end = cursor.text[cursor.position..]
                    .find('<')
                    .map_or(cursor.text.len(), |value| cursor.position + value);
                text_value.push_str(&cursor.text[cursor.position..end]);
                cursor.position = end;
            }
        }
        if children.is_empty() {
            return Some((tag, DynValue::String(xml_unescape(text_value.trim()))));
        }
        let object = DynValue::plain();
        if !text_value.trim().is_empty() {
            set_property(
                &object,
                "text",
                DynValue::String(xml_unescape(text_value.trim())),
            );
        }
        for (child_name, child_value) in children {
            let current = get_property(&object, &child_name);
            if current.is_undefined() {
                set_property(&object, &child_name, child_value);
            } else if let DynValue::Array(values) = current {
                values.borrow_mut().push(child_value);
            } else {
                set_property(
                    &object,
                    &child_name,
                    DynValue::array(vec![current, child_value]),
                );
            }
        }
        Some((tag, object))
    }

    let mut cursor = Cursor {
        text: text.trim(),
        position: 0,
    };
    let (_, value) = parse_node(&mut cursor)?;
    if let DynValue::Object(_object) = &value {
        if let Some(inner) = object_properties(&value)
            .into_iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("value"))
            .map(|(_, value)| value)
        {
            return Some(inner);
        }
        return Some(value);
    }
    Some(value)
}

fn format_string(args: &[DynValue]) -> String {
    let Some(format) = args.first().map(value_string) else {
        return String::new();
    };
    let mut result = String::new();
    let mut values = args.iter().skip(1);
    let mut chars = format.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            if let Some(next) = chars.peek().copied() {
                if matches!(next, 's' | 'd' | 'f' | 'i') {
                    chars.next();
                    if let Some(value) = values.next() {
                        result.push_str(&value_string(value));
                    }
                    continue;
                }
            }
        }
        result.push(ch);
    }
    result
}

fn format2_string(args: &[DynValue]) -> String {
    let Some(format) = args.first().map(value_string) else {
        return String::new();
    };
    let values = if args
        .get(1)
        .is_some_and(|value| matches!(value, DynValue::Array(_)))
    {
        array_values(args.get(1).unwrap_or(&DynValue::Undefined))
    } else {
        args.iter().skip(1).cloned().collect()
    };
    let mut index = 0usize;
    let mut output = String::new();
    let mut chars = format.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' && chars.peek().is_some_and(|next| *next == 's') {
            let _ = chars.next();
            if let Some(value) = values.get(index) {
                output.push_str(&value_string(value));
            }
            index += 1;
        } else {
            output.push(ch);
        }
    }
    output
}

fn query_unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => output.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                let high = (bytes[index + 1] as char).to_digit(16);
                let low = (bytes[index + 2] as char).to_digit(16);
                if let (Some(high), Some(low)) = (high, low) {
                    output.push((high * 16 + low) as u8);
                    index += 2;
                } else {
                    output.push(bytes[index]);
                }
            }
            byte => output.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn escape_mysql_string(value: &str, keep_newlines: bool) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if ch < ' ' && (!keep_newlines || !matches!(ch, '\n' | '\r')) {
            continue;
        }
        match ch {
            '\\' => output.push_str("\\\\"),
            '\'' => output.push_str("\\'"),
            '"' => output.push_str("\\\""),
            _ => output.push(ch),
        }
    }
    output
}

fn escape_filename_value(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            output.push(ch);
        } else {
            output.push_str(&format!("%{:03}", ch as u32));
        }
    }
    output
}

fn unescape_filename_value(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = String::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 3 < bytes.len() {
            if let Ok(code) = value[index + 1..index + 4].parse::<u32>() {
                if let Some(ch) = char::from_u32(code) {
                    output.push(ch);
                    index += 4;
                    continue;
                }
            }
        }
        output.push(bytes[index] as char);
        index += 1;
    }
    output
}

fn wrap_text(length: i32, delims: &str, text: &str) -> Vec<String> {
    if length <= 0 {
        return vec![text.to_string()];
    }
    let words = text
        .split(|ch: char| ch.is_whitespace() || delims.contains(ch))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut line = String::new();
    for word in words {
        if line.is_empty() {
            line = word.to_string();
        } else if line.len() + 1 + word.len() <= length as usize {
            line.push(' ');
            line.push_str(word);
        } else {
            output.push(std::mem::take(&mut line));
            line = word.to_string();
        }
    }
    if !line.is_empty() {
        output.push(line);
    }
    output
}

fn mutate_string_array(value: &DynValue, mode: &str, index: i64, needle: &str, replacement: &str) {
    let mut lines = value_lines(value);
    match mode {
        "add" => lines.push(needle.to_string()),
        "insert" => {
            let index = index.max(0).min(lines.len() as i64) as usize;
            lines.insert(index, needle.to_string());
        }
        "replace" => {
            if let Some(item) = lines.iter_mut().find(|item| item.as_str() == needle) {
                *item = replacement.to_string();
            }
        }
        "remove" => {
            if let Some(index) = lines.iter().position(|item| item == needle) {
                lines.remove(index);
            }
        }
        "delete" if index >= 0 && (index as usize) < lines.len() => {
            lines.remove(index as usize);
        }
        _ => {}
    }
    replace_sequence(value, lines.into_iter().map(DynValue::String).collect());
}

#[derive(Clone)]
enum SimpleRegexAtom {
    Literal(char),
    Any,
    Class(Vec<(char, char)>, bool),
    Start,
    End,
}

#[derive(Clone)]
struct SimpleRegexToken {
    atom: SimpleRegexAtom,
    min: usize,
    max: usize,
}

fn simple_regex_patterns(pattern: &str) -> Vec<&str> {
    let mut output = Vec::new();
    let mut start = 0usize;
    let mut escaped = false;
    let mut class = false;
    let mut depth = 0usize;
    for (index, ch) in pattern.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '[' {
            class = true;
        } else if ch == ']' {
            class = false;
        } else if !class && ch == '(' {
            depth += 1;
        } else if !class && ch == ')' {
            depth = depth.saturating_sub(1);
        } else if !class && depth == 0 && ch == '|' {
            output.push(&pattern[start..index]);
            start = index + 1;
        }
    }
    output.push(&pattern[start..]);
    output
}

fn parse_simple_regex(pattern: &str) -> Option<Vec<SimpleRegexToken>> {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        let atom = match chars[index] {
            '^' if index == 0 => {
                index += 1;
                SimpleRegexAtom::Start
            }
            '$' if index + 1 == chars.len() => {
                index += 1;
                SimpleRegexAtom::End
            }
            '.' => {
                index += 1;
                SimpleRegexAtom::Any
            }
            '\\' => {
                index += 1;
                let escaped = *chars.get(index)?;
                index += 1;
                match escaped {
                    'd' => SimpleRegexAtom::Class(vec![('0', '9')], false),
                    'D' => SimpleRegexAtom::Class(vec![('0', '9')], true),
                    'w' => SimpleRegexAtom::Class(
                        vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')],
                        false,
                    ),
                    'W' => SimpleRegexAtom::Class(
                        vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')],
                        true,
                    ),
                    's' => SimpleRegexAtom::Class(
                        vec![('\t', '\t'), ('\n', '\n'), ('\r', '\r'), (' ', ' ')],
                        false,
                    ),
                    'S' => SimpleRegexAtom::Class(
                        vec![('\t', '\t'), ('\n', '\n'), ('\r', '\r'), (' ', ' ')],
                        true,
                    ),
                    value => SimpleRegexAtom::Literal(value),
                }
            }
            '[' => {
                index += 1;
                let mut inverted = false;
                if chars.get(index) == Some(&'^') {
                    inverted = true;
                    index += 1;
                }
                let mut ranges = Vec::new();
                while index < chars.len() && chars[index] != ']' {
                    let first = if chars[index] == '\\' {
                        index += 1;
                        *chars.get(index)?
                    } else {
                        chars[index]
                    };
                    index += 1;
                    if index + 1 < chars.len() && chars[index] == '-' && chars[index + 1] != ']' {
                        index += 1;
                        let last = if chars[index] == '\\' {
                            index += 1;
                            *chars.get(index)?
                        } else {
                            chars[index]
                        };
                        index += 1;
                        ranges.push((first, last));
                    } else {
                        ranges.push((first, first));
                    }
                }
                if chars.get(index) != Some(&']') || ranges.is_empty() {
                    return None;
                }
                index += 1;
                SimpleRegexAtom::Class(ranges, inverted)
            }
            '(' | ')' => {
                index += 1;
                continue;
            }
            '|' => {
                index += 1;
                continue;
            }
            value => {
                index += 1;
                SimpleRegexAtom::Literal(value)
            }
        };
        let (min, max) = match chars.get(index) {
            Some('*') => {
                index += 1;
                (0, usize::MAX)
            }
            Some('+') => {
                index += 1;
                (1, usize::MAX)
            }
            Some('?') => {
                index += 1;
                (0, 1)
            }
            Some('{') => {
                let close = chars[index + 1..]
                    .iter()
                    .position(|ch| *ch == '}')
                    .map(|value| value + index + 1);
                if let Some(close) = close {
                    let numbers = chars[index + 1..close].iter().collect::<String>();
                    let mut parts = numbers.splitn(2, ',');
                    let min = parts.next().and_then(|value| value.parse().ok())?;
                    let max = parts.next().map_or(min, |value| {
                        if value.trim().is_empty() {
                            usize::MAX
                        } else {
                            value.parse().unwrap_or(min)
                        }
                    });
                    index = close + 1;
                    (min, max)
                } else {
                    (1, 1)
                }
            }
            _ => (1, 1),
        };
        tokens.push(SimpleRegexToken { atom, min, max });
    }
    Some(tokens)
}

fn simple_regex_atom_matches(atom: &SimpleRegexAtom, value: char) -> bool {
    match atom {
        SimpleRegexAtom::Literal(wanted) => *wanted == value,
        SimpleRegexAtom::Any => true,
        SimpleRegexAtom::Class(ranges, inverted) => {
            let found = ranges
                .iter()
                .any(|(first, last)| *first <= value && value <= *last);
            if *inverted { !found } else { found }
        }
        SimpleRegexAtom::Start | SimpleRegexAtom::End => false,
    }
}

fn simple_regex_match_tokens(
    tokens: &[SimpleRegexToken],
    token_index: usize,
    text: &[char],
    text_index: usize,
) -> Option<usize> {
    if token_index >= tokens.len() {
        return Some(text_index);
    }
    let token = &tokens[token_index];
    if matches!(token.atom, SimpleRegexAtom::Start) {
        return (text_index == 0)
            .then(|| simple_regex_match_tokens(tokens, token_index + 1, text, text_index))?;
    }
    if matches!(token.atom, SimpleRegexAtom::End) {
        return (text_index == text.len())
            .then_some(text_index)
            .and_then(|index| simple_regex_match_tokens(tokens, token_index + 1, text, index));
    }
    let mut max_count = 0usize;
    while max_count < token.max
        && text_index + max_count < text.len()
        && simple_regex_atom_matches(&token.atom, text[text_index + max_count])
    {
        max_count += 1;
    }
    let mut count = max_count;
    loop {
        if count >= token.min {
            if let Some(end) =
                simple_regex_match_tokens(tokens, token_index + 1, text, text_index + count)
            {
                return Some(end);
            }
        }
        if count == 0 {
            break;
        }
        count -= 1;
        while count > 0 && !simple_regex_atom_matches(&token.atom, text[text_index + count - 1]) {
            count -= 1;
        }
    }
    None
}

fn simple_regex_find_one(text: &str, pattern: &str) -> Option<(usize, usize)> {
    let text_chars = text.chars().collect::<Vec<_>>();
    for alternative in simple_regex_patterns(pattern) {
        let tokens = parse_simple_regex(alternative)?;
        for start in 0..=text_chars.len() {
            if let Some(end) = simple_regex_match_tokens(&tokens, 0, &text_chars, start) {
                return Some((start, end));
            }
        }
    }
    None
}

fn char_slice(text: &[char], start: usize, end: usize) -> String {
    text[start.min(text.len())..end.min(text.len())]
        .iter()
        .collect()
}

fn simple_regex_match(text: &str, pattern: &str, full: bool) -> bool {
    let chars = text.chars().collect::<Vec<_>>();
    simple_regex_patterns(pattern)
        .into_iter()
        .any(|alternative| {
            let Some(tokens) = parse_simple_regex(alternative) else {
                return false;
            };
            simple_regex_match_tokens(&tokens, 0, &chars, 0) == Some(chars.len())
                || (!full && simple_regex_match_tokens(&tokens, 0, &chars, 0).is_some())
        })
}

fn simple_regex_find(text: &str, pattern: &str) -> Option<String> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut found = None;
    for alternative in simple_regex_patterns(pattern) {
        let tokens = parse_simple_regex(alternative)?;
        for start in 0..=chars.len() {
            if let Some(end) = simple_regex_match_tokens(&tokens, 0, &chars, start) {
                if found.map_or(true, |(old_start, _): (usize, usize)| start < old_start) {
                    found = Some((start, end));
                }
                break;
            }
        }
    }
    found.map(|(start, end)| char_slice(&chars, start, end))
}

fn simple_regex_find_all(text: &str, pattern: &str) -> Vec<String> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut cursor = 0usize;
    while cursor <= chars.len() {
        let Some((start, end)) = simple_regex_find_range_from_chars(&chars, pattern, cursor) else {
            break;
        };
        output.push(char_slice(&chars, start, end));
        cursor = if end > start {
            end
        } else {
            start.saturating_add(1)
        };
    }
    output
}

fn simple_regex_find_range_from_chars(
    chars: &[char],
    pattern: &str,
    cursor: usize,
) -> Option<(usize, usize)> {
    let mut found = None;
    for alternative in simple_regex_patterns(pattern) {
        let tokens = parse_simple_regex(alternative)?;
        for start in cursor..=chars.len() {
            if let Some(end) = simple_regex_match_tokens(&tokens, 0, chars, start) {
                if found.map_or(true, |(old_start, _)| start < old_start) {
                    found = Some((start, end));
                }
                break;
            }
        }
    }
    found
}

fn simple_regex_replace(text: &str, pattern: &str, replacement: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut cursor = 0usize;
    while cursor <= chars.len() {
        let Some((start, end)) = simple_regex_find_range_from_chars(&chars, pattern, cursor) else {
            break;
        };
        output.push_str(&char_slice(&chars, cursor, start));
        output.push_str(replacement);
        cursor = if end > start {
            end
        } else {
            start.saturating_add(1)
        };
    }
    output.push_str(&char_slice(&chars, cursor, chars.len()));
    output
}

fn simple_regex_split(text: &str, pattern: &str) -> Vec<String> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut cursor = 0usize;
    while cursor <= chars.len() {
        let Some((start, end)) = simple_regex_find_range_from_chars(&chars, pattern, cursor) else {
            break;
        };
        output.push(char_slice(&chars, cursor, start));
        cursor = if end > start {
            end
        } else {
            start.saturating_add(1)
        };
    }
    output.push(char_slice(&chars, cursor, chars.len()));
    output
}

fn path_value_key(value: &DynValue) -> String {
    match value {
        DynValue::Undefined | DynValue::Null => "nil".to_string(),
        DynValue::String(value) => format!("s:{value}"),
        DynValue::Bool(value) => format!("b:{}", if *value { 1 } else { 0 }),
        DynValue::Number(value) => format!("n:{value}"),
        _ => format!("v:{}", value_string(value)),
    }
}

fn find_path_in_array(args: &[DynValue]) -> DynValue {
    if args.len() < 8 {
        return DynValue::Null;
    }
    let rows = array_values(&args[0]);
    if rows.is_empty() {
        return DynValue::Null;
    }
    let grid = rows.iter().map(array_values).collect::<Vec<_>>();
    let walkable = array_values(&args[1])
        .iter()
        .map(path_value_key)
        .collect::<std::collections::HashSet<_>>();
    let blocking = array_values(&args[2])
        .iter()
        .map(path_value_key)
        .collect::<std::collections::HashSet<_>>();
    let stop = array_values(&args[3])
        .iter()
        .map(path_value_key)
        .collect::<std::collections::HashSet<_>>();
    let no_stop = array_values(&args[4])
        .iter()
        .map(path_value_key)
        .collect::<std::collections::HashSet<_>>();
    let start_y = number_i64(&args[5]) as i32;
    let start_x = number_i64(&args[6]) as i32;
    let mut max_length = number_i64(&args[7]);
    if max_length <= 0 {
        max_length = (grid.len() * grid.first().map_or(0, Vec::len)) as i64;
    }
    let in_bounds = |y: i32, x: i32| {
        y >= 0 && (y as usize) < grid.len() && x >= 0 && (x as usize) < grid[y as usize].len()
    };
    if !in_bounds(start_y, start_x) {
        return DynValue::Null;
    }
    let allowed = |value: &DynValue| {
        let key = path_value_key(value);
        !blocking.contains(&key) && (walkable.is_empty() || walkable.contains(&key))
    };
    if !allowed(&grid[start_y as usize][start_x as usize]) {
        return DynValue::Null;
    }
    #[derive(Clone, Copy)]
    struct Node {
        y: i32,
        x: i32,
        parent: i32,
        depth: i64,
    }
    let mut queue = vec![Node {
        y: start_y,
        x: start_x,
        parent: -1,
        depth: 0,
    }];
    let mut visited = std::collections::HashSet::new();
    visited.insert((start_y, start_x));
    let mut found = None;
    let mut head = 0usize;
    while head < queue.len() {
        let current = queue[head];
        let value = &grid[current.y as usize][current.x as usize];
        let key = path_value_key(value);
        if stop.contains(&key) && !no_stop.contains(&key) {
            found = Some(head);
            break;
        }
        if current.depth + 1 < max_length {
            for (dy, dx) in [(0, 1), (1, 0), (0, -1), (-1, 0)] {
                let y = current.y + dy;
                let x = current.x + dx;
                if !in_bounds(y, x) || !allowed(&grid[y as usize][x as usize]) {
                    continue;
                }
                if visited.insert((y, x)) {
                    queue.push(Node {
                        y,
                        x,
                        parent: head as i32,
                        depth: current.depth + 1,
                    });
                }
            }
        }
        head += 1;
    }
    let Some(found) = found else {
        return DynValue::Null;
    };
    let mut indices = Vec::new();
    let mut current = found as i32;
    while current >= 0 {
        indices.push(current as usize);
        current = queue[current as usize].parent;
    }
    indices.reverse();
    DynValue::array(
        indices
            .into_iter()
            .map(|index| {
                let object = DynValue::plain();
                set_property(&object, "y", DynValue::Number(queue[index].y as f64));
                set_property(&object, "x", DynValue::Number(queue[index].x as f64));
                object
            })
            .collect(),
    )
}

fn add_socket_class(socket: &DynValue, class: &str) {
    if class.is_empty() {
        return;
    }
    let mut classes = array_values(&get_property(socket, "__classes"));
    if !classes
        .iter()
        .any(|value| value_string(value).eq_ignore_ascii_case(class))
    {
        classes.push(DynValue::String(class.to_string()));
    }
    set_property(socket, "__classes", DynValue::array(classes));
}

fn player_matches(player: &PlayerContext, target: &str) -> bool {
    let target = target.trim();
    [
        player.account.as_str(),
        player.nick.as_str(),
        player.nickname.as_str(),
    ]
    .iter()
    .any(|value| value.eq_ignore_ascii_case(target))
}

fn player_matches_insensitive(player: &PlayerContext, target: &str) -> bool {
    let target = target.trim().to_ascii_lowercase();
    !target.is_empty()
        && [
            player.account.as_str(),
            player.nick.as_str(),
            player.nickname.as_str(),
        ]
        .iter()
        .any(|value| value.trim().to_ascii_lowercase().contains(&target))
}

fn distance_sq(left_x: f64, left_y: f64, right_x: f64, right_y: f64) -> f64 {
    (left_x - right_x).powi(2) + (left_y - right_y).powi(2)
}

fn trigger_action_event_name(target: &str) -> String {
    match target.trim().to_ascii_lowercase().as_str() {
        "leftmouse" => "onActionLeftMouse".to_string(),
        "rightmouse" => "onActionRightMouse".to_string(),
        "middlemouse" => "onActionMiddleMouse".to_string(),
        "doublemouse" => "onActionDoubleMouse".to_string(),
        "" => "onAction".to_string(),
        _ => {
            let mut chars = target.chars();
            let first = chars
                .next()
                .map(|x| x.to_ascii_uppercase())
                .unwrap_or_default();
            format!("onAction{first}{}", chars.collect::<String>())
        }
    }
}

fn map_tile_type(tile: i32, layout: i32) -> i32 {
    if tile < 0 {
        return 0;
    }
    if layout != 0 {
        return TILE_TYPES1.get(tile as usize).copied().unwrap_or(0) as i32;
    }
    TILE_TYPES0.get(tile as usize).copied().unwrap_or(0) as i32
}

fn map_position_from_files(root: &str, target: &str) -> Option<(i32, i32)> {
    if root.trim().is_empty() || target.trim().is_empty() {
        return None;
    }
    let levels = Path::new(root).join("levels");
    let mut files = Vec::new();
    collect_gmap_files(&levels, &mut files);
    files.into_iter().find_map(|path| {
        fs::read(path)
            .ok()
            .and_then(|data| parse_gmap_position(&data, target))
    })
}

fn collect_gmap_files(root: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_gmap_files(&path, output);
        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("gmap"))
        {
            output.push(path);
        }
    }
}

fn parse_gmap_position(data: &[u8], target: &str) -> Option<(i32, i32)> {
    let target = target.trim();
    let normalized = String::from_utf8_lossy(data).replace("\r\n", "\n");
    let lines = normalized.lines().map(str::trim).collect::<Vec<_>>();
    let mut width = 0usize;
    let mut index = 0usize;
    while index < lines.len() {
        let parts = lines[index].split_whitespace().collect::<Vec<_>>();
        if parts
            .first()
            .is_some_and(|value| value.eq_ignore_ascii_case(&"WIDTH"))
        {
            width = parts
                .get(1)
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
        }
        if parts
            .first()
            .is_some_and(|value| value.eq_ignore_ascii_case(&"LEVELNAMES"))
        {
            let mut y = 0i32;
            index += 1;
            while index < lines.len() && !lines[index].eq_ignore_ascii_case("LEVELNAMESEND") {
                if !lines[index].is_empty() {
                    let decoded = lines[index].replace("\\n", "\n");
                    for (x, name) in decoded.split('\n').enumerate() {
                        if (width == 0 || x < width) && name.trim().eq_ignore_ascii_case(target) {
                            return Some((x as i32, y));
                        }
                    }
                    y += 1;
                }
                index += 1;
            }
        }
        index += 1;
    }
    None
}

fn mutate_array(receiver: &DynValue, operation: &str, args: Vec<DynValue>) {
    let DynValue::Array(values) = receiver else {
        return;
    };
    let mut values = values.borrow_mut();
    match operation {
        "push" | "add" => values.extend(args),
        "unshift" => {
            let mut prefix = args;
            prefix.append(&mut values);
            *values = prefix;
        }
        "delete" => {
            let index = args.first().map(number_i64).unwrap_or(-1);
            if index >= 0 && (index as usize) < values.len() {
                values.remove(index as usize);
            }
        }
        "remove" => {
            if let Some(needle) = args.first() {
                if let Some(index) = values.iter().position(|value| equal_values(value, needle)) {
                    values.remove(index);
                }
            }
        }
        _ => {}
    }
}

fn array_insert(receiver: &DynValue, index: i64, value: DynValue) {
    let DynValue::Array(values) = receiver else {
        return;
    };
    let mut values = values.borrow_mut();
    let index = index.max(0) as usize;
    let index = index.min(values.len());
    values.insert(index, value);
}

fn array_insert_values(receiver: &DynValue, index: i64, inserted: Vec<DynValue>) {
    let DynValue::Array(values) = receiver else {
        return;
    };
    let mut values = values.borrow_mut();
    let index = index.max(0) as usize;
    let index = index.min(values.len());
    values.splice(index..index, inserted);
}

fn array_splice(
    receiver: &DynValue,
    start: i64,
    count: Option<i64>,
    inserted: Vec<DynValue>,
) -> DynValue {
    let DynValue::Array(values) = receiver else {
        return DynValue::array(Vec::new());
    };
    let mut values = values.borrow_mut();
    let length = values.len() as i64;
    let start = if start < 0 { length + start } else { start }
        .max(0)
        .min(length) as usize;
    let count = count
        .unwrap_or(length - start as i64)
        .max(0)
        .min((length - start as i64).max(0)) as usize;
    let removed = values
        .splice(start..start.saturating_add(count), inserted)
        .collect::<Vec<_>>();
    DynValue::array(removed)
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    // filepath.Match semantics: `*` and `?` never cross a path separator.
    // File rights use this matcher as well as ordinary file enumeration, so
    // treating a slash as an ordinary character would grant a parent rule to
    // arbitrarily deep descendants.
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let mut states = vec![vec![false; text.len() + 1]; pattern.len() + 1];
    states[0][0] = true;
    for i in 0..pattern.len() {
        for j in 0..=text.len() {
            if !states[i][j] {
                continue;
            }
            match pattern[i] {
                b'*' => {
                    states[i + 1][j] = true;
                    if j < text.len() && text[j] != b'/' {
                        states[i][j + 1] = true;
                    }
                }
                b'?' => {
                    if j < text.len() && text[j] != b'/' {
                        states[i + 1][j + 1] = true;
                    }
                }
                byte => {
                    if j < text.len() && byte == text[j] {
                        states[i + 1][j + 1] = true;
                    }
                }
            }
        }
    }
    states[pattern.len()][text.len()]
}

fn save_mode(value: &DynValue) -> bool {
    match value {
        DynValue::Bool(value) => *value,
        DynValue::Number(value) => *value != 0.0,
        DynValue::String(value) => {
            value == "1"
                || value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("append")
        }
        _ => false,
    }
}

fn normalized_file_name(name: &str) -> String {
    name.trim().replace('\\', "/")
}

fn resolve_vm_file(root: &str, name: &str) -> Option<PathBuf> {
    let root = root.trim();
    if root.is_empty() {
        return None;
    }
    let name = normalized_file_name(name);
    if name.is_empty() || name.starts_with('/') || name.contains('\0') {
        return None;
    }
    let path = Path::new(&name);
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::RootDir
        )
    }) {
        return None;
    }
    if name.contains(':') {
        return None;
    }
    Some(Path::new(root).join(path))
}

fn resolve_vm_level_file(root: &str, name: &str) -> Option<(PathBuf, String)> {
    let name = normalized_file_name(name);
    if name.is_empty() || name.starts_with('/') || name.contains(':') || name.contains('\0') {
        return None;
    }
    if name.split('/').any(|part| part == "..") {
        return None;
    }
    let relative = format!("levels/{name}");
    resolve_vm_file(root, &relative).map(|path| (path, relative))
}

fn vm_file_has_right(entries: &[String], name: &str, right: char) -> bool {
    if entries.is_empty() {
        return true;
    }
    let name = normalized_file_name(name)
        .trim_start_matches('/')
        .to_string();
    if name.is_empty() || name.contains("..") || name.contains(':') {
        return false;
    }
    entries.iter().any(|entry| {
        let mut parts = entry.splitn(2, char::is_whitespace);
        let first = parts.next().unwrap_or_default();
        let (rights, pattern) = if let Some(rest) = parts.next() {
            (first.to_ascii_lowercase(), rest.trim())
        } else {
            ("r".to_string(), first.trim())
        };
        if !rights.contains(right) {
            return false;
        }
        let pattern = normalized_file_name(pattern)
            .trim_start_matches('/')
            .to_string();
        wildcard_match(&pattern, &name) || wildcard_match(&pattern, &format!("{name}/x"))
    })
}

fn load_vm_bytes(root: &str, name: &str) -> Option<Vec<u8>> {
    resolve_vm_file(root, name).and_then(|path| fs::read(path).ok())
}

fn load_vm_string(root: &str, name: &str) -> Option<String> {
    load_vm_bytes(root, name).map(|data| String::from_utf8_lossy(&data).into_owned())
}

fn load_vm_lines(root: &str, name: &str) -> Option<Vec<String>> {
    let mut text = load_vm_string(root, name)?.replace("\r\n", "\n");
    if text.ends_with('\n') {
        text.pop();
    }
    if text.is_empty() {
        Some(Vec::new())
    } else {
        Some(text.split('\n').map(str::to_string).collect())
    }
}

fn save_vm_bytes(root: &str, name: &str, data: &[u8], append: bool) -> std::io::Result<()> {
    let Some(path) = resolve_vm_file(root, name) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid path",
        ));
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).write(true);
    if append {
        options.append(true);
    } else {
        options.truncate(true);
    }
    options.open(path)?.write_all(data)
}

fn save_vm_string(root: &str, name: &str, text: &str, append: bool) -> std::io::Result<()> {
    save_vm_bytes(root, name, text.as_bytes(), append)
}

fn save_vm_lines(root: &str, name: &str, lines: &[String], append: bool) -> std::io::Result<()> {
    let mut text = lines.join("\n");
    if !lines.is_empty() {
        text.push('\n');
    }
    save_vm_string(root, name, &text, append)
}

fn find_vm_files(
    root: &str,
    pattern: &str,
    recursive: bool,
    include_directories: bool,
) -> Vec<String> {
    let clean = normalized_file_name(pattern);
    if clean.is_empty() || resolve_vm_file(root, &clean).is_none() {
        return Vec::new();
    }
    let (match_pattern, recursive_pattern) = if recursive && !clean.contains("**") {
        let path = Path::new(&clean);
        let parent = path.parent().and_then(Path::to_str).unwrap_or("");
        let base = path.file_name().and_then(|x| x.to_str()).unwrap_or("");
        (
            if parent.is_empty() || parent == "." {
                format!("**/{base}")
            } else {
                format!("{}/**/{base}", parent.trim_end_matches('/'))
            },
            true,
        )
    } else {
        (clean.clone(), clean.contains("**"))
    };
    let root_path = Path::new(root).to_path_buf();
    let mut candidates = Vec::<(String, bool)>::new();
    fn walk(path: &Path, root: &Path, out: &mut Vec<(String, bool)>) {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let relative = match path.strip_prefix(root) {
                Ok(value) => value.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if metadata.is_dir() {
                out.push((relative, true));
                walk(&path, root, out);
            } else if metadata.is_file() {
                out.push((relative, false));
            }
        }
    }
    walk(&root_path, &root_path, &mut candidates);
    let suffix = match_pattern
        .split_once("**")
        .map(|(_, value)| value.trim_start_matches('/'));
    let mut result = candidates
        .into_iter()
        .filter(|(relative, is_directory)| {
            if *is_directory && !include_directories {
                return false;
            }
            if let Some(suffix) = suffix {
                let base = Path::new(relative)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("");
                return wildcard_match(suffix, base);
            }
            if recursive_pattern {
                return wildcard_match(&match_pattern, relative);
            }
            wildcard_match(&match_pattern, relative)
        })
        .map(|(relative, _)| relative)
        .collect::<Vec<_>>();
    result.sort();
    result
}

fn generate_zip_bytes(value: &DynValue) -> Vec<u8> {
    let values = array_values(value);
    let mut output = Vec::new();
    let mut central = Vec::new();
    let mut entries = 0u16;
    for pair in values.chunks(2) {
        if pair.len() != 2 {
            break;
        }
        let name = normalized_file_name(&value_string(&pair[0]));
        if name.is_empty()
            || name.starts_with('/')
            || name.split('/').any(|part| part.is_empty() || part == "..")
        {
            continue;
        }
        let data = value_bytes(&pair[1]);
        let crc = crc32(&data);
        let offset = output.len() as u32;
        let name_bytes = name.as_bytes();
        let length = data.len() as u32;
        output.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        output.extend_from_slice(&20u16.to_le_bytes());
        output.extend_from_slice(&0u16.to_le_bytes());
        output.extend_from_slice(&0u16.to_le_bytes());
        output.extend_from_slice(&0u16.to_le_bytes());
        output.extend_from_slice(&0u16.to_le_bytes());
        output.extend_from_slice(&crc.to_le_bytes());
        output.extend_from_slice(&length.to_le_bytes());
        output.extend_from_slice(&length.to_le_bytes());
        output.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        output.extend_from_slice(&0u16.to_le_bytes());
        output.extend_from_slice(name_bytes);
        output.extend_from_slice(&data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&length.to_le_bytes());
        central.extend_from_slice(&length.to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
        entries = entries.saturating_add(1);
    }
    let central_offset = output.len() as u32;
    output.extend_from_slice(&central);
    let central_size = central.len() as u32;
    output.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes());
    output.extend_from_slice(&entries.to_le_bytes());
    output.extend_from_slice(&entries.to_le_bytes());
    output.extend_from_slice(&central_size.to_le_bytes());
    output.extend_from_slice(&central_offset.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes());
    output
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn encode_vm_vars(object: &DynValue) -> String {
    let mut properties = object_properties(object);
    properties.sort_by_key(|(key, _)| key.to_ascii_lowercase());
    let mut output = String::new();
    for (key, value) in properties {
        if key.is_empty()
            || key.eq_ignore_ascii_case("length")
            || key.starts_with("__")
            || matches!(
                value,
                DynValue::Undefined | DynValue::Null | DynValue::Function(_) | DynValue::Builtin(_)
            )
        {
            continue;
        }
        output.push_str(&key);
        output.push('=');
        output.push_str(&value_string(&value));
        output.push('\n');
    }
    output
}

fn map_file_value(value: &str) -> DynValue {
    if value.contains(',') {
        DynValue::array(value.split(',').map(typed_string_value).collect())
    } else {
        typed_string_value(value)
    }
}

fn load_vars_from_lines(object: &DynValue, lines: &[String]) {
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            set_property(object, key.trim(), map_file_value(value.trim()));
        } else {
            let mut parts = line.split_whitespace();
            if let Some(key) = parts.next() {
                let value = parts.collect::<Vec<_>>().join(" ");
                set_property(object, key, map_file_value(&value));
            }
        }
    }
}

fn load_ini_from_lines(object: &DynValue, lines: &[String]) {
    let mut section = object.clone();
    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.contains(']') {
            let end = line.find(']').unwrap_or(1);
            let name = line[1..end].trim();
            let next = get_property(&section, name);
            section = if next.is_undefined() || matches!(next, DynValue::Null) {
                let value = DynValue::plain();
                set_property(&section, name, value.clone());
                value
            } else {
                next
            };
        } else if let Some((key, value)) = line.split_once('=') {
            set_property(&section, key.trim(), map_file_value(value.trim()));
        }
    }
}

fn format_time_value(format: &str, timestamp: f64) -> String {
    format_time_value_impl(format, timestamp)
}

/*
fn format_time_value_legacy_invalid(format: &str, timestamp: f64) -> String {
    let time = UNIX_EPOCH
        .checked_add(Duration::from_secs(timestamp.max(0.0) as u64))
        .unwrap_or(UNIX_EPOCH);
    let date = chrono::DateTime::<chrono::Local>::from(time);
    let mut output = String::new();
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            output.push(ch);
            continue;
        }
        let Some(specifier) = chars.next() else {
            output.push('%');
            break;
        };
        let value = match specifier {
            '%' => \"%\".to_string(),
            'a' => date.format(\"%a\").to_string(),
            'A' => date.format(\"%A\").to_string(),
            'b' | 'h' => date.format(\"%b\").to_string(),
            'B' => date.format(\"%B\").to_string(),
            'c' => date.format(\"%a %b %-d %H:%M:%S %Y\").to_string(),
            'd' => date.format(\"%d\").to_string(),
            'e' => date.format(\"%e\").to_string(),
            'H' => date.format(\"%H\").to_string(),
            'I' => date.format(\"%I\").to_string(),
            'j' => date.format(\"%j\").to_string(),
            'k' => format!(\"{:>2}\", date.hour()),
            'l' => format!(\"{:>2}\", date.hour12().1),
            'm' => date.format(\"%m\").to_string(),
            'M' => date.format(\"%M\").to_string(),
            'n' => \"\\n\".to_string(),
            'p' => date.format(\"%p\").to_string(),
            'r' => date.format(\"%I:%M:%S %p\").to_string(),
            'S' => date.format(\"%S\").to_string(),
            's' => date.timestamp().to_string(),
            't' => \"\\t\".to_string(),
            'u' => date.weekday().number_from_monday().to_string(),
            'w' => date.weekday().num_days_from_sunday().to_string(),
            'x' => date.format(\"%m/%d/%y\").to_string(),
            'X' => date.format(\"%H:%M:%S\").to_string(),
            'y' => date.format(\"%y\").to_string(),
            'Y' => date.format(\"%Y\").to_string(),
            'z' => date.format(\"%z\").to_string(),
            'Z' => date.format(\"%Z\").to_string(),
            other => format!(\"%{other}\"),
        };
        output.push_str(&value);
    }
    output
}

fn escape_mysql(value: &str, keep_newlines: bool) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if ch.is_control()
            && (!keep_newlines || !matches!(ch, '\n' | '\r'))
            && !matches!(ch, '\t')
        {
            continue;
        }
        match ch {
            '\\' => output.push_str(\"\\\\\\\\\"),
            '\\'' => output.push_str(\"\\\\'\"),
            '\"' => output.push_str(\"\\\\\\\"\"),
            _ => output.push(ch),
        }
    }
    output
}

*/

fn escape_filename(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            output.push(ch);
        } else {
            output.push_str(&format!("%{:03}", ch as u32));
        }
    }
    output
}

fn unescape_filename(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = String::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 3 < bytes.len()
            && bytes[index + 1..index + 4]
                .iter()
                .all(|byte| byte.is_ascii_digit())
        {
            let number = std::str::from_utf8(&bytes[index + 1..index + 4])
                .ok()
                .and_then(|value| value.parse::<u32>().ok());
            if let Some(number) = number {
                if let Some(ch) = char::from_u32(number) {
                    output.push(ch);
                    index += 4;
                    continue;
                }
            }
        }
        output.push(bytes[index] as char);
        index += 1;
    }
    output
}

fn format_time_value_impl(format: &str, timestamp: f64) -> String {
    let time = UNIX_EPOCH
        .checked_add(Duration::from_secs(timestamp.max(0.0) as u64))
        .unwrap_or(UNIX_EPOCH);
    let date = chrono::DateTime::<chrono::Local>::from(time);
    let mut output = String::new();
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            output.push(ch);
            continue;
        }
        let Some(specifier) = chars.next() else {
            output.push('%');
            break;
        };
        let value = match specifier {
            '%' => "%".to_string(),
            'a' => date.format("%a").to_string(),
            'A' => date.format("%A").to_string(),
            'b' | 'h' => date.format("%b").to_string(),
            'B' => date.format("%B").to_string(),
            'c' => date.format("%a %b %-d %H:%M:%S %Y").to_string(),
            'd' => date.format("%d").to_string(),
            'e' => date.format("%e").to_string(),
            'H' => date.format("%H").to_string(),
            'I' => date.format("%I").to_string(),
            'j' => date.format("%j").to_string(),
            'k' => format!("{:>2}", date.hour()),
            'l' => format!("{:>2}", date.hour12().1),
            'm' => date.format("%m").to_string(),
            'M' => date.format("%M").to_string(),
            'n' => "\n".to_string(),
            'p' => date.format("%p").to_string(),
            'r' => date.format("%I:%M:%S %p").to_string(),
            'S' => date.format("%S").to_string(),
            's' => date.timestamp().to_string(),
            't' => "\t".to_string(),
            'u' => date.weekday().number_from_monday().to_string(),
            'w' => date.weekday().num_days_from_sunday().to_string(),
            'x' => date.format("%m/%d/%y").to_string(),
            'X' => date.format("%H:%M:%S").to_string(),
            'y' => date.format("%y").to_string(),
            'Y' => date.format("%Y").to_string(),
            'z' => date.format("%z").to_string(),
            'Z' => date.format("%Z").to_string(),
            other => format!("%{other}"),
        };
        output.push_str(&value);
    }
    output
}

fn substring_value(receiver: &DynValue, args: &[DynValue]) -> DynValue {
    let text = value_string(receiver);
    let start = number_i64(&args.first().cloned().unwrap_or(DynValue::Number(0.0))).max(0) as usize;
    let length = args
        .get(1)
        .map(number_i64)
        .unwrap_or((text.chars().count() as i64 - start as i64).max(0))
        .max(0) as usize;
    let chars = text.chars().collect::<Vec<_>>();
    let start = start.min(chars.len());
    let end = start.saturating_add(length).min(chars.len());
    DynValue::String(chars[start..end].iter().collect())
}

fn object_event_ready(object: &DynValue, event: &str) -> bool {
    let event_value = get_property(object, &format!("__event_{event}"));
    if !event_value.is_undefined() {
        return event_value.truthy();
    }
    get_property(object, event).truthy() || get_property(object, &format!("on{event}")).truthy()
}

fn get_path(root: &DynValue, path: &str) -> DynValue {
    let mut current = root.clone();
    for name in path.split('.').filter(|x| !x.is_empty()) {
        if current.is_undefined() || matches!(current, DynValue::Null) {
            return DynValue::Undefined;
        }
        current = get_property(&current, name);
    }
    current
}

fn set_path(root: &DynValue, path: &str, value: DynValue) {
    let names = path
        .split('.')
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>();
    if names.is_empty() {
        return;
    }
    let mut current = root.clone();
    for name in &names[..names.len() - 1] {
        let child = get_property(&current, name);
        let child = if child.is_undefined() || matches!(child, DynValue::Null) {
            let value = DynValue::plain();
            set_property(&current, name, value.clone());
            value
        } else {
            child
        };
        current = child;
    }
    set_property(&current, names[names.len() - 1], value);
}

fn unset_path(root: &DynValue, path: &str) {
    let names = path
        .split('.')
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>();
    if names.is_empty() {
        return;
    }
    let mut current = root.clone();
    for name in &names[..names.len() - 1] {
        current = get_property(&current, name);
        if current.is_undefined() {
            return;
        }
    }
    if let Some(object) = current.object_ref() {
        object
            .borrow_mut()
            .properties
            .remove(names[names.len() - 1]);
    }
}

fn random_string(length: usize) -> DynValue {
    const CHARACTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut output = String::with_capacity(length);
    for _ in 0..length {
        output.push(CHARACTERS[rand::random::<usize>() % CHARACTERS.len()] as char);
    }
    DynValue::String(output)
}

fn random_gs2_string(value: &DynValue) -> DynValue {
    if let DynValue::Array(values) = value {
        let values = values.borrow();
        return values
            .get(rand::random::<usize>() % values.len().max(1))
            .cloned()
            .map_or_else(
                || DynValue::String(String::new()),
                |value| DynValue::String(value_string(&value)),
            );
    }
    random_string(number_i64(value).max(0) as usize)
}

#[derive(Clone, Copy)]
struct PokerCard {
    rank: i32,
    suit: u8,
    code: [u8; 2],
}

fn poker_eval_value(
    kind: &str,
    hands_value: &DynValue,
    common_value: &DynValue,
    dead_value: &DynValue,
    iterations: i32,
) -> DynValue {
    let kind = kind.trim().to_ascii_lowercase();
    let hands = poker_card_groups(hands_value);
    if hands.is_empty() {
        return DynValue::array(Vec::new());
    }
    let iterations = if iterations <= 0 { 1000 } else { iterations };
    let common = poker_cards(common_value);
    let dead = poker_cards(dead_value);
    let mut known = common.clone();
    known.extend(dead.iter().copied());
    for hand in &hands {
        known.extend(hand.iter().copied());
    }
    let deck = poker_deck_without(&known);
    let mut scores = vec![0.0; hands.len()];
    for _ in 0..iterations {
        let mut draw = deck.clone();
        // The result is deterministic whenever the inputs fully specify the
        // board (the normal server use).  For incomplete boards use the same
        // uniform random sampling contract used by the runtime.
        for index in (1..draw.len()).rev() {
            let swap = rand::random::<usize>() % (index + 1);
            draw.swap(index, swap);
        }
        let mut board = common.clone();
        let mut player_hands = Vec::with_capacity(hands.len());
        let mut draw_index = 0usize;
        let player_target = poker_player_target(&kind);
        for hand in &hands {
            let mut current = hand.clone();
            while current.len() < player_target && draw_index < draw.len() {
                current.push(draw[draw_index]);
                draw_index += 1;
            }
            player_hands.push(current);
        }
        let board_target = poker_board_target(&kind);
        while board.len() < board_target && draw_index < draw.len() {
            board.push(draw[draw_index]);
            draw_index += 1;
        }
        let mut high = Vec::with_capacity(player_hands.len());
        let mut low = Vec::with_capacity(player_hands.len());
        for hand in &player_hands {
            high.push(poker_best(&kind, hand, &board, false));
            low.push(poker_best(&kind, hand, &board, true));
        }
        if kind.ends_with('8') {
            let qualified = poker_qualify_eight(&low);
            if poker_has_qualified_low(&qualified) {
                poker_award(&mut scores, &high, true, 0.5);
                poker_award(&mut scores, &qualified, false, 0.5);
            } else {
                poker_award(&mut scores, &high, true, 1.0);
            }
        } else if kind == "7studnsq" {
            poker_award(&mut scores, &high, true, 0.5);
            poker_award(&mut scores, &low, false, 0.5);
        } else if kind == "razz" || kind == "lowball27" {
            poker_award(&mut scores, &low, false, 1.0);
        } else {
            poker_award(&mut scores, &high, true, 1.0);
        }
    }
    DynValue::array(
        scores
            .into_iter()
            .map(|score| DynValue::Number(score / iterations as f64))
            .collect(),
    )
}

fn poker_card_groups(value: &DynValue) -> Vec<Vec<PokerCard>> {
    array_values(value)
        .into_iter()
        .map(|group| poker_cards(&group))
        .collect()
}

fn poker_cards(value: &DynValue) -> Vec<PokerCard> {
    array_values(value)
        .into_iter()
        .filter_map(|value| parse_poker_card(&value_string(&value)))
        .collect()
}

fn parse_poker_card(value: &str) -> Option<PokerCard> {
    let text = value.trim().trim_matches('"').to_ascii_lowercase();
    let bytes = text.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let rank = match bytes[0] {
        b'2'..=b'9' => (bytes[0] - b'0') as i32,
        b't' => 10,
        b'j' => 11,
        b'q' => 12,
        b'k' => 13,
        b'a' => 14,
        _ => return None,
    };
    let suit = *bytes.last()?;
    if !matches!(suit, b'c' | b'd' | b'h' | b's') {
        return None;
    }
    Some(PokerCard {
        rank,
        suit,
        code: [bytes[0], suit],
    })
}

fn poker_deck_without(known: &[PokerCard]) -> Vec<PokerCard> {
    let mut used = std::collections::HashSet::<[u8; 2]>::new();
    for card in known {
        used.insert(card.code);
    }
    let mut output = Vec::new();
    for rank in b"23456789tjqka" {
        for suit in b"cdhs" {
            let code = [*rank, *suit];
            if !used.contains(&code) {
                output.push(PokerCard {
                    rank: parse_poker_card(&String::from_utf8_lossy(&code))
                        .map_or(0, |card| card.rank),
                    suit: *suit,
                    code,
                });
            }
        }
    }
    output
}

fn poker_board_target(kind: &str) -> usize {
    if kind.starts_with("holdem") || kind.starts_with("omaha") {
        5
    } else {
        0
    }
}

fn poker_player_target(kind: &str) -> usize {
    if kind.starts_with("holdem") {
        2
    } else if kind.starts_with("omaha") {
        4
    } else {
        7
    }
}

fn poker_best(kind: &str, hand: &[PokerCard], board: &[PokerCard], low: bool) -> i64 {
    let mut best = if low { i64::MAX } else { 0 };
    if kind.starts_with("omaha") {
        poker_choose(hand, 2, &mut |h| {
            poker_choose(board, 3, &mut |b| {
                let mut cards = h.to_vec();
                cards.extend_from_slice(b);
                let score = poker_score(&cards, low, kind == "lowball27");
                if (!low && score > best) || (low && score < best) {
                    best = score;
                }
            });
        });
        return best;
    }
    let mut cards = hand.to_vec();
    cards.extend_from_slice(board);
    poker_choose(&cards, 5, &mut |combo| {
        let score = poker_score(combo, low, kind == "lowball27");
        if (!low && score > best) || (low && score < best) {
            best = score;
        }
    });
    best
}

fn poker_choose<F: FnMut(&[PokerCard])>(cards: &[PokerCard], count: usize, callback: &mut F) {
    if count == 0 || cards.len() < count {
        return;
    }
    fn walk<F: FnMut(&[PokerCard])>(
        cards: &[PokerCard],
        start: usize,
        count: usize,
        selected: &mut Vec<PokerCard>,
        callback: &mut F,
    ) {
        if count == 0 {
            callback(selected);
            return;
        }
        for index in start..=cards.len() - count {
            selected.push(cards[index]);
            walk(cards, index + 1, count - 1, selected, callback);
            selected.pop();
        }
    }
    walk(cards, 0, count, &mut Vec::with_capacity(count), callback);
}

fn poker_score(cards: &[PokerCard], low: bool, deuce: bool) -> i64 {
    let mut ranks = Vec::with_capacity(cards.len());
    let mut counts = HashMap::<i32, i32>::new();
    let mut suits = HashMap::<u8, i32>::new();
    for card in cards {
        let rank = if low && !deuce && card.rank == 14 {
            1
        } else {
            card.rank
        };
        ranks.push(rank);
        *counts.entry(rank).or_default() += 1;
        *suits.entry(card.suit).or_default() += 1;
    }
    ranks.sort_by(|left, right| right.cmp(left));
    if low {
        if deuce {
            return poker_deuce_low_score(ranks, counts, suits);
        }
        if counts.values().any(|count| *count > 1) {
            let count = counts
                .values()
                .find(|count| **count > 1)
                .copied()
                .unwrap_or(2);
            return poker_pack(9 + count, &ranks);
        }
        ranks.sort();
        return poker_pack(0, &ranks);
    }
    let flush = suits.values().any(|count| *count == 5);
    let straight = poker_straight_high(&ranks);
    let groups = poker_groups(&counts);
    if flush && straight > 0 {
        return poker_pack(8, &[straight]);
    }
    if groups.first().is_some_and(|group| group.1 == 4) {
        return poker_pack(7, &[groups[0].0, groups[1].0]);
    }
    if groups.len() > 1 && groups[0].1 == 3 && groups[1].1 == 2 {
        return poker_pack(6, &[groups[0].0, groups[1].0]);
    }
    if flush {
        return poker_pack(5, &ranks);
    }
    if straight > 0 {
        return poker_pack(4, &[straight]);
    }
    if groups.first().is_some_and(|group| group.1 == 3) {
        return poker_pack(3, &poker_group_ranks(&groups));
    }
    if groups.len() > 1 && groups[0].1 == 2 && groups[1].1 == 2 {
        return poker_pack(2, &poker_group_ranks(&groups));
    }
    if groups.first().is_some_and(|group| group.1 == 2) {
        return poker_pack(1, &poker_group_ranks(&groups));
    }
    poker_pack(0, &ranks)
}

fn poker_deuce_low_score(
    mut ranks: Vec<i32>,
    counts: HashMap<i32, i32>,
    suits: HashMap<u8, i32>,
) -> i64 {
    let mut class = 0;
    for count in counts.values() {
        if *count == 4 && class < 7 {
            class = 7;
        } else if *count == 3 && class < 3 {
            class = 3;
        } else if *count == 2 {
            if class == 1 {
                class = 2;
            } else if class < 1 {
                class = 1;
            }
        }
    }
    let flush = suits.values().any(|count| *count == 5);
    let straight = poker_straight_high(&ranks);
    if flush && straight > 0 {
        class = 8;
    } else if flush && class < 5 {
        class = 5;
    } else if straight > 0 && class < 4 {
        class = 4;
    }
    ranks.sort_by(|left, right| right.cmp(left));
    poker_pack(class, &ranks)
}

fn poker_qualify_eight(values: &[i64]) -> Vec<i64> {
    let base = 15_i64.pow(5);
    values
        .iter()
        .map(|value| {
            if value / base != 0 || value.rem_euclid(15) > 8 {
                i64::MAX
            } else {
                *value
            }
        })
        .collect()
}

fn poker_has_qualified_low(values: &[i64]) -> bool {
    values.iter().any(|value| *value != i64::MAX)
}

fn poker_groups(counts: &HashMap<i32, i32>) -> Vec<(i32, i32)> {
    let mut groups = counts
        .iter()
        .map(|(rank, count)| (*rank, *count))
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| right.0.cmp(&left.0)));
    groups
}

fn poker_group_ranks(groups: &[(i32, i32)]) -> Vec<i32> {
    let mut output = Vec::new();
    for (rank, count) in groups {
        for _ in 0..*count {
            output.push(*rank);
        }
    }
    output
}

fn poker_straight_high(ranks: &[i32]) -> i32 {
    let seen = ranks
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    if [14, 5, 4, 3, 2].iter().all(|rank| seen.contains(rank)) {
        return 5;
    }
    for high in (6..=14).rev() {
        if (0..5).all(|offset| seen.contains(&(high - offset))) {
            return high;
        }
    }
    0
}

fn poker_pack(class: i32, ranks: &[i32]) -> i64 {
    let mut score = class as i64;
    for index in 0..5 {
        score *= 15;
        if let Some(rank) = ranks.get(index) {
            score += *rank as i64;
        }
    }
    score
}

fn poker_award(scores: &mut [f64], values: &[i64], high: bool, weight: f64) {
    let Some(&best) = values
        .iter()
        .filter(|value| **value != i64::MAX)
        .reduce(|left, right| {
            if (high && *left > *right) || (!high && *left < *right) {
                left
            } else {
                right
            }
        })
    else {
        return;
    };
    let winners = values.iter().filter(|value| **value == best).count();
    if winners == 0 {
        return;
    }
    let share = weight / winners as f64;
    for (index, value) in values.iter().enumerate() {
        if *value == best {
            scores[index] += share;
        }
    }
}

fn md5_hex(input: &[u8]) -> String {
    // RFC 1321 MD5, kept local so the standalone NPC server does not need a
    // crypto dependency merely for the legacy script helper.
    const S: [[u32; 4]; 4] = [
        [7, 12, 17, 22],
        [5, 9, 14, 20],
        [4, 11, 16, 23],
        [6, 10, 15, 21],
    ];
    let mut k = [0u32; 64];
    for (i, item) in k.iter_mut().enumerate() {
        *item = (f64::sin((i + 1) as f64).abs() * 4_294_967_296.0).floor() as u32;
    }
    let mut message = input.to_vec();
    let bit_len = (message.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());

    let mut state = [
        0x6745_2301u32,
        0xefcd_ab89u32,
        0x98ba_dcfeu32,
        0x1032_5476u32,
    ];
    for chunk in message.chunks_exact(64) {
        let mut words = [0u32; 16];
        for (i, word) in words.iter_mut().enumerate() {
            let offset = i * 4;
            *word = u32::from_le_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
        for i in 0..64 {
            let (f, g) = if i < 16 {
                ((b & c) | ((!b) & d), i)
            } else if i < 32 {
                ((d & b) | ((!d) & c), (5 * i + 1) % 16)
            } else if i < 48 {
                (b ^ c ^ d, (3 * i + 5) % 16)
            } else {
                (c ^ (b | !d), (7 * i) % 16)
            };
            let round = i / 16;
            let rotated = a
                .wrapping_add(f)
                .wrapping_add(k[i])
                .wrapping_add(words[g])
                .rotate_left(S[round][i % 4]);
            let next = b.wrapping_add(rotated);
            a = d;
            d = c;
            c = b;
            b = next;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
    let mut output = String::with_capacity(32);
    for word in state {
        for byte in word.to_le_bytes() {
            output.push_str(&format!("{byte:02x}"));
        }
    }
    output
}

fn make_date_object(timestamp: Option<f64>) -> DynValue {
    let date = timestamp.map_or_else(Utc::now, |value| {
        Utc.timestamp_opt(value as i64, 0)
            .single()
            .unwrap_or_else(Utc::now)
    });
    let object = DynValue::object(ObjectKind::Date);
    set_property(&object, "year", DynValue::Number(date.year() as f64));
    set_property(&object, "month", DynValue::Number(date.month() as f64));
    set_property(&object, "day", DynValue::Number(date.day() as f64));
    set_property(&object, "hour", DynValue::Number(date.hour() as f64));
    set_property(&object, "minute", DynValue::Number(date.minute() as f64));
    set_property(&object, "second", DynValue::Number(date.second() as f64));
    set_property(&object, "time", DynValue::Number(date.timestamp() as f64));
    set_property(&object, "objecttype", DynValue::String("Date".to_string()));
    set_property(
        &object,
        "__date_string",
        DynValue::String(date.format("%Y-%m-%d %H:%M:%S").to_string()),
    );
    object
}

fn format_date_value(value: &DynValue) -> String {
    value_string(&get_property(value, "__date_string"))
}

fn make_npc_object(context: &NPCContext) -> DynValue {
    let object = DynValue::object(ObjectKind::NPC { id: context.id });
    set_property(&object, "id", DynValue::Number(context.id as f64));
    set_property(&object, "name", DynValue::String(context.name.clone()));
    set_property(&object, "level", DynValue::String(context.level.clone()));
    set_property(&object, "x", DynValue::Number(context.x));
    set_property(&object, "y", DynValue::Number(context.y));
    set_property(&object, "width", DynValue::Number(context.width));
    set_property(&object, "height", DynValue::Number(context.height));
    hydrate_object_state(&object, &context.this);
    object
}

fn flag_values(values: &HashMap<String, String>, prefix: &str) -> HashMap<String, String> {
    let prefix = prefix.to_ascii_lowercase();
    values
        .iter()
        .filter_map(|(key, value)| {
            key.to_ascii_lowercase()
                .strip_prefix(&prefix)
                .map(|name| (name.to_string(), value.clone()))
        })
        .collect()
}

fn player_context_from_object(object: &DynValue) -> PlayerContext {
    let rights = array_values(&get_property(object, "__rights"))
        .into_iter()
        .map(|value| value_string(&value))
        .collect::<Vec<_>>();
    let folders = array_values(&get_property(object, "__folders"))
        .into_iter()
        .map(|value| value_string(&value))
        .collect::<Vec<_>>();
    let mut flags = HashMap::new();
    for (prefix, name) in [("client.", "client"), ("clientr.", "clientr")] {
        for (key, value) in object_properties(&get_property(object, name)) {
            flags.insert(format!("{prefix}{key}"), value_string(&value));
        }
    }
    PlayerContext {
        id: number_i64(&get_property(object, "id")) as u16,
        account: value_string(&get_property(object, "account")),
        nick: value_string(&get_property(object, "nick")),
        nickname: value_string(&get_property(object, "nickname")),
        guild: value_string(&get_property(object, "guild")),
        level: value_string(&get_property(object, "levelname")),
        dir: number_i32(&get_property(object, "dir")),
        x: number_f64(&get_property(object, "x")),
        y: number_f64(&get_property(object, "y")),
        online_time: number_i32(&get_property(object, "onlinetime")),
        admin_level: number_i32(&get_property(object, "adminlevel")),
        flags,
        rights,
        folders,
    }
}

fn player_has_right_flag_value(object: &DynValue, wanted: &str) -> bool {
    let wanted = wanted.trim().replace('-', "").to_ascii_lowercase();
    array_values(&get_property(object, "__rights"))
        .into_iter()
        .map(|value| value_string(&value).replace('-', "").to_ascii_lowercase())
        .any(|value| value == wanted)
}

fn player_has_folder_right_value(object: &DynValue, rights: &str, name: &str) -> bool {
    array_values(&get_property(object, "__folders"))
        .into_iter()
        .map(|value| value_string(&value))
        .any(|entry| {
            let mut parts = entry.split_whitespace();
            let Some(grants) = parts.next() else {
                return false;
            };
            let Some(pattern) = parts.next() else {
                return false;
            };
            grants.contains(rights) && folder_pattern_matches(pattern, name)
        })
}

fn request_text_value(kind: &str, key: &str, player: &PlayerContext) -> String {
    if kind.trim().eq_ignore_ascii_case("folder") && key.trim().eq_ignore_ascii_case("personal") {
        let account = if player.account.trim().is_empty() {
            "guest"
        } else {
            player.account.trim()
        };
        let prefix = account
            .chars()
            .take(2)
            .collect::<String>()
            .to_ascii_lowercase();
        return format!("personaluploads/{prefix}/{account}/");
    }
    String::new()
}

fn folder_pattern_matches(pattern: &str, name: &str) -> bool {
    let pattern = pattern.trim().trim_start_matches('/').replace('\\', "/");
    let name = name.trim().trim_start_matches('/').replace('\\', "/");
    wildcard_match(&pattern, &name) || wildcard_match(&pattern, &format!("{name}/x"))
}

fn collect_npc_flag_value(
    output: &mut Vec<NPCFlag>,
    id: u32,
    name: &str,
    value: &DynValue,
    initial: Option<&Value>,
) {
    if matches!(value, DynValue::Function(_) | DynValue::Builtin(_)) {
        return;
    }
    if let DynValue::Object(object) = value {
        let properties = object.borrow().properties.clone();
        let runtime_state = properties
            .keys()
            .any(|key| key.starts_with("__") && key != "__gs2value")
            || initial.is_some_and(|value| {
                value.as_object().is_some_and(|object| {
                    object
                        .keys()
                        .any(|key| key.starts_with("__") && key != "__gs2value")
                })
            });
        if runtime_state {
            return;
        }
        if let Some(scalar) = properties.get("__gs2value") {
            let old = initial
                .and_then(|value| value.as_object())
                .and_then(|object| object.get("__gs2value"))
                .map(json_value_string)
                .unwrap_or_default();
            let current = value_string(scalar);
            if current != old {
                output.push(NPCFlag {
                    id,
                    name: name.to_string(),
                    value: current,
                });
            }
        }
        for (key, child) in properties {
            if key.starts_with("__") {
                continue;
            }
            let old = initial.and_then(|value| value.get(&key));
            collect_npc_flag_value(output, id, &format!("{name}.{key}"), &child, old);
        }
        return;
    }
    let current = value_string(value);
    let old = initial.map(json_value_string).unwrap_or_default();
    if current != old {
        output.push(NPCFlag {
            id,
            name: name.to_string(),
            value: current,
        });
    }
}

fn json_value_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value
            .as_f64()
            .map(|value| value_string(&DynValue::Number(value)))
            .unwrap_or_default(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(json_value_string)
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(values) => values
            .get("__gs2value")
            .map(json_value_string)
            .unwrap_or_default(),
    }
}

fn object_properties(value: &DynValue) -> Vec<(String, DynValue)> {
    value
        .object_ref()
        .map(|object| object.borrow().properties.clone().into_iter().collect())
        .unwrap_or_default()
}

fn is_script_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || matches!(first, '_' | '$'))
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$'))
}

fn value_string(value: &DynValue) -> String {
    if let DynValue::Object(object) = value {
        let scalar = object.borrow().properties.get("__gs2value").cloned();
        if let Some(scalar) = scalar {
            return value_string(&scalar);
        }
    }
    match value {
        DynValue::Undefined | DynValue::Null => String::new(),
        DynValue::Bool(value) => value.to_string(),
        DynValue::Number(value) => {
            if value.is_nan() {
                return "NaN".to_string();
            }
            if value.is_infinite() {
                return if value.is_sign_negative() {
                    "-Infinity".to_string()
                } else {
                    "Infinity".to_string()
                };
            }
            if value.fract() == 0.0 {
                format!("{value:.0}")
            } else {
                value.to_string()
            }
        }
        DynValue::String(value) => value.clone(),
        DynValue::Bytes(value) => String::from_utf8_lossy(value).into_owned(),
        DynValue::Array(values) => values
            .borrow()
            .iter()
            .map(value_string_export)
            .collect::<Vec<_>>()
            .join(","),
        DynValue::Object(object) => {
            let object = object.borrow();
            if matches!(object.kind, ObjectKind::Date) {
                return object
                    .properties
                    .get("__date_string")
                    .map(value_string)
                    .unwrap_or_default();
            }
            if let Some(name) = object.properties.get("name") {
                let name = value_string(name);
                if !name.is_empty() {
                    return name;
                }
            }
            "[object Object]".to_string()
        }
        DynValue::Function(_) | DynValue::Builtin(_) => "function".to_string(),
    }
}

/// The legacy unary `@` operator is the VM's explicit string-coercion
/// operator.  It differs from ordinary script stringification for nullish
/// values (which become `0`) and for an empty plain object (which is an empty
/// string in the Goja bridge's compatibility path).
fn coerce_string(value: &DynValue) -> String {
    match value {
        DynValue::Undefined | DynValue::Null => "0".to_string(),
        DynValue::Object(object) => {
            let scalar = object.borrow().properties.get("__gs2value").cloned();
            if let Some(scalar) = scalar {
                return value_string(&scalar);
            }
            let empty = {
                let object = object.borrow();
                object.properties.is_empty()
                    && object.methods.is_empty()
                    && matches!(object.kind, ObjectKind::Plain)
            };
            if empty {
                String::new()
            } else {
                value_string(value)
            }
        }
        value => value_string(value),
    }
}

// Goja's valueString formats booleans specially when they are elements of an
// exported array (the legacy script representation is 1/0 rather than
// true/false).  Keep direct boolean conversion unchanged and apply that rule
// only while recursively formatting array exports.
fn value_string_export(value: &DynValue) -> String {
    match value {
        DynValue::Bool(value) => {
            if *value {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        DynValue::Array(values) => values
            .borrow()
            .iter()
            .map(value_string_export)
            .collect::<Vec<_>>()
            .join(","),
        value => value_string(value),
    }
}

fn value_type_name(value: &DynValue) -> &'static str {
    match value {
        DynValue::Undefined => "undefined",
        DynValue::Null => "object",
        DynValue::Bool(_) => "boolean",
        DynValue::Number(_) => "number",
        DynValue::String(_) => "string",
        DynValue::Bytes(_) => "string",
        DynValue::Array(_) | DynValue::Object(_) => "object",
        DynValue::Function(_) | DynValue::Builtin(_) => "function",
    }
}

fn output_value_string(value: &DynValue) -> String {
    if matches!(value, DynValue::Function(_) | DynValue::Builtin(_)) {
        "function".to_string()
    } else {
        value_string(value)
    }
}

fn truthy(value: &DynValue) -> bool {
    if let DynValue::Object(object) = value {
        if let Some(scalar) = object.borrow().properties.get("__gs2value").cloned() {
            return truthy(&scalar);
        }
    }
    match value {
        DynValue::Undefined | DynValue::Null => false,
        DynValue::Bool(value) => *value,
        DynValue::Number(value) => *value != 0.0 && !value.is_nan(),
        DynValue::String(value) => !value.is_empty(),
        DynValue::Bytes(value) => !value.is_empty(),
        DynValue::Array(_) | DynValue::Object(_) | DynValue::Function(_) | DynValue::Builtin(_) => {
            true
        }
    }
}

fn number_f64(value: &DynValue) -> f64 {
    if let DynValue::Object(object) = value {
        if let Some(scalar) = object.borrow().properties.get("__gs2value").cloned() {
            return number_f64(&scalar);
        }
    }
    match value {
        DynValue::Number(value) => *value,
        DynValue::Bool(value) => {
            if *value {
                1.0
            } else {
                0.0
            }
        }
        DynValue::String(value) => value.trim().parse::<f64>().unwrap_or(0.0),
        DynValue::Null | DynValue::Undefined => 0.0,
        _ => value_string(value).trim().parse::<f64>().unwrap_or(0.0),
    }
}

fn number_i64(value: &DynValue) -> i64 {
    number_f64(value) as i64
}
fn number_i32(value: &DynValue) -> i32 {
    number_f64(value) as i32
}

// The host uses crypto/des in ECB mode with PKCS#5 padding. Keep the
// implementation local so the standalone server does not depend on a system
// crypto provider or on a runtime-specific cipher API.
const DES_IP: [u8; 64] = [
    58, 50, 42, 34, 26, 18, 10, 2, 60, 52, 44, 36, 28, 20, 12, 4, 62, 54, 46, 38, 30, 22, 14, 6,
    64, 56, 48, 40, 32, 24, 16, 8, 57, 49, 41, 33, 25, 17, 9, 1, 59, 51, 43, 35, 27, 19, 11, 3, 61,
    53, 45, 37, 29, 21, 13, 5, 63, 55, 47, 39, 31, 23, 15, 7,
];
const DES_FP: [u8; 64] = [
    40, 8, 48, 16, 56, 24, 64, 32, 39, 7, 47, 15, 55, 23, 63, 31, 38, 6, 46, 14, 54, 22, 62, 30,
    37, 5, 45, 13, 53, 21, 61, 29, 36, 4, 44, 12, 52, 20, 60, 28, 35, 3, 43, 11, 51, 19, 59, 27,
    34, 2, 42, 10, 50, 18, 58, 26, 33, 1, 41, 9, 49, 17, 57, 25,
];
const DES_E: [u8; 48] = [
    32, 1, 2, 3, 4, 5, 4, 5, 6, 7, 8, 9, 8, 9, 10, 11, 12, 13, 12, 13, 14, 15, 16, 17, 16, 17, 18,
    19, 20, 21, 20, 21, 22, 23, 24, 25, 24, 25, 26, 27, 28, 29, 28, 29, 30, 31, 32, 1,
];
const DES_P: [u8; 32] = [
    16, 7, 20, 21, 29, 12, 28, 17, 1, 15, 23, 26, 5, 18, 31, 10, 2, 8, 24, 14, 32, 27, 3, 9, 19,
    13, 30, 6, 22, 11, 4, 25,
];
const DES_PC1: [u8; 56] = [
    57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59, 51, 43, 35, 27, 19, 11, 3, 60,
    52, 44, 36, 63, 55, 47, 39, 31, 23, 15, 7, 62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29,
    21, 13, 5, 28, 20, 12, 4,
];
const DES_PC2: [u8; 48] = [
    14, 17, 11, 24, 1, 5, 3, 28, 15, 6, 21, 10, 23, 19, 12, 4, 26, 8, 16, 7, 27, 20, 13, 2, 41, 52,
    31, 37, 47, 55, 30, 40, 51, 45, 33, 48, 44, 49, 39, 56, 34, 53, 46, 42, 50, 36, 29, 32,
];
const DES_ROTATIONS: [u8; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];
const DES_SBOX: [[[u8; 16]; 4]; 8] = [
    [
        [14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7],
        [0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12, 11, 9, 5, 3, 8],
        [4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0],
        [15, 12, 8, 2, 4, 9, 1, 7, 5, 11, 3, 14, 10, 0, 6, 13],
    ],
    [
        [15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10],
        [3, 13, 4, 7, 15, 2, 8, 14, 12, 0, 1, 10, 6, 9, 11, 5],
        [0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15],
        [13, 8, 10, 1, 3, 15, 4, 2, 11, 6, 7, 12, 0, 5, 14, 9],
    ],
    [
        [10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8],
        [13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5, 14, 12, 11, 15, 1],
        [13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7],
        [1, 10, 13, 0, 6, 9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12],
    ],
    [
        [7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15],
        [13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2, 12, 1, 10, 14, 9],
        [10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4],
        [3, 15, 0, 6, 10, 1, 13, 8, 9, 4, 5, 11, 12, 7, 2, 14],
    ],
    [
        [2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9],
        [14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15, 10, 3, 9, 8, 6],
        [4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14],
        [11, 8, 12, 7, 1, 14, 2, 13, 6, 15, 0, 9, 10, 4, 5, 3],
    ],
    [
        [12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11],
        [10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13, 14, 0, 11, 3, 8],
        [9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6],
        [4, 3, 2, 12, 9, 5, 15, 10, 11, 14, 1, 7, 6, 0, 8, 13],
    ],
    [
        [4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1],
        [13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5, 12, 2, 15, 8, 6],
        [1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2],
        [6, 11, 13, 8, 1, 4, 10, 7, 9, 5, 0, 15, 14, 2, 3, 12],
    ],
    [
        [13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7],
        [1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6, 11, 0, 14, 9, 2],
        [7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8],
        [2, 1, 14, 7, 4, 10, 8, 13, 15, 12, 9, 0, 3, 5, 6, 11],
    ],
];

fn des_permute(value: u64, input_bits: usize, table: &[u8]) -> u64 {
    let mut output = 0u64;
    for bit in table {
        output = (output << 1) | ((value >> (input_bits - *bit as usize)) & 1);
    }
    output
}

fn des_subkeys(key: &[u8]) -> [u64; 16] {
    let mut raw = 0u64;
    for value in key.iter().take(8) {
        raw = (raw << 8) | *value as u64;
    }
    let selected = des_permute(raw, 64, &DES_PC1);
    let mut c = (selected >> 28) & 0x0fff_ffff;
    let mut d = selected & 0x0fff_ffff;
    let mut result = [0u64; 16];
    for (index, rotation) in DES_ROTATIONS.iter().enumerate() {
        c = ((c << *rotation as u32) | (c >> (28 - *rotation as u32))) & 0x0fff_ffff;
        d = ((d << *rotation as u32) | (d >> (28 - *rotation as u32))) & 0x0fff_ffff;
        result[index] = des_permute((c << 28) | d, 56, &DES_PC2);
    }
    result
}

fn des_block(input: [u8; 8], subkeys: &[u64; 16], decrypt: bool) -> [u8; 8] {
    let mut raw = 0u64;
    for value in input {
        raw = (raw << 8) | value as u64;
    }
    let permuted = des_permute(raw, 64, &DES_IP);
    let mut left = (permuted >> 32) as u32;
    let mut right = permuted as u32;
    for round in 0..16 {
        let key = if decrypt {
            subkeys[15 - round]
        } else {
            subkeys[round]
        };
        let expanded = des_permute(right as u64, 32, &DES_E);
        let mixed = expanded ^ key;
        let mut substituted = 0u32;
        for box_index in 0..8 {
            let six = ((mixed >> (42 - box_index * 6)) & 0x3f) as usize;
            let row = ((six & 0x20) >> 4) | (six & 1);
            let col = (six >> 1) & 0xf;
            substituted = (substituted << 4) | DES_SBOX[box_index][row][col] as u32;
        }
        let f = des_permute(substituted as u64, 32, &DES_P) as u32;
        let next = left ^ f;
        left = right;
        right = next;
    }
    des_permute(((right as u64) << 32) | left as u64, 64, &DES_FP).to_be_bytes()
}

fn legacy_des_encrypt(key: &str, text: &str) -> Vec<u8> {
    let mut key_bytes = [0u8; 8];
    for (slot, value) in key_bytes.iter_mut().zip(key.as_bytes().iter()) {
        *slot = *value;
    }
    let pad = 8 - text.len() % 8;
    let mut data = text.as_bytes().to_vec();
    data.resize(data.len() + pad, pad as u8);
    let subkeys = des_subkeys(&key_bytes);
    data.chunks_exact(8)
        .map(|chunk| des_block(chunk.try_into().expect("DES block"), &subkeys, false))
        .flat_map(|block| block.into_iter())
        .collect()
}

fn legacy_des_decrypt(key: &str, data: &[u8]) -> Option<String> {
    if data.is_empty() || data.len() % 8 != 0 {
        return None;
    }
    let mut key_bytes = [0u8; 8];
    for (slot, value) in key_bytes.iter_mut().zip(key.as_bytes().iter()) {
        *slot = *value;
    }
    let subkeys = des_subkeys(&key_bytes);
    let mut out = data
        .chunks_exact(8)
        .map(|chunk| des_block(chunk.try_into().expect("DES block"), &subkeys, true))
        .flat_map(|block| block.into_iter())
        .collect::<Vec<_>>();
    let pad = *out.last()? as usize;
    if pad == 0
        || pad > 8
        || pad > out.len()
        || !out[out.len() - pad..].iter().all(|x| *x as usize == pad)
    {
        return None;
    }
    out.truncate(out.len() - pad);
    String::from_utf8(out).ok()
}

fn legacy_equality_expression(left: &Expr, right: &Expr) -> bool {
    fn operand(expression: &Expr) -> bool {
        matches!(
            expression,
            Expr::Variable(_)
                | Expr::Member(_, _)
                | Expr::DynamicMember(_, _)
                | Expr::Index(_, _)
                | Expr::Value(DynValue::Bool(_))
        ) || matches!(
            expression,
            Expr::Value(DynValue::Null | DynValue::Undefined)
        ) || matches!(expression, Expr::Value(DynValue::String(value)) if value.is_empty())
            || matches!(expression, Expr::Unary(operator, _, _) if operator == "@")
    }
    operand(left) && operand(right)
}

fn equal_values(left: &DynValue, right: &DynValue) -> bool {
    if let DynValue::Object(object) = left {
        if let Some(scalar) = object.borrow().properties.get("__gs2value").cloned() {
            return equal_values(&scalar, right);
        }
    }
    if let DynValue::Object(object) = right {
        if let Some(scalar) = object.borrow().properties.get("__gs2value").cloned() {
            return equal_values(left, &scalar);
        }
    }
    if (matches!(left, DynValue::Null | DynValue::Undefined)
        && matches!(right, DynValue::String(value) if value.is_empty()))
        || (matches!(right, DynValue::Null | DynValue::Undefined)
            && matches!(left, DynValue::String(value) if value.is_empty()))
    {
        return true;
    }
    if (matches!(left, DynValue::Null | DynValue::Undefined)
        && matches!(right, DynValue::Number(value) if *value == 0.0))
        || (matches!(right, DynValue::Null | DynValue::Undefined)
            && matches!(left, DynValue::Number(value) if *value == 0.0))
    {
        return true;
    }
    match (left, right) {
        (DynValue::Undefined, DynValue::Undefined) | (DynValue::Null, DynValue::Null) => true,
        (DynValue::Undefined, DynValue::Null) | (DynValue::Null, DynValue::Undefined) => true,
        (DynValue::Bool(a), DynValue::Bool(b)) => a == b,
        (DynValue::Number(a), DynValue::Number(b)) => a == b,
        (DynValue::String(a), DynValue::String(b)) => a == b,
        (DynValue::Bool(_), DynValue::Number(_)) | (DynValue::Number(_), DynValue::Bool(_)) => {
            number_f64(left) == number_f64(right)
        }
        (DynValue::Object(a), DynValue::Object(b)) => Rc::ptr_eq(a, b),
        (DynValue::Array(a), DynValue::Array(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}

fn binary_value(left: &DynValue, operator: &str, right: &DynValue) -> DynValue {
    match operator {
        "@" => DynValue::String(format!(
            "{}{}",
            js_concat_string(left),
            js_concat_string(right)
        )),
        "+" => {
            if matches!(left, DynValue::String(_)) || matches!(right, DynValue::String(_)) {
                DynValue::String(format!(
                    "{}{}",
                    js_concat_string(left),
                    js_concat_string(right)
                ))
            } else {
                DynValue::Number(number_f64(left) + number_f64(right))
            }
        }
        "-" => DynValue::Number(number_f64(left) - number_f64(right)),
        "*" => DynValue::Number(number_f64(left) * number_f64(right)),
        "/" => DynValue::Number(number_f64(left) / number_f64(right)),
        "%" => DynValue::Number(number_f64(left) % number_f64(right)),
        "^" => DynValue::Number(number_f64(left).powf(number_f64(right))),
        "==" | "===" => DynValue::Bool(equal_values(left, right)),
        "!=" | "!==" => DynValue::Bool(!equal_values(left, right)),
        "<" => DynValue::Bool(compare_values(left, right).is_some_and(|x| x < 0)),
        ">" => DynValue::Bool(compare_values(left, right).is_some_and(|x| x > 0)),
        "<=" => DynValue::Bool(compare_values(left, right).is_some_and(|x| x <= 0)),
        ">=" => DynValue::Bool(compare_values(left, right).is_some_and(|x| x >= 0)),
        "|" => DynValue::Number((number_i64(left) | number_i64(right)) as f64),
        "&" => DynValue::Number((number_i64(left) & number_i64(right)) as f64),
        "xor" => DynValue::Number((number_i64(left) ^ number_i64(right)) as f64),
        "in" => {
            let needle = value_string(left);
            let found = match right {
                DynValue::Array(values) => values.borrow().iter().any(|x| equal_values(left, x)),
                DynValue::Object(object) => object
                    .borrow()
                    .properties
                    .keys()
                    .any(|x| x.eq_ignore_ascii_case(&needle)),
                DynValue::String(value) => value.contains(&needle),
                _ => false,
            };
            DynValue::Bool(found)
        }
        _ => DynValue::Undefined,
    }
}

fn js_concat_string(value: &DynValue) -> String {
    match value {
        DynValue::Undefined => "undefined".to_string(),
        DynValue::Null => "null".to_string(),
        _ => value_string(value),
    }
}

fn compare_values(left: &DynValue, right: &DynValue) -> Option<i8> {
    if matches!(left, DynValue::String(_)) || matches!(right, DynValue::String(_)) {
        return Some(value_string(left).cmp(&value_string(right)) as i8);
    }
    number_f64(left)
        .partial_cmp(&number_f64(right))
        .map(|value| match value {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        })
}

fn find_function(
    functions: &HashMap<String, Rc<ScriptFunction>>,
    event_name: &str,
) -> Option<Rc<ScriptFunction>> {
    let event_name = event_name.replace('.', "_");
    let mut names = vec![event_name.clone()];
    if let Some(index) = event_name.rfind('_') {
        names.push(event_name[index + 1..].to_string());
    }
    if !event_name.to_ascii_lowercase().starts_with("on") && !event_name.is_empty() {
        let mut chars = event_name.chars();
        if let Some(first) = chars.next() {
            names.push(format!(
                "on{}{}",
                first.to_ascii_uppercase(),
                chars.collect::<String>()
            ));
        }
    }
    let original = names.clone();
    for name in original {
        if !name.to_ascii_lowercase().starts_with("on") && !name.is_empty() {
            let mut chars = name.chars();
            if let Some(first) = chars.next() {
                names.push(format!(
                    "on{}{}",
                    first.to_ascii_uppercase(),
                    chars.collect::<String>()
                ));
            }
        }
    }
    names.into_iter().find_map(|name| {
        let normalized = name.replace('.', "_");
        functions
            .iter()
            .find(|(key, _)| {
                key.eq_ignore_ascii_case(&name)
                    || key.replace('.', "_").eq_ignore_ascii_case(&normalized)
            })
            .map(|(_, value)| Rc::clone(value))
    })
}

fn is_player_lifecycle_event(event: &str) -> bool {
    matches!(
        event.to_ascii_lowercase().as_str(),
        "onplayerlogin" | "onplayerlogout"
    )
}

fn import_constructor_name(name: &str) -> String {
    let name = name.trim();
    let base = name.rsplit(['.', '/']).next().unwrap_or(name);
    if base.len() >= 6 && base[..6].eq_ignore_ascii_case("class_") {
        base[6..].to_string()
    } else {
        base.to_string()
    }
}

fn extract_imports(source: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = HashMap::new();
    for line in source.lines() {
        let line = line.trim();
        if !line.to_ascii_lowercase().starts_with("import") {
            continue;
        }
        let rest = line[6..].trim();
        let name = rest.trim_end_matches(';').trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|x| x.is_ascii_alphanumeric() || matches!(x, '_' | '.' | '/'))
        {
            continue;
        }
        let key = name.to_ascii_lowercase();
        if seen.insert(key, true).is_none() {
            result.push(name.to_string());
        }
    }
    result
}

fn extract_literal_import_joins(source: &str) -> Vec<String> {
    let source = strip_comments(source);
    let lower = source.to_ascii_lowercase();
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut seen = HashMap::new();
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find("join") {
        let start = cursor + relative;
        let end = start + 4;
        let before_ok =
            start == 0 || !bytes[start - 1].is_ascii_alphanumeric() && bytes[start - 1] != b'_';
        let after_ok =
            end >= bytes.len() || !bytes[end].is_ascii_alphanumeric() && bytes[end] != b'_';
        if !before_ok || !after_ok {
            cursor = end;
            continue;
        }
        let mut position = end;
        while position < bytes.len() && bytes[position].is_ascii_whitespace() {
            position += 1;
        }
        if position >= bytes.len() || bytes[position] != b'(' {
            cursor = end;
            continue;
        }
        position += 1;
        while position < bytes.len() && bytes[position].is_ascii_whitespace() {
            position += 1;
        }
        if position >= bytes.len() || !matches!(bytes[position], b'"' | b'\'') {
            cursor = end;
            continue;
        }
        let quote = bytes[position];
        position += 1;
        let value_start = position;
        while position < bytes.len() && bytes[position] != quote {
            position += 1;
        }
        if position > value_start && position < bytes.len() {
            let name = &source[value_start..position];
            let key = name.to_ascii_lowercase();
            if seen.insert(key, true).is_none() {
                result.push(name.to_string());
            }
        }
        cursor = position.saturating_add(1);
    }
    result
}

fn make_imports(
    config: &VMConfig,
    names: &[String],
) -> std::result::Result<HashMap<String, ParsedProgram>, String> {
    make_imports_with_requirement(config, names, true)
}

fn make_imports_with_requirement(
    config: &VMConfig,
    names: &[String],
    require_constructor: bool,
) -> std::result::Result<HashMap<String, ParsedProgram>, String> {
    let mut result = HashMap::new();
    for name in names {
        let source = config
            .imports
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .or_else(|| {
                config
                    .import_resolver
                    .as_ref()
                    .and_then(|resolver| resolver(name))
            })
            .ok_or_else(|| format!("import {name} was not found"))?;
        let nested_names = extract_imports(&source);
        let nested = make_imports_with_requirement(config, &nested_names, true)?;
        for (key, value) in nested {
            result.entry(key).or_insert(value);
        }
        let joined_names = extract_literal_import_joins(&source);
        let joined = make_imports_with_requirement(config, &joined_names, false)?;
        for (key, value) in joined {
            result.entry(key).or_insert(value);
        }
        let clean = translate_server_script(&source);
        let program = Parser::new(&clean)
            .parse()
            .map_err(|error| format!("import {name}: {error}"))?;
        result.insert(name.to_ascii_lowercase(), program);
        if require_constructor {
            let module = result
                .get(&name.to_ascii_lowercase())
                .expect("just inserted import module");
            let constructor_name = import_constructor_name(name);
            if !module.functions.iter().any(|function| {
                function.public && function.name.eq_ignore_ascii_case(&constructor_name)
            }) {
                return Err(format!(
                    "import {name} has no public constructor {constructor_name}"
                ));
            }
        }
    }
    Ok(result)
}

fn is_reserved_identifier(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "this"
            | "thiso"
            | "temp"
            | "player"
            | "client"
            | "clientr"
            | "params"
            | "name"
            | "server"
            | "serverr"
            | "serveroptions"
            | "allplayers"
            | "players"
            | "weapons"
            | "servers"
    )
}

pub fn run(config: VMConfig) -> VMResult {
    let mut config = config;
    config.script = translate_server_script(&config.script);
    if config.script.trim().is_empty() {
        return VMResult::default();
    }
    let program = match Parser::new(&config.script).parse() {
        Ok(value) => value,
        Err(error) => {
            return VMResult {
                err: error,
                ..VMResult::default()
            };
        }
    };
    // Keep the parser boundary explicit; the execution path below is the only
    // place where the source is turned into a live program.
    let imports = match make_imports(&config, &extract_imports(&config.script)) {
        Ok(value) => value,
        Err(error) => {
            return VMResult {
                err: error,
                ..VMResult::default()
            };
        }
    };
    EvalState::new(config, &program, imports).run(program)
}
