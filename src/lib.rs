#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
#![allow(non_snake_case)]

pub mod config;
pub mod http_api;
pub mod model;
pub mod network;
pub mod protocol;
pub mod websocket;

pub use config::{FileInfo, FileSystem, Logger, Settings};
pub use model::{
    Account, CachedLevel, CachedListserverServer, Character, FilePermissions, GS2SocketManager,
    Level, LevelBaddy, LevelBoardChange, LevelChest, LevelHorse, LevelItem, LevelItemType,
    LevelLink, LevelSign, LevelTiles, Map, MapLevel, MapType, MapTypeBigMap, MapTypeGmap,
    NPCServer, NPCSnapshot, Permission, PermissionCount, PermissionRead, PermissionType,
    PermissionWrite, Player, ScriptClass, ScriptHelpEntry, ScriptScanMatch, Server, ServerList,
    UpdatePackage, UpdatePackageFileEntry, Weapon, WordFilter, WordFilterRule, NPC,
};
pub use network::{Buffer, Encryption, SocketManager, ITERATOR_START};
pub use protocol::*;
pub use websocket::{make_websocket_frame, parse_websocket_frame, WebSocketFrame};

pub const APP_NAME: &str = "GameServer";
pub const APP_VERSION: &str = "0.1.262";

pub fn appVersion() -> &'static str {
    APP_VERSION
}
