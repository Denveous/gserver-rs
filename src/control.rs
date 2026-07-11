use std::sync::Arc;
use tokio::sync::RwLock;

use crate::server::Server;
use crate::player::Player;
use crate::protocol::*;
use crate::buffer::Buffer;

pub async fn handle_packet( _server: Arc<RwLock<Server>>, player_arc: Arc<RwLock<Player>>, packet: &[u8]) {
    if packet.is_empty() { return; }
    
    let packet_id = packet[0];
    let p = player_arc.clone();
    let mut player_lock = p.write().await;
    
    match packet_id {
        PLI_RC_SERVEROPTIONSGET => { player_lock.msg_pli_rc_serveroptionsget(packet).await; }
        PLI_RC_SERVEROPTIONSSET => { player_lock.msg_pli_rc_serveroptionsset(packet).await; }
        PLI_RC_FOLDERCONFIGGET => { player_lock.msg_pli_rc_folderconfigget(packet).await; }
        PLI_RC_FOLDERCONFIGSET => { player_lock.msg_pli_rc_folderconfigset(packet).await; }
        PLI_RC_RESPAWNSET => { player_lock.msg_pli_rc_respawnset(packet).await; }
        PLI_RC_HORSELIFESET => { player_lock.msg_pli_rc_horselifeset(packet).await; }
        PLI_RC_APINCREMENTSET => { player_lock.msg_pli_rc_apincrementset(packet).await; }
        PLI_RC_BADDYRESPAWNSET => { player_lock.msg_pli_rc_baddyrespawnset(packet).await; }
        PLI_RC_PLAYERPROPSGET => { player_lock.msg_pli_rc_playerpropsget(packet).await; }
        PLI_RC_PLAYERPROPSSET => { player_lock.msg_pli_rc_playerpropsset(packet).await; }
        PLI_RC_DISCONNECTPLAYER => { player_lock.msg_pli_rc_disconnectplayer(packet).await; }
        PLI_RC_UPDATELEVELS => { player_lock.msg_pli_rc_updatelevels(packet).await; }
        PLI_RC_ADMINMESSAGE => { player_lock.msg_pli_rc_adminmessage(packet).await; }
        PLI_RC_PRIVADMINMESSAGE => { player_lock.msg_pli_rc_privadminmessage(packet).await; }
        PLI_RC_LISTRCS => { player_lock.msg_pli_rc_listrcs(packet).await; }
        PLI_RC_DISCONNECTRC => { player_lock.msg_pli_rc_disconnectrc(packet).await; }
        PLI_RC_APPLYREASON => { player_lock.msg_pli_rc_applyreason(packet).await; }
        PLI_RC_SERVERFLAGSGET => { player_lock.msg_pli_rc_serverflagsget(packet).await; }
        PLI_RC_SERVERFLAGSSET => { player_lock.msg_pli_rc_serverflagsset(packet).await; }
        PLI_RC_ACCOUNTADD => { player_lock.msg_pli_rc_accountadd(packet).await; }
        PLI_RC_ACCOUNTDEL => { player_lock.msg_pli_rc_accountdel(packet).await; }
        PLI_RC_ACCOUNTLISTGET => { player_lock.msg_pli_rc_accountlistget(packet).await; }
        PLI_RC_PLAYERPROPSGET2 => { player_lock.msg_pli_rc_playerpropsget2(packet).await; }
        PLI_RC_PLAYERPROPSGET3 => { player_lock.msg_pli_rc_playerpropsget3(packet).await; }
        PLI_RC_PLAYERPROPSRESET => { player_lock.msg_pli_rc_playerpropsreset(packet).await; }
        PLI_RC_PLAYERPROPSSET2 => { player_lock.msg_pli_rc_playerpropsset2(packet).await; }
        _ => {
            player_lock.handle_general_packet(packet).await;
        }
    }
}
