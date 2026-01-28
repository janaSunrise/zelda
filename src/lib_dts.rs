//! Built-in TypeScript type declarations (lib.d.ts).
//!
//! This module embeds lib.d.ts in the binary and provides loading functionality
//! to make built-in types like Array, String, Console available.

use oxc_allocator::Allocator;
use oxc_parser::{Parser, ParserReturn};
use oxc_span::SourceType;

use crate::binder::Binder;
use crate::symbols::SymbolTable;

/// Embedded lib.d.ts content, compiled into the binary.
pub const LIB_DTS: &str = include_str!("lib/lib.d.ts");

/// Load built-in types from lib.d.ts into the given symbol table.
/// This should be called once when creating a new Project.
pub fn load_lib_dts(symbols: &mut SymbolTable) {
    let allocator = Allocator::default();
    let source_type = SourceType::d_ts();

    let ParserReturn { program, errors, panicked, .. } =
        Parser::new(&allocator, LIB_DTS, source_type).parse();

    if panicked || !errors.is_empty() {
        // lib.d.ts should always parse successfully
        #[cfg(debug_assertions)]
        {
            for err in &errors {
                eprintln!("lib.d.ts parse error: {}", err);
            }
            panic!("Failed to parse built-in lib.d.ts");
        }
        #[cfg(not(debug_assertions))]
        return;
    }

    // Create a temporary binder to process lib.d.ts
    // We'll transfer the symbols to the provided symbol table
    let mut binder = Binder::with_symbols(std::mem::take(symbols));
    binder.bind_program(&program);

    // Transfer back the enriched symbol table
    *symbols = binder.into_symbols();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lib_dts_parses() {
        let allocator = Allocator::default();
        let source_type = SourceType::d_ts();

        let ParserReturn { errors, panicked, .. } =
            Parser::new(&allocator, LIB_DTS, source_type).parse();

        assert!(!panicked, "lib.d.ts should not cause parser panic");
        assert!(errors.is_empty(), "lib.d.ts should parse without errors: {:?}", errors);
    }

    #[test]
    fn test_load_lib_dts_adds_symbols() {
        let mut symbols = SymbolTable::new();
        load_lib_dts(&mut symbols);

        // Check that some key types are defined
        assert!(symbols.lookup_type("String").is_some(), "String should be defined");
        assert!(symbols.lookup_type("Array").is_some(), "Array should be defined");
        assert!(symbols.lookup_type("Map").is_some(), "Map should be defined");
        assert!(symbols.lookup_type("Promise").is_some(), "Promise should be defined");
        assert!(symbols.lookup_type("Console").is_some(), "Console should be defined");

        // Check primitive wrapper interfaces
        assert!(symbols.lookup_type("Number").is_some(), "Number should be defined");
        assert!(symbols.lookup_type("Boolean").is_some(), "Boolean should be defined");

        // Check that some global values are defined
        assert!(symbols.lookup("console").is_some(), "console should be defined");
        assert!(symbols.lookup("Math").is_some(), "Math should be defined");
        assert!(symbols.lookup("JSON").is_some(), "JSON should be defined");
    }
}
