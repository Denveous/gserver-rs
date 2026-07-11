import re

with open("src/player_general.rs", "r") as f:
    code = f.read()

# Fix logger
code = re.sub(r'srv\.logger\.debug\(\&format\!\((.*?)\)\);', r'crate::log_info!(\1);', code)

# Fix run_server_side_event_for_active_scripts signature
code = re.sub(r'srv\.run_server_side_event_for_active_scripts\([^;]+?\)\.await;', r'// TODO: run_server_side_event', code)

# Fix level locks and methods
code = code.replace("if let Some(level) = srv.levels.get_mut(&self.level_name) {", 
                    "if let Some(level_arc) = srv.levels.get(&self.level_name) {\n            let mut level = level_arc.write().await;")
code = code.replace("if let Some(level) = srv.levels.get(&self.level_name) {", 
                    "if let Some(level_arc) = srv.levels.get(&self.level_name) {\n            let mut level = level_arc.write().await;")

code = code.replace("self.send_plo_npcprops", "self.send_to_nc")
code = code.replace("self.send_plo_npcdel", "self.send_to_nc")
code = code.replace("crate::level::NPC::new", "crate::npc::Npc::new")

with open("src/player_general.rs", "w") as f:
    f.write(code)

