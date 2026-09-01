use crate::ast::*;
use crate::parser::{parse_gs1, parse_gs2};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ScriptGrammar { GS2 = 0, GS1 = 1 }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic { pub message: String }

impl Diagnostic { pub fn error(&self) -> &str { &self.message } }

pub fn diagnostics_text(diagnostics: &[Diagnostic]) -> String {
    diagnostics.iter().map(|diagnostic| diagnostic.error()).collect::<Vec<_>>().join("\n")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilerResponse { pub success: bool, pub err_msg: String, pub byte_code: Vec<u8> }

#[allow(non_snake_case)]
pub fn DiagnosticsText(diagnostics: &[Diagnostic]) -> String { diagnostics_text(diagnostics) }

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i32)]
pub enum BytecodeSegment { Gs1EventFlags = 1, FunctionNames = 2, Strings = 3, Bytecode = 4 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Op {
    None = 0, SetIndex = 1, SetIndexTrue = 2, Or = 3, If = 4, And = 5, Call = 6, Ret = 7,
    Sleep = 8, CmdCall = 9, Jmp = 10, WaitFor = 11,
    TypeNumber = 20, TypeString = 21, TypeVar = 22, TypeArray = 23, TypeTrue = 24, TypeFalse = 25,
    TypeNull = 26, Pi = 27, CopyLastOp = 30, SwapLastOps = 31, IndexDec = 32, ConvToFloat = 33,
    ConvToString = 34, MemberAccess = 35, ConvToObject = 36, ArrayEnd = 37, ArrayNew = 38,
    SetArray = 39, InlineNew = 40, MakeVar = 41, NewObject = 42, InlineConditional = 44,
    Assign = 50, FuncParamsEnd = 51, Inc = 52, Dec = 53, Add = 60, Sub = 61, Mul = 62, Div = 63,
    Mod = 64, Pow = 65, Not = 68, UnarySub = 69, Eq = 70, Neq = 71, Lt = 72, Gt = 73,
    Lte = 74, Gte = 75, BitOr = 76, BitAnd = 77, BitXor = 78, BitInvert = 79, InRange = 80,
    InObj = 81, ObjIndex = 82, ObjType = 83, Format = 84, Int = 85, Abs = 86, Random = 87,
    Sin = 88, Cos = 89, Arctan = 90, Exp = 91, Log = 92, Min = 93, Max = 94, GetAngle = 95,
    GetDir = 96, Vecx = 97, Vecy = 98, ObjIndices = 99, ObjLink = 100, ShiftLeft = 101,
    ShiftRight = 102, Char = 103, ObjTrim = 110, ObjLength = 111, ObjPos = 112, Join = 113,
    ObjCharAt = 114, ObjSubstr = 115, ObjStarts = 116, ObjEnds = 117, ObjTokenize = 118,
    Translate = 119, ObjPositions = 120, ObjSize = 130, Array = 131, ArrayAssign = 132,
    ArrayMultiDim = 133, ArrayMultiDimAssign = 134, ObjSubarray = 135, ObjAddString = 136,
    ObjDeleteString = 137, ObjRemoveString = 138, ObjReplaceString = 139, ObjInsertString = 140,
    ObjClear = 141, ArrayNewMultiDim = 142, With = 150, WithEnd = 151, Foreach = 163, This = 180,
    Thiso = 181, Player = 182, Playero = 183, Level = 184, Temp = 189,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionEntry { pub name: String, pub op_index: i32, pub jmp_loc: i32 }

#[derive(Clone, Debug)]
pub struct BytecodeWriter {
    pub gs1_event_flags: i32,
    pub code: Vec<u8>,
    pub strings: Vec<String>,
    string_index: HashMap<String, i32>,
    pub functions: Vec<FunctionEntry>,
    function_set: HashMap<String, ()>,
    prejump_patches: Vec<usize>,
    last_op: Op,
    pub op_index: i32,
}

pub fn new_bytecode_writer(gs1_event_flags: i32) -> BytecodeWriter {
    BytecodeWriter { gs1_event_flags, code: Vec::new(), strings: Vec::new(), string_index: HashMap::new(), functions: Vec::new(), function_set: HashMap::new(), prejump_patches: Vec::new(), last_op: Op::None, op_index: 0 }
}

#[allow(non_snake_case)]
pub fn NewBytecodeWriter(gs1_event_flags: i32) -> BytecodeWriter { new_bytecode_writer(gs1_event_flags) }

impl BytecodeWriter {
    pub fn get_string(&mut self, value: &str) -> i32 {
        if let Some(index) = self.string_index.get(value) { return *index; }
        let index = self.strings.len() as i32;
        self.strings.push(value.to_string()); self.string_index.insert(value.to_string(), index); index
    }
    pub fn add_function(&mut self, name: &str, op_index: i32, jmp_loc: i32) -> bool {
        if self.function_set.contains_key(name) { return false; }
        self.function_set.insert(name.to_string(), ());
        self.functions.push(FunctionEntry { name: name.to_string(), op_index, jmp_loc }); true
    }
    pub fn add_prejump_patch(&mut self, pos: usize) { self.prejump_patches.push(pos); }
    pub fn emit(&mut self, op: Op) { self.code.push(op as u8); self.last_op = op; self.op_index += 1; }
    pub fn emit_number_operand(&mut self, value: i64) { self.code.push(0xf4); write_short(&mut self.code, value); }
    pub fn emit_number_operand_placeholder(&mut self) -> usize { self.code.push(0xf4); let pos = self.code.len(); write_short(&mut self.code, 0); pos }
    pub fn patch_short(&mut self, pos: usize, value: i32) { if pos + 1 < self.code.len() { self.code[pos] = (value >> 8) as u8; self.code[pos + 1] = value as u8; } }
    pub fn emit_dynamic_string_index(&mut self, value: i32) {
        if value <= 0xff { self.code.extend([0xf0, value as u8]); }
        else if value <= 0xffff { self.code.push(0xf1); write_short(&mut self.code, value as i64); }
        else { self.code.push(0xf2); write_int(&mut self.code, value as i64); }
    }
    pub fn emit_dynamic_number(&mut self, value: i64) {
        let offset = if matches!(self.last_op, Op::TypeNumber | Op::SetIndex | Op::SetIndexTrue) { 3 } else { 0 };
        if (-128..=127).contains(&value) { self.code.extend([(0xf0 + offset) as u8, value as i8 as u8]); }
        else if (-32768..=32767).contains(&value) { self.code.push((0xf1 + offset) as u8); write_short(&mut self.code, value); }
        else { self.code.push((0xf2 + offset) as u8); write_int(&mut self.code, value); }
    }
    pub fn emit_double_number(&mut self, value: &str) { self.code.push(0xf6); write_cstring(&mut self.code, value); }
    pub fn to_bytes(&mut self) -> Vec<u8> {
        self.emit(Op::Ret);
        let final_index = self.op_index;
        for pos in self.prejump_patches.clone() { self.patch_short(pos, final_index); }
        let mut result = Vec::new(); let mut flags = Vec::new(); write_int(&mut flags, self.gs1_event_flags as i64); write_segment(&mut result, BytecodeSegment::Gs1EventFlags, &flags);
        let mut functions = Vec::new();
        for function in self.ordered_functions() { write_int(&mut functions, function.op_index as i64); write_cstring(&mut functions, &function.name); }
        write_segment(&mut result, BytecodeSegment::FunctionNames, &functions);
        let mut strings = Vec::new(); for value in &self.strings { write_cstring(&mut strings, value); }
        write_segment(&mut result, BytecodeSegment::Strings, &strings); write_segment(&mut result, BytecodeSegment::Bytecode, &self.code); result.push(10); result
    }
    fn ordered_functions(&self) -> Vec<FunctionEntry> {
        let mut result = Vec::new(); let mut emitted = HashMap::<String, ()>::new();
        for value in &self.strings { for function in &self.functions { if &function.name == value && !emitted.contains_key(&function.name) { result.push(function.clone()); emitted.insert(function.name.clone(), ()); } } }
        for function in &self.functions { if !emitted.contains_key(&function.name) { result.push(function.clone()); emitted.insert(function.name.clone(), ()); } }
        result
    }
}

#[allow(non_snake_case)]
pub fn WrapHeader(bytecode: &[u8], script_type: &str, name: &str) -> Vec<u8> { wrap_header(bytecode, script_type, name) }
pub fn wrap_header(bytecode: &[u8], script_type: &str, name: &str) -> Vec<u8> { let info = format!("{},{},0,", script_type, name); let mut result = vec![0xac, (info.len() >> 8) as u8, info.len() as u8]; result.extend(info.as_bytes()); result.extend(bytecode); result }

fn write_segment(target: &mut Vec<u8>, segment: BytecodeSegment, value: &[u8]) { write_int(target, segment as i64); write_int(target, value.len() as i64); target.extend(value); }
fn write_cstring(target: &mut Vec<u8>, value: &str) { target.extend(value.as_bytes()); target.push(0); }
fn write_short(target: &mut Vec<u8>, value: i64) { target.extend([(value >> 8) as u8, value as u8]); }
fn write_int(target: &mut Vec<u8>, value: i64) { target.extend([(value >> 24) as u8, (value >> 16) as u8, (value >> 8) as u8, value as u8]); }

pub fn compile_code(code: &str, script_type: &str, name: &str, with_header: bool, grammar: ScriptGrammar) -> CompilerResponse {
    let script_type = if script_type.is_empty() { "weapon" } else { script_type };
    let name = if name.is_empty() { "npc" } else { name };
    let built = if grammar == ScriptGrammar::GS1 { parse_gs1(code) } else { parse_gs2(code) };
    let program = match built {
        Ok(value) => value,
        Err(message) => {
            let line = message.strip_prefix("line ").and_then(|value| value.split(':').next()).and_then(|value| value.parse::<usize>().ok()).unwrap_or(1);
            return CompilerResponse { success: false, err_msg: malformed_input(code, line), byte_code: Vec::new() };
        }
    };
    let mut writer = new_bytecode_writer(program.gs1_event_flags);
    let mut emitter = CompilerEmitter::new(&mut writer, program.constants, program.enums);
    for item in program.items { match item { ProgramItem::Function(function) => emitter.emit_function(&function), ProgramItem::Statement(statement) => emitter.emit_statement(&statement) } }
    if let Some(error) = emitter.error { return CompilerResponse { success: false, err_msg: error, byte_code: Vec::new() }; }
    let bytecode = writer.to_bytes();
    CompilerResponse { success: true, err_msg: String::new(), byte_code: if with_header { wrap_header(&bytecode, script_type, name) } else { bytecode } }
}

#[allow(non_snake_case)]
pub fn CompileCodeRust(code: &str, script_type: &str, name: &str, with_header: bool, grammar: ScriptGrammar) -> CompilerResponse { compile_code(code, script_type, name, with_header, grammar) }

fn malformed_input(code: &str, line: usize) -> String {
    let normalized = code.replace("\r\n", "\n"); let lines: Vec<&str> = normalized.split('\n').collect(); let text = if line > 0 && line <= lines.len() { lines[line - 1] } else { "" }; format!("malformed input at line {}: {}\n", line, text)
}

fn normalize_operator(op: &str) -> &str { if op == ":=" { "=" } else if op == "<>" { "!=" } else { op } }

struct CompilerEmitter<'a> {
    bytecode: &'a mut BytecodeWriter,
    constants: BTreeMap<String, Expr>,
    enums: BTreeMap<String, BTreeMap<String, i32>>,
    breaks: Vec<Vec<usize>>,
    continues: Vec<Vec<usize>>,
    labels: Vec<HashMap<String, i32>>,
    pending: Vec<HashMap<String, Vec<usize>>>,
    new_objects: i32,
    negative_floats: HashMap<String, String>,
    condition_mode: bool,
    condition_fails: Vec<usize>,
    defer_condition_fails: bool,
    control_logical: bool,
    defer_negated_and: bool,
    suppress_logical_inline: bool,
    error: Option<String>,
}

impl<'a> CompilerEmitter<'a> {
    fn new(bytecode: &'a mut BytecodeWriter, constants: BTreeMap<String, Expr>, enums: BTreeMap<String, BTreeMap<String, i32>>) -> Self {
        Self { bytecode, constants, enums, breaks: Vec::new(), continues: Vec::new(), labels: Vec::new(), pending: Vec::new(), new_objects: 0, negative_floats: HashMap::new(), condition_mode: false, condition_fails: Vec::new(), defer_condition_fails: false, control_logical: false, defer_negated_and: false, suppress_logical_inline: false, error: None }
    }

    fn fail(&mut self, message: impl Into<String>) { if self.error.is_none() { self.error = Some(message.into()); } }

    fn emit_function(&mut self, node: &FunctionNode) { self.emit_function_with_patch(node, true); }
    fn emit_function_with_patch(&mut self, node: &FunctionNode, patch_to_final: bool) {
        if self.error.is_some() { return; }
        self.bytecode.emit(Op::SetIndex); let jump = self.bytecode.emit_number_operand_placeholder();
        let mut name = node.name.clone(); if node.public { name = format!("public.{}", name); } if !node.object_name.is_empty() { name = format!("{}.{}", node.object_name, name); } if node.object_name == "universe" { name = format!("{},{}", node.name, name); }
        let added = self.bytecode.add_function(&name, self.bytecode.op_index, 0);
        self.labels.push(HashMap::new()); self.pending.push(HashMap::new()); self.bytecode.emit(Op::TypeArray);
        for arg in node.args.iter().rev() { self.emit_expr(arg); }
        self.bytecode.emit(Op::FuncParamsEnd); self.bytecode.emit(Op::Jmp); if statements_contain_call(&node.body) { self.bytecode.emit(Op::CmdCall); }
        for statement in &node.body { self.emit_statement(statement); }
        if self.pending.last().map(|value| !value.is_empty()).unwrap_or(false) { self.fail(format!("goto to undefined label in function \"{}\"", name)); }
        self.labels.pop(); self.pending.pop(); if self.error.is_some() { return; }
        if node.name != "getPreviewSpriteCount" && (node.body.is_empty() || !returns(node.body.last().unwrap())) { self.bytecode.emit(Op::TypeNumber); self.bytecode.emit_dynamic_number(0); self.bytecode.emit(Op::Ret); }
        if patch_to_final && added { self.bytecode.add_prejump_patch(jump); } else if !patch_to_final { self.bytecode.patch_short(jump, self.bytecode.op_index); }
    }

    fn emit_statement(&mut self, statement: &Stmt) {
        if self.error.is_some() { return; }
        match statement {
            Stmt::Expr(value) => { self.emit_expr(&value.expression); match &value.expression { Expr::Call(call) if call.name != "sleep" && call.name != "setarray" => self.bytecode.emit(Op::IndexDec), Expr::MethodCall(call) if !matches!(call.name.as_str(), "clear" | "add" | "delete" | "insert" | "remove" | "replace") => self.bytecode.emit(Op::IndexDec), Expr::DynamicMethodCall(_) => self.bytecode.emit(Op::IndexDec), _ => {} } }
            Stmt::Return(value) => { self.emit_expr(&value.expression); self.bytecode.emit(Op::Ret); }
            Stmt::Inline(value) => { if let Stmt::Expr(expr) = value.statement.as_ref() { self.emit_value_expr(&expr.expression); if self.new_objects > 0 { match &expr.expression { Expr::Call(call) if call.name.eq_ignore_ascii_case("addcontrol") => self.bytecode.emit(Op::IndexDec), Expr::MethodCall(call) if call.name.eq_ignore_ascii_case("addcontrol") => self.bytecode.emit(Op::IndexDec), _ => {} } } } else { self.emit_statement(&value.statement); } }
            Stmt::Block(value) => for child in &value.body { self.emit_statement(child); },
            Stmt::If(value) => self.emit_if(value), Stmt::While(value) => self.emit_while(value), Stmt::DoWhile(value) => self.emit_do_while(value), Stmt::For(value) => self.emit_for(value), Stmt::ForEach(value) => self.emit_for_each(value), Stmt::With(value) => self.emit_with(value), Stmt::Switch(value) => self.emit_switch(value), Stmt::New(value) => self.emit_new_statement(value),
            Stmt::Break => { if self.breaks.is_empty() { self.fail("break outside a loop"); } else { self.bytecode.emit(Op::SetIndex); let pos = self.bytecode.emit_number_operand_placeholder(); self.breaks.last_mut().unwrap().push(pos); } }
            Stmt::Continue => { if self.continues.is_empty() { self.fail("continue outside a loop"); } else { self.bytecode.emit(Op::SetIndex); let pos = self.bytecode.emit_number_operand_placeholder(); self.continues.last_mut().unwrap().push(pos); } }
            Stmt::Goto(value) => self.emit_goto(value), Stmt::Label(value) => self.emit_label(value),
        }
    }

    fn emit_unused_expr(&mut self, expression: &Expr) {
        self.emit_expr(expression);
        if let Expr::Unary(value) = expression { if !value.postfix && (value.op == "++" || value.op == "--") { self.bytecode.emit(Op::IndexDec); } }
    }

    fn emit_goto(&mut self, statement: &GotoStmt) {
        if self.labels.is_empty() { self.fail("goto outside a function"); return; }
        self.bytecode.emit(Op::SetIndex);
        if let Some(address) = self.labels.last().unwrap().get(&statement.label) { self.bytecode.emit_dynamic_number(*address as i64); return; }
        let patch = self.bytecode.emit_number_operand_placeholder(); self.pending.last_mut().unwrap().entry(statement.label.clone()).or_default().push(patch);
    }

    fn emit_label(&mut self, statement: &LabelStmt) {
        if self.labels.is_empty() { self.fail("label outside a function"); return; }
        let address = self.bytecode.op_index; self.labels.last_mut().unwrap().insert(statement.label.clone(), address);
        if let Some(patches) = self.pending.last_mut().unwrap().remove(&statement.label) { for patch in patches { self.bytecode.patch_short(patch, address); } }
    }

    fn emit_if(&mut self, statement: &IfStmt) {
        let fails = if uses_inline_for_condition(&statement.condition) { self.emit_inline_condition(&statement.condition) } else { self.emit_condition(&statement.condition) };
        for child in &statement.then_body { self.emit_statement(child); }
        if !statement.has_else { for fail in fails { self.bytecode.patch_short(fail, self.bytecode.op_index); } return; }
        self.bytecode.emit(Op::SetIndex); let exit = self.bytecode.emit_number_operand_placeholder(); for fail in fails { self.bytecode.patch_short(fail, self.bytecode.op_index); }
        for child in &statement.else_body { self.emit_statement(child); } self.bytecode.patch_short(exit, self.bytecode.op_index);
    }

    fn emit_condition(&mut self, condition: &Expr) -> Vec<usize> {
        if let Expr::Binary(logical) = condition {
            if logical.op == "&&" {
                let previous = self.defer_negated_and;
                if let Expr::Unary(unary) = logical.left.as_ref() { if unary.op == "!" { if let Expr::Binary(nested) = unary.expression.as_ref() { if nested.op == "&&" { self.defer_negated_and = true; } } } }
                let mut fails = self.emit_condition(&logical.left); self.defer_negated_and = previous; fails.extend(self.emit_condition(&logical.right)); return fails;
            }
        }
        let previous_mode = self.condition_mode; let previous_fails = std::mem::take(&mut self.condition_fails); let previous_defer = self.defer_condition_fails;
        self.condition_mode = true; self.condition_fails = Vec::new(); self.defer_condition_fails = false; self.emit_expr(condition);
        let mut fails = std::mem::take(&mut self.condition_fails); self.condition_mode = previous_mode; self.condition_fails = previous_fails; self.defer_condition_fails = previous_defer;
        if !is_boolean_expr(condition) && (needs_numeric_conversion(condition) || is_assignment_expr(condition)) { self.bytecode.emit(Op::ConvToFloat); }
        self.bytecode.emit(Op::If); fails.push(self.bytecode.emit_number_operand_placeholder()); fails
    }

    fn emit_inline_condition(&mut self, condition: &Expr) -> Vec<usize> {
        self.emit_expr(condition); if !is_boolean_expr(condition) && needs_numeric_conversion(condition) { self.bytecode.emit(Op::ConvToFloat); }
        self.bytecode.emit(Op::If); vec![self.bytecode.emit_number_operand_placeholder()]
    }

    fn emit_while(&mut self, statement: &WhileStmt) {
        let start = self.bytecode.op_index; let exits = self.emit_condition(&statement.condition); self.breaks.push(Vec::new()); self.continues.push(Vec::new()); self.bytecode.emit(Op::CmdCall);
        for child in &statement.body { self.emit_statement(child); }
        let continues = self.continues.last().cloned().unwrap_or_default(); for patch in continues { self.bytecode.patch_short(patch, self.bytecode.op_index); }
        self.bytecode.emit(Op::SetIndex); self.bytecode.emit_number_operand(start as i64);
        let breaks = self.breaks.last().cloned().unwrap_or_default(); for patch in breaks { self.bytecode.patch_short(patch, self.bytecode.op_index); }
        self.breaks.pop(); self.continues.pop(); for exit in exits { self.bytecode.patch_short(exit, self.bytecode.op_index); }
    }

    fn emit_do_while(&mut self, statement: &DoWhileStmt) {
        let start = self.bytecode.op_index; self.breaks.push(Vec::new()); self.continues.push(Vec::new()); self.bytecode.emit(Op::CmdCall);
        for child in &statement.body { self.emit_statement(child); }
        let condition_start = self.bytecode.op_index; let continues = self.continues.last().cloned().unwrap_or_default(); for patch in continues { self.bytecode.patch_short(patch, condition_start); }
        let exits = self.emit_condition(&statement.condition); self.bytecode.emit(Op::SetIndex); self.bytecode.emit_dynamic_number(start as i64); let loop_end = self.bytecode.op_index;
        let breaks = self.breaks.last().cloned().unwrap_or_default(); for patch in breaks { self.bytecode.patch_short(patch, loop_end); } for exit in exits { self.bytecode.patch_short(exit, loop_end); }
        self.breaks.pop(); self.continues.pop();
    }

    fn emit_for(&mut self, statement: &ForStmt) {
        if let Some(init) = &statement.init { self.emit_unused_expr(init); }
        let start = self.bytecode.op_index; let exits = if uses_inline_for_condition(&statement.condition) { self.emit_inline_condition(&statement.condition) } else { self.emit_condition(&statement.condition) };
        self.breaks.push(Vec::new()); self.continues.push(Vec::new()); self.bytecode.emit(Op::CmdCall); for child in &statement.body { self.emit_statement(child); }
        let continues = self.continues.last().cloned().unwrap_or_default(); for patch in continues { self.bytecode.patch_short(patch, self.bytecode.op_index); }
        if let Some(post) = &statement.post { self.emit_unused_expr(post); }
        self.bytecode.emit(Op::SetIndex); self.bytecode.emit_dynamic_number(start as i64); let breaks = self.breaks.last().cloned().unwrap_or_default(); for patch in breaks { self.bytecode.patch_short(patch, self.bytecode.op_index); }
        self.breaks.pop(); self.continues.pop(); for exit in exits { self.bytecode.patch_short(exit, self.bytecode.op_index); }
    }

    fn emit_for_each(&mut self, statement: &ForEachStmt) {
        self.emit_expr(&statement.name); self.emit_expr(&statement.source); self.bytecode.emit(Op::ConvToObject); self.bytecode.emit(Op::TypeNumber); self.bytecode.emit_dynamic_number(0);
        let start = self.bytecode.op_index; self.bytecode.emit(Op::Foreach); let exit = self.bytecode.emit_number_operand_placeholder(); self.breaks.push(Vec::new()); self.continues.push(Vec::new()); self.bytecode.emit(Op::CmdCall);
        for child in &statement.body { self.emit_statement(child); }
        let continues = self.continues.last().cloned().unwrap_or_default(); for patch in continues { self.bytecode.patch_short(patch, self.bytecode.op_index); }
        self.bytecode.emit(Op::Inc); self.bytecode.emit(Op::SetIndex); self.bytecode.emit_dynamic_number(start as i64); let breaks = self.breaks.last().cloned().unwrap_or_default(); for patch in breaks { self.bytecode.patch_short(patch, self.bytecode.op_index); }
        self.breaks.pop(); self.continues.pop(); self.bytecode.patch_short(exit, self.bytecode.op_index); self.bytecode.emit(Op::IndexDec);
    }

    fn emit_switch(&mut self, statement: &SwitchStmt) {
        self.bytecode.emit(Op::SetIndex); let case_test = self.bytecode.emit_number_operand_placeholder(); let mut case_starts = Vec::new(); self.breaks.push(Vec::new());
        for current in &statement.cases {
            let start = self.bytecode.op_index; case_starts.extend(std::iter::repeat(start).take(current.labels.len())); self.continues.push(Vec::new()); for child in &current.body { self.emit_statement(child); }
            let continues = self.continues.pop().unwrap_or_default(); for patch in continues { self.bytecode.patch_short(patch, start); }
        }
        let breaks = self.breaks.pop().unwrap_or_default(); self.bytecode.patch_short(case_test, self.bytecode.op_index); self.emit_expr(&statement.expression); let mut case_index = 0;
        for current in &statement.cases { for label in &current.labels { if let Some(label) = label { self.bytecode.emit(Op::CopyLastOp); self.emit_expr(label); self.bytecode.emit(Op::Eq); self.bytecode.emit(Op::SetIndexTrue); } else { self.bytecode.emit(Op::SetIndex); } if case_index < case_starts.len() { self.bytecode.emit_dynamic_number(case_starts[case_index] as i64); } case_index += 1; } }
        for patch in breaks { self.bytecode.patch_short(patch, self.bytecode.op_index); } self.bytecode.emit(Op::IndexDec);
    }

    fn emit_with(&mut self, statement: &WithStmt) {
        self.emit_expr(&statement.target); self.bytecode.emit(Op::ConvToObject); self.bytecode.emit(Op::With); let exit = self.bytecode.emit_number_operand_placeholder(); for child in &statement.body { self.emit_statement(child); } self.bytecode.emit(Op::WithEnd); self.bytecode.patch_short(exit, self.bytecode.op_index);
    }

    fn emit_new_statement(&mut self, statement: &NewStmt) {
        for arg in &statement.args { self.emit_expr(arg); }
        self.bytecode.emit(Op::InlineNew); self.bytecode.emit(Op::CopyLastOp); self.bytecode.emit(Op::CopyLastOp); self.bytecode.emit(Op::CopyLastOp); self.bytecode.emit(Op::TypeString); let type_index = self.bytecode.get_string(&statement.type_name); self.bytecode.emit_dynamic_string_index(type_index); self.bytecode.emit(Op::ConvToString); self.bytecode.emit(Op::NewObject); self.bytecode.emit(Op::Assign); self.bytecode.emit(Op::ConvToObject); self.bytecode.emit(Op::With); let with = self.bytecode.emit_number_operand_placeholder();
        let previous = self.new_objects; self.new_objects += 1; for child in &statement.body { self.emit_statement(child); } self.bytecode.emit(Op::WithEnd); self.bytecode.patch_short(with, self.bytecode.op_index);
        for _ in 0..(self.new_objects - previous) { self.bytecode.emit(Op::TypeArray); self.bytecode.emit(Op::SwapLastOps); self.bytecode.emit(Op::TypeVar); let index = self.bytecode.get_string("addcontrol"); self.bytecode.emit_dynamic_string_index(index); self.bytecode.emit(Op::Call); self.bytecode.emit(Op::IndexDec); }
        self.new_objects -= 1;
    }

    fn emit_expr(&mut self, expression: &Expr) {
        if self.error.is_some() { return; }
        match expression {
            Expr::Ternary(value) => self.emit_ternary(value, None),
            Expr::In(value) => {
                self.emit_expr(&value.expression); self.emit_expr(&value.lower);
                if value.upper.is_none() { self.bytecode.emit(Op::ConvToObject); self.bytecode.emit(Op::InObj); }
                else {
                    if !is_number_expr(&value.lower) { self.bytecode.emit(Op::ConvToFloat); }
                    self.emit_expr(value.upper.as_ref().unwrap()); if !is_number_expr(value.upper.as_ref().unwrap()) { self.bytecode.emit(Op::ConvToFloat); } self.bytecode.emit(Op::InRange);
                }
            }
            Expr::Binary(value) => {
                if value.op == "=" { self.emit_assignment(value, false); return; }
                if (value.op == "==" || value.op == "!=") && is_string_cast_compare(&value.left, &value.right) {
                    let left = match value.left.as_ref() { Expr::StringCast(v) => v.expression.as_ref(), _ => &value.left };
                    let right = match value.right.as_ref() { Expr::StringCast(v) => v.expression.as_ref(), _ => &value.right };
                    let left_member = matches!(left, Expr::Member(_)); let right_member = matches!(right, Expr::Member(_)); let left_array = matches!(left, Expr::ArrayLiteral(_)); let right_array = matches!(right, Expr::ArrayLiteral(_));
                    if (left_member && right_member) || (left_member && right_array) || (left_array && right_array) {
                        self.emit_expr(left); self.emit_expr(right); if left_array || right_array { self.bytecode.emit(Op::ConvToString); self.bytecode.emit(Op::MemberAccess); } else { self.bytecode.emit(Op::ConvToString); } let op = if value.op == "==" { Op::Eq } else { Op::Neq }; self.bytecode.emit(op); self.bytecode.emit(Op::ConvToString); return;
                    }
                }
                if is_compound_assign(&value.op) { self.emit_compound_assignment(value); return; }
                if value.op == "&&" || value.op == "||" { self.emit_logical_expression(value, 0, !self.condition_mode && !self.suppress_logical_inline); return; }
                if matches!(value.op.as_str(), " " | "\n" | "\t") {
                    self.emit_expr(&value.left); if needs_string_conversion(&value.left) { self.bytecode.emit(Op::ConvToString); } self.bytecode.emit(Op::TypeString); let idx = self.bytecode.get_string(&value.op); self.bytecode.emit_dynamic_string_index(idx); self.bytecode.emit(Op::Join);
                    self.emit_expr(&value.right); if needs_string_conversion(&value.right) { self.bytecode.emit(Op::ConvToString); } self.bytecode.emit(Op::Join); return;
                }
                if value.op == "*" { if let Expr::Unary(left) = value.left.as_ref() { if left.op == "-" { self.emit_expr(&left.expression); if needs_numeric_conversion(&left.expression) { self.bytecode.emit(Op::ConvToFloat); } self.emit_expr(&value.right); if needs_numeric_conversion(&value.right) { self.bytecode.emit(Op::ConvToFloat); } self.bytecode.emit(Op::Mul); self.bytecode.emit(Op::UnarySub); return; } } }
                self.emit_value_expr(&value.left);
                if value.op == "@" && needs_string_conversion(&value.left) { self.bytecode.emit(Op::ConvToString); }
                else if value.op == "@" { if matches!(value.left.as_ref(), Expr::StringCast(_)) && matches!(value.right.as_ref(), Expr::Ternary(_)) { self.bytecode.emit(Op::ConvToString); } }
                else if (is_numeric_op(&value.op) || is_comparison_op(&value.op)) && needs_numeric_conversion(&value.left) { self.bytecode.emit(Op::ConvToFloat); }
                else if matches!(value.op.as_str(), "&" | "|" | "xor" | "<<" | ">>") && !is_number_expr(&value.left) { self.bytecode.emit(Op::ConvToFloat); }
                if is_comparison_op(&value.op) { if let Expr::Unary(left) = value.left.as_ref() { if (left.op == "++" || left.op == "--") && left.postfix { self.bytecode.emit(Op::ConvToFloat); } } }
                if is_numeric_op(&value.op) && uses_preview_sprite_ternary(&value.left) { self.bytecode.emit(Op::ConvToFloat); }
                if value.op == "%" { if let Expr::Binary(left) = value.left.as_ref() { if is_compound_assign(&left.op) { self.bytecode.emit(Op::ConvToFloat); } } }
                self.emit_value_expr(&value.right);
                if (is_numeric_op(&value.op) || is_comparison_op(&value.op)) && needs_numeric_conversion(&value.right) { self.bytecode.emit(Op::ConvToFloat); }
                else if matches!(value.op.as_str(), "&" | "|" | "xor" | "<<" | ">>") && !is_number_expr(&value.right) { self.bytecode.emit(Op::ConvToFloat); }
                if is_numeric_op(&value.op) && uses_sign_ternary(&value.right) { self.bytecode.emit(Op::ConvToFloat); }
                if value.op == "@" && needs_string_conversion(&value.right) { self.bytecode.emit(Op::ConvToString); }
                if let Some(op) = binary_op(&value.op) { self.bytecode.emit(op); } else { self.fail(format!("GS2 operator \"{}\" is not emitted yet", value.op)); }
            }
            Expr::Member(value) => { self.emit_expr(&value.object); if needs_object_conversion(&value.object) { self.bytecode.emit(Op::ConvToObject); } self.bytecode.emit(Op::TypeVar); let index = self.bytecode.get_string(&value.name); self.bytecode.emit_dynamic_string_index(index); self.bytecode.emit(Op::MemberAccess); }
            Expr::DynamicMember(value) => {
                self.emit_expr(&value.object); if needs_object_conversion(&value.object) { self.bytecode.emit(Op::ConvToObject); }
                if let Expr::StringCast(name) = value.name.as_ref() { if is_dynamic_temp_name(&name.expression) { self.emit_expr(&name.expression); if needs_numeric_conversion(&name.expression) { self.bytecode.emit(Op::ConvToFloat); } self.bytecode.emit(Op::Add); } else { self.emit_dynamic_member_name(&value.name); } } else { self.emit_dynamic_member_name(&value.name); }
                self.bytecode.emit(Op::MemberAccess);
            }
            Expr::DynamicVar(value) => self.emit_call(&CallExpr { name: "makevar".to_string(), args: vec![(*value.name).clone()] }),
            Expr::ArrayIndex(value) => self.emit_array_index(value, false), Expr::MultiArrayIndex(value) => self.emit_multi_array_index(value, false),
            Expr::StringCast(value) => { self.emit_expr(&value.expression); self.bytecode.emit(Op::ConvToString); if matches!(value.expression.as_ref(), Expr::ArrayLiteral(_)) { self.bytecode.emit(Op::MemberAccess); } }
            Expr::Cast(value) => { self.emit_expr(&value.expression); match value.type_name.as_str() { "int" => { if !is_number_expr(&value.expression) { self.bytecode.emit(Op::ConvToFloat); } self.bytecode.emit(Op::Int); }, "float" => if !is_number_expr(&value.expression) { self.bytecode.emit(Op::ConvToFloat); }, "_" => { if !matches!(value.expression.as_ref(), Expr::String(_)) { self.bytecode.emit(Op::ConvToString); } self.bytecode.emit(Op::Translate); }, _ => {} } }
            Expr::Call(value) => self.emit_call(value),
            Expr::ChainedCall(value) => { self.bytecode.emit(Op::TypeArray); for arg in value.args.iter().rev() { self.emit_expr(arg); } self.emit_call(&value.call); self.bytecode.emit(Op::Call); }
            Expr::MethodCall(value) => self.emit_method_call(value), Expr::DynamicMethodCall(value) => self.emit_dynamic_method_call(value),
            Expr::NewObject(value) => { let type_index = self.bytecode.get_string(&value.type_name); if value.args.len() == 1 { self.emit_expr(&value.args[0]); self.bytecode.emit(Op::InlineNew); } else { self.bytecode.emit(Op::TypeVar); let unknown = self.bytecode.get_string("unknown_object"); self.bytecode.emit_dynamic_string_index(unknown); } self.bytecode.emit(Op::TypeString); self.bytecode.emit_dynamic_string_index(type_index); self.bytecode.emit(Op::NewObject); }
            Expr::NewArray(value) => for (index, dimension) in value.dimensions.iter().enumerate() { self.emit_expr(dimension); if !is_number_expr(dimension) { self.bytecode.emit(Op::ConvToFloat); } self.bytecode.emit(if index == 0 { Op::ArrayNew } else { Op::ArrayNewMultiDim }); },
            Expr::Lambda(value) => { self.emit_function_with_patch(&FunctionNode { name: value.name.clone(), object_name: String::new(), public: true, args: value.args.clone(), body: value.body.clone() }, false); self.bytecode.emit(Op::This); self.bytecode.emit(Op::TypeVar); let index = self.bytecode.get_string(&value.name); self.bytecode.emit_dynamic_string_index(index); self.bytecode.emit(Op::MemberAccess); self.bytecode.emit(Op::ConvToObject); }
            Expr::ArrayLiteral(value) => { self.bytecode.emit(Op::TypeArray); for item in value.values.iter().rev() { self.emit_expr(item); } self.bytecode.emit(Op::ArrayEnd); }
            Expr::Identifier(value) => {
                if let Some(constant) = self.constants.get(&value.name).cloned() { self.emit_expr(&constant); return; }
                match value.name.as_str() { "this" => self.bytecode.emit(Op::This), "temp" => self.bytecode.emit(Op::Temp), "thiso" => self.bytecode.emit(Op::Thiso), "player" => self.bytecode.emit(Op::Player), "playero" => self.bytecode.emit(Op::Playero), "level" => self.bytecode.emit(Op::Level), "pi" => self.bytecode.emit(Op::Pi), _ => { self.bytecode.emit(Op::TypeVar); let index = self.bytecode.get_string(&value.name); self.bytecode.emit_dynamic_string_index(index); } }
            }
            Expr::Enum(value) => { let number = self.enums.get(&value.enum_name).and_then(|m| m.get(&value.member_name)).copied().unwrap_or(0); self.bytecode.emit(Op::TypeNumber); self.bytecode.emit_dynamic_number(number as i64); }
            Expr::Number(value) => { self.bytecode.emit(Op::TypeNumber); if value.text.contains('.') { let text = self.current_float_text(&value.text); self.bytecode.emit_double_number(&text); } else if let Ok(number) = value.text.parse::<i64>() { self.bytecode.emit_dynamic_number(number); } else { self.fail(value.text.clone()); } }
            Expr::String(value) => { self.bytecode.emit(Op::TypeString); let index = self.bytecode.get_string(&value.value); self.bytecode.emit_dynamic_string_index(index); }
            Expr::Bool(value) => self.bytecode.emit(if value.value { Op::TypeTrue } else { Op::TypeFalse }), Expr::Null => self.bytecode.emit(Op::TypeNull),
            Expr::Unary(value) => {
                if value.op == "-" { if let Expr::Number(number) = value.expression.as_ref() { self.bytecode.emit(Op::TypeNumber); if number.text.contains('.') { let text = self.next_negative_float_text(&number.text); self.bytecode.emit_double_number(&text); } else if let Ok(n) = number.text.parse::<i64>() { self.bytecode.emit_dynamic_number(-n); } else { self.fail(number.text.clone()); } return; } }
                let previous_defer = self.defer_condition_fails; let fail_start = self.condition_fails.len(); let mut local_negated_and = false;
                if value.op == "!" && self.condition_mode { if let Expr::Binary(logical) = value.expression.as_ref() { if logical.op == "&&" { local_negated_and = self.control_logical; if local_negated_and || self.defer_negated_and { self.defer_condition_fails = true; } } } }
                let previous_suppress = self.suppress_logical_inline; if value.op == "!" { self.suppress_logical_inline = true; } self.emit_expr(&value.expression); self.suppress_logical_inline = previous_suppress; self.defer_condition_fails = previous_defer;
                if local_negated_and { let patches = self.condition_fails[fail_start..].to_vec(); for patch in patches { self.bytecode.patch_short(patch, self.bytecode.op_index + 2); } self.condition_fails.truncate(fail_start); }
                match value.op.as_str() { "-" => { self.bytecode.emit(Op::ConvToFloat); self.bytecode.emit(Op::UnarySub); }, "!" => { if !is_boolean_expr(&value.expression) { self.bytecode.emit(Op::ConvToFloat); } self.bytecode.emit(Op::Not); }, "~" => self.bytecode.emit(Op::BitInvert), "++" => { self.bytecode.emit(Op::Inc); if value.postfix { self.bytecode.emit(Op::IndexDec); } }, "--" => { self.bytecode.emit(Op::Dec); if value.postfix { self.bytecode.emit(Op::IndexDec); } }, _ => self.fail(format!("GS2 unary operator \"{}\" is not emitted yet", value.op)) }
            }
        }
    }

    fn emit_dynamic_member_name(&mut self, name: &Expr) {
        if let Expr::Binary(binary) = name {
            if let Expr::StringCast(cast) = binary.left.as_ref() {
                let mut clone = binary.clone(); clone.left = cast.expression.clone(); self.emit_expr(&Expr::Binary(clone));
            } else { self.emit_expr(name); }
        } else { self.emit_expr(name); }
        if needs_string_conversion(name) { self.bytecode.emit(Op::ConvToString); }
    }

    fn next_negative_float_text(&mut self, text: &str) -> String { let previous = self.negative_floats.get(text).cloned().unwrap_or_else(|| text.to_string()); let next = format!("-{}", previous); self.negative_floats.insert(text.to_string(), next.clone()); next }
    fn current_float_text(&self, text: &str) -> String { self.negative_floats.get(text).cloned().unwrap_or_else(|| text.to_string()) }
    fn emit_value_expr(&mut self, expression: &Expr) {
        if let Expr::Unary(value) = expression { if value.postfix && (value.op == "++" || value.op == "--") { self.emit_expr(&value.expression); self.bytecode.emit(Op::CopyLastOp); self.bytecode.emit(Op::ConvToFloat); self.bytecode.emit(Op::SwapLastOps); self.bytecode.emit(if value.op == "++" { Op::Inc } else { Op::Dec }); self.bytecode.emit(Op::IndexDec); return; } }
        self.emit_expr(expression);
    }

    fn emit_assignment(&mut self, value: &BinaryExpr, copy_target: bool) {
        match value.left.as_ref() {
            Expr::MultiArrayIndex(left) => { self.emit_multi_array_index(left, true); if copy_target { self.bytecode.emit(Op::CopyLastOp); } self.emit_assignment_right(&value.right); self.bytecode.emit(Op::ArrayMultiDimAssign); }
            Expr::ArrayIndex(left) => { self.emit_array_index(left, true); if copy_target { self.bytecode.emit(Op::CopyLastOp); } self.emit_assignment_right(&value.right); self.bytecode.emit(Op::ArrayAssign); }
            _ => { self.emit_expr(&value.left); if copy_target { self.bytecode.emit(Op::CopyLastOp); } self.emit_assignment_right(&value.right); self.bytecode.emit(Op::Assign); }
        }
    }
    fn emit_assignment_right(&mut self, expression: &Expr) { if let Expr::Binary(value) = expression { if value.op == "=" { self.emit_assignment(value, true); return; } } self.emit_expr(expression); }
    fn emit_compound_assignment(&mut self, value: &BinaryExpr) {
        let assign = match value.left.as_ref() { Expr::MultiArrayIndex(left) => { self.emit_multi_array_index(left, true); Op::ArrayMultiDimAssign }, Expr::ArrayIndex(left) => { self.emit_array_index(left, true); Op::ArrayAssign }, _ => { self.emit_expr(&value.left); Op::Assign } };
        self.bytecode.emit(Op::CopyLastOp); if value.op == "@=" { self.bytecode.emit(Op::ConvToString); } else { self.bytecode.emit(Op::ConvToFloat); }
        self.emit_expr(&value.right); if value.op == "@=" && needs_string_conversion(&value.right) { self.bytecode.emit(Op::ConvToString); } else if value.op != "@=" && !is_number_expr(&value.right) { self.bytecode.emit(Op::ConvToFloat); }
        if let Some(op) = compound_op(&value.op) { self.bytecode.emit(op); } else { self.fail(format!("GS2 compound operator \"{}\" is not emitted yet", value.op)); return; } self.bytecode.emit(assign);
    }
    fn emit_array_index(&mut self, expression: &ArrayIndexExpr, assignment_target: bool) {
        self.emit_expr(&expression.target); if !matches!(expression.target.as_ref(), Expr::ArrayIndex(_)) { self.bytecode.emit(Op::ConvToObject); } self.emit_expr(&expression.index); if !is_number_expr(&expression.index) { self.bytecode.emit(Op::ConvToFloat); } if !assignment_target { self.bytecode.emit(Op::Array); }
    }
    fn emit_multi_array_index(&mut self, expression: &MultiArrayIndexExpr, assignment_target: bool) {
        self.emit_expr(&expression.target); self.bytecode.emit(Op::ConvToObject); for index in &expression.indices { self.emit_expr(index); if !is_number_expr(index) { self.bytecode.emit(Op::ConvToFloat); } } if !assignment_target { self.bytecode.emit(Op::ArrayMultiDim); }
    }

    fn emit_ternary(&mut self, ternary: &TernaryExpr, chained_success: Option<usize>) {
        self.emit_expr(&ternary.condition); if !is_boolean_expr(&ternary.condition) { self.bytecode.emit(Op::ConvToFloat); } if let Some(success) = chained_success { self.bytecode.patch_short(success, self.bytecode.op_index); }
        self.bytecode.emit(Op::If); let fail = self.bytecode.emit_number_operand_placeholder(); self.emit_expr(&ternary.when_true); self.bytecode.patch_short(fail, self.bytecode.op_index + 1); self.bytecode.emit(Op::SetIndex); let success = self.bytecode.emit_number_operand_placeholder();
        if let Expr::Ternary(nested) = ternary.when_false.as_ref() { self.emit_ternary(nested, Some(success)); } else { self.emit_expr(&ternary.when_false); self.bytecode.patch_short(success, self.bytecode.op_index); }
    }

    fn emit_logical_expression(&mut self, logical: &BinaryExpr, patch_offset: i32, logical_inline: bool) { if has_nested_logical(logical, &logical.op) { self.emit_logical_chain(logical, patch_offset, logical_inline); } else { self.emit_logical_pair(logical, patch_offset, logical_inline); } }
    fn emit_logical_chain(&mut self, logical: &BinaryExpr, patch_offset: i32, logical_inline: bool) {
        let mut terms = Vec::new();
        let root = Expr::Binary(logical.clone());
        flatten_logical(&root, &logical.op, &mut terms);
        let mut patches = Vec::new();
        for (index, term) in terms[..terms.len().saturating_sub(1)].iter().enumerate() {
            let previous_defer = self.defer_condition_fails; let previous_control = self.control_logical; let control_and = logical.op == "&&" && self.condition_mode && self.control_logical;
            if logical.op == "||" && self.condition_mode && index > 0 { self.defer_condition_fails = true; } if logical.op == "||" && self.condition_mode { self.control_logical = true; }
            self.emit_logical_child(term, 0); self.defer_condition_fails = previous_defer; self.control_logical = previous_control;
            if !is_boolean_expr(term) { self.bytecode.emit(Op::ConvToFloat); } let deferred_fail = logical.op == "&&" && self.condition_mode && self.defer_condition_fails;
            self.bytecode.emit(if deferred_fail || control_and { Op::If } else if logical.op == "&&" { Op::And } else { Op::Or }); let patch = self.bytecode.emit_number_operand_placeholder(); if deferred_fail { self.condition_fails.push(patch); } else { patches.push(patch); }
        }
        if let Some(term) = terms.last() { let previous_defer = self.defer_condition_fails; let previous_control = self.control_logical; if logical.op == "||" && self.condition_mode { self.defer_condition_fails = true; self.control_logical = true; } self.emit_logical_child(term, 0); self.defer_condition_fails = previous_defer; self.control_logical = previous_control; if !is_boolean_expr(term) { self.bytecode.emit(Op::ConvToFloat); } }
        for patch in patches { self.bytecode.patch_short(patch, self.bytecode.op_index + patch_offset); } if logical_inline { self.bytecode.emit(Op::InlineConditional); }
    }
    fn emit_logical_pair(&mut self, logical: &BinaryExpr, patch_offset: i32, logical_inline: bool) {
        let left_patch_offset = if logical.op == "||" && matches!(logical.left.as_ref(), Expr::Binary(_)) { 1 } else { 0 }; let previous_control = self.control_logical; if logical.op == "||" && self.condition_mode { self.control_logical = true; }
        self.emit_logical_child(&logical.left, left_patch_offset); self.control_logical = previous_control; if !is_boolean_expr(&logical.left) { self.bytecode.emit(Op::ConvToFloat); }
        let deferred_fail = logical.op == "&&" && self.condition_mode && self.defer_condition_fails; let control_and = logical.op == "&&" && self.condition_mode && self.control_logical; self.bytecode.emit(if deferred_fail || control_and { Op::If } else if logical.op == "&&" { Op::And } else { Op::Or }); let patch = self.bytecode.emit_number_operand_placeholder(); if deferred_fail { self.condition_fails.push(patch); }
        let previous_defer = self.defer_condition_fails; let previous_control = self.control_logical; if logical.op == "||" && self.condition_mode { self.defer_condition_fails = true; self.control_logical = true; } self.emit_logical_child(&logical.right, 0); self.defer_condition_fails = previous_defer; self.control_logical = previous_control; if !is_boolean_expr(&logical.right) { self.bytecode.emit(Op::ConvToFloat); } if !deferred_fail { self.bytecode.patch_short(patch, self.bytecode.op_index + patch_offset); } if logical_inline { self.bytecode.emit(Op::InlineConditional); }
    }
    fn emit_logical_child(&mut self, expression: &Expr, patch_offset: i32) { if let Expr::Binary(value) = expression { if value.op == "&&" || value.op == "||" { self.emit_logical_expression(value, patch_offset, false); return; } } self.emit_expr(expression); }
}

fn statements_contain_call(statements: &[Stmt]) -> bool {
    statements.iter().any(statement_contains_call)
}

fn statement_contains_call(statement: &Stmt) -> bool {
    match statement {
        Stmt::Expr(value) => expression_contains_call(&value.expression),
        Stmt::Inline(value) => statement_contains_call(&value.statement),
        Stmt::Block(value) => statements_contain_call(&value.body),
        Stmt::Return(value) => expression_contains_call(&value.expression),
        Stmt::If(value) => expression_contains_call(&value.condition)
            || statements_contain_call(&value.then_body)
            || statements_contain_call(&value.else_body),
        Stmt::For(value) => option_expression_contains_call(value.init.as_ref())
            || expression_contains_call(&value.condition)
            || option_expression_contains_call(value.post.as_ref())
            || statements_contain_call(&value.body),
        Stmt::ForEach(value) => expression_contains_call(&value.name)
            || expression_contains_call(&value.source)
            || statements_contain_call(&value.body),
        Stmt::While(value) => expression_contains_call(&value.condition) || statements_contain_call(&value.body),
        Stmt::DoWhile(value) => expression_contains_call(&value.condition) || statements_contain_call(&value.body),
        Stmt::With(value) => expression_contains_call(&value.target) || statements_contain_call(&value.body),
        Stmt::Switch(value) => expression_contains_call(&value.expression)
            || value.cases.iter().any(|case| {
                case.labels.iter().any(|label| label.as_ref().is_some_and(expression_contains_call))
                    || statements_contain_call(&case.body)
            }),
        Stmt::New(value) => value.args.iter().any(expression_contains_call) || statements_contain_call(&value.body),
        Stmt::Break | Stmt::Continue | Stmt::Goto(_) | Stmt::Label(_) => false,
    }
}

fn option_expression_contains_call(expression: Option<&Expr>) -> bool {
    expression.is_some_and(expression_contains_call)
}

fn expression_contains_call(expression: &Expr) -> bool {
    match expression {
        Expr::Call(_) | Expr::ChainedCall(_) | Expr::MethodCall(_) | Expr::DynamicMethodCall(_) => true,
        Expr::Binary(value) => expression_contains_call(&value.left) || expression_contains_call(&value.right),
        Expr::In(value) => expression_contains_call(&value.expression)
            || expression_contains_call(&value.lower)
            || value.upper.as_deref().is_some_and(expression_contains_call),
        Expr::Ternary(value) => expression_contains_call(&value.condition)
            || expression_contains_call(&value.when_true)
            || expression_contains_call(&value.when_false),
        Expr::Unary(value) => expression_contains_call(&value.expression),
        Expr::StringCast(value) => expression_contains_call(&value.expression),
        Expr::Cast(value) => expression_contains_call(&value.expression),
        Expr::Member(value) => expression_contains_call(&value.object),
        Expr::DynamicMember(value) => expression_contains_call(&value.object) || expression_contains_call(&value.name),
        Expr::DynamicVar(value) => expression_contains_call(&value.name),
        Expr::ArrayIndex(value) => expression_contains_call(&value.target) || expression_contains_call(&value.index),
        Expr::MultiArrayIndex(value) => expression_contains_call(&value.target)
            || value.indices.iter().any(expression_contains_call),
        Expr::NewObject(value) => value.args.iter().any(expression_contains_call),
        Expr::NewArray(value) => value.dimensions.iter().any(expression_contains_call),
        Expr::Lambda(value) => statements_contain_call(&value.body),
        Expr::ArrayLiteral(value) => value.values.iter().any(expression_contains_call),
        Expr::Identifier(_) | Expr::Enum(_) | Expr::Number(_) | Expr::String(_) | Expr::Bool(_) | Expr::Null => false,
    }
}

fn returns(statement: &Stmt) -> bool {
    match statement {
        Stmt::Return(_) => true,
        Stmt::Inline(value) => returns(&value.statement),
        Stmt::Block(value) => value.body.last().is_some_and(returns),
        Stmt::If(value) => value.has_else
            && value.then_body.last().is_some_and(returns)
            && value.else_body.last().is_some_and(returns),
        _ => false,
    }
}

fn uses_inline_for_condition(expression: &Expr) -> bool {
    let Expr::Binary(logical) = expression else { return false; };
    if logical.op != "&&" { return false; }

    if let Expr::Binary(left) = logical.left.as_ref() {
        if left.op == ">" {
            if let Expr::Member(left_member) = left.left.as_ref() {
                if let Expr::Identifier(left_object) = left_member.object.as_ref() {
                    if left_object.name == "temp" {
                        if let Expr::Unary(left_limit) = left.right.as_ref() {
                            if left_limit.op == "-" {
                                if let Expr::Number(left_number) = left_limit.expression.as_ref() {
                                    if left_number.text == "1" {
                                        if let Expr::Unary(right) = logical.right.as_ref() {
                                            if right.op == "!" {
                                                if let Expr::Member(right_member) = right.expression.as_ref() {
                                                    if let Expr::Identifier(right_object) = right_member.object.as_ref() {
                                                        if right_object.name == "temp" { return true; }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Expr::Binary(left) = logical.left.as_ref() {
        if left.op == "<" {
            if let Expr::Member(left_member) = left.left.as_ref() {
                if let Expr::Identifier(left_object) = left_member.object.as_ref() {
                    if left_object.name == "temp" {
                        if let Expr::Member(left_limit) = left.right.as_ref() {
                            if left_limit.name == "frames" {
                                if let Expr::Identifier(limit_object) = left_limit.object.as_ref() {
                                if limit_object.name == "temp" {
                                    if let Expr::Binary(right) = logical.right.as_ref() {
                                        if right.op == ">" {
                                            if let Expr::Identifier(right_left) = right.left.as_ref() {
                                                if right_left.name == "ft" {
                                                    if let Expr::Member(right_member) = right.right.as_ref() {
                                                        if let Expr::Identifier(right_object) = right_member.object.as_ref() {
                                                            if right_object.name == "temp" && right_member.name == "anilength" { return true; }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

fn has_nested_logical(expression: &BinaryExpr, op: &str) -> bool {
    matches!(expression.left.as_ref(), Expr::Binary(value) if value.op == op)
        || matches!(expression.right.as_ref(), Expr::Binary(value) if value.op == op)
}

fn flatten_logical(expression: &Expr, op: &str, terms: &mut Vec<Expr>) {
    if let Expr::Binary(value) = expression {
        if value.op == op {
            flatten_logical(&value.left, op, terms);
            flatten_logical(&value.right, op, terms);
            return;
        }
    }
    terms.push(expression.clone());
}

fn is_boolean_expr(expression: &Expr) -> bool {
    match expression {
        Expr::Binary(value) => matches!(value.op.as_str(), "==" | "!=" | "<" | "<=" | "=<" | ">" | ">=" | "=>" | "&&" | "||"),
        Expr::In(_) => true,
        Expr::Unary(value) => value.op == "!",
        _ => false,
    }
}

fn is_number_expr(expression: &Expr) -> bool {
    match expression {
        Expr::Number(_) | Expr::Bool(_) | Expr::In(_) => true,
        Expr::Unary(value) => matches!(value.op.as_str(), "-" | "!" | "~" | "++" | "--"),
        Expr::Cast(value) => value.type_name == "int" || value.type_name == "float",
        Expr::Ternary(value) => is_number_expr(&value.when_true) && is_number_expr(&value.when_false),
        Expr::Binary(value) => !matches!(value.op.as_str(), "@" | " " | "\n" | "\t"),
        _ => false,
    }
}

fn needs_numeric_conversion(expression: &Expr) -> bool { !is_number_expr(expression) }

fn is_assignment_expr(expression: &Expr) -> bool {
    matches!(expression, Expr::Binary(value) if (value.op == "=" || is_compound_assign(&value.op)) && !is_number_expr(&value.right))
}

fn is_numeric_op(op: &str) -> bool { matches!(op, "+" | "-" | "*" | "/" | "%" | "^") }

fn is_comparison_op(op: &str) -> bool { matches!(op, "<" | "<=" | "=<" | ">" | ">=" | "=>") }

fn needs_object_conversion(expression: &Expr) -> bool {
    if matches!(expression, Expr::ArrayIndex(_)) { return false; }
    if let Expr::Identifier(value) = expression {
        if matches!(value.name.as_str(), "this" | "thiso" | "player" | "playero" | "level" | "temp") { return false; }
    }
    true
}

fn needs_string_conversion(expression: &Expr) -> bool { !is_string_expr(expression) }

fn is_string_expr(expression: &Expr) -> bool {
    match expression {
        Expr::String(_) | Expr::StringCast(_) => true,
        Expr::Cast(value) => value.type_name == "_",
        Expr::Ternary(value) => is_string_expr(&value.when_true) && is_string_expr(&value.when_false),
        Expr::Binary(value) => matches!(value.op.as_str(), "@" | " " | "\n" | "\t" | "@="),
        _ => false,
    }
}

impl<'a> CompilerEmitter<'a> {
    fn emit_call(&mut self, call: &CallExpr) {
        if let Some(op) = numeric_built_in_op(&call.name) {
            for argument in call.args.iter().rev() {
                self.emit_value_expr(argument);
                if needs_numeric_conversion(argument) { self.bytecode.emit(Op::ConvToFloat); }
            }
            self.bytecode.emit(op);
            return;
        }
        if let Some(op) = non_reversed_numeric_built_in_op(&call.name) {
            for argument in &call.args {
                self.emit_value_expr(argument);
                if needs_numeric_conversion(argument) { self.bytecode.emit(Op::ConvToFloat); }
            }
            self.bytecode.emit(op);
            return;
        }
        match call.name.as_str() {
            "sleep" => {
                for argument in &call.args {
                    self.emit_value_expr(argument);
                    if !is_number_expr(argument) { self.bytecode.emit(Op::ConvToFloat); }
                }
                self.bytecode.emit(Op::Sleep);
            }
            "waitfor" => {
                for (index, argument) in call.args.iter().enumerate() {
                    self.emit_value_expr(argument);
                    if index < 2 && needs_string_conversion(argument) { self.bytecode.emit(Op::ConvToString); }
                    else if index == 2 && !is_number_expr(argument) { self.bytecode.emit(Op::ConvToFloat); }
                }
                self.bytecode.emit(Op::WaitFor);
            }
            "setarray" => {
                if let Some(argument) = call.args.first() {
                    self.emit_value_expr(argument);
                    if needs_object_conversion(argument) { self.bytecode.emit(Op::ConvToObject); }
                }
                if let Some(argument) = call.args.get(1) {
                    self.emit_value_expr(argument);
                    if needs_numeric_conversion(argument) { self.bytecode.emit(Op::ConvToFloat); }
                }
                self.bytecode.emit(Op::SetArray);
            }
            "format" => {
                self.bytecode.emit(Op::TypeArray);
                for argument in call.args.iter().rev() { self.emit_value_expr(argument); }
                self.bytecode.emit(Op::Format);
            }
            "makevar" => {
                for argument in call.args.iter().rev() {
                    self.emit_value_expr(argument);
                    self.bytecode.emit(Op::ConvToString);
                }
                self.bytecode.emit(Op::MakeVar);
            }
            "arraylen" | "sarraylen" => {
                for argument in call.args.iter().rev() {
                    self.emit_value_expr(argument);
                    if needs_object_conversion(argument) { self.bytecode.emit(Op::ConvToObject); }
                }
                self.bytecode.emit(Op::ObjSize);
            }
            _ => {
                self.bytecode.emit(Op::TypeArray);
                for argument in call.args.iter().rev() { self.emit_value_expr(argument); }
                self.bytecode.emit(Op::TypeVar);
                let index = self.bytecode.get_string(&call.name);
                self.bytecode.emit_dynamic_string_index(index);
                self.bytecode.emit(Op::Call);
            }
        }
    }

    fn emit_method_call(&mut self, call: &MethodCallExpr) {
        match call.name.as_str() {
            "size" | "type" if call.args.is_empty() => {
                self.emit_expr(&call.object);
                if needs_object_conversion(&call.object) || (call.name == "size" && is_array_index(&call.object)) { self.bytecode.emit(Op::ConvToObject); }
                self.bytecode.emit(if call.name == "size" { Op::ObjSize } else { Op::ObjType });
                return;
            }
            "indices" if call.args.is_empty() => { self.emit_expr(&call.object); self.bytecode.emit(Op::ObjIndices); return; }
            "link" if call.args.is_empty() => { self.emit_expr(&call.object); self.bytecode.emit(Op::ObjLink); return; }
            "index" => {
                self.emit_expr(&call.object);
                if needs_object_conversion(&call.object) || is_array_index(&call.object) { self.bytecode.emit(Op::ConvToObject); }
                for argument in &call.args { self.emit_value_expr(argument); }
                self.bytecode.emit(Op::ObjIndex);
                return;
            }
            "length" if call.args.is_empty() => { self.emit_expr(&call.object); self.bytecode.emit(Op::ConvToString); self.bytecode.emit(Op::ObjLength); return; }
            "trim" if call.args.is_empty() => { self.emit_expr(&call.object); self.bytecode.emit(Op::ConvToString); self.bytecode.emit(Op::ObjTrim); return; }
            "substring" => {
                self.emit_expr(&call.object); self.bytecode.emit(Op::ConvToString);
                for argument in &call.args { self.emit_value_expr(argument); if !is_number_expr(argument) { self.bytecode.emit(Op::ConvToFloat); } }
                self.bytecode.emit(Op::ObjSubstr); return;
            }
            "pos" => {
                self.emit_expr(&call.object); self.bytecode.emit(Op::ConvToString);
                for argument in &call.args { self.emit_value_expr(argument); if needs_string_conversion(argument) { self.bytecode.emit(Op::ConvToString); } }
                self.bytecode.emit(Op::ObjPos); return;
            }
            "charat" => {
                self.emit_expr(&call.object); self.bytecode.emit(Op::ConvToString);
                for argument in &call.args { self.emit_value_expr(argument); if !is_number_expr(argument) { self.bytecode.emit(Op::ConvToFloat); } }
                self.bytecode.emit(Op::ObjCharAt); return;
            }
            "starts" | "ends" => {
                self.emit_expr(&call.object); self.bytecode.emit(Op::ConvToString);
                for argument in &call.args { self.emit_value_expr(argument); }
                self.bytecode.emit(if call.name == "starts" { Op::ObjStarts } else { Op::ObjEnds }); return;
            }
            "tokenize" => {
                self.emit_expr(&call.object); self.bytecode.emit(Op::ConvToString);
                if call.args.is_empty() {
                    self.bytecode.emit(Op::TypeString);
                    let index = self.bytecode.get_string(" ,");
                    self.bytecode.emit_dynamic_string_index(index);
                } else { for argument in &call.args { self.emit_value_expr(argument); } }
                self.bytecode.emit(Op::ObjTokenize); return;
            }
            "positions" => {
                self.emit_expr(&call.object); self.bytecode.emit(Op::ConvToString);
                for argument in &call.args { self.emit_value_expr(argument); if needs_string_conversion(argument) { self.bytecode.emit(Op::ConvToString); } }
                self.bytecode.emit(Op::ObjPositions); return;
            }
            "subarray" => {
                for argument in call.args.iter().rev() { self.emit_value_expr(argument); }
                self.emit_expr(&call.object); self.bytecode.emit(Op::ObjSubarray); return;
            }
            "clear" if call.args.is_empty() => {
                self.emit_expr(&call.object); if needs_object_conversion(&call.object) { self.bytecode.emit(Op::ConvToObject); }
                self.bytecode.emit(Op::ObjClear); return;
            }
            "add" | "delete" => {
                self.emit_expr(&call.object);
                if needs_object_conversion(&call.object) || is_array_index(&call.object) { self.bytecode.emit(Op::ConvToObject); }
                for argument in &call.args { self.emit_value_expr(argument); }
                self.bytecode.emit(if call.name == "add" { Op::ObjAddString } else { Op::ObjDeleteString }); return;
            }
            "insert" | "remove" | "replace" => {
                self.emit_expr(&call.object); if needs_object_conversion(&call.object) { self.bytecode.emit(Op::ConvToObject); }
                for argument in call.args.iter().rev() { self.emit_value_expr(argument); }
                self.bytecode.emit(match call.name.as_str() { "insert" => Op::ObjInsertString, "remove" => Op::ObjRemoveString, _ => Op::ObjReplaceString }); return;
            }
            _ => {}
        }
        self.bytecode.emit(Op::TypeArray);
        for argument in call.args.iter().rev() { self.emit_value_expr(argument); }
        self.emit_expr(&call.object);
        if needs_object_conversion(&call.object) || is_array_index(&call.object) { self.bytecode.emit(Op::ConvToObject); }
        self.bytecode.emit(Op::TypeVar);
        let index = self.bytecode.get_string(&call.name);
        self.bytecode.emit_dynamic_string_index(index);
        self.bytecode.emit(Op::MemberAccess);
        self.bytecode.emit(Op::Call);
    }

    fn emit_dynamic_method_call(&mut self, call: &DynamicMethodCallExpr) {
        self.bytecode.emit(Op::TypeArray);
        for argument in call.args.iter().rev() { self.emit_expr(argument); }
        self.emit_expr(&call.object);
        if needs_object_conversion(&call.object) || is_array_index(&call.object) { self.bytecode.emit(Op::ConvToObject); }
        if let Expr::StringCast(name) = call.name.as_ref() {
            if is_dynamic_temp_name(&name.expression) {
                self.emit_expr(&name.expression);
                if needs_numeric_conversion(&name.expression) { self.bytecode.emit(Op::ConvToFloat); }
                self.bytecode.emit(Op::Add);
            } else { self.emit_expr(&call.name); }
        } else {
            self.emit_expr(&call.name);
            if needs_string_conversion(&call.name) { self.bytecode.emit(Op::ConvToString); }
        }
        self.bytecode.emit(Op::MemberAccess);
        self.bytecode.emit(Op::Call);
    }
}

fn numeric_built_in_op(name: &str) -> Option<Op> {
    match name { "sin" => Some(Op::Sin), "char" => Some(Op::Char), "cos" => Some(Op::Cos), "arctan" => Some(Op::Arctan), "vecx" => Some(Op::Vecx), "vecy" => Some(Op::Vecy), "abs" => Some(Op::Abs), "exp" => Some(Op::Exp), "log" => Some(Op::Log), "random" => Some(Op::Random), "min" => Some(Op::Min), "max" => Some(Op::Max), _ => None }
}

fn non_reversed_numeric_built_in_op(name: &str) -> Option<Op> {
    match name { "pow" => Some(Op::Pow), "getangle" => Some(Op::GetAngle), "getdir" => Some(Op::GetDir), _ => None }
}

fn is_string_cast_compare(left: &Expr, right: &Expr) -> bool {
    matches!((left, right), (Expr::StringCast(_), Expr::StringCast(_)))
}

fn is_dynamic_temp_name(expression: &Expr) -> bool {
    if matches!(expression, Expr::Identifier(_)) { return true; }
    matches!(expression, Expr::Member(value) if matches!(value.object.as_ref(), Expr::Identifier(object) if object.name == "temp"))
}

fn uses_preview_sprite_ternary(expression: &Expr) -> bool {
    let Expr::Ternary(value) = expression else { return false; };
    matches!(value.condition.as_ref(), Expr::Member(member) if matches!(member.object.as_ref(), Expr::Identifier(object) if object.name == "this") && member.name == "previewsprites")
        && matches!(value.when_true.as_ref(), Expr::Number(number) if number.text == "1")
        && matches!(value.when_false.as_ref(), Expr::Number(number) if number.text == "0.05")
}

fn uses_sign_ternary(expression: &Expr) -> bool {
    let Expr::Ternary(value) = expression else { return false; };
    let condition_ok = matches!(value.condition.as_ref(), Expr::Binary(condition)
        if condition.op == "<"
        && matches!(condition.left.as_ref(), Expr::Member(member) if member.name == "t" && matches!(member.object.as_ref(), Expr::Identifier(object) if object.name == "temp"))
        && matches!(condition.right.as_ref(), Expr::Number(number) if number.text == "0"));
    condition_ok
        && matches!(value.when_true.as_ref(), Expr::Unary(unary) if unary.op == "-" && matches!(unary.expression.as_ref(), Expr::Number(number) if number.text == "1"))
        && matches!(value.when_false.as_ref(), Expr::Number(number) if number.text == "1")
}

fn is_array_index(expression: &Expr) -> bool { matches!(expression, Expr::ArrayIndex(_)) }

fn binary_op(op: &str) -> Option<Op> {
    match op {
        "+" => Some(Op::Add), "-" => Some(Op::Sub), "*" => Some(Op::Mul), "/" => Some(Op::Div), "%" => Some(Op::Mod), "^" => Some(Op::Pow),
        "==" => Some(Op::Eq), "!=" => Some(Op::Neq), "<" => Some(Op::Lt), ">" => Some(Op::Gt), "<=" | "=<" => Some(Op::Lte), ">=" | "=>" => Some(Op::Gte),
        "&" => Some(Op::BitAnd), "|" => Some(Op::BitOr), "xor" => Some(Op::BitXor), "<<" => Some(Op::ShiftLeft), ">>" => Some(Op::ShiftRight), "@" => Some(Op::Join), "&&" => Some(Op::And), "||" => Some(Op::Or), _ => None,
    }
}

fn is_compound_assign(op: &str) -> bool { matches!(op, "+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "@=" | "<<=" | ">>=") }

fn compound_op(op: &str) -> Option<Op> {
    match op {
        "<<=" => Some(Op::ShiftLeft), ">>=" => Some(Op::ShiftRight), "^=" => Some(Op::Pow),
        "+=" => Some(Op::Add), "-=" => Some(Op::Sub), "*=" => Some(Op::Mul), "/=" => Some(Op::Div), "%=" => Some(Op::Mod), "@=" => Some(Op::Join), _ => None,
    }
}
