fn rc_payload(packet: &[u8], packet_id: u8) -> &[u8] {
    if !packet.is_empty() && packet[0] == packet_id {
        &packet[1..]
    } else {
        packet
    }
}

fn read_rc_encoded_string_payload(payload: &[u8]) -> String {
    if payload.is_empty() || payload[0] < 32 {
        return String::new();
    }
    let name_len = (payload[0] - 32) as usize;
    if name_len != payload.len() - 1 {
        return String::new();
    }
    String::from_utf8_lossy(&payload[1..1 + name_len]).into_owned()
}

fn sanitize_rc_account_name(account_name: &str) -> String {
    let mut name = account_name.trim().to_string();
    if let Some(idx) = name.find('/') {
        name.truncate(idx);
    }
    if let Some(idx) = name.find('\\') {
        name.truncate(idx);
    }
    name
}

fn read_rc_account_payload(packet: &[u8], packet_id: u8) -> String {
    let payload = rc_payload(packet, packet_id);
    if payload.is_empty() {
        return String::new();
    }
    let account = read_rc_encoded_string_payload(payload);
    if !account.is_empty() {
        return sanitize_rc_account_name(&account);
    }
    let mut buf = Buffer::from_bytes(payload);
    let account = buf.read_gstring();
    if !account.is_empty() {
        return sanitize_rc_account_name(&account);
    }
    sanitize_rc_account_name(&String::from_utf8_lossy(payload))
}

fn read_rc_string8_account_payload(packet: &[u8], packet_id: u8) -> String {
    let payload = rc_payload(packet, packet_id);
    if payload.is_empty() {
        return String::new();
    }
    let account = read_rc_encoded_string_payload(payload);
    if !account.is_empty() {
        return sanitize_rc_account_name(&account);
    }
    let name_len = payload[0] as usize;
    if name_len <= payload.len() - 1 && payload[0] < 32 {
        return sanitize_rc_account_name(&String::from_utf8_lossy(&payload[1..1 + name_len]));
    }
    let mut buf = Buffer::from_bytes(payload);
    let account = buf.read_gstring();
    if !account.is_empty() {
        return sanitize_rc_account_name(&account);
    }
    sanitize_rc_account_name(&String::from_utf8_lossy(payload))
}

impl Player {
    pub async fn msg_pli_rc_updatelevels(&mut self, packet: &[u8]) -> bool {
        if (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        if !self.has_right(PLPERM_UPDATELEVEL) {
            let mut buf = Buffer::new();
            buf.write_byte(PLO_RC_CHAT);
            buf.write_bytes(format!("Server: {} is not authorized to update levels.", self.account_name).as_bytes());
            self.send(&buf.bytes()).await;
            return true;
        }
        if packet.len() > 1 {
            let mut buf = Buffer::from_bytes(&packet[1..]);
            let level_count = buf.read_gshort() as usize;
            for _ in 0..level_count {
                let _level_name = buf.read_gchar_string();
                // TODO: reload level
            }
        }
        true
    }

    pub async fn msg_pli_rc_adminmessage(&mut self, packet: &[u8]) -> bool {
        if self.player_type != PLTYPE_RC && self.player_type != PLTYPE_RC2 && (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        if !self.has_right(PLPERM_ADMINMSG) {
            let mut buf = Buffer::new();
            buf.write_byte(PLO_RC_CHAT);
            buf.write_bytes(b"Server: You are not authorized to send an admin message.");
            self.send(&buf.bytes()).await;
            return true;
        }
        if packet.len() > 1 {
            let mut buf = Buffer::from_bytes(&packet[1..]);
            let message = buf.read_string();
            
            let mut buf2 = Buffer::new();
            buf2.write_byte(PLO_RC_ADMINMESSAGE);
            buf2.write_string8(&format!("Admin {}:\xa7{}", self.account_name, message));
            
            let srv = self.server.read().await;
            for p_arc in srv.players.values() {
                let p = p_arc.read().await;
                if p.id != self.id {
                    p.send(&buf2.bytes()).await;
                }
            }
        }
        true
    }

    pub async fn msg_pli_rc_privadminmessage(&mut self, packet: &[u8]) -> bool {
        if self.player_type != PLTYPE_RC && self.player_type != PLTYPE_RC2 && (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        if !self.has_right(PLPERM_ADMINMSG) {
            let mut buf = Buffer::new();
            buf.write_byte(PLO_RC_CHAT);
            buf.write_bytes(b"Server: You are not authorized to send an admin message.");
            self.send(&buf.bytes()).await;
            return true;
        }
        if packet.len() > 1 {
            let mut buf = Buffer::from_bytes(&packet[1..]);
            let target_id = buf.read_gshort();
            let message = buf.read_string();
            
            let srv = self.server.read().await;
            if let Some(target_arc) = srv.players.get(&target_id) {
                let target = target_arc.read().await;
                let mut buf2 = Buffer::new();
                buf2.write_byte(PLO_RC_ADMINMESSAGE);
                buf2.write_string8(&format!("Admin {}:\xa7{}", self.account_name, message));
                target.send(&buf2.bytes()).await;
            }
        }
        true
    }

    pub async fn msg_pli_rc_listrcs(&mut self, _packet: &[u8]) -> bool {
        true
    }

    pub async fn msg_pli_rc_disconnectrc(&mut self, _packet: &[u8]) -> bool {
        true
    }

    pub async fn msg_pli_rc_applyreason(&mut self, _packet: &[u8]) -> bool {
        true
    }

    pub async fn msg_pli_rc_serverflagsget(&mut self, _packet: &[u8]) -> bool {
        if self.player_type != PLTYPE_RC && self.player_type != PLTYPE_RC2 && (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        let mut buf = Buffer::new();
        buf.write_byte(PLO_RC_SERVERFLAGSGET);
        
        let srv = self.server.read().await;
        let mut valid_flags = std::collections::HashMap::new();
        for (flag, value) in &srv.flags {
            valid_flags.insert(flag.clone(), value.clone());
        }
        buf.write_gshort(valid_flags.len() as u16);
        for (flag, value) in valid_flags {
            let flag_str = format!("{}={}", flag, value);
            buf.write_string8_encoded(&flag_str);
        }
        self.send(&buf.bytes()).await;
        true
    }

    pub async fn msg_pli_rc_serverflagsset(&mut self, packet: &[u8]) -> bool {
        if self.player_type != PLTYPE_RC && self.player_type != PLTYPE_RC2 && (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        if !self.has_right(PLPERM_SETSERVERFLAGS) {
            let mut buf = Buffer::new();
            buf.write_byte(PLO_RC_CHAT);
            buf.write_bytes(b"Server: You are not authorized to set the server flags.");
            self.send(&buf.bytes()).await;
            return true;
        }
        if packet.len() > 1 {
            let mut buf = Buffer::from_bytes(&packet[1..]);
            let count = buf.read_gshort();
            let mut srv = self.server.write().await;
            
            srv.flags.clear();
            for _ in 0..count {
                let flag_str = buf.read_gchar_string();
                if let Some(idx) = flag_str.find('=') {
                    let name = flag_str[..idx].trim().to_string();
                    let value = flag_str[idx + 1..].to_string();
                    srv.flags.insert(name, value);
                } else if !flag_str.is_empty() {
                    srv.flags.insert(flag_str, String::new());
                }
            }
            srv.send_rc_chat(&format!("{} has updated the server flags.", self.account_name)).await;
        }
        true
    }

    pub async fn msg_pli_rc_accountadd(&mut self, packet: &[u8]) -> bool {
        if self.player_type != PLTYPE_RC && self.player_type != PLTYPE_RC2 && (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        if !self.has_right(PLPERM_MODIFYSTAFFACCOUNT) {
            let mut buf = Buffer::new();
            buf.write_byte(PLO_RC_CHAT);
            buf.write_bytes(b"Server: You are not authorized to create new accounts.");
            self.send(&buf.bytes()).await;
            return true;
        }
        let payload = rc_payload(packet, PLI_RC_ACCOUNTADD);
        let mut buf = Buffer::from_bytes(payload);
        let account_name = buf.read_gchar_string();
        let _ = buf.read_gchar_string();
        let email = buf.read_gchar_string();
        let banned = buf.read_gchar() != 0;
        let load_only = buf.read_gchar() != 0;
        let _ = buf.read_gchar();
        
        let mut account = Player::new(self.server.clone(), 0);
        account.account_name = account_name.clone();
        account.email = email;
        account.is_banned = banned;
        account.is_load_only = load_only;
        account.save_account().await;
        
        let srv = self.server.read().await;
        srv.send_rc_chat(&format!("{} has created a new account: {}", self.account_name, account_name)).await;
        true
    }

    pub async fn msg_pli_rc_accountdel(&mut self, packet: &[u8]) -> bool {
        if self.player_type != PLTYPE_RC && self.player_type != PLTYPE_RC2 && (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        if !self.has_right(PLPERM_MODIFYSTAFFACCOUNT) {
            let mut buf = Buffer::new();
            buf.write_byte(PLO_RC_CHAT);
            buf.write_bytes(b"Server: You are not authorized to delete accounts.");
            self.send(&buf.bytes()).await;
            return true;
        }
        let account_name = read_rc_account_payload(packet, PLI_RC_ACCOUNTDEL);
        if account_name.eq_ignore_ascii_case("defaultaccount") {
            let mut buf = Buffer::new();
            buf.write_byte(PLO_RC_CHAT);
            buf.write_bytes(b"Server: You are not allowed to delete the default account.");
            self.send(&buf.bytes()).await;
            return true;
        }
        if account_name.is_empty() {
            return true;
        }
        let srv = self.server.read().await;
        let account_path = format!("accounts/{}.txt", account_name);
        if srv.fs.file_exists(&account_path) {
            let _ = srv.fs.delete_file(&account_path);
            srv.send_rc_chat(&format!("{} has deleted the account: {}", self.account_name, account_name)).await;
        }
        true
    }

    pub async fn msg_pli_rc_accountlistget(&mut self, packet: &[u8]) -> bool {
        if self.player_type != PLTYPE_RC && self.player_type != PLTYPE_RC2 && (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        let payload = rc_payload(packet, PLI_RC_ACCOUNTLISTGET);
        let mut buf = Buffer::from_bytes(payload);
        let mut name = buf.read_gchar_string();
        let _conditions = buf.read_gchar_string();
        name = name.replace("%", "*");
        if name.is_empty() {
            name = "*".to_string();
        }
        
        let mut buf2 = Buffer::new();
        buf2.write_byte(PLO_RC_ACCOUNTLISTGET);
        
        let srv = self.server.read().await;
        if let Ok(files) = srv.fs.list_files("accounts") {
            let mut keys = Vec::new();
            for file in files {
                if file.ends_with(".txt") {
                    let acct = file.trim_end_matches(".txt").to_string();
                    keys.push(acct);
                }
            }
            keys.sort();
            for key in keys {
                buf2.write_string8_encoded(&key);
            }
        }
        self.send(&buf2.bytes()).await;
        true
    }

    pub async fn msg_pli_rc_playerpropsget2(&mut self, packet: &[u8]) -> bool {
        let payload = rc_payload(packet, PLI_RC_PLAYERPROPSGET2);
        let mut buf = Buffer::from_bytes(payload);
        let player_id = buf.read_gshort();
        
        let srv = self.server.read().await;
        let target_arc = match srv.players.get(&player_id) {
            Some(p) => p.clone(),
            None => return true,
        };
        
        if self.player_type != PLTYPE_RC && self.player_type != PLTYPE_RC2 && (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        if !self.has_right(PLPERM_VIEWATTRIBUTES) {
            let mut chat_buf = Buffer::new();
            chat_buf.write_byte(PLO_RC_CHAT);
            chat_buf.write_bytes(b"Server: You are not authorized to view player props.");
            self.send(&chat_buf.bytes()).await;
            return true;
        }
        
        let target = target_arc.read().await;
        let mut buf2 = Buffer::new();
        buf2.write_byte(PLO_RC_PLAYERPROPSGET);
        buf2.write_gshort(target.id);
        buf2.write_bytes(&target.get_props_rc());
        
        self.send(&buf2.bytes()).await;
        srv.send_rc_chat(&format!("{} has opened the attributes of {}", self.account_name, target.account_name)).await;
        true
    }

    pub async fn msg_pli_rc_playerpropsget3(&mut self, packet: &[u8]) -> bool {
        let account_name = read_rc_string8_account_payload(packet, PLI_RC_PLAYERPROPSGET3);
        
        let srv = self.server.read().await;
        let mut target_arc_opt = None;
        for p_arc in srv.players.values() {
            let p = p_arc.read().await;
            if p.account_name.eq_ignore_ascii_case(&account_name) {
                target_arc_opt = Some(p_arc.clone());
                break;
            }
        }
        
        let target_arc = if let Some(arc) = target_arc_opt {
            arc
        } else {
            if account_name.is_empty() || account_name.contains('/') || account_name.contains('\\') {
                let mut buf = Buffer::new();
                buf.write_byte(PLO_RC_CHAT);
                buf.write_bytes(format!("Server: Account {} does not exist.", account_name).as_bytes());
                self.send(&buf.bytes()).await;
                return true;
            }
            
            let mut temp_player = Player::new(self.server.clone(), 0);
            if !temp_player.load_account(&account_name, false).await {
                let mut buf = Buffer::new();
                buf.write_byte(PLO_RC_CHAT);
                buf.write_bytes(format!("Server: Account {} does not exist.", account_name).as_bytes());
                self.send(&buf.bytes()).await;
                return true;
            }
            Arc::new(RwLock::new(temp_player))
        };
        
        if self.player_type != PLTYPE_RC && self.player_type != PLTYPE_RC2 && (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        if !self.has_right(PLPERM_VIEWATTRIBUTES) {
            let mut buf = Buffer::new();
            buf.write_byte(PLO_RC_CHAT);
            buf.write_bytes(b"Server: You are not authorized to view player props.");
            self.send(&buf.bytes()).await;
            return true;
        }
        
        let target = target_arc.read().await;
        let mut buf2 = Buffer::new();
        buf2.write_byte(PLO_RC_PLAYERPROPSGET);
        buf2.write_gshort(target.id);
        buf2.write_bytes(&target.get_props_rc());
        self.send(&buf2.bytes()).await;
        srv.send_rc_chat(&format!("{} has opened the attributes of {}", self.account_name, account_name)).await;
        
        true
    }

    pub async fn msg_pli_rc_playerpropsreset(&mut self, packet: &[u8]) -> bool {
        let account_name = read_rc_account_payload(packet, PLI_RC_PLAYERPROPSRESET);
        if self.player_type != PLTYPE_RC && self.player_type != PLTYPE_RC2 && (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        if !self.has_right(PLPERM_RESETATTRIBUTES) {
            let mut buf = Buffer::new();
            buf.write_byte(PLO_RC_CHAT);
            buf.write_bytes(b"Server: You are not authorized to reset accounts.");
            self.send(&buf.bytes()).await;
            return true;
        }
        let srv = self.server.read().await;
        let mut target_arcs = Vec::new();
        for p_arc in srv.players.values() {
            let p = p_arc.read().await;
            if p.account_name.eq_ignore_ascii_case(&account_name) {
                target_arcs.push(p_arc.clone());
            }
        }
        
        let account_path = format!("accounts/{}.txt", account_name);
        
        if target_arcs.is_empty() {
            if !srv.fs.file_exists(&account_path) {
                return true;
            }
            let _ = srv.fs.delete_file(&account_path);
            srv.send_rc_chat(&format!("{} has reset the attributes of account: {}", self.account_name, account_name)).await;
            return true;
        }
        
        for target_arc in target_arcs {
            let mut target = target_arc.write().await;
            let mut buf = Buffer::new();
            buf.write_byte(PLO_DISCMESSAGE);
            buf.write_string(&format!("Your account was reset by {}", self.account_name));
            target.send(&buf.bytes()).await;
            target.disconnect().await;
            target.reset_account();
        }
        let _ = srv.fs.delete_file(&account_path);
        srv.send_rc_chat(&format!("{} has reset the attributes of account: {}", self.account_name, account_name)).await;
        true
    }

    pub async fn msg_pli_rc_playerpropsset2(&mut self, packet: &[u8]) -> bool {
        if packet.len() <= 1 {
            return true;
        }
        let mut buf = Buffer::from_bytes(&packet[1..]);
        let account_name_len = buf.read_gchar();
        let mut account_name = String::from_utf8_lossy(buf.read_bytes(account_name_len as usize)).into_owned();
        if let Some(idx) = account_name.find('/') {
            account_name.truncate(idx);
        }
        if let Some(idx) = account_name.find('\\') {
            account_name.truncate(idx);
        }
        
        if self.player_type != PLTYPE_RC && self.player_type != PLTYPE_RC2 && (self.player_type & PLTYPE_ANYRC) == 0 {
            return true;
        }
        
        let srv = self.server.read().await;
        let mut target_arc_opt = None;
        for p_arc in srv.players.values() {
            let p = p_arc.read().await;
            if p.account_name.eq_ignore_ascii_case(&account_name) {
                target_arc_opt = Some(p_arc.clone());
                break;
            }
        }
        
        let target_arc = if let Some(arc) = target_arc_opt {
            arc
        } else {
            let account_path = format!("accounts/{}.txt", account_name);
            if !srv.fs.file_exists(&account_path) {
                return true;
            }
            let mut temp_player = Player::new(self.server.clone(), 0);
            if !temp_player.load_account(&account_name, false).await {
                return true;
            }
            Arc::new(RwLock::new(temp_player))
        };
        
        let mut target = target_arc.write().await;
        if (target.account_name != self.account_name && !self.has_right(PLPERM_SETATTRIBUTES)) ||
            (target.account_name == self.account_name && !self.has_right(PLPERM_SETSELFATTRIBUTES)) {
            let mut chat_buf = Buffer::new();
            chat_buf.write_byte(PLO_RC_CHAT);
            chat_buf.write_bytes(format!("Server: {} is not authorized to set the properties of {}", self.account_name, target.account_name).as_bytes());
            self.send(&chat_buf.bytes()).await;
            return true;
        }
        
        if !self.has_right(PLPERM_MODIFYSTAFFACCOUNT) && target.is_staff {
            let mut chat_buf = Buffer::new();
            chat_buf.write_byte(PLO_RC_CHAT);
            chat_buf.write_bytes(b"Server: You are not authorized to modify staff accounts.");
            self.send(&chat_buf.bytes()).await;
            return true;
        }
        
        target.set_props_from_rc(&mut buf, self);
        target.save_account().await;
        
        srv.send_rc_chat(&format!("{} set the attributes of player {}", self.account_name, target.account_name)).await;
        
        true
    }
}
