use crate::ast::*;
use std::collections::BTreeMap;

/// Tokens emitted by the two source lexers.  The names intentionally follow
/// the names in the canonical ANTLR grammars.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Identifier,
    Number,
    String,
    Command,
    MessageCode,
    RawMessageCode,
    Item,
    Carry,
    Direction,
    Gender,
    Color,
    Baddy,
    Concat,
    Walrus,
    LteAlt,
    GteAlt,
    NeqAlt,
    PlusAssign,
    MinusAssign,
    MulAssign,
    DivAssign,
    PowAssign,
    ModAssign,
    ConcatAssign,
    BwOrAssign,
    BwAndAssign,
    ShlAssign,
    ShrAssign,
    Inc,
    Dec,
    Shl,
    Shr,
    And,
    Or,
    Eq,
    Neq,
    Lte,
    Gte,
    Const,
    Enum,
    Public,
    Function,
    If,
    ElseIf,
    Else,
    For,
    While,
    Do,
    Switch,
    Case,
    Default,
    With,
    New,
    Return,
    Break,
    Continue,
    Goto,
    IntCast,
    FloatCast,
    Translate,
    In,
    True,
    False,
    Null,
    Bxor,
    Assign,
    Lt,
    Gt,
    Plus,
    Minus,
    Mul,
    Div,
    Mod,
    Pow,
    At,
    Not,
    BitInvert,
    Band,
    Bor,
    Question,
    Colon,
    Semi,
    Comma,
    Dot,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Eof,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub line: usize,
    pub column: usize,
    pub start: usize,
    pub end: usize,
}

impl Token {
    fn new(kind: TokenKind, text: impl Into<String>, line: usize, column: usize, start: usize, end: usize) -> Self {
        Self { kind, text: text.into(), line, column, start, end }
    }
}

#[derive(Clone, Debug)]
struct Lexed {
    tokens: Vec<Token>,
    error_line: Option<usize>,
}

fn unescape_string(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }
        match chars.next() {
            None => result.push('\\'),
            Some('n') => result.push('\n'),
            Some('r') => result.push('\r'),
            Some('t') => result.push('\t'),
            Some(other) => result.push(other),
        }
    }
    result.replace("\r\n", "\n")
}

fn is_identifier_start(ch: char) -> bool { ch.is_ascii_alphabetic() || ch == '_' || ch == '$' }
fn is_identifier_part(ch: char) -> bool { ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' }

fn keyword_kind(word: &str) -> Option<TokenKind> {
    Some(match word {
        "const" => TokenKind::Const,
        "enum" => TokenKind::Enum,
        "public" => TokenKind::Public,
        "function" => TokenKind::Function,
        "if" => TokenKind::If,
        "elseif" => TokenKind::ElseIf,
        "else" => TokenKind::Else,
        "for" => TokenKind::For,
        "while" => TokenKind::While,
        "do" => TokenKind::Do,
        "switch" => TokenKind::Switch,
        "case" => TokenKind::Case,
        "default" => TokenKind::Default,
        "with" => TokenKind::With,
        "new" => TokenKind::New,
        "return" => TokenKind::Return,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "goto" => TokenKind::Goto,
        "int" => TokenKind::IntCast,
        "float" => TokenKind::FloatCast,
        "_" => TokenKind::Translate,
        "in" => TokenKind::In,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        "null" => TokenKind::Null,
        "xor" => TokenKind::Bxor,
        "NL" | "SPC" | "TAB" => TokenKind::Concat,
        _ => return None,
    })
}

fn gs2_lexer(input: &str) -> Lexed {
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let bytes = input.as_bytes();
    let mut line = 1usize;
    let mut column = 0usize;
    let mut error_line = None;

    let advance = |text: &str, line: &mut usize, column: &mut usize| {
        for ch in text.chars() {
            if ch == '\n' { *line += 1; *column = 0; } else { *column += 1; }
        }
    };
    let emit = |tokens: &mut Vec<Token>, kind: TokenKind, text: String, start: usize, end: usize, line: usize, column: usize| {
        tokens.push(Token::new(kind, text, line, column, start, end));
    };

    // ANTLR chooses the longest rule at a position, then the first rule on a
    // tie.  Keeping operators longest-first gives the same result.
    let operators: &[(&str, TokenKind)] = &[
        ("<<=", TokenKind::ShlAssign), (">>=", TokenKind::ShrAssign),
        ("+=", TokenKind::PlusAssign), ("-=", TokenKind::MinusAssign),
        ("*=", TokenKind::MulAssign), ("/=", TokenKind::DivAssign),
        ("^=", TokenKind::PowAssign), ("%=", TokenKind::ModAssign),
        ("@=", TokenKind::ConcatAssign), ("|=", TokenKind::BwOrAssign),
        ("&=", TokenKind::BwAndAssign), ("++", TokenKind::Inc), ("--", TokenKind::Dec),
        ("<<", TokenKind::Shl), (">>", TokenKind::Shr), ("&&", TokenKind::And),
        ("||", TokenKind::Or), ("==", TokenKind::Eq), ("!=", TokenKind::Neq),
        ("<=", TokenKind::Lte), (">=", TokenKind::Gte), (":=", TokenKind::Walrus),
        ("=<", TokenKind::LteAlt), ("=>", TokenKind::GteAlt), ("<>", TokenKind::NeqAlt),
        ("=", TokenKind::Assign), ("<", TokenKind::Lt), (">", TokenKind::Gt),
        ("+", TokenKind::Plus), ("-", TokenKind::Minus), ("*", TokenKind::Mul),
        ("/", TokenKind::Div), ("%", TokenKind::Mod), ("^", TokenKind::Pow),
        ("@", TokenKind::At), ("!", TokenKind::Not), ("~", TokenKind::BitInvert),
        ("&", TokenKind::Band), ("|", TokenKind::Bor), ("?", TokenKind::Question),
        (":", TokenKind::Colon), (";", TokenKind::Semi), (",", TokenKind::Comma),
        (".", TokenKind::Dot), ("(", TokenKind::LParen), (")", TokenKind::RParen),
        ("{", TokenKind::LBrace), ("}", TokenKind::RBrace), ("[", TokenKind::LBracket),
        ("]", TokenKind::RBracket),
    ];

    while i < bytes.len() {
        let start = i;
        let start_line = line;
        let start_col = column;
        let rest = &input[i..];
        let first = rest.chars().next().unwrap();

        if rest.starts_with("//") {
            let end = rest.find('\n').unwrap_or(rest.len());
            advance(&rest[..end], &mut line, &mut column);
            i += end;
            continue;
        }
        if rest.starts_with("/*") {
            if let Some(close) = rest[2..].find("*/") {
                let end = 2 + close + 2;
                advance(&rest[..end], &mut line, &mut column);
                i += end;
            } else {
                advance(rest, &mut line, &mut column);
                i = bytes.len();
            }
            continue;
        }
        if first.is_whitespace() {
            let mut end = 0;
            for ch in rest.chars() {
                if !ch.is_whitespace() { break; }
                end += ch.len_utf8();
            }
            advance(&rest[..end], &mut line, &mut column);
            i += end;
            continue;
        }
        if first == '"' || first == '\'' {
            let quote = first;
            let mut end = first.len_utf8();
            let mut escaped = false;
            let mut closed = false;
            for ch in rest[first.len_utf8()..].chars() {
                end += ch.len_utf8();
                if escaped { escaped = false; continue; }
                if ch == '\\' { escaped = true; continue; }
                if ch == quote { closed = true; break; }
            }
            if !closed {
                error_line.get_or_insert(start_line);
                emit(&mut tokens, TokenKind::Invalid, rest.to_string(), start, bytes.len(), start_line, start_col);
                advance(rest, &mut line, &mut column);
                i = bytes.len();
            } else {
                let raw = &rest[quote.len_utf8()..end - quote.len_utf8()];
                emit(&mut tokens, TokenKind::String, unescape_string(raw), start, start + end, start_line, start_col);
                advance(&rest[..end], &mut line, &mut column);
                i += end;
            }
            continue;
        }
        if first == '#' {
            let mut end = 1usize;
            while end < rest.len() {
                let ch = rest[end..].chars().next().unwrap();
                if ch.is_ascii_alphanumeric() || ch == '_' { end += ch.len_utf8(); } else { break; }
            }
            // Message-code arguments may carry a parenthesized parameter
            // list; it remains a single string-like token to the GS1 AST.
            if end < rest.len() && rest.as_bytes()[end] == b'(' {
                let mut depth = 0i32;
                while end < rest.len() {
                    let ch = rest[end..].chars().next().unwrap();
                    end += ch.len_utf8();
                    if ch == '(' { depth += 1; } else if ch == ')' { depth -= 1; if depth == 0 { break; } }
                }
            }
            emit(&mut tokens, TokenKind::MessageCode, rest[..end].to_string(), start, start + end, start_line, start_col);
            advance(&rest[..end], &mut line, &mut column);
            i += end;
            continue;
        }
        if first.is_ascii_digit() || (first == '.' && rest[1..].chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)) {
            let mut end = 0usize;
            if rest.starts_with("0x") || rest.starts_with("0X") {
                end = 2;
                while end < rest.len() && rest.as_bytes()[end].is_ascii_hexdigit() { end += 1; }
            } else {
                while end < rest.len() && rest.as_bytes()[end].is_ascii_digit() { end += 1; }
                if end < rest.len() && rest.as_bytes()[end] == b'.' {
                    end += 1;
                    while end < rest.len() && rest.as_bytes()[end].is_ascii_digit() { end += 1; }
                }
            }
            let raw = &rest[..end];
            let text = if raw.len() > 2 && (raw.starts_with("0x") || raw.starts_with("0X")) {
                match i64::from_str_radix(&raw[2..], 16) { Ok(n) => n.to_string(), Err(_) => raw.to_string() }
            } else { raw.to_string() };
            emit(&mut tokens, TokenKind::Number, text, start, start + end, start_line, start_col);
            advance(&rest[..end], &mut line, &mut column);
            i += end;
            continue;
        }
        if is_identifier_start(first) {
            let mut end = first.len_utf8();
            while end < rest.len() {
                let ch = rest[end..].chars().next().unwrap();
                if !is_identifier_part(ch) { break; }
                end += ch.len_utf8();
            }
            while end + 1 < rest.len() && &rest[end..end + 2] == "::" {
                let after = end + 2;
                if after >= rest.len() { break; }
                let ch = rest[after..].chars().next().unwrap();
                if !is_identifier_start(ch) { break; }
                end = after + ch.len_utf8();
                while end < rest.len() {
                    let ch = rest[end..].chars().next().unwrap();
                    if !is_identifier_part(ch) { break; }
                    end += ch.len_utf8();
                }
            }
            let word = &rest[..end];
            let kind = keyword_kind(word).unwrap_or(TokenKind::Identifier);
            let text = match word {
                "NL" => "\n".to_string(),
                "SPC" => " ".to_string(),
                "TAB" => "\t".to_string(),
                _ => word.to_string(),
            };
            emit(&mut tokens, kind, text, start, start + end, start_line, start_col);
            advance(&rest[..end], &mut line, &mut column);
            i += end;
            continue;
        }
        let mut matched = false;
        for (literal, kind) in operators {
            if rest.starts_with(literal) {
                emit(&mut tokens, kind.clone(), (*literal).to_string(), start, start + literal.len(), start_line, start_col);
                advance(literal, &mut line, &mut column);
                i += literal.len();
                matched = true;
                break;
            }
        }
        if matched { continue; }
        error_line.get_or_insert(start_line);
        let end = first.len_utf8();
        emit(&mut tokens, TokenKind::Invalid, rest[..end].to_string(), start, start + end, start_line, start_col);
        advance(&rest[..end], &mut line, &mut column);
        i += end;
    }
    tokens.push(Token::new(TokenKind::Eof, "", line, column, input.len(), input.len()));
    Lexed { tokens, error_line }
}

/// Lex GS2 source, including the C#-compatibility token normalization used by
/// the original implementation.
pub fn lex_gs2(input: &str) -> Vec<Token> { gs2_lexer(input).tokens }

// The GS1 command vocabulary is deliberately kept as data.  The original
// lexer uses the same names in its command predicates and command-mode stack.
const GS1_COMMANDS: &[(&str, &str)] = &[
    ("gr-debugger", ""), ("setstring", "VS"), ("addstring", "VS"),
    ("insertstring", "VES"), ("replacestring", "VES"), ("removestring", "VS"),
    ("deletestring", "VE"), ("set ", "V"), ("unset ", "V"), ("sleep", "E"),
    ("setarray", "VE"), ("timereverywhere", ""), ("setgif ", "S"),
    ("setimg", "S"), ("setimgpart", "SEEEE"), ("hide", ""), ("show", ""),
    ("dontblock", ""), ("blockagain", ""), ("drawoverplayer", ""),
    ("drawovertrees", ""), ("drawunderplayer", ""), ("drawaslight", ""),
    ("seteffectmode ", "EEEE"), ("canbecarried", ""), ("cannotbecarried", ""),
    ("canbepushed", ""), ("cannotbepushed", ""), ("canbepulled", ""),
    ("cannotbepulled", ""), ("move ", "EEEE"), ("say ", "E"), ("say2", "R"),
    ("lay ", "I"), ("lay2", "IEE"), ("take ", "I"), ("take2", "E"),
    ("message", "S"), ("timershow", ""), ("showcharacter", ""),
    ("setcharprop", "MS"), ("setcharani", "S"), ("setchargender", "G"),
    ("triggeraction", "EESL"), ("putnpc", "SSEE"), ("putnpc2", "EEZ"),
    ("callnpc", "ES"), ("callweapon", "ESS"), ("destroy", ""),
    ("carryobject", "U"), ("throwcarry", ""), ("followplayer", ""),
    ("toinventory", "S"), ("toweapons", "S"), ("setcoloreffect", "EEEE"),
    ("setzoomeffect", "E"), ("showimg", "ESEE"), ("showimg2", "ESEEE"),
    ("showani", "EEEDS"), ("showani2", "EEEEDS"), ("showpoly", "EE"),
    ("showpoly2", "EE"), ("showtext", "EEESSS"), ("showtext2", "EEEESSS"),
    ("hideimg", "E"), ("hideimgs", "EE"), ("changeimgpart", "EEEEE"),
    ("changeimgvis", "EE"), ("changeimgcolors", "EEEEE"), ("changeimgzoom", "EE"),
    ("changeimgmode", "EE"), ("shootarrow", "D"), ("shootfireball", "D"),
    ("shootfireblast", "D"), ("shootnuke", "D"), ("shootball", ""),
    ("spyfire", "EE"), ("hitplayer", "EEEE"), ("hitnpc", "EEEE"),
    ("hitobjects", "EEE"), ("hidelocal", ""), ("showlocal", ""),
    ("dontblocklocal", ""), ("blockagainlocal", ""), ("takehorse", ""),
    ("tokenize ", "S"), ("tokenize2", "SS"), ("setshape ", "EEE"),
    ("setshape2", "EEE"), ("wraptext ", "ESS"), ("wraptext2 ", "EESS"),
    ("setshootparams ", "L"), ("shoot ", "EEEEEES"), ("setlevel ", "S"),
    ("setlevel2", "SEE"), ("seturllevel", "S"), ("setbody", "S"),
    ("sethead", "S"), ("setsword", "SE"), ("setshield", "SE"),
    ("setbow", "S"), ("setani", "S"), ("setplayerdir", "D"),
    ("setgender", "G"), ("setskincolor", "C"), ("setcoatcolor", "C"),
    ("setsleevecolor", "C"), ("setshoecolor", "C"), ("setbeltcolor", "C"),
    ("setplayerprop", "MS"), ("takeplayercarry", ""), ("takeplayerhorse", ""),
    ("disableweapons", ""), ("enableweapons", ""), ("freezeplayer ", "E"),
    ("freezeplayer2", ""), ("unfreezeplayer", ""), ("hideplayer", "E"),
    ("hidesword", "E"), ("hurt ", "E"), ("disabledefmovement", ""),
    ("enabledefmovement", ""), ("disableselectweapons", ""),
    ("enableselectweapons", ""), ("disablepause", ""), ("enablepause", ""),
    ("disablemap", ""), ("enablemap", ""), ("enablefeatures", "E"),
    ("replaceani", "SS"), ("attachplayertoobj", "EE"), ("detachplayer", ""),
    ("updateboard", "EEEE"), ("updateboard2", "EEEE"), ("putobject", "SEE"),
    ("putbomb", "EEE"), ("putexplosion ", "EEE"), ("putexplosion2", "EEEE"),
    ("putleaps", "EEE"), ("puthorse", "SEE"), ("setbackpal", "S"),
    ("setbacktile", "E"), ("setbacktile2", "EEEEE"), ("setletters", "S"),
    ("setmap", "SSEE"), ("setminimap", "SSEE"), ("seteffect ", "EEEE"),
    ("setfocus", "EE"), ("resetfocus", ""), ("noplayerkilling", ""),
    ("noplayeronwall", ""), ("removebomb", "E"), ("removearrow", "E"),
    ("removeitem", "E"), ("removeexplo", "E"), ("removehorse", "E"),
    ("explodebomb", "E"), ("reflectarrow", "E"), ("addtiledef ", "SSE"),
    ("addtiledef2", "SSEE"), ("removetiledefs", "S"), ("loadmap", "S"),
    ("updateterrain", ""), ("showstats", "E"), ("putcomp", "BEE"),
    ("putnewcomp", "BEESE"), ("hitcompu", "EEEE"), ("removecompus", ""),
    ("play ", "S"), ("play2 ", "SEEE"), ("playlooped", "S"),
    ("stopsound", "S"), ("stopmidi", ""), ("setmusicvolume", "EE"),
    ("openurl ", "S"), ("openurl2 ", "SEE"), ("showfile", "S"),
    ("join", "S"), ("setcursor ", "E"), ("setcursor2", "S"),
    ("canwarp", ""), ("canwarp2", ""), ("cannotwarp", ""),
    ("addweapon", "S"), ("removeweapon", "S"), ("setspritesimage", "S"),
    ("setstatusimage", "S"), ("addguildmember", "SSS"), ("removeguildmember", "SSS"),
    ("removeguild", "S"), ("copystrings", "SS"), ("sendtorc", "S"),
    ("sendtonc", "S"), ("sendpm", "R"), ("setpm", "S"),
    ("sendrpgmessage", "S"), ("serverwarp", "S"), ("setz ", "EEEEEEEE"),
    ("copylevel", "SS"), ("deletelevel", "S"), ("saveinfo", "SS"),
    ("savelog", "S"), ("savelog2", "SS"), ("warpto", "SEE"),
    ("enabledamagereactions", ""), ("disabledamagereactions", ""),
];

fn command_at(input: &str, position: usize) -> Option<(&'static str, &'static str, usize)> {
    let rest = &input[position..];
    // Literal order in the ANTLR grammar is significant for equal prefixes;
    // longest matching text is the useful equivalent here.
    let mut best = None;
    for &(name, modes) in GS1_COMMANDS {
        if rest.starts_with(name) && best.map(|v: (&str, &str, usize)| name.len() > v.2).unwrap_or(true) {
            best = Some((name, modes, name.len()));
        }
    }
    best
}

/// Lex enough of GS1's mode-sensitive command language to retain the exact
/// source spans needed by the builder.  Ordinary GS1 expressions use the same
/// token spellings as GS2, while a command at statement position is marked so
/// its mode-specific arguments can be reconstructed by the parser.
pub fn lex_gs1(input: &str) -> Vec<Token> {
    let mut base = gs2_lexer(input);
    let mut command_starts = Vec::<(usize, &'static str)>::new();
    let mut pos = 0usize;
    let mut statement_start = true;
    while pos < input.len() {
        let ch = input[pos..].chars().next().unwrap();
        if ch.is_whitespace() { pos += ch.len_utf8(); continue; }
        if input[pos..].starts_with("//") {
            pos += input[pos..].find('\n').unwrap_or(input.len() - pos);
            continue;
        }
        if input[pos..].starts_with("/*") {
            if let Some(n) = input[pos + 2..].find("*/") { pos += n + 4; } else { break; }
            continue;
        }
        if statement_start {
            if let Some((name, _, length)) = command_at(input, pos) {
                command_starts.push((pos, name));
                pos += length;
                statement_start = false;
                continue;
            }
        }
        match ch {
            ';' | '{' | '}' => statement_start = true,
            _ => statement_start = false,
        }
        pos += ch.len_utf8();
    }
    // Replace the first ordinary token at each command span.  command spans
    // with a trailing space deliberately consume that space, as GS1's lexer
    // does when it pushes its parameter mode.
    for (start, name) in command_starts.into_iter().rev() {
        if let Some(index) = base.tokens.iter().position(|t| t.start >= start && t.kind != TokenKind::Eof) {
            let token = &base.tokens[index];
            if token.start == start || (token.start < start + name.len() && token.end >= start + name.len()) {
                let end = start + name.len();
                let line = token.line;
                let column = token.column;
                base.tokens[index] = Token::new(TokenKind::Command, name.trim_end(), line, column, start, end);
            }
        }
    }
    base.tokens
}

#[derive(Clone, Debug)]
struct ParseError { line: usize, message: String }

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    first_error: Option<ParseError>,
    gs1: bool,
    allow_gs1_assignment: bool,
    lambda_id: i32,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str, lexed: Lexed, gs1: bool) -> Self {
        let Lexed { tokens, error_line } = lexed;
        let mut parser = Self { source, tokens, pos: 0, first_error: None, gs1, allow_gs1_assignment: false, lambda_id: 100 };
        if let Some(line) = error_line { parser.error_at(line, "invalid token"); }
        parser
    }
    fn current(&self) -> &Token { &self.tokens[self.pos.min(self.tokens.len() - 1)] }
    fn at(&self, kind: &TokenKind) -> bool { &self.current().kind == kind }
    fn at_text(&self, text: &str) -> bool { self.current().text == text }
    fn advance(&mut self) -> Token { let token = self.current().clone(); if self.pos + 1 < self.tokens.len() { self.pos += 1; } token }
    fn take(&mut self, kind: &TokenKind) -> Option<Token> { if self.at(kind) { Some(self.advance()) } else { None } }
    fn expect(&mut self, kind: TokenKind) -> Option<Token> {
        if self.at(&kind) { Some(self.advance()) } else { self.error_current(&format!("expected {:?}", kind)); None }
    }
    fn error_current(&mut self, message: &str) { self.error_at(self.current().line, message); }
    fn error_at(&mut self, line: usize, message: &str) { if self.first_error.is_none() { self.first_error = Some(ParseError { line, message: message.to_string() }); } }
    fn good(&self) -> bool { self.first_error.is_none() }

    fn expression_with_gs1_assignment(&mut self, enabled: bool) -> Expr {
        let previous = self.allow_gs1_assignment;
        self.allow_gs1_assignment = enabled;
        let value = self.expression();
        self.allow_gs1_assignment = previous;
        value
    }

    fn script(&mut self) -> Result<ProgramNode, ParseError> {
        let mut constants = BTreeMap::new();
        let mut enums = BTreeMap::new();
        let mut items = Vec::new();
        let mut flags = 0i32;
        while !self.at(&TokenKind::Eof) && self.good() {
            if self.at(&TokenKind::Const) {
                self.advance();
                let name = self.expect(TokenKind::Identifier).map(|t| t.text).unwrap_or_default();
                self.expect(TokenKind::Assign);
                let expression = self.expression();
                self.take(&TokenKind::Semi);
                constants.insert(name, expression);
            } else if self.at(&TokenKind::Enum) {
                self.advance();
                let name = self.expect(TokenKind::Identifier).map(|t| t.text).unwrap_or_default();
                self.expect(TokenKind::LBrace);
                let mut values = BTreeMap::new();
                let mut index = 0i32;
                while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) && self.good() {
                    let member = self.expect(TokenKind::Identifier).map(|t| t.text).unwrap_or_default();
                    if self.take(&TokenKind::Assign).is_some() { index = enum_value(&self.expression()); }
                    values.insert(member, index);
                    index += 1;
                    if self.take(&TokenKind::Comma).is_none() { break; }
                }
                self.expect(TokenKind::RBrace);
                self.take(&TokenKind::Semi);
                enums.insert(name, values);
            } else {
                let item = self.declaration_item();
                match item {
                    Some(ParsedItem::Function(function)) => items.push(ProgramItem::Function(function)),
                    Some(ParsedItem::Statement(statement)) => {
                        if !self.gs1 { flags |= scan_gs1_event_flags(&statement); }
                        items.push(ProgramItem::Statement(statement));
                    }
                    None => { if self.good() { self.error_current("expected declaration"); } else { break; } }
                }
            }
        }
        if let Some(error) = self.first_error.take() { return Err(error); }
        let gs1_event_flags = if self.gs1 { scan_program_events(&items) } else { flags };
        Ok(ProgramNode { constants, enums, items, gs1_event_flags })
    }

    fn declaration_item(&mut self) -> Option<ParsedItem> {
        let public = self.take(&TokenKind::Public).is_some();
        if self.at(&TokenKind::Function) {
            self.advance();
            return Some(ParsedItem::Function(self.function(public)));
        }
        if public { self.error_current("expected function"); return None; }
        self.statement().map(ParsedItem::Statement)
    }

    fn function(&mut self, public: bool) -> FunctionNode {
        let mut parts = Vec::new();
        if let Some(t) = self.expect(TokenKind::Identifier) { parts.push(t.text); }
        while self.take(&TokenKind::Dot).is_some() {
            if let Some(t) = self.expect(TokenKind::Identifier) { parts.push(t.text); }
        }
        let full = parts.join(".");
        let (name, object_name) = if let Some(index) = full.rfind('.') { (full[index + 1..].to_string(), full[..index].to_string()) } else { (full, String::new()) };
        self.expect(TokenKind::LParen);
        let args = if self.at(&TokenKind::RParen) { Vec::new() } else { self.arguments() };
        self.expect(TokenKind::RParen);
        let body = if self.at(&TokenKind::Semi) { self.advance(); Vec::new() }
            else if self.at(&TokenKind::Eof) { Vec::new() }
            else if self.at(&TokenKind::RBrace) { Vec::new() }
            else if matches!(self.current().kind, TokenKind::Const | TokenKind::Enum | TokenKind::Function | TokenKind::Public) { Vec::new() }
            else { self.function_body_statement() };
        FunctionNode { name, object_name, public, args, body }
    }

    fn function_body_statement(&mut self) -> Vec<Stmt> {
        if self.at(&TokenKind::LBrace) { self.block_body() }
        else { self.statement().into_iter().collect() }
    }

    fn statement_body(&mut self) -> Vec<Stmt> {
        if self.at(&TokenKind::LBrace) { return self.block_body(); }
        if self.at(&TokenKind::Semi) { self.advance(); return Vec::new(); }
        self.statement().map(|s| vec![Stmt::Inline(InlineStmt { statement: Box::new(s) })]).unwrap_or_default()
    }

    fn block_body(&mut self) -> Vec<Stmt> {
        self.expect(TokenKind::LBrace);
        let mut body = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) && self.good() {
            if let Some(statement) = self.statement() {
                if !matches!(&statement, Stmt::Block(BlockStmt { body }) if body.is_empty()) { body.push(statement); }
            }
        }
        self.expect(TokenKind::RBrace);
        body
    }

    fn statement(&mut self) -> Option<Stmt> {
        if self.at(&TokenKind::Semi) { self.advance(); return Some(Stmt::Block(BlockStmt { body: Vec::new() })); }
        if self.at(&TokenKind::LBrace) { return Some(Stmt::Block(BlockStmt { body: self.block_body() })); }
        if self.at(&TokenKind::If) { return Some(self.if_statement()); }
        if self.at(&TokenKind::For) { return Some(self.for_statement()); }
        if self.at(&TokenKind::While) { return Some(self.while_statement()); }
        if self.at(&TokenKind::Do) { return Some(self.do_while_statement()); }
        if self.at(&TokenKind::Switch) { return Some(self.switch_statement()); }
        if self.at(&TokenKind::With) { return Some(self.with_statement()); }
        if self.at(&TokenKind::New) && self.looks_like_new_statement() { return Some(self.new_statement()); }
        if self.at(&TokenKind::Return) {
            self.advance();
            let expression = if self.at(&TokenKind::Semi) || self.at(&TokenKind::RBrace) || self.at(&TokenKind::Eof) { Expr::number("0") } else { self.expression() };
            self.take(&TokenKind::Semi);
            return Some(Stmt::Return(ReturnStmt { expression }));
        }
        if self.at(&TokenKind::Break) { self.advance(); self.expect(TokenKind::Semi); return Some(Stmt::Break); }
        if self.at(&TokenKind::Continue) { self.advance(); self.expect(TokenKind::Semi); return Some(Stmt::Continue); }
        if self.at(&TokenKind::Goto) {
            self.advance(); let label = self.expect(TokenKind::Identifier).map(|t| t.text).unwrap_or_default(); self.expect(TokenKind::Semi);
            return Some(Stmt::Goto(GotoStmt { label }));
        }
        if self.at(&TokenKind::Command) { return Some(self.command_statement()); }
        if self.at(&TokenKind::Identifier) && self.pos + 1 < self.tokens.len() && self.tokens[self.pos + 1].kind == TokenKind::Colon {
            let label = self.advance().text; self.advance(); return Some(Stmt::Label(LabelStmt { label }));
        }
        let first = if self.gs1 { self.expression_with_gs1_assignment(true) } else { self.expression() };
        let mut expressions = vec![first];
        while self.take(&TokenKind::Comma).is_some() {
            expressions.push(if self.gs1 { self.expression_with_gs1_assignment(true) } else { self.expression() });
        }
        if self.gs1 { self.take(&TokenKind::Semi); } else { self.expect(TokenKind::Semi); }
        if expressions.len() == 1 { Some(Stmt::Expr(ExprStmt { expression: expressions.remove(0) })) }
        else { Some(Stmt::Block(BlockStmt { body: expressions.into_iter().map(|expression| Stmt::Expr(ExprStmt { expression })).collect() })) }
    }

    fn looks_like_new_statement(&self) -> bool {
        self.pos + 1 < self.tokens.len() && self.tokens[self.pos + 1].kind == TokenKind::Identifier
    }

    fn if_statement(&mut self) -> Stmt {
        self.advance(); self.expect(TokenKind::LParen); let condition = self.expression(); self.expect(TokenKind::RParen);
        let then_body = self.statement_body();
        let mut else_body = Vec::new(); let mut has_else = false;
        if self.take(&TokenKind::Else).is_some() { has_else = true; else_body = self.statement_body(); }
        else if self.take(&TokenKind::ElseIf).is_some() {
            has_else = true;
            self.expect(TokenKind::LParen); let nested_condition = self.expression(); self.expect(TokenKind::RParen);
            let nested_then = self.statement_body();
            let mut nested_else = Vec::new(); let mut nested_has = false;
            if self.take(&TokenKind::Else).is_some() { nested_has = true; nested_else = self.statement_body(); }
            else if self.take(&TokenKind::ElseIf).is_some() { nested_has = true; nested_else = vec![self.if_tail()]; }
            else_body.push(Stmt::If(IfStmt { condition: nested_condition, then_body: nested_then, else_body: nested_else, has_else: nested_has }));
        }
        Stmt::If(IfStmt { condition, then_body, else_body, has_else })
    }

    fn if_tail(&mut self) -> Stmt {
        self.expect(TokenKind::LParen); let condition = self.expression(); self.expect(TokenKind::RParen);
        let then_body = self.statement_body();
        let mut else_body = Vec::new(); let mut has_else = false;
        if self.take(&TokenKind::Else).is_some() { has_else = true; else_body = self.statement_body(); }
        else if self.take(&TokenKind::ElseIf).is_some() { has_else = true; else_body = vec![self.if_tail()]; }
        Stmt::If(IfStmt { condition, then_body, else_body, has_else })
    }

    fn for_statement(&mut self) -> Stmt {
        self.advance(); self.expect(TokenKind::LParen);
        let first = if self.at(&TokenKind::Semi) { self.advance(); None } else { Some(if self.gs1 { self.expression_with_gs1_assignment(true) } else { self.expression() }) };
        if self.take(&TokenKind::Colon).is_some() {
            let source = self.expression(); self.expect(TokenKind::RParen); let body = self.statement_body();
            return Stmt::ForEach(ForEachStmt { name: first.unwrap_or_else(|| Expr::Null), source, body });
        }
        if self.take(&TokenKind::Semi).is_none() { self.expect(TokenKind::Semi); }
        let condition = if self.at(&TokenKind::Semi) { Expr::boolean(true) } else { self.expression_with_gs1_assignment(false) };
        self.expect(TokenKind::Semi);
        let post = if self.at(&TokenKind::RParen) { None } else { Some(if self.gs1 { self.expression_with_gs1_assignment(true) } else { self.expression() }) };
        self.expect(TokenKind::RParen); let body = self.statement_body();
        Stmt::For(ForStmt { init: first, condition, post, body })
    }

    fn while_statement(&mut self) -> Stmt {
        self.advance(); self.expect(TokenKind::LParen); let condition = self.expression(); self.expect(TokenKind::RParen); let body = self.statement_body();
        Stmt::While(WhileStmt { condition, body })
    }

    fn do_while_statement(&mut self) -> Stmt {
        self.advance(); let body = self.statement_body(); self.expect(TokenKind::While); self.expect(TokenKind::LParen); let condition = self.expression(); self.expect(TokenKind::RParen); self.take(&TokenKind::Semi);
        Stmt::DoWhile(DoWhileStmt { body, condition })
    }

    fn switch_statement(&mut self) -> Stmt {
        self.advance(); self.expect(TokenKind::LParen); let expression = self.expression(); self.expect(TokenKind::RParen); self.expect(TokenKind::LBrace);
        let mut cases = Vec::new(); let mut labels: Vec<Option<Expr>> = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) && self.good() {
            if self.take(&TokenKind::Case).is_some() { labels.push(Some(self.expression())); self.expect(TokenKind::Colon); }
            else if self.take(&TokenKind::Default).is_some() { labels.push(None); self.expect(TokenKind::Colon); }
            else { self.error_current("expected case"); break; }
            let mut body = Vec::new();
            while !self.at(&TokenKind::Case) && !self.at(&TokenKind::Default) && !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) && self.good() {
                if let Some(statement) = self.statement() { body.push(statement); }
            }
            if body.is_empty() { continue; }
            if labels.len() > 1 { labels.reverse(); }
            cases.push(SwitchCase { labels: labels.clone(), body });
            labels.clear();
        }
        if !labels.is_empty() { if labels.len() > 1 { labels.reverse(); } cases.push(SwitchCase { labels, body: Vec::new() }); }
        self.expect(TokenKind::RBrace);
        Stmt::Switch(SwitchStmt { expression, cases })
    }

    fn with_statement(&mut self) -> Stmt {
        self.advance(); self.expect(TokenKind::LParen); let target = self.expression(); self.expect(TokenKind::RParen); let body = self.statement_body();
        Stmt::With(WithStmt { target, body })
    }

    fn new_statement(&mut self) -> Stmt {
        self.advance(); let type_name = self.expect(TokenKind::Identifier).map(|t| t.text).unwrap_or_default();
        self.expect(TokenKind::LParen); let args = if self.at(&TokenKind::RParen) { Vec::new() } else { self.arguments() }; self.expect(TokenKind::RParen);
        let body = if self.at(&TokenKind::LBrace) { self.block_body() } else { Vec::new() };
        Stmt::New(NewStmt { type_name, args, body })
    }

    fn command_statement(&mut self) -> Stmt {
        let command = self.advance();
        let name = command.text.trim().to_string();
        let end_start = self.current().start;
        let mut end_pos = end_start;
        while !self.at(&TokenKind::Semi) && !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) { end_pos = self.advance().end; }
        let raw = if end_pos >= end_start && end_pos <= self.source.len() { self.source[end_start..end_pos].trim() } else { "" };
        let modes = GS1_COMMANDS.iter().find(|(n, _)| n.trim() == name).map(|(_, m)| *m).unwrap_or("");
        let args = command_args(raw, modes, self.gs1);
        self.take(&TokenKind::Semi);
        if name == "set" && !args.is_empty() { Stmt::Expr(ExprStmt { expression: Expr::binary(args[0].clone(), "=", Expr::boolean(true)) }) }
        else if name == "unset" && !args.is_empty() { Stmt::Expr(ExprStmt { expression: Expr::binary(args[0].clone(), "=", Expr::boolean(false)) }) }
        else { Stmt::Expr(ExprStmt { expression: Expr::Call(CallExpr { name, args }) }) }
    }

    fn arguments(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        if self.at(&TokenKind::RParen) || self.at(&TokenKind::RBracket) || self.at(&TokenKind::RBrace) { return args; }
        args.push(self.expression());
        while self.take(&TokenKind::Comma).is_some() {
            if self.at(&TokenKind::RParen) || self.at(&TokenKind::RBracket) || self.at(&TokenKind::RBrace) { break; }
            args.push(self.expression());
        }
        args
    }

    fn expression(&mut self) -> Expr { self.assignment() }
    fn assignment(&mut self) -> Expr {
        let left = self.conditional();
        let op = if !self.gs1 || self.allow_gs1_assignment {
            match self.current().kind {
                TokenKind::Assign | TokenKind::Walrus | TokenKind::PlusAssign | TokenKind::MinusAssign | TokenKind::MulAssign | TokenKind::DivAssign | TokenKind::PowAssign | TokenKind::ModAssign | TokenKind::ConcatAssign | TokenKind::ShlAssign | TokenKind::ShrAssign => Some(self.advance().text),
                _ => None,
            }
        } else { None };
        if let Some(op) = op {
            let op = if op == ":=" { "=".to_string() } else { op };
            let right = if self.gs1 { self.expression_with_gs1_assignment(false) } else { self.assignment() };
            Expr::binary(left, op, right)
        } else { left }
    }
    fn conditional(&mut self) -> Expr {
        let condition = self.logical_or();
        if self.take(&TokenKind::Question).is_some() { let when_true = self.expression(); self.expect(TokenKind::Colon); let when_false = self.expression(); Expr::Ternary(TernaryExpr { condition: Box::new(condition), when_true: Box::new(when_true), when_false: Box::new(when_false) }) } else { condition }
    }
    fn logical_or(&mut self) -> Expr { self.binary_chain(Parser::logical_and, &[TokenKind::Or]) }
    fn logical_and(&mut self) -> Expr { self.binary_chain(Parser::bitwise, &[TokenKind::And]) }
    fn bitwise(&mut self) -> Expr { self.binary_chain(Parser::equality, &[TokenKind::Band, TokenKind::Bor, TokenKind::Bxor, TokenKind::Shl, TokenKind::Shr]) }
    fn equality(&mut self) -> Expr {
        let mut value = self.comparison();
        loop {
            let kind = self.current().kind.clone();
            let op = match kind {
                TokenKind::Eq | TokenKind::Neq | TokenKind::NeqAlt => Some(self.advance().text),
                TokenKind::Assign | TokenKind::Walrus if self.gs1 && !self.allow_gs1_assignment => { self.advance(); Some("==".to_string()) },
                TokenKind::In => { self.advance(); Some("in".to_string()) },
                _ => None,
            };
            if let Some(op) = op {
                if op == "in" { let range = self.range_expression(); value = Expr::In(InExpr { expression: Box::new(value), lower: Box::new(range.0), upper: range.1.map(Box::new) }); }
                else {
                    let op = if op == "<>" { "!=".to_string() } else { op };
                    value = Expr::binary(value, op, self.comparison());
                }
            } else { break; }
        }
        value
    }
    fn range_expression(&mut self) -> (Expr, Option<Expr>) {
        if self.take(&TokenKind::Bor).is_some() {
            // A range uses the same token (`|`) as bitwise OR. Parse each
            // bound below the bitwise level so the closing delimiter cannot
            // be consumed as an operator.
            let lower = self.comparison();
            let upper = if self.take(&TokenKind::Comma).is_some() { Some(self.comparison()) } else { None };
            self.expect(TokenKind::Bor); return (lower, upper);
        }
        if self.take(&TokenKind::Lt).is_some() {
            let lower = self.comparison(); self.expect(TokenKind::Comma); let upper = self.comparison(); self.expect(TokenKind::Gt); return (lower, Some(upper));
        }
        (self.comparison(), None)
    }
    fn comparison(&mut self) -> Expr { self.binary_chain(Parser::concat, &[TokenKind::Lt, TokenKind::Lte, TokenKind::LteAlt, TokenKind::Gt, TokenKind::Gte, TokenKind::GteAlt]) }
    fn concat(&mut self) -> Expr { self.binary_chain(Parser::additive, &[TokenKind::At, TokenKind::Concat]) }
    fn additive(&mut self) -> Expr { self.binary_chain(Parser::multiplicative, &[TokenKind::Plus, TokenKind::Minus]) }
    fn multiplicative(&mut self) -> Expr { self.binary_chain(Parser::prefix, &[TokenKind::Mul, TokenKind::Div, TokenKind::Mod, TokenKind::Pow]) }

    fn binary_chain(&mut self, next: fn(&mut Parser<'a>) -> Expr, kinds: &[TokenKind]) -> Expr {
        let mut value = next(self);
        loop {
            if !kinds.iter().any(|kind| &self.current().kind == kind) { break; }
            let op = self.advance().text;
            let right = next(self);
            value = Expr::binary(value, op, right);
        }
        value
    }

    fn prefix(&mut self) -> Expr {
        match self.current().kind {
            TokenKind::Inc | TokenKind::Dec | TokenKind::Not | TokenKind::Minus | TokenKind::At | TokenKind::BitInvert => {
                let op = self.advance().text;
                let expression = self.prefix();
                if op == "@" { Expr::StringCast(StringCastExpr { expression: Box::new(expression) }) }
                else { Expr::Unary(UnaryExpr { op, expression: Box::new(expression), postfix: false }) }
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Expr {
        let mut expression = self.primary();
        loop {
            if self.take(&TokenKind::Dot).is_some() {
                if self.take(&TokenKind::LParen).is_some() { let name = self.expression(); self.expect(TokenKind::RParen); expression = Expr::DynamicMember(DynamicMemberExpr { object: Box::new(expression), name: Box::new(name) }); }
                else { let name = self.expect(TokenKind::Identifier).map(|t| t.text).unwrap_or_default(); expression = Expr::Member(MemberExpr { object: Box::new(expression), name }); }
            } else if self.take(&TokenKind::LBracket).is_some() {
                let indices = if self.at(&TokenKind::RBracket) { Vec::new() } else { self.arguments() };
                self.expect(TokenKind::RBracket);
                expression = match indices.len() { 0 => Expr::ArrayIndex(ArrayIndexExpr { target: Box::new(expression), index: Box::new(Expr::number("0")) }), 1 => Expr::ArrayIndex(ArrayIndexExpr { target: Box::new(expression), index: Box::new(indices.into_iter().next().unwrap()) }), _ => Expr::MultiArrayIndex(MultiArrayIndexExpr { target: Box::new(expression), indices }) };
            } else if self.at(&TokenKind::Inc) || self.at(&TokenKind::Dec) {
                let op = self.advance().text; expression = Expr::Unary(UnaryExpr { op, expression: Box::new(expression), postfix: true });
            } else if self.take(&TokenKind::LParen).is_some() {
                let args = if self.at(&TokenKind::RParen) { Vec::new() } else { self.arguments() }; self.expect(TokenKind::RParen);
                expression = match expression {
                    Expr::Identifier(value) => Expr::Call(CallExpr { name: value.name, args }),
                    Expr::Member(value) => Expr::MethodCall(MethodCallExpr { object: value.object, name: value.name, args }),
                    Expr::DynamicMember(value) => Expr::DynamicMethodCall(DynamicMethodCallExpr { object: value.object, name: value.name, args }),
                    Expr::Call(value) => Expr::ChainedCall(ChainedCallExpr { call: Box::new(value), args }),
                    other => other,
                };
            } else { break; }
        }
        expression
    }

    fn primary(&mut self) -> Expr {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Number => { self.advance(); Expr::number(token.text) }
            TokenKind::String => { self.advance(); Expr::string(token.text) }
            TokenKind::True => { self.advance(); Expr::boolean(true) }
            TokenKind::False => { self.advance(); Expr::boolean(false) }
            TokenKind::Null => { self.advance(); Expr::Null }
            TokenKind::MessageCode | TokenKind::RawMessageCode => { self.advance(); Expr::string(token.text) }
            TokenKind::IntCast | TokenKind::FloatCast | TokenKind::Translate if self.pos + 1 < self.tokens.len() && self.tokens[self.pos + 1].kind == TokenKind::LParen => {
                let kind = match token.kind { TokenKind::IntCast => "int", TokenKind::FloatCast => "float", _ => "_" }.to_string(); self.advance(); self.advance(); let expression = self.expression(); self.expect(TokenKind::RParen); Expr::Cast(CastExpr { type_name: kind, expression: Box::new(expression) })
            }
            TokenKind::Identifier => {
                self.advance();
                if let Some(index) = token.text.find("::") { if !token.text[..index].contains('$') { Expr::Enum(EnumExpr { enum_name: token.text[..index].to_string(), member_name: token.text[index + 2..].to_string() }) } else { Expr::identifier(token.text) } } else { Expr::identifier(token.text) }
            }
            TokenKind::LBrace => {
                self.advance(); let mut values = Vec::new();
                if !self.at(&TokenKind::RBrace) {
                    if self.take(&TokenKind::Comma).is_some() { values.push(Expr::number("0")); }
                    else { values.push(self.expression()); }
                    while self.take(&TokenKind::Comma).is_some() {
                        if self.at(&TokenKind::RBrace) { break; }
                        if self.at(&TokenKind::Comma) { values.push(Expr::number("0")); } else { values.push(self.expression()); }
                    }
                }
                self.expect(TokenKind::RBrace); Expr::ArrayLiteral(ArrayLiteralExpr { values })
            }
            TokenKind::Function => {
                self.advance(); self.expect(TokenKind::LParen); let args = if self.at(&TokenKind::RParen) { Vec::new() } else { self.arguments() }; self.expect(TokenKind::RParen);
                let body = self.statement_body(); let name = format!("function_{}_1", self.lambda_id); self.lambda_id += 1; Expr::Lambda(LambdaExpr { name, args, body })
            }
            TokenKind::New => {
                self.advance();
                if self.at(&TokenKind::Identifier) {
                    let type_name = self.advance().text; self.expect(TokenKind::LParen); let args = if self.at(&TokenKind::RParen) { Vec::new() } else { self.arguments() }; self.expect(TokenKind::RParen); Expr::NewObject(NewObjectExpr { type_name, args })
                } else {
                    let mut dimensions = Vec::new(); while self.take(&TokenKind::LBracket).is_some() { dimensions.push(self.expression()); self.expect(TokenKind::RBracket); } Expr::NewArray(NewArrayExpr { dimensions })
                }
            }
            TokenKind::LParen => { self.advance(); let value = self.expression(); self.expect(TokenKind::RParen); value }
            _ => { self.error_current("expected expression"); self.advance(); Expr::Null }
        }
    }
}

enum ParsedItem { Function(FunctionNode), Statement(Stmt) }

fn enum_value(expression: &Expr) -> i32 {
    match expression {
        Expr::Number(value) => value.text.parse::<i32>().unwrap_or(0),
        Expr::Unary(value) if value.op == "-" => match value.expression.as_ref() { Expr::Number(number) => -number.text.parse::<i32>().unwrap_or(0), _ => 0 },
        _ => 0,
    }
}

fn command_args(raw: &str, modes: &str, gs1: bool) -> Vec<Expr> {
    let parts = split_top_level(raw);
    let mut result = Vec::new();
    for (index, part) in parts.into_iter().enumerate() {
        if part.is_empty() { continue; }
        let mode = modes.as_bytes().get(index).copied().unwrap_or(b'S') as char;
        match mode {
            'S' | 'R' | 'L' | 'M' | 'B' | 'I' | 'C' | 'G' | 'U' | 'D' | 'X' | 'Z' => result.push(Expr::string(part)),
            _ => {
                // Parse a command expression in isolation.  If it is not an
                // expression under the current grammar, GS1 treats it as a
                // literal string argument.
                let mut parser = Parser::new(&part, gs2_lexer(&part), gs1);
                let value = parser.expression();
                if parser.good() && parser.at(&TokenKind::Eof) { result.push(value); } else { result.push(Expr::string(part)); }
            }
        }
    }
    result
}

fn split_top_level(value: &str) -> Vec<String> {
    let mut result = Vec::new(); let mut start = 0usize; let mut depth = 0i32; let mut quote = None; let mut escaped = false;
    for (index, ch) in value.char_indices() {
        if let Some(q) = quote {
            if escaped { escaped = false; } else if ch == '\\' { escaped = true; } else if ch == q { quote = None; }
            continue;
        }
        match ch { '"' | '\'' => quote = Some(ch), '(' | '[' | '{' => depth += 1, ')' | ']' | '}' => depth -= 1, ',' if depth == 0 => { result.push(value[start..index].trim().to_string()); start = index + ch.len_utf8(); }, _ => {} }
    }
    result.push(value[start..].trim().to_string()); result
}

fn scan_program_events(items: &[ProgramItem]) -> i32 {
    items.iter().filter_map(|item| match item { ProgramItem::Statement(statement) => Some(scan_gs1_event_flags(statement)), _ => None }).fold(0, |a, b| a | b)
}

fn scan_gs1_event_flags(value: &Stmt) -> i32 {
    fn expression(value: &Expr) -> i32 {
        match value {
            Expr::Identifier(v) => event_bit(&v.name),
            Expr::Binary(v) => if matches!(v.op.as_str(), "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "@=" | "<<=" | ">>=") { expression(&v.right) } else { expression(&v.left) | expression(&v.right) },
            Expr::In(v) => expression(&v.expression) | expression(&v.lower) | v.upper.as_deref().map(expression).unwrap_or(0),
            Expr::Ternary(v) => expression(&v.condition) | expression(&v.when_true) | expression(&v.when_false),
            Expr::Unary(v) => expression(&v.expression),
            Expr::StringCast(v) => expression(&v.expression),
            Expr::Cast(v) => expression(&v.expression),
            Expr::Member(v) => expression(&v.object),
            Expr::DynamicMember(v) => expression(&v.object) | expression(&v.name),
            Expr::DynamicVar(v) => expression(&v.name),
            Expr::ArrayIndex(v) => expression(&v.target) | expression(&v.index),
            Expr::MultiArrayIndex(v) => expression(&v.target) | v.indices.iter().map(expression).fold(0, |a, b| a | b),
            Expr::Call(v) => v.args.iter().map(expression).fold(0, |a, b| a | b),
            Expr::ChainedCall(v) => expression(&Expr::Call((*v.call).clone())) | v.args.iter().map(expression).fold(0, |a, b| a | b),
            Expr::MethodCall(v) => expression(&v.object) | v.args.iter().map(expression).fold(0, |a, b| a | b),
            Expr::DynamicMethodCall(v) => expression(&v.object) | expression(&v.name) | v.args.iter().map(expression).fold(0, |a, b| a | b),
            Expr::NewObject(v) => v.args.iter().map(expression).fold(0, |a, b| a | b),
            Expr::NewArray(v) => v.dimensions.iter().map(expression).fold(0, |a, b| a | b),
            Expr::Lambda(v) => v.body.iter().map(statement).fold(0, |a, b| a | b),
            Expr::ArrayLiteral(v) => v.values.iter().map(expression).fold(0, |a, b| a | b),
            _ => 0,
        }
    }
    fn statements(values: &[Stmt]) -> i32 { values.iter().map(statement).fold(0, |a, b| a | b) }
    fn statement(value: &Stmt) -> i32 {
        match value {
            Stmt::Expr(v) => expression(&v.expression), Stmt::Inline(v) => statement(&v.statement), Stmt::Block(v) => statements(&v.body), Stmt::Return(v) => expression(&v.expression),
            Stmt::If(v) => expression(&v.condition) | statements(&v.then_body) | statements(&v.else_body), Stmt::For(v) => v.init.as_ref().map(expression).unwrap_or(0) | expression(&v.condition) | v.post.as_ref().map(expression).unwrap_or(0) | statements(&v.body),
            Stmt::ForEach(v) => expression(&v.name) | expression(&v.source) | statements(&v.body), Stmt::While(v) => expression(&v.condition) | statements(&v.body), Stmt::DoWhile(v) => expression(&v.condition) | statements(&v.body), Stmt::With(v) => expression(&v.target) | statements(&v.body),
            Stmt::Switch(v) => expression(&v.expression) | v.cases.iter().map(|c| c.labels.iter().filter_map(|x| x.as_ref().map(expression)).fold(0, |a, b| a | b) | statements(&c.body)).fold(0, |a, b| a | b),
            Stmt::New(v) => v.args.iter().map(expression).fold(0, |a, b| a | b) | statements(&v.body), _ => 0,
        }
    }
    statement(value)
}

fn event_bit(name: &str) -> i32 {
    match name.to_ascii_lowercase().as_str() {
        "playerenters" => 1 << 0, "playertouchsme" | "playertouchesme" => 1 << 1, "playertouchsother" | "playertouchesother" => 1 << 2,
        "playerchats" => 1 << 3, "playerhurt" | "playerhurted" => 1 << 4, "playerdies" => 1 << 5, "playerlaysitem" => 1 << 6, "playerendsreading" => 1 << 7,
        "compusdied" => 1 << 8, "emoticon" => 1 << 9, "mousedown" => 1 << 10, "mouseup" => 1 << 11, "mousewheel" => 1 << 12, "exploded" => 1 << 13,
        "wasshot" | "wasshooted" => 1 << 14, "waspelt" => 1 << 15, "keypressed" => 1 << 16, "actionprojectile2" => 1 << 17, "playerleaves" => 1 << 21,
        "washit" => 1 << 22, "shapetrigger" => 1 << 25, "playanimation" => 1 << 26, "pkzonechanges" => 1 << 27, _ => 0,
    }
}

pub fn parse_gs2(input: &str) -> Result<ProgramNode, String> {
    let lexed = gs2_lexer(input);
    let mut parser = Parser::new(input, lexed, false);
    parser.script().map_err(|e| format!("line {}: {}", e.line, e.message))
}

pub fn parse_gs1(input: &str) -> Result<ProgramNode, String> {
    let lexed = Lexed { tokens: lex_gs1(input), error_line: None };
    let mut parser = Parser::new(input, lexed, true);
    let mut program = parser.script().map_err(|e| format!("line {}: {}", e.line, e.message))?;
    // GS1's builder sees each top-level block as a separate wrapper statement;
    // functions are the sole exception.
    for item in &mut program.items {
        if let ProgramItem::Statement(statement) = item {
            let old = std::mem::replace(statement, Stmt::Block(BlockStmt { body: Vec::new() }));
            *statement = match old { Stmt::Block(_) => old, other => Stmt::Block(BlockStmt { body: vec![other] }) };
        }
    }
    program.gs1_event_flags = program.items.iter().filter_map(|item| match item { ProgramItem::Statement(v) => Some(scan_gs1_event_flags(v)), _ => None }).fold(0, |a, b| a | b);
    Ok(program)
}
