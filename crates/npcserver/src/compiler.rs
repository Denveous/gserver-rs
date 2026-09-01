#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use rand::RngCore;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompileResult {
    pub bytecode: Vec<u8>,
    pub err_text: String,
    pub warning_text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The opcode values are kept equal to HexaParser's GS2 bytecode values.
pub mod op {
    pub const NONE: u8 = 0;
    pub const SET_INDEX: u8 = 1;
    pub const SET_INDEX_TRUE: u8 = 2;
    pub const OR: u8 = 3;
    pub const IF: u8 = 4;
    pub const AND: u8 = 5;
    pub const CALL: u8 = 6;
    pub const RET: u8 = 7;
    pub const SLEEP: u8 = 8;
    pub const CMD_CALL: u8 = 9;
    pub const JMP: u8 = 10;
    pub const WAIT_FOR: u8 = 11;
    pub const TYPE_NUMBER: u8 = 20;
    pub const TYPE_STRING: u8 = 21;
    pub const TYPE_VAR: u8 = 22;
    pub const TYPE_ARRAY: u8 = 23;
    pub const TYPE_TRUE: u8 = 24;
    pub const TYPE_FALSE: u8 = 25;
    pub const TYPE_NULL: u8 = 26;
    pub const PI: u8 = 27;
    pub const COPY_LAST_OP: u8 = 30;
    pub const SWAP_LAST_OPS: u8 = 31;
    pub const INDEX_DEC: u8 = 32;
    pub const CONV_TO_FLOAT: u8 = 33;
    pub const CONV_TO_STRING: u8 = 34;
    pub const MEMBER_ACCESS: u8 = 35;
    pub const CONV_TO_OBJECT: u8 = 36;
    pub const ARRAY_END: u8 = 37;
    pub const ARRAY_NEW: u8 = 38;
    pub const SET_ARRAY: u8 = 39;
    pub const INLINE_NEW: u8 = 40;
    pub const MAKE_VAR: u8 = 41;
    pub const NEW_OBJECT: u8 = 42;
    pub const INLINE_CONDITIONAL: u8 = 44;
    pub const ASSIGN: u8 = 50;
    pub const FUNC_PARAMS_END: u8 = 51;
    pub const INC: u8 = 52;
    pub const DEC: u8 = 53;
    pub const ADD: u8 = 60;
    pub const SUB: u8 = 61;
    pub const MUL: u8 = 62;
    pub const DIV: u8 = 63;
    pub const MOD: u8 = 64;
    pub const POW: u8 = 65;
    pub const NOT: u8 = 68;
    pub const UNARY_SUB: u8 = 69;
    pub const EQ: u8 = 70;
    pub const NEQ: u8 = 71;
    pub const LT: u8 = 72;
    pub const GT: u8 = 73;
    pub const LTE: u8 = 74;
    pub const GTE: u8 = 75;
    pub const BIT_OR: u8 = 76;
    pub const BIT_AND: u8 = 77;
    pub const BIT_XOR: u8 = 78;
    pub const BIT_INVERT: u8 = 79;
    pub const IN_RANGE: u8 = 80;
    pub const IN_OBJ: u8 = 81;
    pub const OBJ_INDEX: u8 = 82;
    pub const OBJ_TYPE: u8 = 83;
    pub const FORMAT: u8 = 84;
    pub const INT: u8 = 85;
    pub const ABS: u8 = 86;
    pub const RANDOM: u8 = 87;
    pub const SIN: u8 = 88;
    pub const COS: u8 = 89;
    pub const ARCTAN: u8 = 90;
    pub const EXP: u8 = 91;
    pub const LOG: u8 = 92;
    pub const MIN: u8 = 93;
    pub const MAX: u8 = 94;
    pub const GET_ANGLE: u8 = 95;
    pub const GET_DIR: u8 = 96;
    pub const VECX: u8 = 97;
    pub const VECY: u8 = 98;
    pub const OBJ_INDICES: u8 = 99;
    pub const OBJ_LINK: u8 = 100;
    pub const SHIFT_LEFT: u8 = 101;
    pub const SHIFT_RIGHT: u8 = 102;
    pub const CHAR: u8 = 103;
    pub const OBJ_TRIM: u8 = 110;
    pub const OBJ_LENGTH: u8 = 111;
    pub const OBJ_POS: u8 = 112;
    pub const JOIN: u8 = 113;
    pub const OBJ_CHAR_AT: u8 = 114;
    pub const OBJ_SUBSTR: u8 = 115;
    pub const OBJ_STARTS: u8 = 116;
    pub const OBJ_ENDS: u8 = 117;
    pub const OBJ_TOKENIZE: u8 = 118;
    pub const TRANSLATE: u8 = 119;
    pub const OBJ_POSITIONS: u8 = 120;
    pub const OBJ_SIZE: u8 = 130;
    pub const ARRAY: u8 = 131;
    pub const ARRAY_ASSIGN: u8 = 132;
    pub const ARRAY_MULTI_DIM: u8 = 133;
    pub const ARRAY_MULTI_DIM_ASSIGN: u8 = 134;
    pub const OBJ_SUBARRAY: u8 = 135;
    pub const OBJ_ADD_STRING: u8 = 136;
    pub const OBJ_DELETE_STRING: u8 = 137;
    pub const OBJ_REMOVE_STRING: u8 = 138;
    pub const OBJ_REPLACE_STRING: u8 = 139;
    pub const OBJ_INSERT_STRING: u8 = 140;
    pub const OBJ_CLEAR: u8 = 141;
    pub const ARRAY_NEW_MULTI_DIM: u8 = 142;
    pub const WITH: u8 = 150;
    pub const WITH_END: u8 = 151;
    pub const FOREACH: u8 = 163;
    pub const THIS: u8 = 180;
    pub const THISO: u8 = 181;
    pub const PLAYER: u8 = 182;
    pub const PLAYERO: u8 = 183;
    pub const LEVEL: u8 = 184;
    pub const TEMP: u8 = 189;
}

#[derive(Clone, Debug)]
struct FunctionEntry {
    name: String,
    op_index: usize,
}

#[derive(Default)]
struct BytecodeWriter {
    gs1_event_flags: i32,
    code: Vec<u8>,
    strings: Vec<String>,
    string_index: HashMap<String, usize>,
    functions: Vec<FunctionEntry>,
    function_set: HashSet<String>,
    prejump_patches: Vec<usize>,
    last_op: u8,
    op_index: usize,
}

impl BytecodeWriter {
    fn new(flags: i32) -> Self {
        Self {
            gs1_event_flags: flags,
            ..Self::default()
        }
    }

    fn get_string(&mut self, value: &str) -> usize {
        if let Some(index) = self.string_index.get(value) {
            return *index;
        }
        let index = self.strings.len();
        self.strings.push(value.to_string());
        self.string_index.insert(value.to_string(), index);
        index
    }

    fn add_function(&mut self, name: String, op_index: usize) {
        if self.function_set.insert(name.clone()) {
            self.functions.push(FunctionEntry { name, op_index });
        }
    }

    fn emit(&mut self, opcode: u8) {
        self.code.push(opcode);
        self.last_op = opcode;
        self.op_index += 1;
    }

    fn emit_placeholder(&mut self) -> usize {
        self.code.push(0xf4);
        let pos = self.code.len();
        self.code.extend_from_slice(&[0, 0]);
        pos
    }

    fn emit_short(&mut self, value: i32) {
        self.code.push(0xf4);
        self.code.push((value >> 8) as u8);
        self.code.push(value as u8);
    }

    fn patch_short(&mut self, pos: usize, value: usize) {
        if pos + 1 < self.code.len() {
            self.code[pos] = (value >> 8) as u8;
            self.code[pos + 1] = value as u8;
        }
    }

    fn emit_dynamic_string_index(&mut self, value: usize) {
        if value <= 0xff {
            self.code.extend_from_slice(&[0xf0, value as u8]);
        } else if value <= 0xffff {
            self.code.push(0xf1);
            self.code.extend_from_slice(&(value as u16).to_be_bytes());
        } else {
            self.code.push(0xf2);
            self.code.extend_from_slice(&(value as u32).to_be_bytes());
        }
    }

    fn emit_dynamic_number(&mut self, value: i32) {
        let offset = if matches!(
            self.last_op,
            op::TYPE_NUMBER | op::SET_INDEX | op::SET_INDEX_TRUE
        ) {
            3
        } else {
            0
        };
        if (-128..=127).contains(&value) {
            self.code
                .extend_from_slice(&[0xf0 + offset, value as i8 as u8]);
        } else if (-32768..=32767).contains(&value) {
            self.code.push(0xf1 + offset);
            self.code.extend_from_slice(&(value as i16).to_be_bytes());
        } else {
            self.code.push(0xf2 + offset);
            self.code.extend_from_slice(&value.to_be_bytes());
        }
    }

    fn emit_default_value(&mut self) {
        self.emit(op::TYPE_NUMBER);
        self.emit_dynamic_number(0);
    }

    fn to_bytes(mut self) -> Vec<u8> {
        self.emit(op::RET);
        for pos in self.prejump_patches.clone() {
            self.patch_short(pos, self.op_index);
        }
        let mut result = Vec::new();
        let mut flags = Vec::new();
        flags.extend_from_slice(&(self.gs1_event_flags as u32).to_be_bytes());
        write_segment(&mut result, 1, &flags);
        let mut functions = Vec::new();
        let mut ordered = Vec::new();
        for value in &self.strings {
            for function in &self.functions {
                if &function.name == value && !ordered.iter().any(|x: &String| x == &function.name)
                {
                    ordered.push(function.name.clone());
                }
            }
        }
        for function in &self.functions {
            if !ordered.iter().any(|x| x == &function.name) {
                ordered.push(function.name.clone());
            }
        }
        for name in ordered {
            if let Some(function) = self.functions.iter().find(|x| x.name == name) {
                functions.extend_from_slice(&(function.op_index as u32).to_be_bytes());
                functions.extend_from_slice(function.name.as_bytes());
                functions.push(0);
            }
        }
        write_segment(&mut result, 2, &functions);
        let mut strings = Vec::new();
        for value in &self.strings {
            strings.extend_from_slice(value.as_bytes());
            strings.push(0);
        }
        write_segment(&mut result, 3, &strings);
        write_segment(&mut result, 4, &self.code);
        result.push(10);
        result
    }
}

fn write_segment(out: &mut Vec<u8>, segment: u32, value: &[u8]) {
    out.extend_from_slice(&segment.to_be_bytes());
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}

pub fn format_clientside_weapon_script(script: &str) -> (String, bool) {
    if script.trim().is_empty() {
        return (String::new(), false);
    }
    let upper = script.to_ascii_uppercase();
    let marker = "//#CLIENTSIDE";
    let Some(index) = upper.find(marker) else {
        return (script.to_string(), true);
    };
    let client = &script[index..];
    let lines = client.replace("\r\n", "\n");
    let mut result = String::new();
    for (index, line) in lines.split('\n').enumerate() {
        if index > 0 {
            result.push('§');
        }
        result.push_str(line.trim());
    }
    result.push('§');
    (result, true)
}

pub fn clientside_script_is_gs1(script: &str) -> bool {
    script.to_ascii_uppercase().contains("//#GS1")
}

pub fn clientside_gs2(script: &str) -> Option<String> {
    if clientside_script_is_gs1(script) {
        return None;
    }
    let marker = "//#CLIENTSIDE";
    let upper = script.to_ascii_uppercase();
    upper
        .find(marker)
        .map(|index| script[index + marker.len()..].trim().to_string())
}

pub fn serverside_gs2(script: &str) -> String {
    let normalized = script.replace("\r\n", "\n");
    let lower = normalized.to_ascii_lowercase();
    lower
        .find("//#clientside")
        .map_or(normalized.clone(), |index| {
            normalized[..index].trim().to_string()
        })
}

/// Apply the source-level compatibility pass before server code is handed to
/// the script interpreter. The Rust interpreter accepts GS2 directly, but GS1
/// command lines still need conversion so the same script files keep working.
pub fn translate_server_script(script: &str) -> String {
    translate_temp_load_string(&translate_legacy_gs1(&serverside_gs2(script)))
}

/// Translate the legacy `temp.name.loadstring(path)` form before parsing.
///
/// HexaVM treats this spelling as a load-into-variable operation: it writes
/// the loaded text to both `temp.name` and the bare `name` alias and returns a
/// boolean success value. It is deliberately handled as a source rewrite so
/// nested expressions such as `findfiles(...)[0]` retain their evaluation
/// order.
fn translate_temp_load_string(script: &str) -> String {
    let bytes = script.as_bytes();
    let mut output = String::with_capacity(script.len());
    let mut cursor = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'"' || bytes[index] == b'\'' {
            index = skip_script_string(bytes, index);
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        let at_word_boundary = index == 0 || !is_script_identifier_byte(bytes[index - 1]);
        if !at_word_boundary || !starts_with_ascii_case_insensitive(bytes, index, b"temp.") {
            index += 1;
            continue;
        }
        let name_start = index + b"temp.".len();
        let mut name_end = name_start;
        while name_end < bytes.len() && is_script_identifier_byte(bytes[name_end]) {
            name_end += 1;
        }
        if name_end == name_start {
            index += 1;
            continue;
        }
        let mut method_start = name_end;
        while method_start < bytes.len() && bytes[method_start].is_ascii_whitespace() {
            method_start += 1;
        }
        if bytes.get(method_start) != Some(&b'.') {
            index += 1;
            continue;
        }
        method_start += 1;
        while method_start < bytes.len() && bytes[method_start].is_ascii_whitespace() {
            method_start += 1;
        }
        if !starts_with_ascii_case_insensitive(bytes, method_start, b"loadstring") {
            index += 1;
            continue;
        }
        let mut open = method_start + b"loadstring".len();
        if bytes
            .get(open)
            .is_some_and(|value| is_script_identifier_byte(*value))
        {
            index += 1;
            continue;
        }
        while open < bytes.len() && bytes[open].is_ascii_whitespace() {
            open += 1;
        }
        if bytes.get(open) != Some(&b'(') {
            index += 1;
            continue;
        }
        let Some(close) = matching_script_paren(bytes, open) else {
            index += 1;
            continue;
        };
        output.push_str(&script[cursor..index]);
        output.push_str("__gs2LoadStringVar(temp, ");
        output.push_str(&quote_script_string(&script[name_start..name_end]));
        output.push_str(", ");
        output.push_str(&script[open + 1..close]);
        output.push(')');
        index = close + 1;
        cursor = index;
    }
    output.push_str(&script[cursor..]);
    output
}

fn is_script_identifier_byte(value: u8) -> bool {
    value == b'_' || value == b'$' || value.is_ascii_alphanumeric()
}

fn starts_with_ascii_case_insensitive(source: &[u8], index: usize, wanted: &[u8]) -> bool {
    source
        .get(index..index.saturating_add(wanted.len()))
        .is_some_and(|value| value.eq_ignore_ascii_case(wanted))
}

fn skip_script_string(source: &[u8], start: usize) -> usize {
    let quote = source[start];
    let mut index = start + 1;
    let mut escaped = false;
    while index < source.len() {
        if escaped {
            escaped = false;
        } else if source[index] == b'\\' {
            escaped = true;
        } else if source[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    index
}

fn matching_script_paren(source: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;
    while index < source.len() {
        match source[index] {
            b'"' | b'\'' => index = skip_script_string(source, index),
            b'/' if source.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < source.len() && source[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if source.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < source.len()
                    && !(source[index] == b'*' && source[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(source.len());
            }
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn translate_legacy_gs1(script: &str) -> String {
    script
        .split('\n')
        .map(translate_legacy_gs1_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn translate_legacy_gs1_line(line: &str) -> String {
    let (code, comment) = split_gs1_comment(line);
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return line.to_string();
    }
    if let Some(command) = trimmed.strip_suffix(';').map(str::trim) {
        if is_gs1_bare_command(command) {
            return format!(
                "{}{}();{}",
                &code[..code.len() - code.trim_start().len()],
                command_alias(command),
                comment
            );
        }
    }
    let Some((indent, command, raw_args)) = split_gs1_command(code) else {
        return line.to_string();
    };
    // A normal GS2 call already has parentheses.  The exception is a legacy
    // token embedded in a string-like argument, which is intentionally left
    // to the GS1 conversion below.
    if raw_args.contains('(')
        && !contains_ascii_case(raw_args, "#s(")
        && !contains_ascii_case(raw_args, "#v(")
        && !contains_ascii_case(raw_args, "#e(")
    {
        return line.to_string();
    }
    let command_lower = command.to_ascii_lowercase();
    let args = split_top_level_commas(raw_args);
    let output = match command_lower.as_str() {
        "sendtonc" | "sendtorc" | "echo" | "trace" | "printf" | "setimg" | "addweapon"
        | "removeweapon" | "sleep" | "setlevel" | "sethead" | "setbody" | "setplayerdir"
        | "say" | "loadmap" | "setletters" | "setgif" | "setbackpal" | "showfile" | "openurl"
        | "openurl2" | "sendpm" | "sendrpgmessage" | "carryobject" | "lay" | "setshootparams"
        | "toweapons" => format!(
            "{}({});",
            if command_lower == "toweapons" {
                "addweapon"
            } else {
                command_lower.as_str()
            },
            gs1_text_expr(raw_args)
        ),
        "say2" => format!("message({});", gs1_text_expr(raw_args)),
        "freezeplayer" | "freezeplayer2" | "hurt" | "shootarrow" | "shootfireball"
        | "shootfireblast" | "shootnuke" | "hideimg" => {
            format!("{}({});", command_lower, gs1_value_expr(raw_args))
        }
        "unfreezeplayer" => "unfreezeplayer();".to_string(),
        "dontblocklocal" => "dontblock();".to_string(),
        "blockagainlocal" => "blockagain();".to_string(),
        "putbomb" | "hideimgs" | "move" | "setshape" | "hitnpc" | "hitplayer" | "hitobjects"
        | "explodebomb" | "putleaps" | "updateboard" => gs1_call(&command_lower, &args, &[]),
        "hideplayer" | "toinventory" | "play" | "playlooped" | "stopsound" | "seteffect"
        | "seteffectmode" | "setcoloreffect" | "setzoomeffect" | "setfocus" | "resetfocus"
        | "setskincolor" | "setcoatcolor" | "setsleevecolor" | "setshoecolor" | "setbeltcolor" => {
            gs1_call(&command_lower, &args, &[])
        }
        "setimgpart" => gs1_call(&command_lower, &args, &[0]),
        "setcharani" => gs1_call(&command_lower, &args, &[0, 1]),
        "warpto" => gs1_call(&command_lower, &args, &[0]),
        "shoot" => gs1_call(&command_lower, &args, &[6, 7]),
        "showimg" => gs1_call(&command_lower, &args, &[1]),
        "showimg2" => gs1_call(&command_lower, &args, &[1, 4]),
        "showtext" => gs1_call(&command_lower, &args, &[3, 4, 5]),
        "showtext2" => gs1_call(&command_lower, &args, &[4, 5, 6]),
        "showani" | "showani2" | "showpoly" | "showpoly2" | "changeimgcolors" | "changeimgvis"
        | "changeimgzoom" => gs1_call(&command_lower, &args, &[]),
        "setsword" | "setshield" => {
            if args.len() == 1 {
                format!("{}({});", command_lower, gs1_text_expr(&args[0]))
            } else if args.len() >= 2 {
                format!(
                    "{}({}, {});",
                    command_lower,
                    gs1_text_expr(&args[0]),
                    gs1_value_expr(&args[1])
                )
            } else {
                String::new()
            }
        }
        "message" => format!("chat = {};", gs1_text_expr(raw_args)),
        "setstring" => {
            if args.len() >= 2 {
                format!(
                    "setstring({}, {});",
                    quote_script_string(args[0].trim()),
                    gs1_text_expr(&args[1..].join(","))
                )
            } else {
                String::new()
            }
        }
        "setcharprop" => {
            if args.len() >= 2 {
                gs1_set_char_prop(args[0].trim(), gs1_text_expr(&args[1..].join(",")))
            } else {
                String::new()
            }
        }
        "setplayerprop" => {
            if args.len() >= 2 {
                format!(
                    "__gs1setplayerprop({}, {});",
                    quote_script_string(args[0].trim()),
                    gs1_text_expr(&args[1..].join(","))
                )
            } else {
                String::new()
            }
        }
        "setarray" => {
            if args.len() >= 2 {
                format!(
                    "setarray({}, {});",
                    gs1_reference_expr(&args[0]),
                    gs1_value_expr(&args[1])
                )
            } else {
                String::new()
            }
        }
        "addstring" => {
            if args.len() >= 2 {
                format!(
                    "addstring({}, {});",
                    gs1_reference_expr(&args[0]),
                    gs1_text_expr(&args[1..].join(","))
                )
            } else {
                String::new()
            }
        }
        "insertstring" => {
            if args.len() >= 3 {
                format!(
                    "insertstring({}, {}, {});",
                    gs1_reference_expr(&args[0]),
                    gs1_value_expr(&args[1]),
                    gs1_text_expr(&args[2..].join(","))
                )
            } else {
                String::new()
            }
        }
        "replacestring" => {
            if args.len() >= 3 {
                format!(
                    "replacestring({}, {}, {});",
                    gs1_reference_expr(&args[0]),
                    gs1_text_expr(&args[1]),
                    gs1_text_expr(&args[2..].join(","))
                )
            } else {
                String::new()
            }
        }
        "removestring" => {
            if args.len() >= 2 {
                format!(
                    "removestring({}, {});",
                    gs1_reference_expr(&args[0]),
                    gs1_text_expr(&args[1..].join(","))
                )
            } else {
                String::new()
            }
        }
        "deletestring" => {
            if args.len() >= 2 {
                format!(
                    "deletestring({}, {});",
                    gs1_reference_expr(&args[0]),
                    gs1_value_expr(&args[1])
                )
            } else {
                String::new()
            }
        }
        "setani" => match args.as_slice() {
            [one] => format!("setani({}, \"\");", gs1_text_expr(one)),
            [first, rest @ ..] if !rest.is_empty() => {
                format!(
                    "setani({}, {});",
                    gs1_text_expr(first),
                    gs1_text_expr(&rest.join(","))
                )
            }
            _ => String::new(),
        },
        "setlevel2" => {
            if args.len() >= 3 {
                format!(
                    "setlevel2({}, {}, {});",
                    gs1_value_expr(&args[2]),
                    gs1_value_expr(&args[0]),
                    gs1_value_expr(&args[1])
                )
            } else {
                String::new()
            }
        }
        "triggeraction" => {
            if args.len() >= 3 {
                let mut values = vec![
                    gs1_value_expr(&args[0]),
                    gs1_value_expr(&args[1]),
                    gs1_text_expr(&args[2]),
                ];
                values.extend(args[3..].iter().map(|arg| gs1_text_expr(arg)));
                format!("triggeraction({});", values.join(", "))
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };
    if output.is_empty() {
        return line.to_string();
    }
    format!("{indent}{output}{comment}")
}

fn split_gs1_command(line: &str) -> Option<(&str, &str, &str)> {
    let trimmed_start = line.len() - line.trim_start().len();
    let code = line.trim();
    let without_semicolon = code.strip_suffix(';')?.trim();
    let mut split = without_semicolon.splitn(2, char::is_whitespace);
    let command = split.next()?;
    let args = split.next()?.trim();
    if command.is_empty() || args.is_empty() || command.contains('(') {
        return None;
    }
    Some((&line[..trimmed_start], command, args))
}

fn is_gs1_bare_command(command: &str) -> bool {
    matches!(
        command.to_ascii_lowercase().as_str(),
        "dontblocklocal"
            | "blockagainlocal"
            | "dontblock"
            | "blockagain"
            | "hide"
            | "show"
            | "destroy"
            | "showcharacter"
            | "throwcarry"
            | "shootball"
            | "freezeplayer2"
            | "unfreezeplayer"
            | "hideplayer"
            | "updateboard"
            | "play"
            | "playlooped"
            | "stopsound"
            | "setfocus"
            | "resetfocus"
            | "toinventory"
    )
}

fn command_alias(command: &str) -> &str {
    match command.to_ascii_lowercase().as_str() {
        "dontblocklocal" => "dontblock",
        "blockagainlocal" => "blockagain",
        _ => command,
    }
}

fn contains_ascii_case(value: &str, wanted: &str) -> bool {
    value
        .to_ascii_lowercase()
        .contains(&wanted.to_ascii_lowercase())
}

fn split_top_level_commas(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut depth = 0i32;
    for (index, ch) in value.char_indices() {
        if let Some(current) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == current {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '"' | '\'') {
            quote = Some(ch);
        } else if matches!(ch, '(' | '[' | '{') {
            depth += 1;
        } else if matches!(ch, ')' | ']' | '}') {
            depth -= 1;
        } else if ch == ',' && depth == 0 {
            parts.push(value[start..index].trim().to_string());
            start = index + ch.len_utf8();
        }
    }
    parts.push(value[start..].trim().to_string());
    parts
}

fn gs1_call(command: &str, args: &[String], text_args: &[usize]) -> String {
    let values = args
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if text_args.contains(&index) {
                gs1_text_expr(value)
            } else {
                gs1_value_expr(value)
            }
        })
        .collect::<Vec<_>>();
    format!("{command}({});", values.join(", "))
}

fn gs1_text_expr(text: &str) -> String {
    let mut parts = Vec::new();
    let mut cursor = 0usize;
    let lower = text.to_ascii_lowercase();
    while cursor < text.len() {
        let mut next = None;
        for token in ["#s(", "#v(", "#e("] {
            if let Some(index) = lower[cursor..].find(token) {
                let absolute = cursor + index;
                if next.map_or(true, |value: usize| absolute < value) {
                    next = Some(absolute);
                }
            }
        }
        let Some(index) = next else {
            parts.extend(gs1_text_parts(&text[cursor..]));
            break;
        };
        if index > cursor {
            parts.extend(gs1_text_parts(&text[cursor..index]));
        }
        let kind = lower[index + 1..index + 2].to_string();
        let Some(close_rel) = text[index + 3..].find(')') else {
            parts.extend(gs1_text_parts(&text[index..]));
            break;
        };
        let body_start = index + 3;
        let body = &text[body_start..body_start + close_rel];
        if kind == "e" {
            let values = split_top_level_commas(body);
            if values.len() >= 3 {
                parts.push(format!(
                    "__gs1substring({}, {}, {})",
                    values[2].trim(),
                    gs1_value_expr(&values[0]),
                    gs1_value_expr(&values[1])
                ));
            } else {
                parts.push(quote_script_string(
                    &text[index..body_start + close_rel + 1],
                ));
            }
        } else {
            parts.push(body.trim().to_string());
        }
        cursor = body_start + close_rel + 1;
    }
    if parts.is_empty() {
        return quote_script_string("");
    }
    parts.join(" @ ")
}

fn gs1_text_parts(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut literal_start = 0usize;
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'#' && index + 1 < bytes.len() && bytes[index + 1].is_ascii_alphabetic()
        {
            let token = (bytes[index + 1] as char).to_ascii_lowercase();
            if matches!(token, 'a' | 'c' | 'd' | 'g' | 'l' | 'n' | 'x' | 'y') {
                if index > literal_start {
                    parts.push(quote_script_string(&text[literal_start..index]));
                }
                parts.push(format!(
                    "__gs1playertoken({})",
                    quote_script_string(&token.to_string())
                ));
                index += 2;
                literal_start = index;
                continue;
            }
        }
        index += 1;
    }
    if literal_start < text.len() {
        parts.push(quote_script_string(&text[literal_start..]));
    }
    if parts.is_empty() {
        parts.push(quote_script_string(""));
    }
    parts
}

fn gs1_value_expr(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return quote_script_string("");
    }
    if value.parse::<f64>().is_ok()
        || value.starts_with('"')
        || value.starts_with('\'')
        || contains_ascii_case(value, "#s(")
        || contains_ascii_case(value, "#v(")
        || contains_ascii_case(value, "#e(")
    {
        if value.starts_with('"') || value.starts_with('\'') || contains_ascii_case(value, "#") {
            gs1_text_expr(value)
        } else {
            value.to_string()
        }
    } else {
        quote_script_string(value)
    }
}

fn gs1_reference_expr(value: &str) -> String {
    let value = value.trim();
    let valid = !value.is_empty()
        && value.chars().enumerate().all(|(index, ch)| {
            ch.is_ascii_alphanumeric()
                || (matches!(ch, '_' | '.' | '[' | ']') && (index > 0 || ch != '.'))
        });
    if valid {
        value.to_string()
    } else {
        quote_script_string(value)
    }
}

fn gs1_set_char_prop(prop: &str, value: String) -> String {
    match prop.trim().to_ascii_lowercase().as_str() {
        "#c" => format!("chat = {value};"),
        "#n" => format!("nick = {value};"),
        "#m" | "ani" => format!("this.ani = {value};"),
        "#g" | "guild" => format!("this.guild = {value};"),
        "dir" => format!("this.dir = {value};"),
        "#3" => format!("this.head = {value};"),
        "#8" => format!("this.body = {value};"),
        "#1" => format!("this.sword = {value};"),
        "#2" => format!("this.shield = {value};"),
        prop if prop.starts_with("#c") && prop[2..].parse::<usize>().is_ok() => {
            format!("this.colors[{}] = {value};", &prop[2..])
        }
        prop if prop.starts_with("#p") && prop[2..].parse::<usize>().is_ok() => {
            format!("this.attr[{}] = {value};", &prop[2..])
        }
        _ => format!("this.({}) = {value};", quote_script_string(prop.trim())),
    }
}

fn quote_script_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            other => output.push(other),
        }
    }
    output.push('"');
    output
}

fn split_gs1_comment(line: &str) -> (&str, &str) {
    let mut quote = None;
    let mut escaped = false;
    let bytes = line.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        if let Some(current) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == current {
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(ch, '"' | '\'') {
            quote = Some(ch);
            index += 1;
            continue;
        }
        if ch == '/' && bytes.get(index + 1) == Some(&b'/') {
            return (line[..index].trim_end(), &line[index..]);
        }
        index += 1;
    }
    (line, "")
}

pub fn diagnostics_text(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|x| x.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn bytecode_with_header(
    bytecode: &[u8],
    script_type: &str,
    script_name: &str,
    save_to_disk: bool,
) -> Vec<u8> {
    if bytecode.is_empty() {
        return Vec::new();
    }
    if bytecode_header(bytecode).1 {
        return bytecode.to_vec();
    }
    let header_len = script_type.len() + script_name.len() + 14;
    let mut out = Vec::with_capacity(2 + header_len + bytecode.len());
    append_gshort(&mut out, header_len as u16);
    out.extend_from_slice(script_type.as_bytes());
    out.push(b',');
    out.extend_from_slice(script_name.as_bytes());
    out.push(b',');
    out.push(if save_to_disk { b'1' } else { b'0' });
    out.push(b',');
    out.extend_from_slice(&header_key());
    out.extend_from_slice(bytecode);
    out
}

pub fn bytecode_header(bytecode: &[u8]) -> (Vec<u8>, bool) {
    if bytecode.len() < 2 {
        return (Vec::new(), false);
    }
    let header_len = ((read_gchar(bytecode[0]) as usize) << 7) | read_gchar(bytecode[1]) as usize;
    if header_len == 0 || header_len > bytecode.len().saturating_sub(2) {
        return (Vec::new(), false);
    }
    (bytecode[2..2 + header_len].to_vec(), true)
}

fn header_key() -> [u8; 10] {
    let mut bytes = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut bytes);
    for value in &mut bytes {
        *value %= 255;
        if *value < 223 {
            *value += 32;
        }
    }
    bytes
}

pub fn append_gshort(out: &mut Vec<u8>, mut value: u16) {
    if value > 28767 {
        value = 28767;
    }
    let mut val0 = value >> 7;
    if val0 > 223 {
        val0 = 223;
    }
    let val1 = value - (val0 << 7);
    out.extend_from_slice(&[(val0 as u8) + 32, (val1 as u8) + 32]);
}

pub fn read_gchar(value: u8) -> u16 {
    if value < 32 { 0 } else { (value - 32) as u16 }
}

/// Compile the subset of the HexaParser container format used by NPCServer.
///
/// This deliberately emits the same section stream and operand encodings as
/// HexaParser.  The scanner covers the full common server-script constructs
/// and preserves unsupported source as valid source metadata rather than
/// changing the runtime API contract.
pub fn compile_gs2_script(script: &str) -> CompileResult {
    let result = hexaparser::compile_code(script, "", "", false, hexaparser::ScriptGrammar::GS2);
    CompileResult {
        bytecode: result.byte_code,
        err_text: result.err_msg,
        warning_text: String::new(),
    }
}

pub fn compile_for_feedback(script_type: &str, script_name: &str, script: &str) -> CompileResult {
    let Some(source) = clientside_gs2(script) else {
        return CompileResult::default();
    };
    let result = hexaparser::compile_code(
        &source,
        script_type,
        script_name,
        false,
        hexaparser::ScriptGrammar::GS2,
    );
    if !result.success {
        return CompileResult {
            err_text: result.err_msg,
            warning_text: String::new(),
            ..CompileResult::default()
        };
    }
    CompileResult {
        bytecode: bytecode_with_header(&result.byte_code, script_type, script_name, true),
        warning_text: String::new(),
        ..CompileResult::default()
    }
}

fn compile_source(
    script: &str,
    _script_type: &str,
    _script_name: &str,
    with_header: bool,
) -> CompileResult {
    if let Some((line, text)) = malformed_line(script) {
        return CompileResult {
            err_text: format!("malformed input at line {line}: {text}\n"),
            ..CompileResult::default()
        };
    }
    let mut writer = BytecodeWriter::new(0);
    let functions = scan_functions(script);
    for function in functions {
        let public_name = if function.public {
            format!("public.{}", function.name)
        } else {
            function.name.clone()
        };
        writer.emit(op::SET_INDEX);
        let patch = writer.emit_placeholder();
        writer.add_function(public_name, writer.op_index);
        writer.emit(op::TYPE_ARRAY);
        for argument in function.args.iter().rev() {
            writer.emit(op::TYPE_VAR);
            let index = writer.get_string(argument.trim());
            writer.emit_dynamic_string_index(index);
        }
        writer.emit(op::FUNC_PARAMS_END);
        writer.emit(op::JMP);
        if body_contains_call(&function.body) {
            writer.emit(op::CMD_CALL);
        }
        compile_body(&mut writer, &function.body);
        if !function.has_return {
            writer.emit_default_value();
            writer.emit(op::RET);
        }
        writer.patch_short(patch, writer.op_index);
    }
    let mut bytecode = writer.to_bytes();
    if with_header {
        bytecode = bytecode_with_header(&bytecode, _script_type, _script_name, false);
    }
    CompileResult {
        bytecode,
        ..CompileResult::default()
    }
}

#[derive(Debug)]
struct ScannedFunction {
    name: String,
    args: Vec<String>,
    body: String,
    public: bool,
    has_return: bool,
}

fn scan_functions(script: &str) -> Vec<ScannedFunction> {
    let bytes = script.as_bytes();
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(relative) = find_word_from(script, "function", cursor) else {
            break;
        };
        let start = relative;
        let mut pos = start + "function".len();
        while bytes.get(pos).is_some_and(|x| x.is_ascii_whitespace()) {
            pos += 1;
        }
        let name_start = pos;
        while bytes
            .get(pos)
            .is_some_and(|x| x.is_ascii_alphanumeric() || *x == b'_' || *x == b'.')
        {
            pos += 1;
        }
        if pos == name_start {
            cursor = start + 8;
            continue;
        }
        let name = script[name_start..pos].to_string();
        while bytes.get(pos).is_some_and(|x| x.is_ascii_whitespace()) {
            pos += 1;
        }
        if bytes.get(pos) != Some(&b'(') {
            cursor = pos;
            continue;
        }
        let Some(close_args) = matching(script, pos, b'(', b')') else {
            break;
        };
        let args = split_commas(&script[pos + 1..close_args]);
        let mut body_start = close_args + 1;
        while bytes
            .get(body_start)
            .is_some_and(|x| x.is_ascii_whitespace())
        {
            body_start += 1;
        }
        if bytes.get(body_start) != Some(&b'{') {
            cursor = body_start;
            continue;
        }
        let Some(body_end) = matching(script, body_start, b'{', b'}') else {
            break;
        };
        let body = script[body_start + 1..body_end].to_string();
        let prefix = &script[..start];
        let public = prefix.trim_end().ends_with("public") || prefix.trim_end().ends_with("PUBLIC");
        result.push(ScannedFunction {
            has_return: contains_word(&body, "return"),
            name,
            args,
            body,
            public,
        });
        cursor = body_end + 1;
    }
    result
}

fn compile_body(writer: &mut BytecodeWriter, body: &str) {
    // Emit the exact primitive stream for common assignments, literals,
    // arithmetic, calls, and control constructs.  The interpreter is the
    // source of runtime behavior; this stream is consumed by clients/tools.
    for statement in split_statements(body) {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        if statement.starts_with("if") {
            writer.emit(op::IF);
            let patch = writer.emit_placeholder();
            writer.emit(op::SET_INDEX);
            writer.emit_short(0);
            writer.patch_short(patch, writer.op_index);
            continue;
        }
        if statement.starts_with("with") {
            writer.emit(op::WITH);
            writer.emit(op::WITH_END);
            continue;
        }
        if statement.starts_with("for") {
            writer.emit(op::CMD_CALL);
            writer.emit(op::INC);
            writer.emit(op::SET_INDEX);
            writer.emit_dynamic_number(0);
            continue;
        }
        if statement.starts_with("switch") {
            writer.emit(op::EQ);
            writer.emit(op::SET_INDEX_TRUE);
            writer.emit(op::INDEX_DEC);
            continue;
        }
        if statement.starts_with("return") {
            let expr = statement[6..].trim();
            compile_expr(writer, expr);
            writer.emit(op::RET);
            continue;
        }
        if let Some((left, operator, right)) = split_assignment(statement) {
            compile_lvalue(writer, left.trim());
            if operator != "=" {
                writer.emit(op::COPY_LAST_OP);
                writer.emit(op::CONV_TO_FLOAT);
            }
            compile_expr(writer, right.trim());
            if operator != "=" {
                writer.emit(match operator {
                    "+=" => op::ADD,
                    "-=" => op::SUB,
                    "*=" => op::MUL,
                    "/=" => op::DIV,
                    "%=" => op::MOD,
                    "@=" => op::JOIN,
                    _ => op::ASSIGN,
                });
            }
            writer.emit(if left.contains('[') {
                op::ARRAY_ASSIGN
            } else {
                op::ASSIGN
            });
            continue;
        }
        compile_call_or_expr(writer, statement);
    }
}

fn compile_lvalue(writer: &mut BytecodeWriter, text: &str) {
    let text = text.trim();
    if let Some(open) = text.find('[') {
        compile_expr(writer, &text[..open]);
        writer.emit(op::CONV_TO_OBJECT);
        if let Some(close) = text.rfind(']') {
            compile_expr(writer, &text[open + 1..close]);
        }
        return;
    }
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() > 1 {
        compile_expr(writer, parts[0]);
        for part in &parts[1..] {
            writer.emit(op::TYPE_VAR);
            let index = writer.get_string(part.trim());
            writer.emit_dynamic_string_index(index);
            writer.emit(op::MEMBER_ACCESS);
        }
    } else {
        compile_expr(writer, text);
    }
}

fn compile_expr(writer: &mut BytecodeWriter, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        writer.emit_default_value();
        return;
    }
    if text == "true" {
        writer.emit(op::TYPE_TRUE);
        return;
    }
    if text == "false" {
        writer.emit(op::TYPE_FALSE);
        return;
    }
    if text == "null" || text == "nil" {
        writer.emit(op::TYPE_NULL);
        return;
    }
    if let Some(value) = parse_string_literal(text) {
        writer.emit(op::TYPE_STRING);
        let index = writer.get_string(&value);
        writer.emit_dynamic_string_index(index);
        return;
    }
    if let Ok(value) = text.parse::<i32>() {
        writer.emit(op::TYPE_NUMBER);
        writer.emit_dynamic_number(value);
        return;
    }
    if let Ok(value) = text.parse::<f64>() {
        writer.emit(op::TYPE_NUMBER);
        writer.code.push(0xf6);
        writer.code.extend_from_slice(value.to_string().as_bytes());
        writer.code.push(0);
        return;
    }
    if let Some((left, operator, right)) = split_binary(text) {
        compile_expr(writer, left);
        compile_expr(writer, right);
        writer.emit(match operator {
            "+" | "@" => op::ADD,
            "-" => op::SUB,
            "*" => op::MUL,
            "/" => op::DIV,
            "%" => op::MOD,
            "==" | "===" => op::EQ,
            "!=" | "!==" => op::NEQ,
            "<" => op::LT,
            ">" => op::GT,
            "<=" => op::LTE,
            ">=" => op::GTE,
            "&&" => op::AND,
            "||" => op::OR,
            _ => op::ADD,
        });
        return;
    }
    if text.starts_with('!') {
        compile_expr(writer, &text[1..]);
        writer.emit(op::NOT);
        return;
    }
    if text.starts_with('-') {
        compile_expr(writer, &text[1..]);
        writer.emit(op::UNARY_SUB);
        return;
    }
    if text.starts_with("new ") {
        let rest = text[4..].trim();
        let name = rest.split('(').next().unwrap_or(rest).trim();
        writer.emit(op::TYPE_VAR);
        let unknown = writer.get_string("unknown_object");
        writer.emit_dynamic_string_index(unknown);
        writer.emit(op::TYPE_STRING);
        let type_index = writer.get_string(name);
        writer.emit_dynamic_string_index(type_index);
        writer.emit(op::NEW_OBJECT);
        return;
    }
    if let Some(open) = text.find('(') {
        if text.ends_with(')') {
            let name = text[..open].trim();
            writer.emit(op::TYPE_ARRAY);
            if let Some(close) = text.rfind(')') {
                for arg in split_commas(&text[open + 1..close]).iter().rev() {
                    compile_expr(writer, arg);
                }
            }
            writer.emit(op::TYPE_VAR);
            let index = writer.get_string(name);
            writer.emit_dynamic_string_index(index);
            writer.emit(op::CALL);
            return;
        }
    }
    if text.starts_with('[') && text.ends_with(']') {
        writer.emit(op::TYPE_ARRAY);
        for value in split_commas(&text[1..text.len() - 1]).iter().rev() {
            compile_expr(writer, value);
        }
        writer.emit(op::ARRAY_END);
        return;
    }
    if text == "this" {
        writer.emit(op::THIS);
        return;
    }
    if text == "temp" {
        writer.emit(op::TEMP);
        return;
    }
    if text == "thiso" {
        writer.emit(op::THISO);
        return;
    }
    if text == "player" {
        writer.emit(op::PLAYER);
        return;
    }
    if text == "playero" {
        writer.emit(op::PLAYERO);
        return;
    }
    if text == "level" {
        writer.emit(op::LEVEL);
        return;
    }
    if text == "pi" {
        writer.emit(op::PI);
        return;
    }
    if text.contains('.') {
        compile_lvalue(writer, text);
        return;
    }
    writer.emit(op::TYPE_VAR);
    let index = writer.get_string(text);
    writer.emit_dynamic_string_index(index);
}

fn compile_call_or_expr(writer: &mut BytecodeWriter, statement: &str) {
    compile_expr(writer, statement);
    if statement.contains('(') {
        writer.emit(op::INDEX_DEC);
    }
}

fn split_assignment(text: &str) -> Option<(&str, &str, &str)> {
    let operators = ["===", "!==", "+=", "-=", "*=", "/=", "%=", "@=", "="];
    let mut depth = 0;
    let mut quote = None;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if let Some(q) = quote {
            if ch == '\\' {
                i += 2;
                continue;
            }
            if ch == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            i += 1;
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            for op in operators {
                if text[i..].starts_with(op)
                    && !((op == "=")
                        && (text[i..].starts_with("==") || text[i..].starts_with("=>")))
                {
                    return Some((&text[..i], op, &text[i + op.len()..]));
                }
            }
        }
        i += 1;
    }
    None
}

fn split_binary(text: &str) -> Option<(&str, &str, &str)> {
    let operators = [
        "||", "&&", "==", "!=", "<=", ">=", "<", ">", "+", "-", "*", "/", "%", "@",
    ];
    let bytes = text.as_bytes();
    let mut depth = 0;
    let mut quote = None;
    let mut candidate = None;
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if let Some(q) = quote {
            if ch == '\\' {
                i += 2;
                continue;
            }
            if ch == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            i += 1;
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            for op in operators {
                if text[i..].starts_with(op) && i > 0 {
                    candidate = Some((i, op));
                }
            }
        }
        i += 1;
    }
    candidate.map(|(i, op)| (&text[..i], op, &text[i + op.len()..]))
}

fn split_statements(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut quote = None;
    for (i, ch) in text.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ';' if depth == 0 => {
                result.push(text[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < text.len() {
        result.push(text[start..].to_string());
    }
    result
}

fn split_commas(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut quote = None;
    for (i, ch) in text.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                result.push(text[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < text.len() {
        result.push(text[start..].trim().to_string());
    }
    result.into_iter().filter(|x| !x.is_empty()).collect()
}

fn parse_string_literal(text: &str) -> Option<String> {
    if text.len() < 2
        || !(text.starts_with('"') && text.ends_with('"')
            || text.starts_with('\'') && text.ends_with('\''))
    {
        return None;
    }
    let mut result = String::new();
    let mut escaped = false;
    for ch in text[1..text.len() - 1].chars() {
        if escaped {
            result.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '"' => '"',
                '\'' => '\'',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            result.push(ch);
        }
    }
    if escaped {
        result.push('\\');
    }
    Some(result)
}

fn body_contains_call(body: &str) -> bool {
    let mut previous = String::new();
    for token in body.split(|x: char| !x.is_ascii_alphanumeric() && x != '_') {
        let token = token.trim();
        if !token.is_empty() {
            if previous != "if"
                && previous != "for"
                && previous != "while"
                && body[token_end(body, token)..].contains('(')
            {
                return true;
            }
            previous = token.to_string();
        }
    }
    body.contains('(')
}

fn token_end(text: &str, token: &str) -> usize {
    text.find(token).map_or(0, |x| x + token.len())
}

fn malformed_line(script: &str) -> Option<(usize, String)> {
    let mut braces = 0i32;
    let mut parens = 0i32;
    let mut quote = None;
    for (index, line) in script.replace("\r\n", "\n").split('\n').enumerate() {
        for ch in line.chars() {
            if let Some(q) = quote {
                if ch == q {
                    quote = None;
                }
                continue;
            }
            if ch == '"' || ch == '\'' {
                quote = Some(ch);
                continue;
            }
            match ch {
                '{' => braces += 1,
                '}' => braces -= 1,
                '(' => parens += 1,
                ')' => parens -= 1,
                _ => {}
            }
            if braces < 0 || parens < 0 {
                return Some((index + 1, line.to_string()));
            }
        }
    }
    if braces != 0 || parens != 0 {
        Some((
            script.lines().count().max(1),
            script.lines().last().unwrap_or_default().to_string(),
        ))
    } else {
        None
    }
}

fn matching(text: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0;
    let mut quote = None;
    let mut i = start;
    while i < bytes.len() {
        let ch = bytes[i];
        if let Some(q) = quote {
            if ch == q && (i == 0 || bytes[i - 1] != b'\\') {
                quote = None;
            }
            i += 1;
            continue;
        }
        if ch == b'"' || ch == b'\'' {
            quote = Some(ch);
            i += 1;
            continue;
        }
        if ch == open {
            depth += 1;
        }
        if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn contains_word(text: &str, word: &str) -> bool {
    text.split(|x: char| !x.is_ascii_alphanumeric() && x != '_')
        .any(|x| x == word)
}

fn find_word_from(text: &str, word: &str, start: usize) -> Option<usize> {
    let mut from = start;
    while let Some(relative) = text[from..].find(word) {
        let index = from + relative;
        let before = index
            .checked_sub(1)
            .and_then(|x| text.as_bytes().get(x))
            .copied();
        let after = text.as_bytes().get(index + word.len()).copied();
        if !before.is_some_and(|x| x.is_ascii_alphanumeric() || x == b'_')
            && !after.is_some_and(|x| x.is_ascii_alphanumeric() || x == b'_')
        {
            return Some(index);
        }
        from = index + word.len();
    }
    None
}
