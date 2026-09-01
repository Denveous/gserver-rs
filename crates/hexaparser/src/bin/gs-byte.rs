use hexaparser::{compile_code, default_output_path, decompile_code, parse_hex_bytes, read_input, ScriptGrammar};
use std::fs;
use std::path::{Path, PathBuf};

fn usage() { eprintln!("Usage: gs-byte compile|decompile [options] INPUT"); }

fn main() {
    let mut arguments = std::env::args().skip(1);
    let Some(command) = arguments.next() else { usage(); std::process::exit(2); };
    let args: Vec<String> = arguments.collect();
    match command.as_str() {
        "compile" => compile_command(&args),
        "decompile" => decompile_command(&args),
        _ => { usage(); std::process::exit(2); }
    }
}

fn compile_command(args: &[String]) {
    let (output, grammar, script_type, name, header, input) = match parse_compile_args(args) {
        Ok(value) => value,
        Err(message) => exit_error(&message),
    };
    let code = match fs::read(&input) { Ok(value) => String::from_utf8_lossy(&value).into_owned(), Err(error) => exit_error(&error.to_string()) };
    let result = compile_code(&code, &script_type, &name, header, grammar);
    if !result.success { exit_error(&result.err_msg); }
    let output = output.unwrap_or_else(|| {
        let path = Path::new(&input);
        let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or_default();
        path.with_file_name(format!("{}.gs2bc", stem)).to_string_lossy().into_owned()
    });
    if let Some(parent) = Path::new(&output).parent() {
        if !parent.as_os_str().is_empty() { let _ = fs::create_dir_all(parent); }
    }
    if let Err(error) = fs::write(&output, result.byte_code) { exit_error(&error.to_string()); }
    eprintln!("wrote {}", output);
}

fn decompile_command(args: &[String]) {
    let (output, input) = match parse_decompile_args(args) { Ok(value) => value, Err(message) => exit_error(&message) };
    let data = match read_input(&input) { Ok(value) => value, Err(error) => exit_error(&error) };
    let source = match decompile_code(&data) { Ok(value) => value, Err(error) => exit_error(&error) };
    let output = output.unwrap_or_else(|| default_output_path(&input));
    if let Some(parent) = Path::new(&output).parent() {
        if !parent.as_os_str().is_empty() { let _ = fs::create_dir_all(parent); }
    }
    if let Err(error) = fs::write(&output, source.as_bytes()) { exit_error(&error.to_string()); }
    eprintln!("wrote {}", output);
}

fn parse_compile_args(args: &[String]) -> Result<(Option<String>, ScriptGrammar, String, String, bool, String), String> {
    let mut output = None;
    let mut grammar = "gs2".to_string();
    let mut script_type = "weapon".to_string();
    let mut name = "npc".to_string();
    let mut header = false;
    let mut input = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "--output" => { index += 1; output = Some(args.get(index).ok_or_else(|| "missing output path".to_string())?.clone()); }
            "--grammar" => { index += 1; grammar = args.get(index).ok_or_else(|| "missing grammar".to_string())?.clone(); }
            "--type" => { index += 1; script_type = args.get(index).ok_or_else(|| "missing script type".to_string())?.clone(); }
            "--name" => { index += 1; name = args.get(index).ok_or_else(|| "missing script name".to_string())?.clone(); }
            "--header" => header = true,
            value if value.starts_with('-') => return Err(format!("unknown option {}", value)),
            value => if input.replace(value.to_string()).is_some() { return Err("expected exactly one input path".to_string()); },
        }
        index += 1;
    }
    let input = input.ok_or_else(|| "expected exactly one input path".to_string())?;
    let grammar = match grammar.as_str() { "gs1" => ScriptGrammar::GS1, "gs2" => ScriptGrammar::GS2, value => return Err(format!("unknown grammar {:?}", value)) };
    Ok((output, grammar, script_type, name, header, input))
}

fn parse_decompile_args(args: &[String]) -> Result<(Option<String>, String), String> {
    let mut output = None;
    let mut input = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "--output" => { index += 1; output = Some(args.get(index).ok_or_else(|| "missing output path".to_string())?.clone()); }
            value if value.starts_with('-') => return Err(format!("unknown option {}", value)),
            value => if input.replace(value.to_string()).is_some() { return Err("expected exactly one input path".to_string()); },
        }
        index += 1;
    }
    Ok((output, input.ok_or_else(|| "expected exactly one input path".to_string())?))
}

fn exit_error(message: &str) -> ! { eprintln!("error: {}", message); std::process::exit(1) }

