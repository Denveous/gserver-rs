use hexaparser::{lex_gs1, parse_gs1};
use std::fs;

fn main() {
    let path = std::env::args().nth(1).expect("source path");
    let source = fs::read_to_string(path).expect("read source");
    println!("TOKENS: {:?}", lex_gs1(&source));
    match parse_gs1(&source) {
        Ok(program) => println!("{program:#?}"),
        Err(error) => println!("ERROR: {error:?}"),
    }
}
