//! Run with: cargo run --example show_ast

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn main() {
    let source = r#"
const x: number = 1, y = 2;

function add(a: number, b: number): number {
    return a + b;
}

const obj = { name: "alice", age: 30 };
"#;

    let allocator = Allocator::default();
    let source_type = SourceType::ts();
    let result = Parser::new(&allocator, source, source_type).parse();

    println!("Source:\n{}\n", source);
    println!("AST:\n{:#?}", result.program.body);
}
