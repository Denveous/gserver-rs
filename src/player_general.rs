use crate::player::Player;
use crate::buffer::Buffer;
use crate::protocol::*;

impl Player {
    pub async fn handle_general_packet(&mut self, packet: &[u8]) -> bool {
        if packet.is_empty() { return false; }
        
        match packet[0] {
            PLI_LEVELWARP | PLI_LEVELWARPMOD => self.msg_pli_levelwarp(packet).await,
            PLI_TOALL => self.msg_pli_toall(packet).await,
            PLI_PRIVATEMESSAGE => self.msg_pli_privatemessage(packet).await,
            PLI_WEAPONADD => self.msg_pli_weaponadd(packet).await,
            PLI_ITEMADD => self.msg_pli_itemadd(packet).await,
            PLI_ITEMDEL | PLI_ITEMTAKE => self.msg_pli_itemdel(packet).await,
            PLI_CLAIMPKER => self.msg_pli_claimpker(packet).await,
            PLI_BADDYPROPS => self.msg_pli_baddyprops(packet).await,
            PLI_BADDYHURT => self.msg_pli_baddyhurt(packet).await,
            PLI_BADDYADD => self.msg_pli_baddyadd(packet).await,
            PLI_FLAGSET => self.msg_pli_flagset(packet).await,
            PLI_FLAGDEL => self.msg_pli_flagdel(packet).await,
            PLI_OPENCHEST => self.msg_pli_openchest(packet).await,
            PLI_PUTNPC => self.msg_pli_putnpc(packet).await,
            PLI_NPCDEL => self.msg_pli_npcdel(packet).await,
            PLI_WANTFILE => self.msg_pli_wantfile(packet).await,
            PLI_SHOWIMG => self.msg_pli_showimg(packet).await,
            PLI_HURTPLAYER => self.msg_pli_hurtplayer(packet).await,
            PLI_EXPLOSION => self.msg_pli_explosion(packet).await,
            _ => false,
        }
    }

    pub async fn msg_pli_levelwarp(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        let mut _mod_time = 0;
        if packet[0] == PLI_LEVELWARPMOD {
            if buf.bytes_left() < 5 { return true; }
            _mod_time = buf.read_gint5() as i64;
        }
        if buf.bytes_left() < 3 { return true; }
        
        let x = buf.read_gchar() as f64 / 2.0;
        let y = buf.read_gchar() as f64 / 2.0;
        
        let level_name = String::from_utf8_lossy(&buf.read_bytes(buf.bytes_left())).trim().to_string();
        if level_name.is_empty() || level_name.len() < 3 || level_name.contains('\0') || level_name.contains('\r') || level_name.contains('\n') {
            return true;
        }
        
        self.warp(&level_name, x, y);
        // Note: mod_time and broadcasting are typically handled in warp()
        true
    }

    pub async fn msg_pli_toall(&mut self, packet: &[u8]) -> bool {
        if packet.len() > 1 {
            let mut buf = Buffer::from_bytes(packet[1..].to_vec());
            let msg = buf.read_gchar_string();
            // p.lastChat = time.Now()
            self.send_pto_all_chat(&msg).await;
        }
        true
    }
    
    pub async fn send_pto_all_chat(&self, message: &str) {
        let mut buf = Buffer::new();
        buf.write_byte(PLO_TOALL);
        buf.write_gshort(self.id);
        buf.write_gchar(message.len() as u8);
        buf.write_bytes(message.as_bytes());
        
        let srv = self.server.read().await;
        for pl_arc in srv.players.values() {
            let pl = pl_arc.read().await;
            if pl.level_name == self.level_name {
                pl.send(&buf.bytes()).await;
            }
        }
    }

    pub async fn msg_pli_privatemessage(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        if buf.bytes_left() < 2 { return true; }
        let target_count = buf.read_gshort() as usize;
        let mut targets = Vec::with_capacity(target_count);
        for _ in 0..target_count {
            if buf.bytes_left() >= 2 {
                targets.push(buf.read_gshort());
            }
        }
        let msg = String::from_utf8_lossy(&buf.read_bytes(buf.bytes_left())).into_owned();
        let msg_type = if target_count > 1 {
            "\"Mass message:\","
        } else {
            "\"Private message:\","
        };
        
        let srv = self.server.read().await;
        for target_id in targets {
            if let Some(pl_arc) = srv.players.get(&target_id) {
                let pl = pl_arc.read().await;
                if pl.id != self.id {
                    let mut out = Buffer::new();
                    out.write_byte(PLO_PRIVATEMESSAGE);
                    out.write_gshort(self.id);
                    out.write_bytes(b"\"\",");
                    out.write_bytes(msg_type.as_bytes());
                    out.write_bytes(msg.as_bytes());
                    pl.send(&out.bytes()).await;
                }
            }
        }
        true
    }

    pub async fn msg_pli_weaponadd(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        if buf.bytes_left() < 1 { return true; }
        let weapon_type = buf.read_gchar();
        if weapon_type == 0 {
            if buf.bytes_left() >= 1 {
                let _item_type = buf.read_gchar();
                // TODO itemType resolving
            }
            return true;
        }
        
        if buf.bytes_left() < 4 { return true; }
        let npc_id = buf.read_gint();
        let srv = self.server.read().await;
        if let Some(npc_arc) = srv.npcs.get(&npc_id) {
            let npc = npc_arc.read().await;
            if !npc.weapon_name.is_empty() && !self.weapon_list.contains(&npc.weapon_name) {
                self.weapon_list.push(npc.weapon_name.clone());
            }
        }
        true
    }
    pub async fn send_to_current_level_except_self(&self, data: &[u8]) {
        let srv = self.server.read().await;
        for pl_arc in srv.players.values() {
            let pl = pl_arc.read().await;
            if pl.id != self.id && pl.level_name == self.level_name {
                pl.send(data).await;
            }
        }
    }

    pub async fn msg_pli_itemadd(&mut self, packet: &[u8]) -> bool {
        if packet.len() >= 4 {
            let mut buf = Buffer::from_bytes(packet[1..].to_vec());
            let x = buf.read_gchar() as f32;
            let y = buf.read_gchar() as f32;
            let item_type = buf.read_gchar() as i32;
            let srv = self.server.read().await;
            if let Some(level_arc) = srv.levels.get(&self.level_name) {
                let mut level = level_arc.write().await;
                level.add_item(x, y, item_type);
                // TODO: run_server_side_event
            }
        }
        let mut out = Buffer::new();
        out.write_byte(PLO_ITEMADD);
        out.write_bytes(&packet[1..]);
        self.send_to_current_level_except_self(&out.bytes()).await;
        true
    }

    pub async fn msg_pli_itemdel(&mut self, packet: &[u8]) -> bool {
        let mut item_type: i32 = -1;
        if packet.len() >= 3 {
            let mut buf = Buffer::from_bytes(packet[1..].to_vec());
            let x = buf.read_gchar() as f32;
            let y = buf.read_gchar() as f32;
            {
                let srv = self.server.read().await;
                if let Some(level_arc) = srv.levels.get(&self.level_name) {
                    let mut level = level_arc.write().await;
                    item_type = level.remove_item(x, y);
                    if item_type == -1 {
                        item_type = level.remove_item(x / 2.0, y / 2.0);
                    }
                }
            }
        }
        let mut out = Buffer::new();
        out.write_byte(PLO_ITEMDEL);
        out.write_bytes(&packet[1..]);
        self.send_to_current_level_except_self(&out.bytes()).await;
        
        if packet[0] == PLI_ITEMTAKE { if item_type > 0 { self.apply_level_item(item_type as u8).await; } } true
    }

    pub async fn msg_pli_claimpker(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        if buf.bytes_left() < 2 { return true; }
        let pker_id = buf.read_gshort();
        
        let srv = self.server.read().await;
        if let Some(pl_arc) = srv.players.get(&pker_id) {
            let mut pl = pl_arc.write().await;
            pl.set_flag("killer", &self.account_name).await;
        }
        true
    }

    pub async fn msg_pli_baddyprops(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        if buf.bytes_left() < 1 { return true; }
        let baddy_id = buf.read_gchar();
        let bytes_left = buf.bytes_left();
        let props = String::from_utf8_lossy(&buf.read_bytes(bytes_left)).into_owned();
        
        let mut srv = self.server.write().await;
        if let Some(level_arc) = srv.levels.get(&self.level_name) {
            let parts: Vec<&str> = props.splitn(2, '=').collect();
            if parts.len() == 2 {
                let baddy_id = parts[0].parse::<u32>().unwrap_or(0);
                let props = parts[1];
                let mut baddy = crate::level::LevelBaddy::new(0.0, 0.0, 0); // Temporary dummy
                baddy.id = baddy_id as u8;
                baddy.set_props(props.as_bytes());
                self.send_plo_levelbaddyprops(&baddy).await;
            }
        }
        true
    }

    pub async fn msg_pli_baddyhurt(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        if buf.bytes_left() < 2 { return true; }
        let baddy_id = buf.read_gchar();
        let hurt_power = buf.read_gchar();
        
        let srv = self.server.read().await;
        if let Some(level_arc) = srv.levels.get(&self.level_name) {
            let level = level_arc.read().await;
            for pl_id in &level.players {
                if let Some(pl_arc) = srv.players.get(pl_id) {
                    let pl = pl_arc.read().await;
                    pl.send_plo_levelbaddyprops(&crate::level::LevelBaddy::new(0.0, 0.0, 0)).await; // Dummy baddy for hurt 
                }
            }
        }
        true
    }

    pub async fn msg_pli_baddyadd(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        if buf.bytes_left() < 4 { return true; }
        let x = buf.read_gchar() as f32;
        let y = buf.read_gchar() as f32;
        let baddy_type = buf.read_gchar();
        let baddy_power = buf.read_gchar();
        let bytes_left = buf.bytes_left();
        let baddy_image = String::from_utf8_lossy(&buf.read_bytes(bytes_left)).into_owned();
        
        let srv = self.server.read().await;
        if let Some(level_arc) = srv.levels.get(&self.level_name) {
            let mut level = level_arc.write().await;
            let mut baddy = crate::level::LevelBaddy::new(x / 2.0, y / 2.0, baddy_type);
            baddy.id = level.baddies.len() as u8;
            if baddy_power > 0 || !baddy_image.is_empty() {
                let mut props = Buffer::new();
                props.write_gchar(4); // BDPROP_POWERIMAGE
                props.write_gchar(baddy_power);
                props.write_gchar(baddy_image.len() as u8);
                props.write_bytes(baddy_image.as_bytes());
                baddy.set_props(&props.bytes());
            }
            let baddy_id = baddy.id;
            level.baddies.insert(baddy_id, baddy.clone());
            
            for pl_id in level.players.clone() {
                if let Some(pl_arc) = srv.players.get(&pl_id) {
                    let pl = pl_arc.read().await;
                    pl.send_plo_levelbaddyprops(&baddy).await;
                }
            }
        }
        true
    }

    pub async fn msg_pli_flagset(&mut self, packet: &[u8]) -> bool {
        if packet.len() > 1 {
            let s = String::from_utf8_lossy(&packet[1..]);
            let parts: Vec<&str> = s.splitn(2, '=').collect();
            if parts.len() == 2 {
                if self.handle_movement_flag(parts[0], parts[1]).await {
                    return true;
                }
                self.set_flag(parts[0], parts[1]).await;
            } else {
                self.set_flag(parts[0], "").await;
            }
        }
        true
    }

    pub async fn msg_pli_flagdel(&mut self, packet: &[u8]) -> bool {
        if packet.len() > 1 {
            let flag = String::from_utf8_lossy(&packet[1..]).into_owned();
            self.delete_flag(&flag).await;
        }
        true
    }

    pub async fn msg_pli_openchest(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        if buf.bytes_left() < 2 { return true; }
        let x = buf.read_gchar() as i32;
        let y = buf.read_gchar() as i32;
        
        let mut chest_to_open = None;
        {
            let mut srv = self.server.write().await;
            if let Some(level_arc) = srv.levels.get(&self.level_name) {
                let mut level = level_arc.write().await;
                for chest in level.chests.clone() {
                    if chest.x == x as f32 && chest.y == y as f32 {
                        chest_to_open = Some(chest.clone());
                        break;
                    }
                }
            }
        }

        if let Some(chest) = chest_to_open {
            let chest_key = format!("{}_{}_{}", self.level_name, chest.x, chest.y);
            if !self.has_chest(&chest_key).await {
                self.apply_level_item(chest.item_type).await;
                self.send_plo_levelchest(&chest, true).await;
                self.add_chest(&chest_key).await;
            }
        }
        true
    }

    pub async fn msg_pli_putnpc(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        let image = buf.read_gchar_string();
        let npc_code = buf.read_gchar_string();
        if buf.bytes_left() < 2 { return true; }
        let x = buf.read_gchar() as f32 / 2.0;
        let y = buf.read_gchar() as f32 / 2.0;
        
        let mut srv = self.server.write().await;
        if let Some(level_arc) = srv.levels.get(&self.level_name) {
            let mut level = level_arc.write().await;
            let mut npc = crate::npc::NPC::new(crate::npc::NPCType::LEVELNPC); // PUTNPC is 21
            npc.image = image;
            npc.x = (x * 16.0) as i16;
            npc.y = (y * 16.0) as i16;
            npc.script = npc_code;
            npc.level_name = level.level_name.clone();
            
            let npc_id = npc.id;
            level.npcs.insert(npc_id, npc.clone());
            self.send_plo_npcprops(&npc).await;
        }
        true
    }

    pub async fn msg_pli_npcdel(&mut self, packet: &[u8]) -> bool {
        let srv_read = self.server.read().await;
        if srv_read.settings.get_bool("serverside", false) {
            return true;
        }
        drop(srv_read);
        
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        if buf.bytes_left() < 4 { return true; }
        let npc_id = buf.read_gint();
        
        let srv = self.server.read().await;
        if let Some(level_arc) = srv.levels.get(&self.level_name) {
            let mut level = level_arc.write().await;
            if level.npcs.contains_key(&npc_id) {
                level.npcs.remove(&npc_id);
                self.send_plo_npcdel(npc_id).await;
            }
        }
        true
    }

    pub async fn msg_pli_wantfile(&mut self, packet: &[u8]) -> bool {
        if packet.len() > 1 {
            let file_name = String::from_utf8_lossy(&packet[1..]).into_owned();
            
            let srv = self.server.read().await;
            crate::log_info!("WANTFILE: {}", file_name);
            
            if file_name.to_lowercase().ends_with(".gupd") {
                if srv.resolve_requested_file(&file_name).is_ok() {
                    self.send_file(&file_name).await;
                } else {
                    self.send_plo_fileuptodate(&file_name).await;
                }
                return true;
            }
            if srv.is_default_client_file(&file_name) {
                if srv.resolve_requested_file(&file_name).is_ok() {
                    self.send_file(&file_name).await;
                } else {
                    self.send_plo_fileuptodate(&file_name).await;
                }
                return true;
            }
            self.send_file(&file_name).await;
        }
        true
    }

    pub async fn msg_pli_showimg(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::new();
        buf.write_byte(PLO_SHOWIMG);
        buf.write_gshort(self.id);
        buf.write_bytes(&packet[1..]);
        self.send_to_current_level_except_self(&buf.bytes()).await;
        true
    }

    pub async fn msg_pli_hurtplayer(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        if buf.bytes_left() < 8 { return true; }
        let victim_id = buf.read_gshort();
        let hurt_dx = buf.read_gchar();
        let hurt_dy = buf.read_gchar();
        let power = buf.read_gchar();
        let npc_id = buf.read_gint();
        
        let srv = self.server.read().await;
        if let Some(victim_arc) = srv.players.get(&victim_id) {
            let victim = victim_arc.read().await;
            
            let mut out = Buffer::new();
            out.write_byte(PLO_HURTPLAYER);
            out.write_gshort(self.id);
            out.write_gchar(hurt_dx);
            out.write_gchar(hurt_dy);
            out.write_gchar(power);
            out.write_gint(npc_id);
            victim.send(&out.bytes()).await;
            
            // TODO: run_server_side_event
        }
        true
    }

    pub async fn msg_pli_explosion(&mut self, packet: &[u8]) -> bool {
        let mut buf = Buffer::from_bytes(packet[1..].to_vec());
        if buf.bytes_left() < 4 { return true; }
        let radius = buf.read_gchar();
        let x = buf.read_gchar();
        let y = buf.read_gchar();
        let power = buf.read_gchar();
        
        let mut out = Buffer::new();
        out.write_byte(PLO_EXPLOSION);
        out.write_gshort(self.id);
        out.write_gchar(radius);
        out.write_gchar(x);
        out.write_gchar(y);
        out.write_gchar(power);
        self.send(&out.bytes()).await;
        true
}
    }
