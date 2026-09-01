#![allow(clippy::too_many_arguments)]

pub mod compiler;
pub mod runtime;
pub mod settings;
pub mod socket;
pub mod standalone;
pub mod tiletypes;
pub mod vm;

pub use compiler::{
    CompileResult, Diagnostic, bytecode_header, bytecode_with_header, clientside_gs2,
    clientside_script_is_gs1, compile_for_feedback, compile_gs2_script, diagnostics_text,
    format_clientside_weapon_script, serverside_gs2, translate_server_script,
};

#[allow(non_snake_case)]
pub fn CompileForFeedback(script_type: &str, script_name: &str, script: &str) -> CompileResult {
    compile_for_feedback(script_type, script_name, script)
}

#[allow(non_snake_case)]
pub fn CompileGS2Script(script: &str) -> CompileResult {
    compile_gs2_script(script)
}

#[allow(non_snake_case)]
pub fn DiagnosticsText(diagnostics: &[Diagnostic]) -> String {
    diagnostics_text(diagnostics)
}

#[allow(non_snake_case)]
pub fn BytecodeWithHeader(
    bytecode: &[u8],
    script_type: &str,
    script_name: &str,
    save_to_disk: bool,
) -> Vec<u8> {
    bytecode_with_header(bytecode, script_type, script_name, save_to_disk)
}

#[allow(non_snake_case)]
pub fn BytecodeHeader(bytecode: &[u8]) -> (Vec<u8>, bool) {
    bytecode_header(bytecode)
}

#[allow(non_snake_case)]
pub fn ClientsideGS2(script: &str) -> (String, bool) {
    match clientside_gs2(script) {
        Some(value) => (value, true),
        None => (String::new(), false),
    }
}

#[allow(non_snake_case)]
pub fn ClientsideScriptIsGS1(script: &str) -> bool {
    clientside_script_is_gs1(script)
}

#[allow(non_snake_case)]
pub fn FormatClientsideWeaponScript(script: &str) -> (String, bool) {
    format_clientside_weapon_script(script)
}
pub use runtime::{
    ACCOUNT_NAME, AddressFor, ConfiguredNickname, DEFAULT_PM_REPLY, FileEventHandler,
    IsLocationQuery, Logger, New, Runtime, address_for, configured_nickname, is_location_query,
};
pub use settings::{LoadSettings, ParseSettings, Settings, load_settings, parse_settings};
pub use socket::{
    CopyAnyMap, NewSocketManager, SocketEvent, SocketManager, SocketScript, SocketUpdate,
    copy_any_map,
};
pub use standalone::{
    NewStandalone, Standalone, legacy_frame, read_legacy_frame, standalone_login_frame,
    standalone_nickname_frame, wait_cancel, write_all, zlib_compress, zlib_decompress,
};
pub use vm::{
    Any, AnyMap, ChestContext, ClientTrigger, ClientTriggerSink, ClientTriggerSinkFunc, FileAction,
    IRCMessage, LevelAction, NPCAction, NPCContext, NPCFlag, NPCFunctionCall, PlayerAttachment,
    PlayerClass, PlayerContext, PlayerEffect, PlayerFlag, PlayerMessage, PlayerProp, PlayerWarp,
    PlayerWeapon, ScheduledEvent, ServerContext, ServerFlag, SignContext, SocketAction,
    SocketContext, VMConfig, VMResult, WaitEvent, WeaponContext,
};

pub type VMPlayerFlag = PlayerFlag;
pub type VMPlayerProp = PlayerProp;
pub type VMServerFlag = ServerFlag;
pub type VMPlayerMessage = PlayerMessage;
pub type VMPlayerIRCMessage = IRCMessage;
pub type VMPlayerWeapon = PlayerWeapon;
pub type VMPlayerClass = PlayerClass;
pub type VMPlayerWarp = PlayerWarp;
pub type VMNPCFlag = NPCFlag;
pub type VMNPCFunctionCall = NPCFunctionCall;
pub type VMNPCAction = NPCAction;
pub type VMLevelAction = LevelAction;
pub type VMScheduledEvent = ScheduledEvent;
pub type VMWaitEvent = WaitEvent;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptResult {
    pub output: Vec<String>,
    pub client_triggers: Vec<String>,
    pub player_flags: Vec<VMPlayerFlag>,
    pub player_props: Vec<VMPlayerProp>,
    pub server_flags: Vec<VMServerFlag>,
    pub player_messages: Vec<VMPlayerMessage>,
    pub player_rc_messages: Vec<VMPlayerMessage>,
    pub player_irc_messages: Vec<VMPlayerIRCMessage>,
    pub player_effects: Vec<PlayerEffect>,
    pub rc_messages: Vec<String>,
    pub nc_messages: Vec<String>,
    pub player_weapons: Vec<VMPlayerWeapon>,
    pub player_classes: Vec<VMPlayerClass>,
    pub player_warps: Vec<VMPlayerWarp>,
    pub player_attachments: Vec<PlayerAttachment>,
    pub file_actions: Vec<FileAction>,
    pub npc_flags: Vec<VMNPCFlag>,
    pub npc_function_calls: Vec<VMNPCFunctionCall>,
    pub npc_actions: Vec<VMNPCAction>,
    pub level_actions: Vec<VMLevelAction>,
    pub socket_actions: Vec<SocketAction>,
    pub socket_updates: Vec<SocketUpdate>,
    pub scheduled_events: Vec<VMScheduledEvent>,
    pub wait_events: Vec<VMWaitEvent>,
    pub this: AnyMap,
    pub err: String,
    pub script_type: String,
    pub script_name: String,
    pub event_name: String,
    pub script: String,
    pub player_context: std::collections::HashMap<String, String>,
    pub npc_id: u32,
    pub vm_revision: i64,
}

pub fn run_vm(config: VMConfig) -> VMResult {
    vm::run(config)
}

pub fn run_script(
    script_type: &str,
    script_name: &str,
    event_name: &str,
    script: &str,
    mut config: VMConfig,
) -> ScriptResult {
    config.script_name = script_name.to_string();
    config.event_name = event_name.to_string();
    config.script = serverside_gs2(script);
    if config.script.trim().is_empty() {
        return ScriptResult::default();
    }
    let result = run_vm(config.clone());
    convert_result(
        script_type,
        script_name,
        event_name,
        script,
        &config.player,
        config.npc_id,
        &result,
    )
}

pub fn convert_result(
    script_type: &str,
    script_name: &str,
    event_name: &str,
    script: &str,
    player_context: &std::collections::HashMap<String, String>,
    npc_id: u32,
    result: &VMResult,
) -> ScriptResult {
    let mut out = ScriptResult {
        output: result.output.clone(),
        this: result.this.clone(),
        err: result.err.clone(),
        script_type: script_type.to_string(),
        script_name: script_name.to_string(),
        event_name: event_name.to_string(),
        script: script.to_string(),
        player_context: player_context.clone(),
        npc_id,
        ..ScriptResult::default()
    };
    for trigger in &result.client_triggers {
        let mut parts = vec![trigger.name.clone()];
        parts.extend(trigger.args.clone());
        out.client_triggers
            .push(format!("clientside,{}", parts.join(",")));
    }
    out.player_flags = result.player_flags.clone();
    out.player_props = result.player_props.clone();
    out.server_flags = result.server_flags.clone();
    out.player_messages = result.player_messages.clone();
    out.player_rc_messages = result.player_rc_messages.clone();
    out.player_irc_messages = result.player_irc_messages.clone();
    out.player_effects = result.player_effects.clone();
    out.rc_messages = result.rc_messages.clone();
    out.nc_messages = result.nc_messages.clone();
    out.player_weapons = result.player_weapons.clone();
    out.player_classes = result.player_classes.clone();
    out.player_warps = result.player_warps.clone();
    out.player_attachments = result.player_attachments.clone();
    out.file_actions = result.file_actions.clone();
    out.npc_flags = result.npc_flags.clone();
    out.npc_function_calls = result.npc_function_calls.clone();
    out.npc_actions = result.npc_actions.clone();
    out.level_actions = result.level_actions.clone();
    out.socket_actions = result.socket_actions.clone();
    out.socket_updates = result
        .socket_updates
        .iter()
        .map(|value| SocketUpdate {
            name: value.name.clone(),
            id: value.id.clone(),
            address: value.address.clone(),
            port: value.port,
            ip_address: value.ip_address.clone(),
            data: value.data.clone(),
            buffer: value.buffer.clone(),
            package_delimiter: value.package_delimiter.clone(),
            is_connected: value.is_connected,
            state: value.state.clone(),
            joined_classes: value.joined_classes.clone(),
            parent_name: value.parent_name.clone(),
            parent_id: value.parent_id.clone(),
        })
        .collect();
    out.scheduled_events = result.scheduled_events.clone();
    out.wait_events = result.wait_events.clone();
    out
}

pub fn copy_string_map(
    values: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    values.clone()
}

pub fn rename_on_created(script: &str, function_name: &str) -> String {
    let lower = script.to_ascii_lowercase();
    let mut result = String::with_capacity(script.len());
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find("function") {
        let start = cursor + relative;
        let after = start + 8;
        let valid_before = start == 0
            || !script.as_bytes()[start - 1].is_ascii_alphanumeric()
                && script.as_bytes()[start - 1] != b'_';
        let valid_after = after >= script.len()
            || !script.as_bytes()[after].is_ascii_alphanumeric()
                && script.as_bytes()[after] != b'_';
        if valid_before && valid_after {
            let mut pos = after;
            while pos < script.len() && script.as_bytes()[pos].is_ascii_whitespace() {
                pos += 1;
            }
            if lower[pos..].starts_with("oncreated") {
                let end = pos + 9;
                let mut after_name = end;
                while after_name < script.len()
                    && script.as_bytes()[after_name].is_ascii_whitespace()
                {
                    after_name += 1;
                }
                if script.as_bytes().get(after_name) == Some(&b'(') {
                    result.push_str(&script[cursor..pos]);
                    result.push_str(function_name);
                    cursor = end;
                }
            }
        }
        cursor = cursor.max(after);
    }
    result.push_str(&script[cursor..]);
    result
}

pub fn inject_joined_class_on_created(script: &str) -> String {
    let lower = script.to_ascii_lowercase();
    let Some(start) = lower.find("function") else {
        return format!(
            "{script}\nfunction onCreated() {{ __joinedClassOnCreatedBootstrap(); }}\n"
        );
    };
    let Some(name_start) = lower[start + 8..].find("oncreated").map(|x| start + 8 + x) else {
        return format!(
            "{script}\nfunction onCreated() {{ __joinedClassOnCreatedBootstrap(); }}\n"
        );
    };
    let Some(open_rel) = script[name_start..].find('{') else {
        return format!(
            "{script}\nfunction onCreated() {{ __joinedClassOnCreatedBootstrap(); }}\n"
        );
    };
    let open = name_start + open_rel + 1;
    format!(
        "{}\n  __joinedClassOnCreatedBootstrap();{}",
        &script[..open],
        &script[open..]
    )
}

pub use compiler::{append_gshort, read_gchar};
#[allow(non_snake_case)]
pub fn RunScript(
    script_type: &str,
    script_name: &str,
    event_name: &str,
    script: &str,
    config: VMConfig,
) -> ScriptResult {
    run_script(script_type, script_name, event_name, script, config)
}

#[allow(non_snake_case)]
pub fn ConvertResult(
    script_type: &str,
    script_name: &str,
    event_name: &str,
    script: &str,
    player_context: &std::collections::HashMap<String, String>,
    npc_id: u32,
    result: &VMResult,
) -> ScriptResult {
    convert_result(
        script_type,
        script_name,
        event_name,
        script,
        player_context,
        npc_id,
        result,
    )
}

#[allow(non_snake_case)]
pub fn RunVM(config: VMConfig) -> VMResult {
    run_vm(config)
}

#[allow(non_snake_case)]
pub fn CopyStringMap(
    values: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    copy_string_map(values)
}

#[allow(non_snake_case)]
pub fn ServersideGS2(script: &str) -> String {
    serverside_gs2(script)
}

#[allow(non_snake_case)]
pub fn RenameOnCreated(script: &str, function_name: &str) -> String {
    rename_on_created(script, function_name)
}

#[allow(non_snake_case)]
pub fn InjectJoinedClassOnCreated(script: &str) -> String {
    inject_joined_class_on_created(script)
}
