use crate::compiler::BytecodeSegment;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

const OP_NONE: u8 = 0x00;
const OP_JMP: u8 = 0x01;
const OP_JEQ: u8 = 0x02;
const OP_SHORT_OR: u8 = 0x03;
const OP_JNE: u8 = 0x04;
const OP_SHORT_AND: u8 = 0x05;
const OP_CALL: u8 = 0x06;
const OP_RET: u8 = 0x07;
const OP_SLEEP: u8 = 0x08;
const OP_LOOP_COUNTER: u8 = 0x09;
const OP_FUNCTION_START: u8 = 0x0a;
const OP_WAIT_FOR: u8 = 0x0b;
const OP_PUSH_NUMBER: u8 = 0x14;
const OP_PUSH_STRING: u8 = 0x15;
const OP_PUSH_VARIABLE: u8 = 0x16;
const OP_PUSH_ARRAY: u8 = 0x17;
const OP_PUSH_TRUE: u8 = 0x18;
const OP_PUSH_FALSE: u8 = 0x19;
const OP_PUSH_NULL: u8 = 0x1a;
const OP_PI: u8 = 0x1b;
const OP_COPY: u8 = 0x1e;
const OP_SWAP: u8 = 0x1f;
const OP_POP: u8 = 0x20;
const OP_CONVERT_FLOAT: u8 = 0x21;
const OP_CONVERT_STRING: u8 = 0x22;
const OP_ACCESS_MEMBER: u8 = 0x23;
const OP_CONVERT_OBJECT: u8 = 0x24;
const OP_END_ARRAY: u8 = 0x25;
const OP_NEW_UNINIT_ARRAY: u8 = 0x26;
const OP_SET_ARRAY: u8 = 0x27;
const OP_NEW: u8 = 0x28;
const OP_MAKE_VAR: u8 = 0x29;
const OP_NEW_OBJECT: u8 = 0x2a;
const OP_CONVERT_VAR: u8 = 0x2b;
const OP_SHORT_END: u8 = 0x2c;
const OP_SET_REGISTER: u8 = 0x2d;
const OP_GET_REGISTER: u8 = 0x2e;
const OP_MARK_REGISTER_VAR: u8 = 0x2f;
const OP_ASSIGN: u8 = 0x32;
const OP_END_PARAMS: u8 = 0x33;
const OP_INC: u8 = 0x34;
const OP_DEC: u8 = 0x35;
const OP_ASSIGN_MEMBER: u8 = 0x36;
const OP_ADD: u8 = 0x3c;
const OP_SUBTRACT: u8 = 0x3d;
const OP_MULTIPLY: u8 = 0x3e;
const OP_DIVIDE: u8 = 0x3f;
const OP_MODULO: u8 = 0x40;
const OP_POWER: u8 = 0x41;
const OP_BOOL_AND: u8 = 0x42;
const OP_BOOL_OR: u8 = 0x43;
const OP_LOGICAL_NOT: u8 = 0x44;
const OP_UNARY_SUBTRACT: u8 = 0x45;
const OP_EQUAL: u8 = 0x46;
const OP_NOT_EQUAL: u8 = 0x47;
const OP_LESS_THAN: u8 = 0x48;
const OP_GREATER_THAN: u8 = 0x49;
const OP_LE: u8 = 0x4a;
const OP_GE: u8 = 0x4b;
const OP_BITWISE_OR: u8 = 0x4c;
const OP_BITWISE_AND: u8 = 0x4d;
const OP_BITWISE_XOR: u8 = 0x4e;
const OP_BITWISE_INVERT: u8 = 0x4f;
const OP_IN_RANGE: u8 = 0x50;
const OP_IN: u8 = 0x51;
const OP_OBJ_INDEX: u8 = 0x52;
const OP_OBJ_TYPE: u8 = 0x53;
const OP_FORMAT: u8 = 0x54;
const OP_INT: u8 = 0x55;
const OP_ABS: u8 = 0x56;
const OP_RANDOM: u8 = 0x57;
const OP_SIN: u8 = 0x58;
const OP_COS: u8 = 0x59;
const OP_ARCTAN: u8 = 0x5a;
const OP_EXP: u8 = 0x5b;
const OP_LOG: u8 = 0x5c;
const OP_MIN: u8 = 0x5d;
const OP_MAX: u8 = 0x5e;
const OP_GET_ANGLE: u8 = 0x5f;
const OP_GET_DIR: u8 = 0x60;
const OP_VEC_X: u8 = 0x61;
const OP_VEC_Y: u8 = 0x62;
const OP_OBJ_INDICES: u8 = 0x63;
const OP_OBJ_LINK: u8 = 0x64;
const OP_SHIFT_LEFT: u8 = 0x65;
const OP_SHIFT_RIGHT: u8 = 0x66;
const OP_CHAR: u8 = 0x67;
const OP_OBJ_COMPARE: u8 = 0x68;
const OP_OBJ_TRIM: u8 = 0x6e;
const OP_OBJ_LENGTH: u8 = 0x6f;
const OP_OBJ_POS: u8 = 0x70;
const OP_JOIN: u8 = 0x71;
const OP_OBJ_CHAR_AT: u8 = 0x72;
const OP_OBJ_SUBSTRING: u8 = 0x73;
const OP_OBJ_STARTS: u8 = 0x74;
const OP_OBJ_ENDS: u8 = 0x75;
const OP_OBJ_TOKENIZE: u8 = 0x76;
const OP_TRANSLATION: u8 = 0x77;
const OP_OBJ_POSITIONS: u8 = 0x78;
const OP_APPEND: u8 = 0x79;
const OP_OBJ_SIZE: u8 = 0x82;
const OP_ARRAY_ACCESS: u8 = 0x83;
const OP_ASSIGN_ARRAY: u8 = 0x84;
const OP_MULTI_DIM_ARRAY: u8 = 0x85;
const OP_ASSIGN_MULTI_DIM_ARRAY: u8 = 0x86;
const OP_OBJ_SUBARRAY: u8 = 0x87;
const OP_OBJ_ADD_STRING: u8 = 0x88;
const OP_OBJ_DELETE_STRING: u8 = 0x89;
const OP_OBJ_REMOVE_STRING: u8 = 0x8a;
const OP_OBJ_REPLACE_STRING: u8 = 0x8b;
const OP_OBJ_INSERT_STRING: u8 = 0x8c;
const OP_OBJ_CLEAR: u8 = 0x8d;
const OP_NEW_MULTI_DIM_ARRAY: u8 = 0x8e;
const OP_WITH: u8 = 0x96;
const OP_WITH_END: u8 = 0x97;
const OP_FOR_EACH: u8 = 0xa3;
const OP_THIS: u8 = 0xb4;
const OP_THIS_O: u8 = 0xb5;
const OP_PLAYER: u8 = 0xb6;
const OP_PLAYER_O: u8 = 0xb7;
const OP_LEVEL: u8 = 0xb8;
const OP_TEMP: u8 = 0xbd;
const OP_PARAMS: u8 = 0xbe;
const OP_IMM_STRING_BYTE: u8 = 0xf0;
const OP_IMM_STRING_SHORT: u8 = 0xf1;
const OP_IMM_STRING_INT: u8 = 0xf2;
const OP_IMM_BYTE: u8 = 0xf3;
const OP_IMM_SHORT: u8 = 0xf4;
const OP_IMM_INT: u8 = 0xf5;
const OP_IMM_FLOAT: u8 = 0xf6;

#[derive(Clone, Debug, Default)]
struct Operand { str_value: String, number: i32, float_value: String, kind: String }

#[derive(Clone, Debug)]
struct Instruction { addr: usize, op: u8, operand: Option<Operand> }

#[derive(Clone, Debug, Default)]
struct Module { functions: Vec<FunctionDef>, strings: Vec<String>, code: Vec<Instruction> }

#[derive(Clone, Debug, Default)]
struct FunctionDef { name: String, addr: usize, body_start: usize, params: Vec<String> }

#[derive(Clone, Debug, Default)]
struct FunctionRange { function: FunctionDef, start: usize, end: usize }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DecompExpr { text: String, marker: bool, kind: String }

#[derive(Clone, Debug, Default)]
struct DecompileState { registers: HashMap<i32, DecompExpr>, skip: Vec<FunctionRange> }

#[derive(Clone, Debug)]
struct DispatchCase { condition: String, target: usize }

#[derive(Clone, Debug)]
struct ConditionalAssignmentCase { condition: String, value: String }

pub fn read_input(input_path: &str) -> Result<Vec<u8>, String> {
    let data = if input_path.is_empty() {
        let mut data = Vec::new();
        io::stdin().read_to_end(&mut data).map_err(|e| e.to_string())?;
        data
    } else { fs::read(input_path).map_err(|e| e.to_string())? };
    match parse_hex_bytes(&String::from_utf8_lossy(&data)) {
        Ok(parsed) => Ok(parsed),
        Err(_) => Ok(data),
    }
}

pub fn decompile_code(data: &[u8]) -> Result<String, String> {
    let module = parse_module(data)?;
    Ok(decompile_module(&module))
}

pub fn default_output_path(input_path: &str) -> String {
    let path = Path::new(input_path);
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("");
    if extension.eq_ignore_ascii_case("gs2bc") {
        let mut output = input_path.to_string();
        let suffix = format!(".{}", extension);
        output.truncate(output.len() - suffix.len());
        output.push_str(".gs2");
        output
    } else { format!("{}.gs2", input_path) }
}

pub fn parse_hex_bytes(value: &str) -> Result<Vec<u8>, String> {
    let mut digits = String::new();
    for character in value.chars() {
        if character == '\0' || character == '\u{feff}' || character == '\u{fffd}' || character.is_whitespace() { continue; }
        if !character.is_ascii_hexdigit() { return Err(format!("non-hex character {}", go_quote_char(character))); }
        digits.push(character);
    }
    if digits.is_empty() || digits.len() % 2 != 0 { return Err("hex input must contain an even number of digits".to_string()); }
    let mut result = Vec::with_capacity(digits.len() / 2);
    for index in (0..digits.len()).step_by(2) {
        result.push(u8::from_str_radix(&digits[index..index + 2], 16).map_err(|e| e.to_string())?);
    }
    Ok(result)
}

pub fn read_segments(value: &[u8]) -> Result<HashMap<BytecodeSegment, Vec<u8>>, String> {
    let mut segments = HashMap::new();
    let mut pos = 0usize;
    while pos < value.len() {
        if value.len() - pos == 1 && value[pos] == 10 { return Ok(segments); }
        if value.len() - pos < 8 { return Err(format!("truncated bytecode segment at {}", pos)); }
        let segment = i32::from_be_bytes(value[pos..pos + 4].try_into().unwrap());
        let length = i32::from_be_bytes(value[pos + 4..pos + 8].try_into().unwrap());
        pos += 8;
        if length < 0 || length as usize > value.len() - pos { return Err(format!("invalid bytecode segment length at {}", pos - 4)); }
        let bytes = value[pos..pos + length as usize].to_vec();
        let key = match segment { 1 => BytecodeSegment::Gs1EventFlags, 2 => BytecodeSegment::FunctionNames, 3 => BytecodeSegment::Strings, 4 => BytecodeSegment::Bytecode, _ => unsafe { std::mem::transmute::<i32, BytecodeSegment>(segment) } };
        segments.insert(key, bytes);
        pos += length as usize;
    }
    Err("bytecode trailer is missing".to_string())
}

fn parse_module(data: &[u8]) -> Result<Module, String> {
    let payload = bytecode_payload(data);
    let mut reader = ByteReader::new(payload);
    let mut module = Module::default();
    for _ in 0..4 {
        if reader.left() == 0 { break; }
        let section_type = reader.u32()?;
        match section_type {
            1 => { let length = reader.u32()? as usize; reader.skip(length)?; }
            2 => {
                let length = reader.u32()? as usize;
                let end = reader.pos.checked_add(length).ok_or_else(|| "invalid bytecode section length".to_string())?;
                if end > payload.len() { return Err("unexpected end of file".to_string()); }
                while reader.pos < end {
                    let addr = reader.u32()? as usize;
                    let name = reader.cstr()?;
                    module.functions.push(FunctionDef { name, addr, body_start: 0, params: Vec::new() });
                }
                if reader.pos != end { return Err("invalid function name section".to_string()); }
            }
            3 => {
                let length = reader.u32()? as usize;
                let end = reader.pos.checked_add(length).ok_or_else(|| "invalid bytecode section length".to_string())?;
                if end > payload.len() { return Err("unexpected end of file".to_string()); }
                while reader.pos < end { module.strings.push(reader.cstr()?); }
                if reader.pos != end { return Err("invalid string section".to_string()); }
            }
            4 => {
                let length = reader.u32()? as usize;
                let end = reader.pos.checked_add(length).ok_or_else(|| "invalid bytecode section length".to_string())?;
                if end > payload.len() { return Err("unexpected end of file".to_string()); }
                module.code = read_instructions(&payload[reader.pos..end], &module.strings)?;
                reader.pos = end;
            }
            value => return Err(format!("unknown section type {}", value)),
        }
    }
    discover_function_prologues(&mut module);
    Ok(module)
}

fn bytecode_payload(data: &[u8]) -> &[u8] {
    if valid_section_stream(data) { return data; }
    for offset in 1..data.len().saturating_sub(7) {
        if data[offset..offset + 4] == 1u32.to_be_bytes() && valid_section_stream(&data[offset..]) { return &data[offset..]; }
    }
    data
}

fn valid_section_stream(data: &[u8]) -> bool {
    let mut pos = 0usize;
    let mut seen_code = false;
    for _ in 0..4 {
        if pos + 8 > data.len() { return false; }
        let section_type = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap());
        let length = u32::from_be_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        if !(1..=4).contains(&section_type) || pos.checked_add(length).is_none_or(|end| end > data.len()) { return false; }
        if section_type == 4 { seen_code = true; }
        pos += length;
    }
    seen_code
}

fn discover_function_prologues(module: &mut Module) {
    for function in &mut module.functions {
        function.body_start = function.addr;
        if function.addr >= module.code.len() || module.code[function.addr].op != OP_PUSH_ARRAY { continue; }
        let mut params = Vec::new();
        let mut pos = function.addr + 1;
        while pos < module.code.len() && module.code[pos].op == OP_PUSH_VARIABLE {
            if let Some(operand) = &module.code[pos].operand { params.push(operand.str_value.clone()); }
            pos += 1;
        }
        if pos >= module.code.len() || module.code[pos].op != OP_END_PARAMS { continue; }
        params.reverse();
        function.params = params;
        function.body_start = pos + 1;
        if function.body_start < module.code.len() && module.code[function.body_start].op == OP_FUNCTION_START { function.body_start += 1; }
    }
}

fn read_instructions(data: &[u8], strings: &[String]) -> Result<Vec<Instruction>, String> {
    let mut reader = ByteReader::new(data);
    let mut code: Vec<Instruction> = Vec::new();
    while reader.left() > 0 {
        let op = reader.u8()?;
        if is_immediate(op) {
            if code.is_empty() { return Err(format!("immediate {:x} without instruction", op)); }
            let operand = read_immediate(&mut reader, op, strings)?;
            code.last_mut().unwrap().operand = Some(operand);
        } else {
            let addr = code.len();
            code.push(Instruction { addr, op, operand: None });
        }
    }
    Ok(code)
}

fn is_immediate(op: u8) -> bool { matches!(op, OP_IMM_STRING_BYTE | OP_IMM_STRING_SHORT | OP_IMM_STRING_INT | OP_IMM_BYTE | OP_IMM_SHORT | OP_IMM_INT | OP_IMM_FLOAT) }

fn read_immediate(reader: &mut ByteReader<'_>, op: u8, strings: &[String]) -> Result<Operand, String> {
    match op {
        OP_IMM_STRING_BYTE => string_operand(reader.u8()? as usize, strings),
        OP_IMM_STRING_SHORT => string_operand(reader.u16()? as usize, strings),
        OP_IMM_STRING_INT => string_operand(reader.u32()? as usize, strings),
        OP_IMM_BYTE => Ok(Operand { number: reader.u8()? as i8 as i32, kind: "number".to_string(), ..Operand::default() }),
        OP_IMM_SHORT => Ok(Operand { number: reader.u16()? as i16 as i32, kind: "number".to_string(), ..Operand::default() }),
        OP_IMM_INT => Ok(Operand { number: reader.u32()? as i32, kind: "number".to_string(), ..Operand::default() }),
        OP_IMM_FLOAT => Ok(Operand { float_value: reader.cstr()?, kind: "float".to_string(), ..Operand::default() }),
        value => Err(format!("unknown immediate opcode {:x}", value)),
    }
}

fn string_operand(index: usize, strings: &[String]) -> Result<Operand, String> {
    let value = strings.get(index).ok_or_else(|| format!("string index {} out of range", index))?;
    Ok(Operand { str_value: value.clone(), kind: "string".to_string(), ..Operand::default() })
}

fn decompile_module(module: &Module) -> String {
    if !module.functions.is_empty() {
        let mut functions = module.functions.clone();
        functions.sort_by_key(|function| function.addr);
        let ranges = build_function_ranges(&functions, &module.code);
        let mut chunks = Vec::new();
        for function in &ranges {
            if function.addr() >= module.code.len() || function.start >= module.code.len() || function.start >= function.end { continue; }
            let mut state = new_decompile_state();
            if !is_synthetic_function(&function.function.name) { state.skip = nested_function_ranges(function, &ranges); }
            let mut body = decompile_range_with_state(&module.code, function.start, function.end, 1, &mut state);
            body = remove_duplicate_gotos(body);
            body = recover_profile_clone_blocks(body);
            body = recover_bare_constructor_blocks(body);
            body = remove_named_gui_constructor_addcontrols(body);
            body = remove_repeated_assignment_runs(body);
            body = recover_forward_goto_guard_chains(body);
            body = recover_forward_goto_guards_fixed_point(body);
            body = recover_forward_if_goto_loops(body);
            body = recover_inverted_if_goto_loops(body);
            body = recover_loop_goto_continues(body);
            body = recover_sleep_loop_blocks(body);
            chunks.push(format!("{} {{\n{}\n}}", function_signature(&function.function.name, &function.function.params), body.join("\n")));
        }
        return recover_output_goto_guards(chunks.join("\n\n"));
    }
    let mut lines = decompile_range(&module.code, 0, module.code.len(), 0);
    lines = remove_duplicate_gotos(lines);
    lines = recover_profile_clone_blocks(lines);
    lines = recover_bare_constructor_blocks(lines);
    lines = remove_named_gui_constructor_addcontrols(lines);
    lines = remove_repeated_assignment_runs(lines);
    lines = recover_forward_goto_guard_chains(lines);
    lines = recover_forward_goto_guards_fixed_point(lines);
    lines = recover_forward_if_goto_loops(lines);
    lines = recover_inverted_if_goto_loops(lines);
    lines = recover_loop_goto_continues(lines);
    lines = recover_sleep_loop_blocks(lines);
    recover_output_goto_guards(lines.join("\n"))
}

impl FunctionRange {
    fn addr(&self) -> usize { self.function.addr }
}

fn recover_output_goto_guards(output: String) -> String {
    let mut lines = recover_forward_goto_guard_chains(output.split('\n').map(str::to_string).collect());
    lines = recover_infinite_goto_loops(lines);
    lines = recover_loop_goto_tails(lines);
    for _ in 0..16 {
        let next = remove_named_gui_constructor_addcontrols(lines.clone());
        if next.join("\n") == lines.join("\n") { break; }
        lines = next;
    }
    format!("{}\n", lines.join("\n"))
}

fn new_decompile_state() -> DecompileState { DecompileState::default() }

fn decompile_range(code: &[Instruction], start: usize, end: usize, indent: usize) -> Vec<String> {
    let mut state = new_decompile_state();
    decompile_range_with_state(code, start, end, indent, &mut state)
}

fn decompile_range_with_state(code: &[Instruction], start: usize, end: usize, indent: usize, state: &mut DecompileState) -> Vec<String> {
    decompile_range_with_state_and_stack(code, start, end, indent, state, Vec::new())
}

fn decompile_range_with_state_and_stack(code: &[Instruction], start: usize, end: usize, indent: usize, state: &mut DecompileState, initial_stack: Vec<DecompExpr>) -> Vec<String> {
    let mut lines = Vec::new();
    let mut stack = initial_stack;
    let mut pc = start;
    while pc < end && pc < code.len() {
        if let Some(skip_end) = skip_range_end(&state.skip, pc) { pc = skip_end; continue; }
        if let Some((dispatch_lines, new_pc)) = recover_tail_dispatch(code, pc, end, indent, state) { lines.extend(dispatch_lines); pc = new_pc + 1; continue; }
        let instruction = &code[pc];
        match instruction.op {
            OP_NONE => {}
            OP_PUSH_ARRAY => stack.push(DecompExpr { marker: true, ..DecompExpr::default() }),
            OP_PUSH_STRING => stack.push(DecompExpr { text: quote(instruction.operand.as_ref().map(|v| v.str_value.as_str()).unwrap_or("")), kind: "string".to_string(), ..DecompExpr::default() }),
            OP_PUSH_VARIABLE => stack.push(DecompExpr { text: variable_name(instruction.operand.as_ref().map(|v| v.str_value.as_str()).unwrap_or("")), ..DecompExpr::default() }),
            OP_PUSH_NUMBER => stack.push(DecompExpr { text: number_text(instruction.operand.as_ref()), ..DecompExpr::default() }),
            OP_PUSH_TRUE => stack.push(DecompExpr { text: "true".to_string(), ..DecompExpr::default() }),
            OP_PUSH_FALSE => stack.push(DecompExpr { text: "false".to_string(), ..DecompExpr::default() }),
            OP_PUSH_NULL => stack.push(DecompExpr { text: "null".to_string(), ..DecompExpr::default() }),
            OP_PI => stack.push(DecompExpr { text: "pi".to_string(), ..DecompExpr::default() }),
            OP_THIS => stack.push(DecompExpr { text: "this".to_string(), ..DecompExpr::default() }),
            OP_THIS_O => stack.push(DecompExpr { text: "thiso".to_string(), ..DecompExpr::default() }),
            OP_PLAYER => stack.push(DecompExpr { text: "player".to_string(), ..DecompExpr::default() }),
            OP_PLAYER_O => stack.push(DecompExpr { text: "playero".to_string(), ..DecompExpr::default() }),
            OP_LEVEL => stack.push(DecompExpr { text: "level".to_string(), ..DecompExpr::default() }),
            OP_TEMP => stack.push(DecompExpr { text: "temp".to_string(), ..DecompExpr::default() }),
            OP_PARAMS => stack.push(DecompExpr { text: "params".to_string(), ..DecompExpr::default() }),
            OP_CONVERT_FLOAT | OP_CONVERT_STRING | OP_CONVERT_OBJECT | OP_CONVERT_VAR | OP_END_PARAMS | OP_FUNCTION_START | OP_LOOP_COUNTER | OP_SHORT_END => {}
            OP_NEW | OP_WITH_END => {}
            OP_NEW_OBJECT => {
                let class_name = pop_expr(&mut stack);
                let target = pop_expr(&mut stack);
                if is_unknown_object_placeholder(&target.text) {
                    let object = DecompExpr { text: format!("new {}()", unquote_text(&class_name.text)), kind: "object".to_string(), ..DecompExpr::default() };
                    if stack.is_empty() { stack.push(DecompExpr { text: "temp.object".to_string(), ..DecompExpr::default() }); }
                    stack.push(object);
                } else if !stack.is_empty() {
                    stack.push(DecompExpr { text: format!("new {}({})", unquote_text(&class_name.text), constructor_arg(&target)), kind: "object".to_string(), ..DecompExpr::default() });
                } else {
                    let mut class_name = class_name;
                    class_name.kind = "class".to_string();
                    stack.push(target); stack.push(class_name);
                }
            }
            OP_WITH => {
                let target = jump_target(instruction);
                let target_expr = pop_expr(&mut stack);
                if target > pc && target <= end && lines.last().is_some_and(|line| is_constructor_line(line) && constructor_line_matches_target(line, &target_expr.text)) {
                    let assignment_constructor = lines.last().is_some_and(|line| is_assignment_constructor_line(line));
                    if let Some(last) = lines.last_mut() { *last = last.trim_end_matches(';').to_string() + " {"; }
                    lines.extend(decompile_range_with_state(code, pc + 1, target, indent + 1, state));
                    lines.push(format!("{}{}", pad(indent), if assignment_constructor { "};" } else { "}" }));
                    pc = target;
                    continue;
                } else if target > pc && target <= end {
                    lines.push(format!("{}with ({}) {{", pad(indent), target_expr.text));
                    lines.extend(decompile_range_with_state(code, pc + 1, target, indent + 1, state));
                    lines.push(format!("{}}}", pad(indent)));
                    pc = target;
                    continue;
                }
            }
            OP_SHORT_OR | OP_SHORT_AND => {
                if stack.len() < 2 { pc += 1; continue; }
                let rhs = pop_expr(&mut stack); let lhs = pop_expr(&mut stack);
                if lhs.marker { stack.push(lhs); stack.push(rhs); pc += 1; continue; }
                stack.push(DecompExpr { text: format!("{} {} {}", lhs.text, infix(instruction.op), rhs.text), ..DecompExpr::default() });
            }
            OP_END_ARRAY => { let args = collect_args(&mut stack); stack.push(DecompExpr { text: format!("{{{}}}", args.join(", ")), ..DecompExpr::default() }); }
            OP_NEW_UNINIT_ARRAY => { let size = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("new [{}]", size.text), ..DecompExpr::default() }); }
            OP_COPY => { let item = pop_expr(&mut stack); stack.push(item.clone()); stack.push(item); }
            OP_SWAP => { let a = pop_expr(&mut stack); let b = pop_expr(&mut stack); stack.push(a); stack.push(b); }
            OP_SET_REGISTER => { let item = pop_expr(&mut stack); state.registers.insert(operand_number(instruction), item.clone()); stack.push(item); }
            OP_GET_REGISTER => { let id = operand_number(instruction); stack.push(state.registers.get(&id).cloned().unwrap_or(DecompExpr { text: format!("reg{}", id), ..DecompExpr::default() })); }
            OP_MARK_REGISTER_VAR => {}
            OP_INC => { let item = pop_expr(&mut stack); lines.push(format!("{}{} += 1;", pad(indent), item.text)); stack.push(item); }
            OP_DEC => { let item = pop_expr(&mut stack); lines.push(format!("{}{} -= 1;", pad(indent), item.text)); stack.push(item); }
            OP_ACCESS_MEMBER => { let rhs = pop_expr(&mut stack); let lhs = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("{}.{}", member_base(&lhs.text), member_name(&rhs.text)), ..DecompExpr::default() }); }
            OP_ASSIGN_MEMBER => { let rhs = pop_expr(&mut stack); let property = pop_expr(&mut stack); let object = pop_expr(&mut stack); lines.push(format!("{}{}.{} = {};", pad(indent), member_base(&object.text), member_name(&property.text), rhs.text)); }
            OP_ADD | OP_SUBTRACT | OP_MULTIPLY | OP_DIVIDE | OP_MODULO | OP_POWER | OP_BOOL_AND | OP_BOOL_OR | OP_EQUAL | OP_NOT_EQUAL | OP_LESS_THAN | OP_GREATER_THAN | OP_LE | OP_GE | OP_BITWISE_OR | OP_BITWISE_AND | OP_BITWISE_XOR | OP_SHIFT_LEFT | OP_SHIFT_RIGHT | OP_IN | OP_JOIN | OP_APPEND => { let rhs = pop_expr(&mut stack); let lhs = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("{} {} {}", lhs.text, infix(instruction.op), rhs.text), ..DecompExpr::default() }); }
            OP_IN_RANGE => { let upper = pop_expr(&mut stack); let lower = pop_expr(&mut stack); let item = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("{} in <{}, {}>", item.text, lower.text, upper.text), ..DecompExpr::default() }); }
            OP_LOGICAL_NOT => { let item = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("!{}", member_base(&item.text)), ..DecompExpr::default() }); }
            OP_UNARY_SUBTRACT => { let item = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("-{}", member_base(&item.text)), ..DecompExpr::default() }); }
            OP_BITWISE_INVERT => { let item = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("~{}", member_base(&item.text)), ..DecompExpr::default() }); }
            OP_ARRAY_ACCESS => { let index = pop_expr(&mut stack); let array = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("{}[{}]", array.text, index.text), ..DecompExpr::default() }); }
            OP_ASSIGN_ARRAY | OP_SET_ARRAY => { let rhs = pop_expr(&mut stack); let index = pop_expr(&mut stack); let array = pop_expr(&mut stack); lines.push(format!("{}{}[{}] = {};", pad(indent), array.text, index.text, rhs.text)); }
            OP_MULTI_DIM_ARRAY => { let value = multi_dim_target(&mut stack); stack.push(DecompExpr { text: value, ..DecompExpr::default() }); }
            OP_ASSIGN_MULTI_DIM_ARRAY => { let rhs = pop_expr(&mut stack); let mut target = multi_dim_target(&mut stack); if target == "/* missing */" { target = multi_dim_assignment_target(&rhs.text); } lines.push(format!("{}{} = {};", pad(indent), target, rhs.text)); }
            OP_OBJ_STARTS => { let value = object_call(&mut stack, "starts", 1, false); stack.push(value); }
            OP_TRANSLATION => { let arg = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("_({})", arg.text), ..DecompExpr::default() }); }
            OP_OBJ_SUBSTRING => { let value = object_call(&mut stack, "substring", 2, false); stack.push(value); }
            OP_OBJ_SIZE => { let value = object_call(&mut stack, "size", 0, false); stack.push(value); }
            OP_OBJ_INDEX => { let value = object_call(&mut stack, "index", 1, false); stack.push(value); }
            OP_INT => { let value = function_call(&mut stack, "int", 1); stack.push(value); }
            OP_CHAR => { let value = function_call(&mut stack, "char", 1); stack.push(value); }
            OP_SLEEP => { let call = function_call(&mut stack, "sleep", 1); lines.push(format!("{}{};", pad(indent), call.text)); }
            OP_WAIT_FOR => { let call = function_call(&mut stack, "waitfor", 1); lines.push(format!("{}{};", pad(indent), call.text)); }
            OP_MAKE_VAR => { let value = function_call(&mut stack, "makevar", 1); stack.push(value); }
            OP_ABS => { let value = function_call(&mut stack, "abs", 1); stack.push(value); }
            OP_RANDOM => { let value = function_call(&mut stack, "random", 2); stack.push(value); }
            OP_SIN => { let value = function_call(&mut stack, "sin", 1); stack.push(value); }
            OP_COS => { let value = function_call(&mut stack, "cos", 1); stack.push(value); }
            OP_ARCTAN => { let value = function_call(&mut stack, "arctan", 1); stack.push(value); }
            OP_EXP => { let value = function_call(&mut stack, "exp", 1); stack.push(value); }
            OP_LOG => { let value = function_call(&mut stack, "log", 1); stack.push(value); }
            OP_MIN => { let value = function_call(&mut stack, "min", 2); stack.push(value); }
            OP_MAX => { let value = function_call(&mut stack, "max", 2); stack.push(value); }
            OP_GET_ANGLE => { let value = function_call(&mut stack, "getangle", 2); stack.push(value); }
            OP_GET_DIR => { let value = function_call(&mut stack, "getdir", 2); stack.push(value); }
            OP_VEC_X => { let value = function_call(&mut stack, "vecx", 1); stack.push(value); }
            OP_VEC_Y => { let value = function_call(&mut stack, "vecy", 1); stack.push(value); }
            OP_OBJ_COMPARE => { let value = function_call(&mut stack, "objcompare", 2); stack.push(value); }
            OP_FORMAT => { let args = collect_args(&mut stack); stack.push(DecompExpr { text: format!("format({})", args.join(", ")), ..DecompExpr::default() }); }
            OP_OBJ_TYPE => { let value = object_call(&mut stack, "type", 0, false); stack.push(value); }
            OP_OBJ_INDICES => { let value = object_call(&mut stack, "indices", 0, false); stack.push(value); }
            OP_OBJ_LINK => { let value = object_call(&mut stack, "link", 0, false); stack.push(value); }
            OP_OBJ_TRIM => { let value = object_call(&mut stack, "trim", 0, false); stack.push(value); }
            OP_OBJ_LENGTH => { let value = object_call(&mut stack, "length", 0, false); stack.push(value); }
            OP_OBJ_POS => { let value = object_call(&mut stack, "pos", 1, false); stack.push(value); }
            OP_OBJ_CHAR_AT => { let value = object_call(&mut stack, "charat", 1, false); stack.push(value); }
            OP_OBJ_ENDS => { let value = object_call(&mut stack, "ends", 1, false); stack.push(value); }
            OP_OBJ_TOKENIZE => { let value = object_call(&mut stack, "tokenize", 1, false); stack.push(value); }
            OP_OBJ_POSITIONS => { let value = object_call(&mut stack, "positions", 1, false); stack.push(value); }
            OP_OBJ_SUBARRAY => { let value = object_call(&mut stack, "subarray", 2, false); stack.push(value); }
            OP_OBJ_ADD_STRING => { let call = object_call(&mut stack, "add", 1, true); lines.push(format!("{}{};", pad(indent), call.text)); }
            OP_OBJ_DELETE_STRING => { let call = object_call(&mut stack, "delete", 1, true); lines.push(format!("{}{};", pad(indent), call.text)); }
            OP_OBJ_REMOVE_STRING => { let call = object_call(&mut stack, "remove", 1, true); lines.push(format!("{}{};", pad(indent), call.text)); }
            OP_OBJ_REPLACE_STRING => { let call = object_call(&mut stack, "replace", 2, true); lines.push(format!("{}{};", pad(indent), call.text)); }
            OP_OBJ_INSERT_STRING => { let call = object_call(&mut stack, "insert", 2, true); lines.push(format!("{}{};", pad(indent), call.text)); }
            OP_OBJ_CLEAR => { let call = object_call(&mut stack, "clear", 0, true); lines.push(format!("{}{};", pad(indent), call.text)); }
            OP_NEW_MULTI_DIM_ARRAY => { let value = new_multi_dim_array_expr(&mut stack); stack.push(value); }
            OP_ASSIGN => {
                let rhs = pop_expr(&mut stack); let lhs = pop_expr(&mut stack);
                let lhs = if lhs.text == "/* missing */" { DecompExpr { text: multi_dim_assignment_target(&rhs.text), ..lhs } } else { lhs };
                let (lhs, rhs) = recover_format_assignment(lhs, rhs);
                let (lhs, rhs) = recover_new_multi_dim_assignment(lhs, rhs);
                let (lhs, rhs) = recover_embedded_boolean_assignment(lhs, rhs);
                let (lhs, rhs) = recover_swapped_boolean_assignment(lhs, rhs);
                if is_hidden_function_binding(&lhs, &rhs) { pc += 1; continue; }
                let rhs = normalize_assignment_value(&lhs, rhs);
                if is_named_profile_clone_construction(&lhs, &rhs) {
                    let (arg, _) = constructor_expr_arg(&rhs.text).unwrap_or_default(); lines.push(format!("{}new GuiControlProfile({});", pad(indent), arg));
                } else if is_named_gui_construction(&lhs, &rhs) { lines.push(format!("{}{};", pad(indent), rhs.text)); }
                else if is_constructor_target(&lhs, &rhs) { lines.push(format!("{}new {}({});", pad(indent), unquote_text(&rhs.text), constructor_arg(&lhs))); }
                else if rhs.kind == "class" { lines.push(format!("{}{} = new {}();", pad(indent), class_assignment_target(&lhs), unquote_text(&rhs.text))); }
                else { lines.push(format!("{}{} = {};", pad(indent), lhs.text, rhs.text)); }
            }
            OP_CALL => { let call = build_call(&mut stack); stack.push(DecompExpr { text: call, kind: "call".to_string(), ..DecompExpr::default() }); }
            OP_POP => { let item = pop_expr(&mut stack); if item.kind == "call" && !item.text.is_empty() { lines.push(format!("{}{};", pad(indent), item.text)); } }
            OP_JNE | OP_JEQ => {
                let target = jump_target(instruction);
                let condition = pop_expr(&mut stack).text;
                if let Some((assign_lines, new_pc)) = recover_conditional_assignment_chain(code, pc, target, end, indent, state, &condition, instruction.op, &stack) { lines.extend(assign_lines); stack.pop(); pc = new_pc + 1; continue; }
                if let Some((assign_lines, new_pc)) = recover_ternary_assignment(code, pc, target, end, indent, state, &condition, instruction.op, &stack) { lines.extend(assign_lines); stack.pop(); pc = new_pc + 1; continue; }
                if let Some((assign_lines, new_pc)) = recover_self_ternary_assignment(code, pc, target, end, indent, state, &condition, instruction.op, &stack) { lines.extend(assign_lines); stack.pop(); pc = new_pc + 1; continue; }
                if let Some((value, new_pc)) = recover_ternary_expression(code, pc, target, end, state, &condition, instruction.op) { stack.push(DecompExpr { text: value, ..DecompExpr::default() }); pc = new_pc + 1; continue; }
                if target > pc && target <= end {
                    let condition = if instruction.op == OP_JEQ { format!("!({})", condition) } else { condition };
                    let body = trim_after_return(decompile_range_with_state_and_stack(code, pc + 1, target, indent + 1, state, stack.clone()));
                    if let Some(for_lines) = recover_for_loop(&lines, &body, &condition, pc, indent) { lines = for_lines; }
                    else if let Some(while_lines) = recover_while_loop(&body, &condition, pc, indent) { lines.extend(while_lines); }
                    else { lines.push(format!("{}if ({}) {{", pad(indent), condition)); lines.extend(body); lines.push(format!("{}}}", pad(indent))); }
                    pc = target; continue;
                }
                let loop_condition = if instruction.op == OP_JEQ { format!("!({})", condition) } else { condition.clone() };
                let loop_end = loop_recovery_end(code, pc + 1, end, pc);
                if loop_end > pc + 1 {
                    let body = decompile_range_with_state_and_stack(code, pc + 1, loop_end, indent + 1, state, stack.clone());
                    if let Some(for_lines) = recover_for_loop(&lines, &body, &loop_condition, pc, indent) { lines = for_lines; pc = loop_end; continue; }
                }
                lines.push(format!("{}if ({}) goto label_{};", pad(indent), condition, target));
            }
            OP_JMP => {
                let target = jump_target(instruction);
                if let Some((dispatch_lines, new_pc)) = recover_forward_dispatch(code, pc, target, end, indent, state) { lines.extend(dispatch_lines); pc = new_pc + 1; continue; }
                if let Some((dispatch_lines, new_pc)) = recover_backward_dispatch(code, pc, target, end, indent, state) { lines.extend(dispatch_lines); pc = new_pc + 1; continue; }
                if skips_embedded_function(&state.skip, pc + 1, target) || target == end { pc = target; continue; }
                if target > pc && target <= end && is_jump_padding(code, pc + 1, target) { pc = target; continue; }
                if target < end { lines.push(format!("{}goto label_{};", pad(indent), target)); }
            }
            OP_FOR_EACH => {
                let target = jump_target(instruction);
                let _unused = pop_expr(&mut stack); let collection = pop_expr(&mut stack); let iterator = pop_expr(&mut stack);
                let condition = format!("{} in {}", iterator.text, collection.text);
                if target > pc && target <= end { let body = trim_for_each_bookkeeping(decompile_range_with_state(code, pc + 1, target, indent + 1, state)); lines.push(format!("{}for ({}) {{", pad(indent), condition)); lines.extend(body); lines.push(format!("{}}}", pad(indent))); pc = target; continue; }
                stack.push(DecompExpr { text: condition, ..DecompExpr::default() });
            }
            OP_RET => {
                if !stack.is_empty() { let ret = pop_expr(&mut stack).text; if !(indent == 1 && is_terminal_ret(code, pc, end) && ret == "0") { lines.push(format!("{}return {};", pad(indent), ret)); } }
                else if !(indent == 1 && is_terminal_ret(code, pc, end)) { lines.push(format!("{}return;", pad(indent))); }
            }
            value => lines.push(format!("{}// unhandled opcode 0x{:02x} at {}", pad(indent), value, instruction.addr)),
        }
        pc += 1;
    }
    collapse_nested_ifs(lines)
}

fn skip_range_end(ranges: &[FunctionRange], pc: usize) -> Option<usize> {
    ranges.iter().find(|range| range.function.addr == pc).map(|range| range.end)
}

fn skips_embedded_function(ranges: &[FunctionRange], start: usize, target: usize) -> bool {
    ranges.iter().any(|range| range.function.addr == start && range.end == target)
}

fn is_hidden_function_binding(lhs: &DecompExpr, rhs: &DecompExpr) -> bool {
    let lhs_text = lhs.text.trim();
    (lhs_text.is_empty() || lhs_text == "/* missing */") && is_synthetic_function(&rhs.text)
}

fn build_function_ranges(functions: &[FunctionDef], code: &[Instruction]) -> Vec<FunctionRange> {
    let mut ranges = vec![FunctionRange::default(); functions.len()];
    let mut next_concrete = code.len();
    for index in (0..functions.len()).rev() {
        let function = functions[index].clone();
        let start = if function.body_start == 0 { function.addr } else { function.body_start };
        let end = if is_synthetic_function(&function.name) { first_return_end(code, start, next_concrete) } else { next_concrete };
        if !is_synthetic_function(&function.name) { next_concrete = function.addr; }
        ranges[index] = FunctionRange { function, start, end };
    }
    extend_synthetic_function_ranges(&mut ranges, code);
    ranges
}

fn extend_synthetic_function_ranges(ranges: &mut [FunctionRange], code: &[Instruction]) {
    for parent_index in 0..ranges.len() {
        if is_synthetic_function(&ranges[parent_index].function.name) { continue; }
        let parent_start = ranges[parent_index].start;
        let parent_end = ranges[parent_index].end;
        for pc in parent_start..parent_end.min(code.len()) {
            if code[pc].op != OP_JMP { continue; }
            let target = jump_target(&code[pc]);
            if target <= pc + 1 || target > parent_end { continue; }
            for (child_index, child) in ranges.iter_mut().enumerate() {
                if child_index == parent_index || !is_synthetic_function(&child.function.name) || child.function.addr != pc + 1 || child.end >= target { continue; }
                child.end = target;
            }
        }
    }
}

fn first_return_end(code: &[Instruction], start: usize, fallback: usize) -> usize {
    for pc in start..fallback.min(code.len()) { if code[pc].op == OP_RET { return pc + 1; } }
    fallback
}

fn nested_function_ranges(parent: &FunctionRange, functions: &[FunctionRange]) -> Vec<FunctionRange> {
    functions.iter().filter(|function| function.function.addr != parent.function.addr && is_synthetic_function(&function.function.name) && function.function.addr > parent.start && function.end <= parent.end).cloned().collect()
}

fn is_synthetic_function(name: &str) -> bool {
    let name = name.rsplit_once('.').map(|(_, value)| value).unwrap_or(name);
    let Some(rest) = name.strip_prefix("function_") else { return false; };
    let mut parts = rest.split('_');
    parts.next().is_some_and(|value| value.parse::<i32>().is_ok()) && parts.next().is_some_and(|value| value.parse::<i32>().is_ok()) && parts.next().is_none()
}

fn function_signature(name: &str, params: &[String]) -> String {
    for visibility in ["public", "private", "protected"] {
        let prefix = format!("{}.", visibility);
        if let Some(value) = name.strip_prefix(&prefix) { return format!("{} function {}({})", visibility, value, params.join(", ")); }
    }
    format!("function {}({})", name, params.join(", "))
}

fn recover_for_loop(lines: &[String], body: &[String], condition: &str, pc: usize, indent: usize) -> Option<Vec<String>> {
    if lines.is_empty() || body.len() < 2 { return None; }
    let goto_line = body.last()?.trim();
    if !goto_line.starts_with("goto label_") || !goto_line.ends_with(';') { return None; }
    let label = goto_line.strip_prefix("goto label_")?.strip_suffix(';')?.parse::<usize>().ok()?;
    if label > pc || pc - label > 16 { return None; }
    let mut work_body = body[..body.len() - 1].to_vec();
    if work_body.is_empty() { return None; }
    let mut inc_line = work_body.last()?.trim().to_string();
    if work_body.len() >= 2 {
        let maybe_bare = work_body[work_body.len() - 1].trim().to_string();
        let previous = work_body[work_body.len() - 2].trim();
        if previous.ends_with(" += 1;") && maybe_bare == previous.trim_end_matches(" += 1;").trim().to_string() + ";" {
            work_body.pop();
            inc_line = work_body.last()?.trim().to_string();
        }
    }
    let (inc_var, inc) = parse_loop_increment(&inc_line)?;
    let init_line = lines.last()?.trim();
    if !init_line.starts_with(&(inc_var.clone() + " = ")) || !init_line.ends_with(';') || !condition.contains(&inc_var) { return None; }
    let mut result = lines[..lines.len() - 1].to_vec();
    result.push(format!("{}for ({}; {}; {}) {{", pad(indent), init_line.trim_end_matches(';'), condition, inc));
    result.extend(replace_goto_target(&work_body[..work_body.len() - 1], label, "continue;"));
    result.push(format!("{}}}", pad(indent)));
    Some(result)
}

fn replace_goto_target(lines: &[String], target: usize, replacement: &str) -> Vec<String> {
    let wanted = format!("goto label_{};", target);
    lines.iter().map(|line| if line.trim() == wanted { format!("{}{}", " ".repeat(parse_line_indent(line)), replacement) } else { line.clone() }).collect()
}

fn parse_loop_increment(line: &str) -> Option<(String, String)> {
    let line = line.trim().trim_end_matches(';');
    if let Some(value) = line.strip_suffix(" += 1") { return Some((value.to_string(), line.to_string())); }
    if let Some(index) = line.find(" = ") {
        let lhs = &line[..index]; let rhs = &line[index + 3..];
        let prefix = format!("{} + ", lhs);
        if let Some(step) = rhs.strip_prefix(&prefix) { if !step.trim().is_empty() { return Some((lhs.to_string(), format!("{} += {}", lhs, step.trim()))); } }
    }
    None
}

fn loop_recovery_end(code: &[Instruction], start: usize, end: usize, branch_pc: usize) -> usize {
    for index in start..end.min(code.len()) {
        if code[index].op == OP_JMP && jump_target(&code[index]) <= branch_pc { return index + 1; }
        if code[index].op == OP_RET { return 0; }
    }
    0
}

fn trim_for_each_bookkeeping(mut body: Vec<String>) -> Vec<String> {
    if body.last().is_some_and(|line| is_goto_line(line.trim())) { body.pop(); }
    if body.last().is_some_and(|line| { let line = line.trim(); line.ends_with(" += 1;") || line.ends_with(" -= 1;") }) { body.pop(); }
    body
}

fn trim_after_return(body: Vec<String>) -> Vec<String> {
    for (index, line) in body.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("return") && trimmed.ends_with(';') { return body[..index + 1].to_vec(); }
    }
    body
}

fn is_goto_line(line: &str) -> bool {
    let line = line.trim();
    line.strip_prefix("goto label_").and_then(|value| value.strip_suffix(';')).is_some_and(|value| value.parse::<usize>().is_ok())
}

fn is_jump_padding(code: &[Instruction], start: usize, end: usize) -> bool {
    start < end && end <= code.len() && code[start..end].iter().all(|instruction| instruction.op == OP_JMP || instruction.op == OP_NONE)
}

fn is_terminal_ret(code: &[Instruction], pc: usize, end: usize) -> bool {
    for instruction in code.iter().take(end).skip(pc + 1) {
        if instruction.op == OP_JMP && jump_target(instruction) >= end { continue; }
        return false;
    }
    true
}

fn remove_duplicate_gotos(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    for line in lines {
        if output.last().is_some_and(|previous: &String| previous.trim() == line.trim() && is_goto_line(&line)) { continue; }
        output.push(line);
    }
    output
}

fn remove_repeated_assignment_runs(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let mut best = 0usize;
        for length in 2..=8 {
            if index + 2 * length <= lines.len() && same_assignment_run(&lines[index..index + length], &lines[index + length..index + 2 * length]) { best = length; }
        }
        if best > 0 { output.extend_from_slice(&lines[index..index + best]); index += best * 2; }
        else { output.push(lines[index].clone()); index += 1; }
    }
    output
}

fn same_assignment_run(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(left, right)| left.trim() == right.trim() && is_simple_assignment_line(left))
}

fn is_simple_assignment_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.ends_with(';') && trimmed.contains(" = ") && !trimmed.starts_with("if ") && !trimmed.starts_with("for ")
}

fn pad(level: usize) -> String { "  ".repeat(level) }

fn go_quote_char(character: char) -> String { format!("{:?}", character) }

fn build_call(stack: &mut Vec<DecompExpr>) -> String {
    let callee = pop_expr(stack).text;
    let args = collect_call_args(stack);
    format!("{}({})", call_target(&callee), args.join(", "))
}

fn call_target(value: &str) -> String {
    if value.contains(" @ ") && !(value.starts_with('(') && value.ends_with(')')) { format!("({})", value) } else { value.to_string() }
}

fn collect_call_args(stack: &mut Vec<DecompExpr>) -> Vec<String> {
    let mut args = Vec::new();
    while !stack.is_empty() {
        let item = pop_expr(stack);
        if item.marker { break; }
        args.push(item.text);
    }
    args
}

fn function_call(stack: &mut Vec<DecompExpr>, name: &str, argc: usize) -> DecompExpr {
    DecompExpr { text: format!("{}({})", name, fixed_args(stack, argc).join(", ")), kind: "call".to_string(), ..DecompExpr::default() }
}

fn object_call(stack: &mut Vec<DecompExpr>, name: &str, argc: usize, statement: bool) -> DecompExpr {
    let args = fixed_args(stack, argc);
    let object = pop_expr(stack);
    DecompExpr { text: format!("{}.{}({})", member_base(&object.text), name, args.join(", ")), kind: if statement { "call".to_string() } else { String::new() }, ..DecompExpr::default() }
}

fn multi_dim_array_expr(stack: &mut Vec<DecompExpr>) -> DecompExpr { DecompExpr { text: multi_dim_target(stack), ..DecompExpr::default() } }

fn multi_dim_target(stack: &mut Vec<DecompExpr>) -> String {
    let parts = drain_stack(stack);
    if parts.is_empty() { return "/* missing */".to_string(); }
    let mut target = parts[0].clone();
    for index in &parts[1..] { target.push('['); target.push_str(index); target.push(']'); }
    target
}

fn multi_dim_assignment_target(value: &str) -> String {
    let Some(index) = value.find('[') else { return "/* missing */".to_string(); };
    if index == 0 { return "/* missing */".to_string(); }
    let target = &value[..index];
    if !is_assignable_text(target) { "/* missing */".to_string() } else { target.to_string() }
}

fn new_multi_dim_array_expr(stack: &mut Vec<DecompExpr>) -> DecompExpr {
    let dimensions = drain_stack(stack);
    if dimensions.is_empty() { return DecompExpr { text: "new []".to_string(), ..DecompExpr::default() }; }
    DecompExpr { text: format!("new {}", dimensions.iter().map(|value| format!("[{}]", value)).collect::<String>()), ..DecompExpr::default() }
}

fn drain_stack(stack: &mut Vec<DecompExpr>) -> Vec<String> {
    let mut items = Vec::new();
    while !stack.is_empty() { items.push(pop_expr(stack).text); }
    items.reverse();
    items
}

fn fixed_args(stack: &mut Vec<DecompExpr>, argc: usize) -> Vec<String> {
    let mut args = Vec::new();
    for _ in 0..argc { args.push(pop_expr(stack).text); }
    args.reverse();
    args
}

fn collect_args(stack: &mut Vec<DecompExpr>) -> Vec<String> {
    let mut args = Vec::new();
    while !stack.is_empty() {
        let item = pop_expr(stack);
        if item.marker { break; }
        args.push(item.text);
    }
    args.reverse();
    args
}

fn infix(op: u8) -> &'static str {
    match op {
        OP_ADD => "+", OP_SUBTRACT => "-", OP_MULTIPLY => "*", OP_DIVIDE => "/", OP_MODULO => "%", OP_POWER => "^",
        OP_BOOL_AND | OP_SHORT_AND => "&&", OP_BOOL_OR | OP_SHORT_OR => "||", OP_EQUAL => "==", OP_NOT_EQUAL => "!=",
        OP_LESS_THAN => "<", OP_GREATER_THAN => ">", OP_LE => "<=", OP_GE => ">=", OP_BITWISE_OR => "|", OP_BITWISE_AND => "&",
        OP_BITWISE_XOR => "^", OP_SHIFT_LEFT => "<<", OP_SHIFT_RIGHT => ">>", OP_IN => "in", OP_JOIN | OP_APPEND => "@", _ => "?",
    }
}

fn member_base(value: &str) -> String {
    if value.contains(" @ ") && !(value.starts_with('(') && value.ends_with(')')) { format!("({})", value) } else { value.to_string() }
}

fn member_name(value: &str) -> String {
    if value.contains(" @ ") && !(value.starts_with('(') && value.ends_with(')')) { return format!("({})", value); }
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let name = &value[1..value.len() - 1];
        if is_identifier(name) { return name.to_string(); }
    }
    value.to_string()
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else { return false; };
    if !(first == '_' || first.is_ascii_alphabetic()) { return false; }
    chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn pop_expr(stack: &mut Vec<DecompExpr>) -> DecompExpr {
    stack.pop().unwrap_or_else(|| DecompExpr { text: "/* missing */".to_string(), ..DecompExpr::default() })
}

fn jump_target(instruction: &Instruction) -> usize { instruction.operand.as_ref().map(|value| value.number.max(0) as usize).unwrap_or(instruction.addr + 1) }

fn operand_number(instruction: &Instruction) -> i32 { instruction.operand.as_ref().map(|value| value.number).unwrap_or(0) }

fn number_text(operand: Option<&Operand>) -> String {
    let Some(operand) = operand else { return "0".to_string(); };
    if operand.kind == "float" { operand.float_value.clone() } else { operand.number.to_string() }
}

fn quote(value: &str) -> String {
    if is_quoted_hex_color(value) { return value.to_string(); }
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\x07' => output.push_str("\\a"),
            '\x08' => output.push_str("\\b"),
            '\x0c' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\x0b' => output.push_str("\\v"),
            character if character.is_control() => output.push_str(&format!("\\x{:02x}", character as u32)),
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn is_quoted_hex_color(value: &str) -> bool {
    let chars: Vec<char> = value.chars().collect();
    chars.len() == 9 && chars[0] == '"' && chars[1] == '#' && chars[8] == '"' && chars[2..8].iter().all(|character| character.is_ascii_hexdigit())
}

fn variable_name(value: &str) -> String { if value == "unknown_object" { "temp.object".to_string() } else { value.to_string() } }

fn is_unknown_object_placeholder(value: &str) -> bool { value == "unknown_object" || value == "temp.object" }

fn unquote_text(value: &str) -> String {
    if value.len() < 2 || !(value.starts_with('"') && value.ends_with('"')) { return value.to_string(); }
    let body = &value[1..value.len() - 1];
    let mut output = String::new();
    let mut chars = body.chars();
    while let Some(character) = chars.next() {
        if character != '\\' { output.push(character); continue; }
        match chars.next() {
            Some('a') => output.push('\x07'), Some('b') => output.push('\x08'), Some('f') => output.push('\x0c'),
            Some('n') => output.push('\n'), Some('r') => output.push('\r'), Some('t') => output.push('\t'), Some('v') => output.push('\x0b'),
            Some('x') => { let a = chars.next(); let b = chars.next(); if let (Some(a), Some(b)) = (a, b) { if let Ok(byte) = u8::from_str_radix(&format!("{}{}", a, b), 16) { output.push(byte as char); } } }
            Some(other) => output.push(other),
            None => output.push('\\'),
        }
    }
    output
}

fn is_constructor_line(line: &str) -> bool {
    let trimmed = line.trim();
    (trimmed.starts_with("new ") && trimmed.ends_with(");")) || (trimmed.contains(" = new Gui") && trimmed.ends_with(");"))
}

fn is_assignment_constructor_line(line: &str) -> bool { line.trim().contains(" = new ") && line.trim().ends_with(");") }

fn is_named_gui_construction(lhs: &DecompExpr, rhs: &DecompExpr) -> bool {
    if rhs.kind != "object" || !rhs.text.starts_with("new Gui") { return false; }
    constructor_expr_arg(&rhs.text).is_some_and(|(argument, _)| lhs.text.trim() == argument.trim())
}

fn is_named_profile_clone_construction(lhs: &DecompExpr, rhs: &DecompExpr) -> bool {
    if rhs.kind != "object" { return false; }
    let Some((class_name, _)) = constructor_expr_class(&rhs.text) else { return false; };
    if class_name.starts_with("Gui") || !class_name.ends_with("Profile") { return false; }
    constructor_expr_arg(&rhs.text).is_some_and(|(argument, _)| lhs.text.trim() == argument.trim())
}

fn constructor_expr_class(value: &str) -> Option<(String, bool)> {
    let trimmed = value.trim();
    let start = trimmed.find('(')?;
    if !trimmed.starts_with("new ") { return None; }
    Some((trimmed[4..start].trim().to_string(), true))
}

fn constructor_expr_arg(value: &str) -> Option<(String, bool)> {
    let start = value.find('(')?; let end = value.rfind(')')?;
    if end <= start { return None; }
    Some((value[start + 1..end].to_string(), true))
}

fn constructor_line_matches_target(line: &str, target: &str) -> bool {
    if target == "/* missing */" { return true; }
    let trimmed = line.trim().trim_end_matches(';');
    if let Some(value) = trimmed.strip_prefix("new ") {
        let Some(start) = value.find('(') else { return false; };
        let Some(end) = value.rfind(')') else { return false; };
        return end > start && unquote_text(&value[start + 1..end]) == unquote_text(target);
    }
    if let Some(index) = trimmed.find(" = new ") { return trimmed[..index].trim() == target; }
    false
}

fn normalize_assignment_value(lhs: &DecompExpr, mut rhs: DecompExpr) -> DecompExpr {
    if !is_extent_field(&lhs.text) { return rhs; }
    if let Some((a, b)) = parse_numeric_pair_literal(&rhs.text) { rhs.text = format!("{{{}, {}}}", b, a); }
    rhs
}

fn is_extent_field(name: &str) -> bool {
    let last = name.rsplit('.').next().unwrap_or(name);
    matches!(last.to_ascii_lowercase().as_str(), "clientextent" | "extent" | "minextent")
}

fn parse_numeric_pair_literal(value: &str) -> Option<(String, String)> {
    if !value.starts_with('{') || !value.ends_with('}') { return None; }
    let body = value[1..value.len() - 1].trim();
    let mut parts = body.split(',');
    let a = parts.next()?.trim(); let b = parts.next()?.trim();
    if parts.next().is_some() || !is_number_literal(a) || !is_number_literal(b) { return None; }
    Some((a.to_string(), b.to_string()))
}

fn is_number_literal(value: &str) -> bool { !value.is_empty() && value.parse::<f64>().is_ok() }

fn recover_format_assignment(lhs: DecompExpr, rhs: DecompExpr) -> (DecompExpr, DecompExpr) {
    if lhs.text != "/* missing */" || !rhs.text.starts_with("format(") || !rhs.text.ends_with(')') { return (lhs, rhs); }
    let mut args = split_top_level_args(&rhs.text[7..rhs.text.len() - 1]);
    if args.len() < 2 { return (lhs, rhs); }
    if let Some((recovered_lhs, recovered_arg)) = split_format_assignment_arg(&args[0]) {
        args[0] = recovered_arg;
        return (DecompExpr { text: recovered_lhs, ..DecompExpr::default() }, DecompExpr { text: format!("format({})", args.join(", ")), ..DecompExpr::default() });
    }
    args[0] = trim_leading_short_circuit(&args[0]);
    if !is_assignable_text(&args[0]) { return (lhs, rhs); }
    let recovered_lhs = args[0].clone();
    (DecompExpr { text: recovered_lhs, ..DecompExpr::default() }, DecompExpr { text: format!("format({})", args[1..].join(", ")), ..DecompExpr::default() })
}

fn split_format_assignment_arg(value: &str) -> Option<(String, String)> {
    for operator in [" || ", " && "] {
        if let Some(index) = value.find(operator) {
            if index > 0 {
                let left = value[..index].trim();
                let right = value[index + operator.len()..].trim();
                if is_assignable_text(left) && !right.is_empty() { return Some((left.to_string(), right.to_string())); }
            }
        }
    }
    None
}

fn trim_leading_short_circuit(mut value: &str) -> String {
    value = value.trim();
    for operator in ["|| ", "&& "] {
        if let Some(result) = value.strip_prefix(operator) { return result.trim().to_string(); }
    }
    value.to_string()
}

fn recover_new_multi_dim_assignment(lhs: DecompExpr, rhs: DecompExpr) -> (DecompExpr, DecompExpr) {
    if lhs.text != "/* missing */" || !rhs.text.starts_with("new [") { return (lhs, rhs); }
    let Some((target, dimensions)) = split_new_multi_dim(&rhs.text) else { return (lhs, rhs); };
    if !is_assignable_text(&target) || dimensions.is_empty() { return (lhs, rhs); }
    let value = format!("new {}", dimensions.iter().map(|dimension| format!("[{}]", dimension)).collect::<String>());
    (DecompExpr { text: target, ..DecompExpr::default() }, DecompExpr { text: value, ..DecompExpr::default() })
}

fn recover_swapped_boolean_assignment(lhs: DecompExpr, rhs: DecompExpr) -> (DecompExpr, DecompExpr) {
    if is_assignable_text(&lhs.text) || is_assignable_text(&rhs.text) { return (lhs, rhs); }
    if lhs.text.contains(" || ") || lhs.text.contains(" && ") { return (rhs, lhs); }
    (lhs, rhs)
}

fn recover_embedded_boolean_assignment(lhs: DecompExpr, rhs: DecompExpr) -> (DecompExpr, DecompExpr) {
    let Some((target, rest, operator)) = split_boolean_assignment_head(&lhs.text) else { return (lhs, rhs); };
    (DecompExpr { text: target.clone(), ..DecompExpr::default() }, DecompExpr { text: format!("{}{}{}{}{}", target, operator, rest, operator, rhs.text), ..DecompExpr::default() })
}

fn split_boolean_assignment_head(value: &str) -> Option<(String, String, String)> {
    for operator in [" || ", " && "] {
        if let Some(index) = value.find(operator) {
            if index > 0 {
                let left = value[..index].trim();
                let right = value[index + operator.len()..].trim();
                if is_assignable_text(left) && !right.is_empty() { return Some((left.to_string(), right.to_string(), operator.to_string())); }
            }
        }
    }
    None
}

fn split_new_multi_dim(value: &str) -> Option<(String, Vec<String>)> {
    let parts = split_bracket_parts(value)?;
    if parts.len() < 2 { return None; }
    let mut target = parts[0].clone();
    let mut dimensions = parts[1..].to_vec();
    if target.starts_with("new [") {
        let (nested_target, mut nested_dimensions) = split_new_multi_dim(&target)?;
        target = nested_target;
        nested_dimensions.append(&mut dimensions);
        dimensions = nested_dimensions;
    }
    for dimension in &mut dimensions { *dimension = unwrap_single_new_dim(dimension); }
    Some((target, dimensions))
}

fn split_bracket_parts(value: &str) -> Option<Vec<String>> {
    if !value.starts_with("new ") { return None; }
    let mut parts = Vec::new();
    let mut index = 4usize;
    while index < value.len() {
        if value.as_bytes().get(index) != Some(&b'[') { return None; }
        let end = matching_bracket(value, index)?;
        parts.push(value[index + 1..end].trim().to_string());
        index = end + 1;
    }
    Some(parts)
}

fn matching_bracket(value: &str, start: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (index, character) in value.char_indices().skip_while(|(index, _)| *index < start) {
        match character { '[' => depth += 1, ']' => { depth -= 1; if depth == 0 { return Some(index); } }, _ => {} }
    }
    None
}

fn unwrap_single_new_dim(value: &str) -> String {
    if let Some(parts) = split_bracket_parts(value) { if parts.len() == 1 { return parts[0].clone(); } }
    value.to_string()
}

fn split_top_level_args(value: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut quote_char = None;
    let mut escaped = false;
    for character in value.chars() {
        if let Some(quote) = quote_char {
            current.push(character);
            if escaped { escaped = false; }
            else if character == '\\' { escaped = true; }
            else if character == quote { quote_char = None; }
            continue;
        }
        match character {
            '"' | '\'' => { quote_char = Some(character); current.push(character); }
            '(' | '[' | '{' => { depth += 1; current.push(character); }
            ')' | ']' | '}' => { if depth > 0 { depth -= 1; } current.push(character); }
            ',' if depth == 0 => { args.push(current.trim().to_string()); current.clear(); }
            _ => current.push(character),
        }
    }
    args.push(current.trim().to_string());
    args
}

fn is_assignable_text(value: &str) -> bool {
    !value.is_empty() && !value.contains("/* missing */") && !value.starts_with('"') && !value.chars().any(|character| "+-*/<>=!&|".contains(character))
}

fn is_object_name_expr(value: &DecompExpr) -> bool { value.kind == "string" || value.text.contains(" @ ") }

fn is_constructor_target(lhs: &DecompExpr, rhs: &DecompExpr) -> bool {
    rhs.kind == "class" && unquote_text(&rhs.text).starts_with("Gui") && !lhs.text.is_empty() && lhs.text != "/* missing */"
}

fn class_assignment_target(value: &DecompExpr) -> String { if value.kind == "string" { unquote_text(&value.text) } else { value.text.clone() } }

fn constructor_arg(value: &DecompExpr) -> String {
    if is_object_name_expr(value) { return value.text.clone(); }
    if looks_like_gui_object_name(&value.text) { return quote(&value.text); }
    value.text.clone()
}

fn looks_like_gui_object_name(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(|character| ".[]() @".contains(character)) && (value.contains('_') || value.chars().next().is_some_and(|character| character.is_ascii_uppercase()))
}

fn is_extent_placeholder(_: &str) -> bool { false }

fn recover_forward_if_goto_loops(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let init_line = lines[index].trim().to_string();
        if !init_line.ends_with(';') || !init_line.contains(" = ") || index + 1 >= lines.len() || parse_line_indent(&lines[index]) != parse_line_indent(&lines[index + 1]) {
            output.push(lines[index].clone()); index += 1; continue;
        }
        let Some(condition) = parse_block_if_line(&lines[index + 1]) else { output.push(lines[index].clone()); index += 1; continue; };
        let Some(block_end) = matching_block_end(&lines, index + 1) else { output.push(lines[index].clone()); index += 1; continue; };
        let body = &lines[index + 2..block_end];
        if body.len() < 2 || !is_goto_line(body.last().unwrap().trim()) { output.push(lines[index].clone()); index += 1; continue; }
        let Some((inc_var, increment)) = parse_loop_increment(&body[body.len() - 2]) else { output.push(lines[index].clone()); index += 1; continue; };
        if !init_line.starts_with(&(inc_var.clone() + " = ")) || !condition.contains(&inc_var) { output.push(lines[index].clone()); index += 1; continue; }
        let label = body.last().unwrap().trim().strip_prefix("goto label_").and_then(|value| value.strip_suffix(';')).and_then(|value| value.parse::<usize>().ok());
        let Some(label) = label else { output.push(lines[index].clone()); index += 1; continue; };
        let indent = parse_line_indent(&lines[index]);
        output.push(format!("{}for ({}; {}; {}) {{", " ".repeat(indent), init_line.trim_end_matches(';'), condition, increment));
        output.extend(replace_goto_target(&body[..body.len() - 2], label, "continue;"));
        output.push(format!("{}}}", " ".repeat(indent)));
        index = block_end + 1;
    }
    output
}

fn parse_block_if_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with("if (") && trimmed.ends_with(") {") { Some(trimmed[4..trimmed.len() - 3].to_string()) } else { None }
}

fn recover_inverted_if_goto_loops(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let init_line = lines[index].trim().to_string();
        if !init_line.ends_with(';') || !init_line.contains(" = ") {
            output.push(lines[index].clone()); index += 1; continue;
        }
        let indent = parse_line_indent(&lines[index]);
        let parts: Vec<&str> = init_line.trim_end_matches(';').splitn(2, " = ").collect();
        let loop_var = parts[0];
        if loop_var.is_empty() { output.push(lines[index].clone()); index += 1; continue; }
        let mut condition_index = None;
        let mut condition = String::new();
        let limit = (index + 8).min(lines.len().saturating_sub(1));
        for candidate in index + 1..=limit {
            if parse_line_indent(&lines[candidate]) != indent { continue; }
            if let Some(value) = parse_block_if_line(&lines[candidate]) {
                if value.starts_with("!(") && value.ends_with(')') && value.contains(loop_var) {
                    condition_index = Some(candidate); condition = value[2..value.len() - 1].to_string(); break;
                }
            }
        }
        let Some(condition_index) = condition_index else { output.push(lines[index].clone()); index += 1; continue; };
        let Some(block_end) = matching_block_end(&lines, condition_index) else { output.push(lines[index].clone()); index += 1; continue; };
        let mut increment_index = None;
        let mut increment = String::new();
        let limit = (block_end + 12).min(lines.len().saturating_sub(2));
        for candidate in block_end + 1..=limit {
            if let Some((var, value)) = parse_loop_increment(&lines[candidate]) {
                if var == loop_var && is_goto_line(lines[candidate + 1].trim()) { increment_index = Some(candidate); increment = value; break; }
            }
        }
        let Some(increment_index) = increment_index else { output.push(lines[index].clone()); index += 1; continue; };
        output.push(format!("{}for ({}; {}; {}) {{", " ".repeat(indent), init_line.trim_end_matches(';'), condition, increment));
        for line in &lines[index + 1..condition_index] { output.push(reindent_block_line(line, indent, indent + 2)); }
        output.extend_from_slice(&lines[condition_index + 1..block_end]);
        output.extend_from_slice(&lines[block_end + 1..increment_index]);
        output.push(format!("{}}}", " ".repeat(indent)));
        index = increment_index + 2;
    }
    output
}

fn recover_loop_goto_continues(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if !(trimmed.starts_with("for ") || trimmed.starts_with("while ")) || !trimmed.ends_with('{') {
            output.push(lines[index].clone()); index += 1; continue;
        }
        let Some(end) = matching_block_end(&lines, index) else { output.push(lines[index].clone()); index += 1; continue; };
        output.push(lines[index].clone());
        let body = recover_loop_goto_continues(lines[index + 1..end].to_vec());
        let mut body_index = 0usize;
        while body_index < body.len() {
            if let Some((converted, next)) = convert_empty_if_to_continue(&body, body_index) {
                output.extend(converted); body_index = next + 1;
            } else {
                output.extend(convert_goto_to_continue(&body[body_index])); body_index += 1;
            }
        }
        output.push(lines[end].clone());
        index = end + 1;
    }
    output
}

fn convert_goto_to_continue(line: &str) -> Vec<String> {
    let indent = parse_line_indent(line);
    let trimmed = line.trim();
    if is_goto_line(trimmed) { return vec![format!("{}continue;", " ".repeat(indent))]; }
    let Some((condition, _, _, _)) = parse_goto_if_line(line) else { return vec![line.to_string()]; };
    vec![format!("{}if ({}) {{", " ".repeat(indent), condition), format!("{}  continue;", " ".repeat(indent)), format!("{}}}", " ".repeat(indent))]
}

fn convert_empty_if_to_continue(lines: &[String], index: usize) -> Option<(Vec<String>, usize)> {
    if index + 1 >= lines.len() || lines[index + 1].trim() != "}" || parse_line_indent(&lines[index + 1]) != parse_line_indent(&lines[index]) { return None; }
    let condition = parse_block_if_line(&lines[index])?;
    let indent = parse_line_indent(&lines[index]);
    Some((vec![format!("{}if ({}) {{", " ".repeat(indent), condition), format!("{}  continue;", " ".repeat(indent)), format!("{}}}", " ".repeat(indent))], index + 1))
}

fn recover_while_loop(body: &[String], condition: &str, pc: usize, indent: usize) -> Option<Vec<String>> {
    if body.is_empty() { return None; }
    let goto_line = body.last()?.trim();
    let label = goto_line.strip_prefix("goto label_")?.strip_suffix(';')?.parse::<usize>().ok()?;
    if label > pc || pc - label > 16 { return None; }
    let body = fill_empty_loop_exit_ifs(&body[..body.len() - 1]);
    let mut result = vec![format!("{}while ({}) {{", pad(indent), condition)];
    result.extend(body);
    result.push(format!("{}}}", pad(indent)));
    Some(result)
}

fn fill_empty_loop_exit_ifs(lines: &[String]) -> Vec<String> {
    let mut output = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        output.push(line.clone());
        if line.trim_end().ends_with('{') && index + 1 < lines.len() && lines[index + 1].trim() == "}" {
            output.push(format!("{}break;", " ".repeat(parse_line_indent(line) + 2)));
        }
    }
    output
}

fn recover_forward_goto_guards(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let Some((condition, _, indent, _)) = parse_goto_if_line(&lines[index]) else { output.push(lines[index].clone()); index += 1; continue; };
        if index + 1 >= lines.len() { output.push(lines[index].clone()); index += 1; continue; }
        if let Some(block_end) = forward_guard_block_end(&lines, index + 1, indent) {
            output.push(format!("{}if (!({})) {{", " ".repeat(indent), condition));
            output.extend(recover_forward_goto_guards(lines[index + 1..block_end].to_vec()).into_iter().map(|line| reindent_block_line(&line, indent, indent + 2)));
            output.push(format!("{}}}", " ".repeat(indent)));
            index = block_end; continue;
        }
        if !is_simple_statement_line(&lines[index + 1], indent) { output.push(lines[index].clone()); index += 1; continue; }
        output.push(format!("{}if (!({})) {{", " ".repeat(indent), condition));
        output.push(format!("{}{}", " ".repeat(indent + 2), lines[index + 1].trim()));
        output.push(format!("{}}}", " ".repeat(indent)));
        index += 2;
    }
    output
}

fn recover_forward_goto_guard_chains(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let Some((condition, target, indent, _)) = parse_goto_if_line(&lines[index]) else { output.push(lines[index].clone()); index += 1; continue; };
        let mut conditions = vec![condition];
        let mut next = index + 1;
        while next < lines.len() {
            if let Some((condition, next_target, next_indent, _)) = parse_goto_if_line(&lines[next]) {
                if next_target == target && next_indent == indent { conditions.push(condition); next += 1; continue; }
            }
            break;
        }
        if conditions.len() < 2 || next >= lines.len() || parse_line_indent(&lines[next]) != indent || !lines[next].trim_end().ends_with('{') {
            output.push(lines[index].clone()); index += 1; continue;
        }
        let Some(end) = matching_block_end(&lines, next) else { output.push(lines[index].clone()); index += 1; continue; };
        for (offset, condition) in conditions.iter().enumerate() { output.push(format!("{}if (!({})) {{", " ".repeat(indent + offset * 2), condition)); }
        for line in &lines[next..=end] { output.push(reindent_block_line(line, indent, indent + conditions.len() * 2)); }
        for offset in (0..conditions.len()).rev() { output.push(format!("{}}}", " ".repeat(indent + offset * 2))); }
        index = end + 1;
    }
    output
}

fn recover_forward_goto_guards_fixed_point(mut lines: Vec<String>) -> Vec<String> {
    for _ in 0..4 {
        let next = recover_forward_goto_guards(lines.clone());
        if next.join("\n") == lines.join("\n") { return next; }
        lines = next;
    }
    lines
}

fn recover_loop_goto_tails(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        if index + 1 >= lines.len() {
            output.push(lines[index].clone()); index += 1; continue;
        }
        let Some((condition, target, indent, _)) = parse_goto_if_line(&lines[index]) else { output.push(lines[index].clone()); index += 1; continue; };
        if lines[index + 1].trim() != format!("goto label_{};", target) {
            output.push(lines[index].clone()); index += 1; continue;
        }
        let sleep_index = index.checked_sub(1);
        let Some(sleep_index) = sleep_index else { output.push(lines[index].clone()); index += 1; continue; };
        if sleep_index < 1 || parse_line_indent(&lines[sleep_index]) != indent || !lines[sleep_index].trim().starts_with("sleep(") {
            output.push(lines[index].clone()); index += 1; continue;
        }
        let mut body_start = None;
        for candidate in (0..sleep_index).rev() {
            if parse_line_indent(&lines[candidate]) != indent || !lines[candidate].trim_end().ends_with('{') { continue; }
            if matching_block_end(&lines, candidate) == Some(sleep_index - 1) { body_start = Some(candidate); break; }
        }
        let Some(body_start) = body_start else { output.push(lines[index].clone()); index += 1; continue; };
        output.truncate(output.len().saturating_sub(index - body_start));
        output.push(format!("{}while (true) {{", " ".repeat(indent)));
        output.extend_from_slice(&lines[body_start..=sleep_index]);
        output.push(format!("{}if (!({})) {{", " ".repeat(indent + 2), condition));
        output.push(format!("{}break;", " ".repeat(indent + 4)));
        output.push(format!("{}}}", " ".repeat(indent + 2)));
        output.push(format!("{}}}", " ".repeat(indent)));
        index += 2;
    }
    output
}

fn recover_infinite_goto_loops(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let indent = parse_line_indent(&lines[index]);
        if lines[index].trim() != "if (true) {" {
            output.push(lines[index].clone()); index += 1; continue;
        }
        let Some(end) = matching_block_end(&lines, index) else { output.push(lines[index].clone()); index += 1; continue; };
        let body = recover_infinite_goto_loops(lines[index + 1..end].to_vec());
        if body.len() < 2 {
            output.extend_from_slice(&lines[index..=end]); index = end + 1; continue;
        }
        let Some((condition, target, _, _)) = parse_goto_if_line(&body[body.len() - 2]) else {
            output.push(lines[index].clone()); output.extend(body); output.push(lines[end].clone()); index = end + 1; continue;
        };
        if body[body.len() - 1].trim() != format!("goto label_{};", target) {
            output.push(lines[index].clone()); output.extend(body); output.push(lines[end].clone()); index = end + 1; continue;
        }
        output.push(format!("{}while (true) {{", " ".repeat(indent)));
        output.extend_from_slice(&body[..body.len() - 2]);
        output.push(format!("{}if (!({})) {{", " ".repeat(indent + 2), condition));
        output.push(format!("{}break;", " ".repeat(indent + 4)));
        output.push(format!("{}}}", " ".repeat(indent + 2)));
        output.push(format!("{}}}", " ".repeat(indent)));
        index = end + 1;
    }
    output
}

fn recover_sleep_loop_blocks(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let indent = parse_line_indent(&lines[index]);
        if lines[index].trim() != "if (true) {" {
            output.push(lines[index].clone()); index += 1; continue;
        }
        let Some(end) = matching_block_line(&lines, index) else { output.push(lines[index].clone()); index += 1; continue; };
        if let Some(recovered) = recover_sleep_loop_body(&lines[index + 1..end], indent) {
            output.extend(recovered); index = end + 1;
        } else { output.push(lines[index].clone()); index += 1; }
    }
    output
}

fn recover_sleep_loop_body(body: &[String], indent: usize) -> Option<Vec<String>> {
    if body.len() < 4 { return None; }
    let if_line = body[body.len() - 4].trim();
    let sleep_line = body[body.len() - 3].trim();
    let goto_line = body[body.len() - 2].trim();
    let close_line = body[body.len() - 1].trim();
    let condition = parse_if_open_condition(if_line)?;
    if !sleep_line.starts_with("sleep(") || !is_goto_line(goto_line) || close_line != "}" { return None; }
    let mut output = vec![format!("{}while (true) {{", " ".repeat(indent))];
    output.extend_from_slice(&body[..body.len() - 4]);
    output.push(format!("{}if ({}) {{", " ".repeat(indent + 2), condition));
    output.push(format!("{}break;", " ".repeat(indent + 4)));
    output.push(format!("{}}}", " ".repeat(indent + 2)));
    output.push(format!("{}{}", " ".repeat(indent + 2), sleep_line));
    output.push(format!("{}}}", " ".repeat(indent)));
    Some(output)
}

fn parse_if_open_condition(line: &str) -> Option<String> {
    (line.starts_with("if (") && line.ends_with(") {")).then(|| line[4..line.len() - 3].to_string())
}

fn matching_block_line(lines: &[String], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    for index in start..lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.ends_with('{') { depth += 1; }
        if trimmed == "}" { depth -= 1; if depth == 0 { return Some(index); } }
    }
    None
}

fn forward_guard_block_end(lines: &[String], start: usize, indent: usize) -> Option<usize> {
    if start >= lines.len() || parse_line_indent(&lines[start]) != indent || !lines[start].trim_end().ends_with('{') { return None; }
    let end = matching_block_end(lines, start)?;
    (end > start).then_some(end + 1)
}

fn parse_goto_if_line(line: &str) -> Option<(String, usize, usize, bool)> {
    let indent = parse_line_indent(line);
    let trimmed = line.trim();
    if !trimmed.starts_with("if (") || !trimmed.contains(") goto label_") || !trimmed.ends_with(';') { return None; }
    let pivot = trimmed.rfind(") goto label_")?;
    if pivot < 4 { return None; }
    let label = trimmed[pivot + 13..].strip_suffix(';')?.parse::<usize>().ok()?;
    Some((trimmed[4..pivot].to_string(), label, indent, true))
}

fn is_simple_statement_line(line: &str, indent: usize) -> bool {
    if parse_line_indent(line) != indent { return false; }
    let trimmed = line.trim();
    trimmed.ends_with(';') && !trimmed.starts_with("goto label_") && !trimmed.starts_with("if ") && !trimmed.starts_with("for ")
}

fn parse_line_indent(line: &str) -> usize { line.len() - line.trim_start_matches(' ').len() }

fn matching_block_end(lines: &[String], open_index: usize) -> Option<usize> {
    let mut depth = 0i32;
    for index in open_index..lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.ends_with('{') { depth += 1; }
        if trimmed.starts_with('}') { depth -= 1; if depth == 0 { return Some(index); } }
    }
    None
}

fn reindent_block_line(line: &str, source_field_indent: usize, target_field_indent: usize) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() { return String::new(); }
    let extra = parse_line_indent(line).saturating_sub(source_field_indent);
    format!("{}{}", " ".repeat(target_field_indent + extra), trimmed)
}

fn recover_forward_dispatch(code: &[Instruction], pc: usize, target: usize, end: usize, indent: usize, state: &mut DecompileState) -> Option<(Vec<String>, usize)> {
    if target <= pc + 1 || target >= end { return None; }
    let (cases, tail, selector) = parse_forward_dispatch_cases(code, pc, target, end, state)?;
    if cases.is_empty() { return None; }
    let mut targets: Vec<usize> = cases.iter().filter_map(|case| (case.target > pc && case.target < end).then_some(case.target)).collect();
    targets.sort_unstable(); targets.dedup();
    if targets.is_empty() { return None; }
    let mut target_to_next = HashMap::new();
    for (index, current) in targets.iter().copied().enumerate() {
        let mut next = if index + 1 < targets.len() { targets[index + 1] } else if current >= tail { end } else { target };
        if current < target && next > target { next = target; }
        if current >= tail { next = case_body_end(code, current, next); }
        target_to_next.insert(current, next);
    }
    let (common_end, has_common_end) = forward_dispatch_common_end(code, &targets, target);
    let mut lines = Vec::new();
    if selector_needs_binding(&selector) { lines.push(format!("{}temp.switchvalue = {};", pad(indent), selector)); }
    let mut max_end = tail;
    for (index, case) in cases.iter().enumerate() {
        let body_end = *target_to_next.get(&case.target)?;
        if body_end <= case.target { return None; }
        max_end = max_end.max(body_end);
        let mut body = remove_duplicate_gotos(decompile_range_with_state(code, case.target, body_end, indent + 1, state));
        if has_common_end { body = trim_trailing_goto(body, common_end); }
        if case.condition.is_empty() { lines.push(format!("{}else {{", pad(indent))); }
        else if index == 0 { lines.push(format!("{}if ({}) {{", pad(indent), case.condition)); }
        else { lines.push(format!("{}else if ({}) {{", pad(indent), case.condition)); }
        lines.extend(body);
        lines.push(format!("{}}}", pad(indent)));
    }
    Some((lines, max_end.saturating_sub(1)))
}

fn parse_forward_dispatch_cases(code: &[Instruction], pc: usize, target: usize, end: usize, state: &mut DecompileState) -> Option<(Vec<DispatchCase>, usize, String)> {
    let (selector, mut position) = dispatch_selector(code, target, state)?;
    let condition_selector = if selector_needs_binding(&selector) { "temp.switchvalue".to_string() } else { selector.clone() };
    let mut cases = Vec::new();
    while position + 4 < end {
        if code[position].op != OP_COPY || code[position + 2].op != OP_EQUAL { break; }
        let literal = dispatch_literal(&code[position + 1])?;
        let jump = &code[position + 3];
        if jump.op != OP_JEQ && jump.op != OP_JNE { break; }
        let case_target = jump_target(jump);
        if case_target <= pc || case_target >= end { break; }
        let operator = if jump.op == OP_JNE { "!=" } else { "==" };
        cases.push(DispatchCase { condition: format!("{} {} {}", condition_selector, operator, literal), target: case_target });
        position += 4;
    }
    if position < end && code[position].op == OP_JMP {
        let default_target = jump_target(&code[position]);
        if default_target > pc && default_target < target { cases.push(DispatchCase { condition: String::new(), target: default_target }); position += 1; }
    }
    if position < end && code[position].op == OP_POP { position += 1; }
    Some((cases, position, selector))
}

fn selector_needs_binding(selector: &str) -> bool { selector.contains('(') }

fn dispatch_literal(instruction: &Instruction) -> Option<String> {
    let operand = instruction.operand.as_ref()?;
    match instruction.op {
        OP_PUSH_STRING => Some(quote(&operand.str_value)),
        OP_PUSH_NUMBER => Some(number_text(Some(operand))),
        OP_PUSH_VARIABLE => Some(variable_name(&operand.str_value)),
        _ => None,
    }
}

fn forward_dispatch_common_end(code: &[Instruction], targets: &[usize], dispatch_start: usize) -> (usize, bool) {
    let mut common_end = None;
    for (index, target) in targets.iter().copied().enumerate() {
        let limit = targets.get(index + 1).copied().unwrap_or(dispatch_start);
        let end_jump = (target..limit.min(code.len())).find_map(|position| (code[position].op == OP_JMP && jump_target(&code[position]) >= dispatch_start).then(|| jump_target(&code[position])));
        let Some(end_jump) = end_jump else { continue; };
        if let Some(previous) = common_end { if previous != end_jump { return (0, false); } } else { common_end = Some(end_jump); }
    }
    common_end.map_or((0, false), |value| (value, true))
}

fn case_body_end(code: &[Instruction], start: usize, mut limit: usize) -> usize {
    limit = limit.min(code.len());
    for position in start.min(code.len())..limit {
        if code[position].op == OP_RET || code[position].op == OP_JMP { return position + 1; }
    }
    limit
}

fn recover_backward_dispatch(code: &[Instruction], pc: usize, target: usize, end: usize, indent: usize, state: &mut DecompileState) -> Option<(Vec<String>, usize)> {
    if target <= pc || target >= end { return None; }
    let (cases, tail) = parse_backward_dispatch_cases(code, pc, target, end, state)?;
    if cases.is_empty() { return None; }
    let (common_end, ok) = dispatch_common_end(code, &cases, target);
    if !ok || common_end <= target || common_end > end { return None; }
    let mut targets: Vec<usize> = cases.iter().map(|case| case.target).collect();
    targets.sort_unstable();
    let target_to_next: HashMap<usize, usize> = targets.iter().enumerate().map(|(index, target)| (*target, targets.get(index + 1).copied().unwrap_or(*target))).collect();
    let mut lines = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let body_end = *target_to_next.get(&case.target)?;
        if body_end <= case.target { return None; }
        let mut body = remove_duplicate_gotos(decompile_range_with_state(code, case.target, body_end, indent + 1, state));
        body = trim_trailing_goto(body, common_end);
        lines.push(if index == 0 { format!("{}if ({}) {{", pad(indent), case.condition) } else { format!("{}else if ({}) {{", pad(indent), case.condition) });
        lines.extend(body);
        lines.push(format!("{}}}", pad(indent)));
    }
    Some((lines, skip_dispatch_tail(code, tail, common_end, end)))
}

fn parse_backward_dispatch_cases(code: &[Instruction], pc: usize, target: usize, end: usize, state: &mut DecompileState) -> Option<(Vec<DispatchCase>, usize)> {
    let (selector, mut position) = dispatch_selector(code, target, state)?;
    let mut cases = Vec::new();
    while position + 4 < end {
        if code[position].op != OP_COPY || code[position + 2].op != OP_EQUAL { break; }
        let literal = dispatch_literal(&code[position + 1])?;
        let jump = &code[position + 3];
        if jump.op != OP_JEQ && jump.op != OP_JNE { break; }
        let case_target = jump_target(jump);
        if cases.is_empty() && case_target > target { position += 4; continue; }
        if case_target <= pc || case_target >= target { break; }
        let operator = if jump.op == OP_JNE { "!=" } else { "==" };
        cases.push(DispatchCase { condition: format!("{} {} {}", selector, operator, literal), target: case_target });
        position += 4;
    }
    if cases.is_empty() { return None; }
    if position < end && code[position].op == OP_POP { position += 1; }
    Some((cases, position))
}

fn dispatch_selector(code: &[Instruction], target: usize, state: &mut DecompileState) -> Option<(String, usize)> {
    let mut stack: Vec<DecompExpr> = Vec::new();
    for position in target..code.len() {
        let instruction = &code[position];
        if instruction.op == OP_COPY {
            if stack.len() == 1 { return Some((stack[0].text.clone(), position)); }
            return None;
        }
        match instruction.op {
            OP_PUSH_ARRAY => stack.push(DecompExpr { marker: true, ..DecompExpr::default() }),
            OP_PUSH_VARIABLE => stack.push(DecompExpr { text: variable_name(&instruction.operand.as_ref()?.str_value), ..DecompExpr::default() }),
            OP_PUSH_STRING => stack.push(DecompExpr { text: quote(&instruction.operand.as_ref()?.str_value), kind: "string".to_string(), ..DecompExpr::default() }),
            OP_PUSH_NUMBER => stack.push(DecompExpr { text: number_text(instruction.operand.as_ref()), ..DecompExpr::default() }),
            OP_THIS => stack.push(DecompExpr { text: "this".to_string(), ..DecompExpr::default() }),
            OP_THIS_O => stack.push(DecompExpr { text: "thiso".to_string(), ..DecompExpr::default() }),
            OP_TEMP => stack.push(DecompExpr { text: "temp".to_string(), ..DecompExpr::default() }),
            OP_PLAYER => stack.push(DecompExpr { text: "player".to_string(), ..DecompExpr::default() }),
            OP_PLAYER_O => stack.push(DecompExpr { text: "playero".to_string(), ..DecompExpr::default() }),
            OP_LEVEL => stack.push(DecompExpr { text: "level".to_string(), ..DecompExpr::default() }),
            OP_PARAMS => stack.push(DecompExpr { text: "params".to_string(), ..DecompExpr::default() }),
            OP_GET_REGISTER => { let id = operand_number(instruction); stack.push(state.registers.get(&id).cloned().unwrap_or(DecompExpr { text: format!("reg{}", id), ..DecompExpr::default() })); }
            OP_CONVERT_FLOAT | OP_CONVERT_STRING | OP_CONVERT_OBJECT | OP_CONVERT_VAR | OP_END_PARAMS => {}
            OP_CALL => { let call = build_call(&mut stack); stack.push(DecompExpr { text: call, kind: "call".to_string(), ..DecompExpr::default() }); }
            OP_OBJ_SUBSTRING => { let value = object_call(&mut stack, "substring", 2, false); stack.push(value); }
            OP_OBJ_TOKENIZE => { let value = object_call(&mut stack, "tokenize", 1, false); stack.push(value); }
            OP_INT => { let value = function_call(&mut stack, "int", 1); stack.push(value); }
            OP_RANDOM => { let value = function_call(&mut stack, "random", 2); stack.push(value); }
            OP_ADD | OP_SUBTRACT | OP_MULTIPLY | OP_DIVIDE | OP_MODULO | OP_POWER | OP_BOOL_AND | OP_BOOL_OR | OP_EQUAL | OP_NOT_EQUAL | OP_LESS_THAN | OP_GREATER_THAN | OP_LE | OP_GE | OP_BITWISE_OR | OP_BITWISE_AND | OP_BITWISE_XOR | OP_SHIFT_LEFT | OP_SHIFT_RIGHT | OP_IN | OP_JOIN | OP_APPEND => { let rhs = pop_expr(&mut stack); let lhs = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("{} {} {}", lhs.text, infix(instruction.op), rhs.text), ..DecompExpr::default() }); }
            OP_ACCESS_MEMBER => { let rhs = pop_expr(&mut stack); let lhs = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("{}.{}", member_base(&lhs.text), member_name(&rhs.text)), ..DecompExpr::default() }); }
            OP_ARRAY_ACCESS => { let index = pop_expr(&mut stack); let array = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("{}[{}]", array.text, index.text), ..DecompExpr::default() }); }
            _ => return None,
        }
    }
    None
}

fn dispatch_common_end(code: &[Instruction], cases: &[DispatchCase], dispatch_start: usize) -> (usize, bool) {
    let mut common_end = None;
    for case in cases {
        let limit = cases.iter().filter(|other| other.target > case.target && other.target < dispatch_start).map(|other| other.target).min().unwrap_or(dispatch_start);
        let end_jump = (case.target..limit.min(code.len())).find_map(|position| (code[position].op == OP_JMP && jump_target(&code[position]) >= dispatch_start).then(|| jump_target(&code[position])));
        let Some(end_jump) = end_jump else { return (0, false); };
        if let Some(previous) = common_end { if previous != end_jump { return (0, false); } } else { common_end = Some(end_jump); }
    }
    common_end.map_or((0, false), |value| (value, true))
}

fn trim_trailing_goto(mut body: Vec<String>, target: usize) -> Vec<String> {
    if body.last().is_some_and(|line| line.trim() == format!("goto label_{};", target)) { body.pop(); }
    body
}

fn skip_dispatch_tail(code: &[Instruction], tail: usize, common_end: usize, end: usize) -> usize {
    let mut position = tail;
    if position + 1 < end && code.get(position).is_some_and(|instruction| instruction.op == OP_PUSH_NUMBER && operand_number(instruction) == 0) && code.get(position + 1).is_some_and(|instruction| instruction.op == OP_RET) { position += 2; }
    if common_end > position {
        position = common_end;
        if code.get(position).is_some_and(|instruction| instruction.op == OP_POP) { position += 1; }
        if position + 1 < end && code.get(position).is_some_and(|instruction| instruction.op == OP_PUSH_NUMBER && operand_number(instruction) == 0) && code.get(position + 1).is_some_and(|instruction| instruction.op == OP_RET) { position += 2; }
    }
    position.saturating_sub(1)
}

fn recover_ternary_assignment(code: &[Instruction], pc: usize, target: usize, end: usize, indent: usize, state: &mut DecompileState, condition: &str, branch_op: u8, stack: &[DecompExpr]) -> Option<(Vec<String>, usize)> {
    if stack.is_empty() || target <= pc + 1 || target >= end || code.get(target - 1)?.op != OP_JMP { return None; }
    let common = jump_target(code.get(target - 1)?);
    if common <= target || common >= end || code.get(common)?.op != OP_ASSIGN { return None; }
    let mut true_value = eval_expr_range(code, pc + 1, target - 1, state)?;
    let mut false_value = eval_expr_range(code, target, common, state)?;
    if branch_op == OP_JEQ { std::mem::swap(&mut true_value, &mut false_value); }
    let lhs = stack.last()?.text.clone();
    Some((vec![format!("{}if ({}) {{", pad(indent), condition), format!("{}{} = {};", pad(indent + 1), lhs, true_value), format!("{}}}", pad(indent)), format!("{}else {{", pad(indent)), format!("{}{} = {};", pad(indent + 1), lhs, false_value), format!("{}}}", pad(indent))], common))
}

fn recover_self_ternary_assignment(code: &[Instruction], pc: usize, target: usize, end: usize, indent: usize, state: &mut DecompileState, condition: &str, branch_op: u8, stack: &[DecompExpr]) -> Option<(Vec<String>, usize)> {
    if stack.is_empty() || target <= pc + 1 || target >= end || code.get(target)?.op != OP_ASSIGN { return None; }
    let mut false_value = eval_expr_range(code, pc + 1, target, state)?;
    let mut true_value = condition.to_string();
    if branch_op == OP_JEQ { std::mem::swap(&mut true_value, &mut false_value); }
    let lhs = stack.last()?.text.clone();
    Some((vec![format!("{}if ({}) {{", pad(indent), condition), format!("{}{} = {};", pad(indent + 1), lhs, true_value), format!("{}}}", pad(indent)), format!("{}else {{", pad(indent)), format!("{}{} = {};", pad(indent + 1), lhs, false_value), format!("{}}}", pad(indent))], target))
}

fn recover_ternary_expression(code: &[Instruction], pc: usize, target: usize, end: usize, state: &mut DecompileState, condition: &str, branch_op: u8) -> Option<(String, usize)> {
    if target <= pc + 1 || target >= end || code.get(target - 1)?.op != OP_JMP { return None; }
    let common = jump_target(code.get(target - 1)?);
    if common <= target || common > end { return None; }
    let mut true_value = eval_expr_range(code, pc + 1, target - 1, state)?;
    let mut false_value = eval_expr_range(code, target, common, state)?;
    if branch_op == OP_JEQ { std::mem::swap(&mut true_value, &mut false_value); }
    Some((format!("({} ? {} : {})", condition, true_value, false_value), common - 1))
}

fn recover_conditional_assignment_chain(code: &[Instruction], pc: usize, target: usize, end: usize, indent: usize, state: &mut DecompileState, first_condition: &str, first_op: u8, stack: &[DecompExpr]) -> Option<(Vec<String>, usize)> {
    if first_op != OP_JNE || stack.is_empty() || target != pc + 3 || pc + 2 >= end || code.get(pc + 2)?.op != OP_JMP { return None; }
    let lhs = stack.last()?.text.clone();
    let common = jump_target(code.get(pc + 2)?);
    if common <= target || common >= end || code.get(common)?.op != OP_ASSIGN { return None; }
    let value = eval_expr_range(code, pc + 1, pc + 2, state)?;
    let mut cases = vec![ConditionalAssignmentCase { condition: first_condition.to_string(), value }];
    let mut position = target;
    let mut default_value = None;
    while position < common {
        let mut branch = None;
        let limit = (position + 12).min(common);
        for index in position..limit.saturating_sub(2) {
            if code[index].op == OP_JNE && jump_target(&code[index]) == index + 3 && code[index + 2].op == OP_JMP && jump_target(&code[index + 2]) == common { branch = Some(index); break; }
        }
        let Some(branch) = branch else {
            default_value = Some(eval_expr_range(code, position, common, state)?);
            break;
        };
        let condition = eval_expr_range(code, position, branch, state)?;
        let value = eval_expr_range(code, branch + 1, branch + 2, state)?;
        cases.push(ConditionalAssignmentCase { condition, value });
        position = jump_target(&code[branch]);
    }
    let default_value = default_value?;
    let mut lines = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        lines.push(if index == 0 { format!("{}if ({}) {{", pad(indent), case.condition) } else { format!("{}else if ({}) {{", pad(indent), case.condition) });
        lines.push(format!("{}{} = {};", pad(indent + 1), lhs, case.value));
        lines.push(format!("{}}}", pad(indent)));
    }
    lines.push(format!("{}else {{", pad(indent)));
    lines.push(format!("{}{} = {};", pad(indent + 1), lhs, default_value));
    lines.push(format!("{}}}", pad(indent)));
    Some((lines, common))
}

fn eval_expr_range(code: &[Instruction], start: usize, end: usize, state: &mut DecompileState) -> Option<String> {
    let mut stack = Vec::new();
    let mut pc = start;
    while pc < end && pc < code.len() {
        let instruction = &code[pc];
        match instruction.op {
            OP_PUSH_ARRAY => stack.push(DecompExpr { marker: true, ..DecompExpr::default() }),
            OP_PUSH_STRING => stack.push(DecompExpr { text: quote(&instruction.operand.as_ref()?.str_value), kind: "string".to_string(), ..DecompExpr::default() }),
            OP_PUSH_VARIABLE => stack.push(DecompExpr { text: variable_name(&instruction.operand.as_ref()?.str_value), ..DecompExpr::default() }),
            OP_PUSH_NUMBER => stack.push(DecompExpr { text: number_text(instruction.operand.as_ref()), ..DecompExpr::default() }),
            OP_PUSH_TRUE => stack.push(DecompExpr { text: "true".to_string(), ..DecompExpr::default() }),
            OP_PUSH_FALSE => stack.push(DecompExpr { text: "false".to_string(), ..DecompExpr::default() }),
            OP_PUSH_NULL => stack.push(DecompExpr { text: "null".to_string(), ..DecompExpr::default() }),
            OP_PI => stack.push(DecompExpr { text: "pi".to_string(), ..DecompExpr::default() }),
            OP_THIS => stack.push(DecompExpr { text: "this".to_string(), ..DecompExpr::default() }),
            OP_THIS_O => stack.push(DecompExpr { text: "thiso".to_string(), ..DecompExpr::default() }),
            OP_TEMP => stack.push(DecompExpr { text: "temp".to_string(), ..DecompExpr::default() }),
            OP_PLAYER => stack.push(DecompExpr { text: "player".to_string(), ..DecompExpr::default() }),
            OP_PLAYER_O => stack.push(DecompExpr { text: "playero".to_string(), ..DecompExpr::default() }),
            OP_LEVEL => stack.push(DecompExpr { text: "level".to_string(), ..DecompExpr::default() }),
            OP_PARAMS => stack.push(DecompExpr { text: "params".to_string(), ..DecompExpr::default() }),
            OP_GET_REGISTER => { let id = operand_number(instruction); stack.push(state.registers.get(&id).cloned().unwrap_or(DecompExpr { text: format!("reg{}", id), ..DecompExpr::default() })); }
            OP_JNE | OP_JEQ => {
                let target = jump_target(instruction); let condition = pop_expr(&mut stack).text;
                let (value, new_pc) = recover_ternary_expression(code, pc, target, end, state, &condition, instruction.op)?;
                stack.push(DecompExpr { text: value, ..DecompExpr::default() }); pc = new_pc;
            }
            OP_CONVERT_FLOAT | OP_CONVERT_STRING | OP_CONVERT_OBJECT | OP_CONVERT_VAR | OP_END_PARAMS | OP_SHORT_END => {}
            OP_END_ARRAY => { let args = collect_args(&mut stack); stack.push(DecompExpr { text: format!("{{{}}}", args.join(", ")), ..DecompExpr::default() }); }
            OP_ACCESS_MEMBER => { let property = pop_expr(&mut stack); let object = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("{}.{}", member_base(&object.text), member_name(&property.text)), ..DecompExpr::default() }); }
            OP_ARRAY_ACCESS => { let index = pop_expr(&mut stack); let array = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("{}[{}]", array.text, index.text), ..DecompExpr::default() }); }
            OP_ADD | OP_SUBTRACT | OP_MULTIPLY | OP_DIVIDE | OP_MODULO | OP_POWER | OP_BOOL_AND | OP_BOOL_OR | OP_EQUAL | OP_NOT_EQUAL | OP_LESS_THAN | OP_GREATER_THAN | OP_LE | OP_GE | OP_BITWISE_OR | OP_BITWISE_AND | OP_BITWISE_XOR | OP_SHIFT_LEFT | OP_SHIFT_RIGHT | OP_IN | OP_JOIN | OP_APPEND => { let rhs = pop_expr(&mut stack); let lhs = pop_expr(&mut stack); stack.push(DecompExpr { text: format!("{} {} {}", lhs.text, infix(instruction.op), rhs.text), ..DecompExpr::default() }); }
            _ => return None,
        }
        pc += 1;
    }
    (stack.len() == 1).then(|| stack[0].text.clone())
}

fn recover_tail_dispatch(code: &[Instruction], pc: usize, end: usize, indent: usize, state: &mut DecompileState) -> Option<(Vec<String>, usize)> {
    let (cases, tail) = parse_tail_dispatch_cases(code, pc, end, state)?;
    if cases.is_empty() { return None; }
    let (common_end, ok) = dispatch_common_end(code, &cases, pc);
    if !ok || common_end < pc || common_end > end { return None; }
    let mut targets: Vec<usize> = cases.iter().map(|case| case.target).collect();
    targets.sort_unstable(); targets.dedup();
    let target_to_next: HashMap<usize, usize> = targets.iter().enumerate().map(|(index, target)| (*target, targets.get(index + 1).copied().unwrap_or(pc))).collect();
    let mut lines = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let body_end = *target_to_next.get(&case.target)?;
        if body_end <= case.target { return None; }
        let mut body = remove_duplicate_gotos(decompile_range_with_state(code, case.target, body_end, indent + 1, state));
        body = trim_trailing_goto(body, common_end);
        lines.push(if index == 0 { format!("{}if ({}) {{", pad(indent), case.condition) } else { format!("{}else if ({}) {{", pad(indent), case.condition) });
        lines.extend(body);
        lines.push(format!("{}}}", pad(indent)));
    }
    Some((lines, skip_dispatch_tail(code, tail, common_end, end)))
}

fn parse_tail_dispatch_cases(code: &[Instruction], pc: usize, end: usize, state: &mut DecompileState) -> Option<(Vec<DispatchCase>, usize)> {
    let (selector, mut position) = dispatch_selector(code, pc, state)?;
    let mut cases = Vec::new();
    while position + 4 < end {
        if code[position].op != OP_COPY || code[position + 2].op != OP_EQUAL { break; }
        let literal = dispatch_literal(&code[position + 1])?;
        let jump = &code[position + 3];
        if jump.op != OP_JEQ && jump.op != OP_JNE { break; }
        let case_target = jump_target(jump);
        if case_target >= pc { position += 4; continue; }
        let operator = if jump.op == OP_JNE { "!=" } else { "==" };
        cases.push(DispatchCase { condition: format!("{} {} {}", selector, operator, literal), target: case_target });
        position += 4;
    }
    if cases.is_empty() { return None; }
    if position < end && code[position].op == OP_POP { position += 1; }
    Some((cases, position))
}

fn recover_profile_clone_blocks(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let Some((name, _base, indent)) = parse_profile_clone_assignment(&lines[index]) else { output.push(lines[index].clone()); index += 1; continue; };
        if index + 1 < lines.len() && lines[index + 1].trim() == format!("with ({}) {{", quote(&name)) {
            if let Some(block_end) = matching_block_end(&lines, index + 1) {
                if block_end > index + 1 && block_end + 1 < lines.len() && lines[block_end + 1].trim() == format!("addcontrol({});", quote(&name)) {
                    output.push(format!("{}new GuiControlProfile({}) {{", " ".repeat(indent), quote(&name)));
                    let source_indent = parse_line_indent(&lines[index + 1]) + 2;
                    let target_indent = indent + 2;
                    for line in &lines[index + 2..block_end] { output.push(reindent_block_line(line, source_indent, target_indent)); }
                    output.push(format!("{}}}", " ".repeat(indent)));
                    output.push(lines[block_end + 1].clone());
                    index = block_end + 2;
                    continue;
                }
            }
        }
        let mut add_index = None;
        for candidate in index + 1..lines.len() {
            let trimmed = lines[candidate].trim();
            if trimmed == format!("addcontrol({});", quote(&name)) { add_index = Some(candidate); break; }
            if parse_line_indent(&lines[candidate]) != indent || trimmed.ends_with('{') || trimmed.starts_with('}') { break; }
        }
        let Some(add_index) = add_index else { output.push(lines[index].clone()); index += 1; continue; };
        output.push(format!("{}new GuiControlProfile({}) {{", " ".repeat(indent), quote(&name)));
        for line in &lines[index + 1..add_index] { output.push(format!("{}{}", " ".repeat(indent), pad(1) + line.trim())); }
        output.push(format!("{}}}", " ".repeat(indent)));
        output.push(lines[add_index].clone());
        index = add_index + 1;
    }
    output
}

fn parse_profile_clone_assignment(line: &str) -> Option<(String, String, usize)> {
    let indent = parse_line_indent(line);
    let trimmed = line.trim();
    if !trimmed.ends_with(';') { return None; }
    let parts: Vec<&str> = trimmed.trim_end_matches(';').splitn(2, " = ").collect();
    if parts.len() != 2 || !is_quoted_profile_name(parts[0]) || !is_quoted_profile_name(parts[1]) { return None; }
    let name = unquote_text(parts[0]); let base = unquote_text(parts[1]);
    if !name.ends_with("Profile") || !base.ends_with("Profile") { return None; }
    Some((name, base, indent))
}

fn is_quoted_profile_name(value: &str) -> bool { value.starts_with('"') && value.ends_with('"') && !value.contains(" @ ") }

fn recover_bare_constructor_blocks(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let indent = parse_line_indent(&lines[index]);
        let trimmed = lines[index].trim().to_string();
        if !is_bare_gui_constructor_line(&trimmed) {
            output.push(lines[index].clone()); index += 1; continue;
        }
        let mut end = index + 1;
        while end < lines.len() && is_constructor_field_line(&lines[end], indent) { end += 1; }
        if end == index + 1 { output.push(lines[index].clone()); index += 1; continue; }
        output.push(format!("{}{} {{", " ".repeat(indent), trimmed.trim_end_matches(';')));
        for line in &lines[index + 1..end] { output.push(format!("{}{}{}", " ".repeat(indent), pad(1), line.trim())); }
        output.push(format!("{}}}", " ".repeat(indent)));
        index = end;
    }
    output
}

fn remove_named_gui_constructor_addcontrols(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        output.push(lines[index].clone());
        let Some(name) = named_gui_constructor_name(&lines[index]) else { index += 1; continue; };
        let Some(end) = matching_block_end(&lines, index) else { index += 1; continue; };
        if end + 1 >= lines.len() || lines[end + 1].trim() != format!("addcontrol({});", quote(&name)) { index += 1; continue; }
        output.extend_from_slice(&lines[index + 1..=end]);
        index = end + 2;
    }
    output
}

fn named_gui_constructor_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with("new Gui") || !trimmed.ends_with('{') { return None; }
    let (class, _) = constructor_expr_class(trimmed)?;
    if class == "GuiControlProfile" { return None; }
    let (argument, _) = constructor_expr_arg(trimmed)?;
    if !argument.starts_with('"') || !argument.ends_with('"') { return None; }
    Some(unquote_text(&argument))
}

fn is_bare_gui_constructor_line(line: &str) -> bool { line.starts_with("new Gui") && line.ends_with(");") && !line.contains('{') }

fn is_constructor_field_line(line: &str, indent: usize) -> bool {
    if parse_line_indent(line) != indent { return false; }
    let trimmed = line.trim();
    if !trimmed.ends_with(';') || trimmed.starts_with("new ") || trimmed.starts_with("addcontrol(") || trimmed.contains("goto label_") { return false; }
    if let Some(lhs) = trimmed.split(" = ").next() { return trimmed.contains(" = ") && !lhs.chars().any(|character| " (){}".contains(character)); }
    false
}

fn collapse_nested_ifs(mut lines: Vec<String>) -> Vec<String> {
    loop {
        let mut changed = false;
        let mut output = Vec::new();
        let mut index = 0usize;
        while index < lines.len() {
            let Some(condition1) = parse_if_line(&lines[index]) else { output.push(lines[index].clone()); index += 1; continue; };
            if index + 4 >= lines.len() || lines[index + 1].trim() != "{" { output.push(lines[index].clone()); index += 1; continue; }
            let Some(close_outer) = matching_close_brace(&lines, index + 1) else { output.push(lines[index].clone()); index += 1; continue; };
            let Some(condition2) = parse_if_line(&lines[index + 2]) else { output.push(lines[index].clone()); index += 1; continue; };
            if lines.get(index + 3).map(|line| line.trim()) != Some("{") || matching_close_brace(&lines, index + 3) != Some(close_outer - 1) { output.push(lines[index].clone()); index += 1; continue; }
            let indentation = leading_whitespace(&lines[index]);
            output.push(format!("{}if ({} && {})", indentation, condition1, condition2));
            output.push(lines[index + 1].clone());
            output.extend(unindent_once(&lines[index + 4..close_outer - 1]));
            output.push(lines[close_outer].clone());
            index = close_outer + 1;
            changed = true;
        }
        lines = output;
        if !changed { return lines; }
    }
}

fn parse_if_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    (trimmed.starts_with("if (") && trimmed.ends_with(')')).then(|| trimmed[4..trimmed.len() - 1].to_string())
}

fn matching_close_brace(lines: &[String], open_index: usize) -> Option<usize> {
    let mut depth = 0i32;
    for index in open_index..lines.len() {
        match lines[index].trim() { "{" => depth += 1, "}" => { depth -= 1; if depth == 0 { return Some(index); } }, _ => {} }
    }
    None
}

fn leading_whitespace(value: &str) -> String { value[..value.len() - value.trim_start_matches([' ', '\t']).len()].to_string() }

fn unindent_once(lines: &[String]) -> Vec<String> { lines.iter().map(|line| line.strip_prefix("    ").unwrap_or(line).to_string()).collect() }

struct ByteReader<'a> { data: &'a [u8], pos: usize }

impl<'a> ByteReader<'a> {
    fn new(data: &'a [u8]) -> Self { Self { data, pos: 0 } }
    fn left(&self) -> usize { self.data.len().saturating_sub(self.pos) }
    fn skip(&mut self, count: usize) -> Result<(), String> {
        if count > self.left() { return Err("unexpected end of file".to_string()); }
        self.pos += count; Ok(())
    }
    fn u8(&mut self) -> Result<u8, String> {
        let value = *self.data.get(self.pos).ok_or_else(|| "unexpected end of file".to_string())?;
        self.pos += 1; Ok(value)
    }
    fn u16(&mut self) -> Result<u16, String> {
        if self.left() < 2 { return Err("unexpected end of file".to_string()); }
        let value = u16::from_be_bytes(self.data[self.pos..self.pos + 2].try_into().unwrap()); self.pos += 2; Ok(value)
    }
    fn u32(&mut self) -> Result<u32, String> {
        if self.left() < 4 { return Err("unexpected end of file".to_string()); }
        let value = u32::from_be_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap()); self.pos += 4; Ok(value)
    }
    fn cstr(&mut self) -> Result<String, String> {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != 0 { self.pos += 1; }
        if self.pos >= self.data.len() { return Err("unexpected end of file".to_string()); }
        let value = String::from_utf8_lossy(&self.data[start..self.pos]).into_owned();
        self.pos += 1;
        Ok(value)
    }
}
