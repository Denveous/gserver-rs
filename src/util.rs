pub fn gtokenize_text(text: &str) -> String {
    let mut text = text.replace("\r\n", "\n").replace('\r', "\n");
    if text.is_empty() {
        return String::new();
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    
    let mut lines: Vec<&str> = text.split('\n').collect();
    if !lines.is_empty() {
        lines.pop(); // Remove the trailing empty line created by split
    }
    
    let mut tokens = Vec::with_capacity(lines.len());
    
    for line in lines {
        let line = line.replace('\r', "");
        let mut complex = line.trim().is_empty();
        
        if !complex {
            for ch in line.chars() {
                if ch < '!' || ch > '~' || ch == ',' || ch == '/' { // '!' is 33, '~' is 126
                    complex = true;
                    break;
                }
            }
        }
        
        if complex {
            let escaped = line.replace('\\', "\\\\").replace('"', "\"\"");
            tokens.push(format!("\"{}\"", escaped));
        } else {
            tokens.push(line);
        }
    }
    
    tokens.join(",")
}

pub fn guntokenize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_quote = false;
    let mut chars = text.chars().peekable();
    
    if let Some(&'"') = chars.peek() {
        in_quote = true;
        chars.next();
    }
    
    while let Some(ch) = chars.next() {
        match ch {
            ',' => {
                if in_quote {
                    out.push(ch);
                } else {
                    out.push('\n');
                    while let Some(&' ') = chars.peek() {
                        chars.next();
                    }
                    if let Some(&'"') = chars.peek() {
                        in_quote = true;
                        chars.next();
                    }
                }
            }
            '"' => {
                if in_quote {
                    if let Some(&'"') = chars.peek() {
                        out.push('"');
                        chars.next();
                    } else if let Some(&',') = chars.peek() {
                        in_quote = false;
                    }
                } else {
                    out.push(ch);
                }
            }
            '\\' => {
                if let Some(&'\\') = chars.peek() {
                    out.push('\\');
                    chars.next();
                }
            }
            _ => out.push(ch),
        }
    }
    
    out
}
