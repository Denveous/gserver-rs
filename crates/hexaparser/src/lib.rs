//! Rust implementation of the HexaParser GameScript toolchain.
//!
//! The public API exposes the observable compiler data model: callers can
//! compile GS1/GS2 source, inspect bytecode segments, parse hex input, and
//! decompile bytecode back to GameScript text.

mod ast;
mod compiler;
mod decompiler;
mod parser;

pub use ast::*;
pub use compiler::*;
pub use decompiler::{default_output_path, decompile_code, parse_hex_bytes, read_input, read_segments};
pub use parser::{lex_gs1, lex_gs2, parse_gs1, parse_gs2, Token, TokenKind};

/// Preserved public spelling for the compiler entry point.
#[allow(non_snake_case)]
pub fn CompileCode(code: &str, script_type: &str, name: &str, with_header: bool, grammar: ScriptGrammar) -> CompilerResponse {
    compiler::compile_code(code, script_type, name, with_header, grammar)
}

/// Preserved public spelling for bytecode decompilation.
#[allow(non_snake_case)]
pub fn DecompileCode(data: &[u8]) -> Result<String, String> {
    decompile_code(data)
}

/// Preserved public spelling for hex parsing.
#[allow(non_snake_case)]
pub fn ParseHexBytes(value: &str) -> Result<Vec<u8>, String> {
    parse_hex_bytes(value)
}

/// Preserved public spelling for the default decompiler output path.
#[allow(non_snake_case)]
pub fn DefaultOutputPath(input_path: &str) -> String {
    default_output_path(input_path)
}

/// Preserved public spelling for reading binary or textual-hex input.
#[allow(non_snake_case)]
pub fn ReadInput(input_path: &str) -> Result<Vec<u8>, String> {
    read_input(input_path)
}

/// Native DLL API version.
#[no_mangle]
pub unsafe extern "C" fn hexaparser_api_version(output: *mut std::ffi::c_char, capacity: usize) -> i32 {
    if output.is_null() || capacity == 0 {
        return 1;
    }
    write_c_buffer(output, capacity, "1.0");
    0
}

/// Compile a UTF-8 source file through the native C ABI.
#[no_mangle]
pub unsafe extern "C" fn hexaparser_compile_file_utf8(
    input_path: *const std::ffi::c_char,
    output_path: *const std::ffi::c_char,
    grammar: *const std::ffi::c_char,
    script_type: *const std::ffi::c_char,
    name: *const std::ffi::c_char,
    with_header: i32,
    error_buffer: *mut std::ffi::c_char,
    error_capacity: usize,
) -> i32 {
    let result = std::panic::catch_unwind(|| {
        let input = c_string(input_path)?;
        let output = c_string(output_path)?;
        let grammar_text = c_string(grammar)?;
        let mut script_type_text = c_string(script_type)?;
        let mut name_text = c_string(name)?;
        if input.is_empty() {
            return Err("input path is required".to_string());
        }
        if output.is_empty() {
            return Err("output path is required".to_string());
        }
        let grammar = if grammar_text.is_empty() { "gs2" } else { grammar_text.as_str() };
        let grammar = match grammar.to_ascii_lowercase().as_str() {
            "gs1" => ScriptGrammar::GS1,
            "gs2" => ScriptGrammar::GS2,
            _ => return Err(format!("unknown grammar \"{}\"", grammar)),
        };
        if script_type_text.is_empty() { script_type_text = "weapon".to_string(); }
        if name_text.is_empty() { name_text = "npc".to_string(); }
        let code = std::fs::read_to_string(&input).map_err(|e| e.to_string())?;
        let compiled = compiler::compile_code(&code, &script_type_text, &name_text, with_header != 0, grammar);
        if !compiled.success {
            return Err(compiled.err_msg);
        }
        if let Some(parent) = std::path::Path::new(&output).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        std::fs::write(&output, compiled.byte_code).map_err(|e| e.to_string())?;
        Ok(())
    });
    match result {
        Ok(Ok(())) => { write_c_buffer(error_buffer, error_capacity, ""); 0 }
        Ok(Err(message)) => { write_c_buffer(error_buffer, error_capacity, &message); 1 }
        Err(panic) => {
            let message = if let Some(value) = panic.downcast_ref::<&str>() { format!("panic: {}", value) }
                else if let Some(value) = panic.downcast_ref::<String>() { format!("panic: {}", value) }
                else { "panic: unknown panic".to_string() };
            write_c_buffer(error_buffer, error_capacity, &message);
            1
        }
    }
}

/// Decompile a bytecode file through the native C ABI.
#[no_mangle]
pub unsafe extern "C" fn hexaparser_decompile_file_utf8(
    input_path: *const std::ffi::c_char,
    output_path: *const std::ffi::c_char,
    error_buffer: *mut std::ffi::c_char,
    error_capacity: usize,
) -> i32 {
    let result = std::panic::catch_unwind(|| {
        let input = c_string(input_path)?;
        let output = c_string(output_path)?;
        if input.is_empty() { return Err("input path is required".to_string()); }
        if output.is_empty() { return Err("output path is required".to_string()); }
        let data = read_input(&input)?;
        let source = decompile_code(&data)?;
        if let Some(parent) = std::path::Path::new(&output).parent() {
            if !parent.as_os_str().is_empty() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
        }
        std::fs::write(&output, source.as_bytes()).map_err(|e| e.to_string())?;
        Ok(())
    });
    match result {
        Ok(Ok(())) => { write_c_buffer(error_buffer, error_capacity, ""); 0 }
        Ok(Err(message)) => { write_c_buffer(error_buffer, error_capacity, &message); 1 }
        Err(panic) => {
            let message = if let Some(value) = panic.downcast_ref::<&str>() { format!("panic: {}", value) }
                else if let Some(value) = panic.downcast_ref::<String>() { format!("panic: {}", value) }
                else { "panic: unknown panic".to_string() };
            write_c_buffer(error_buffer, error_capacity, &message);
            1
        }
    }
}

unsafe fn c_string(value: *const std::ffi::c_char) -> Result<String, String> {
    if value.is_null() { return Ok(String::new()); }
    Ok(std::ffi::CStr::from_ptr(value).to_string_lossy().into_owned())
}

unsafe fn write_c_buffer(buffer: *mut std::ffi::c_char, capacity: usize, value: &str) {
    if buffer.is_null() || capacity == 0 { return; }
    let bytes = value.as_bytes();
    let len = if capacity <= 1 { 0 } else { bytes.len().min(capacity - 1) };
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.cast::<u8>(), len);
    *buffer.add(len) = 0;
}
