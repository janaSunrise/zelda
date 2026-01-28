use std::collections::HashMap;

use oxc_span::Span;
use serde::Serialize;

use crate::types::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub enum ScopeKind {
    Global,
    Function,
    Block,
    Class,
    Interface,
}

/// TypeScript has two namespaces:
/// - Value namespace: variables, functions, classes (as values)
/// - Type namespace: interfaces, type aliases, classes (as types)
///
/// Entities in the value space have a runtime presence in the resulting JS code. They can be referenced and manipulated at
/// runtime.
/// Entities in the type space are used purely for type-checking at compile time and are completely erased from the resulting JS
/// code. They have no runtime impact.
///
/// This allows `interface User {}` and `const User = {}` to coexist.
#[derive(Debug, Clone)]
pub struct Scope {
    pub id: ScopeId,
    pub kind: ScopeKind,
    pub parent: Option<ScopeId>, // Parent scope. None for Global, because no parent.
    pub symbols: HashMap<String, SymbolId>, // Value namespace: variables, functions
    pub type_symbols: HashMap<String, SymbolId>, // Type namespace: type aliases, interfaces
}

impl Scope {
    pub fn new(id: ScopeId, kind: ScopeKind, parent: Option<ScopeId>) -> Self {
        Self {
            id,
            kind,
            parent,
            symbols: HashMap::new(),
            type_symbols: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Variable,
    Function,
    Parameter,
    Class,
    Interface,
    TypeAlias,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub ty: Type,
    pub kind: SymbolKind,
    pub span: Span, // Location in source code
    pub scope: ScopeId, // Which scope it's declared in
}

impl Symbol {
    pub fn new(
        name: impl Into<String>,
        ty: Type,
        kind: SymbolKind,
        span: Span,
        scope: ScopeId,
    ) -> Self {
        Self {
            name: name.into(),
            ty,
            kind,
            span,
            scope,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateSymbolError {
    pub name: String,
    #[serde(serialize_with = "serialize_span")]
    pub existing: Span,
    #[serde(serialize_with = "serialize_span")]
    pub duplicate: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct UndefinedSymbolError {
    pub name: String,
    #[serde(serialize_with = "serialize_span")]
    pub span: Span,
    pub is_type: bool,
}

/// Custom serializer for oxc_span::Span since it doesn't implement Serialize.
fn serialize_span<S>(span: &Span, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeStruct;
    let mut state = serializer.serialize_struct("Span", 2)?;
    state.serialize_field("start", &span.start)?;
    state.serialize_field("end", &span.end)?;
    state.end()
}

/// The symbol table tracks all declared symbols and their scopes.
///
/// Scopes form a tree via parent pointers. Name lookup walks up the tree
/// from the current scope until a match is found (lexical scoping).
#[derive(Debug, Clone)]
pub struct SymbolTable {
    pub symbols: Vec<Symbol>,
    pub scopes: Vec<Scope>,
    pub current_scope: ScopeId,
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTable {
    pub fn new() -> Self {
        // Global: Scope ID 0
        let global_scope = Scope::new(ScopeId(0), ScopeKind::Global, None);
        Self {
            symbols: Vec::new(),
            scopes: vec![global_scope],
            current_scope: ScopeId(0),
        }
    }

    pub fn push_scope(&mut self, kind: ScopeKind) -> ScopeId {
        let id = ScopeId(self.scopes.len());
        let scope = Scope::new(id, kind, Some(self.current_scope));
        self.scopes.push(scope);
        self.current_scope = id;
        id
    }

    pub fn pop_scope(&mut self) {
        match self.scopes[self.current_scope.0].parent {
            Some(parent) => self.current_scope = parent,
            None => panic!("pop_scope called at global scope - binder bug"),
        }
    }

    /// Define a value symbol in the current scope.
    /// Returns error if name already exists in the same scope.
    pub fn define(
        &mut self,
        name: impl Into<String>,
        ty: Type,
        kind: SymbolKind,
        span: Span,
    ) -> Result<SymbolId, DuplicateSymbolError> {
        let name = name.into();
        let scope = &self.scopes[self.current_scope.0];

        if let Some(&existing_id) = scope.symbols.get(&name) {
            let existing = &self.symbols[existing_id.0];
            return Err(DuplicateSymbolError {
                name,
                existing: existing.span,
                duplicate: span,
            });
        }

        let id = SymbolId(self.symbols.len());
        let symbol = Symbol::new(name.clone(), ty, kind, span, self.current_scope);
        self.symbols.push(symbol);
        self.scopes[self.current_scope.0].symbols.insert(name, id);

        Ok(id)
    }

    /// Define a type symbol in the current scope.
    /// Returns error if name already exists in the type namespace of the same scope.
    pub fn define_type(
        &mut self,
        name: impl Into<String>,
        ty: Type,
        kind: SymbolKind,
        span: Span,
    ) -> Result<SymbolId, DuplicateSymbolError> {
        let name = name.into();
        let scope = &self.scopes[self.current_scope.0];

        if let Some(&existing_id) = scope.type_symbols.get(&name) {
            let existing = &self.symbols[existing_id.0];
            return Err(DuplicateSymbolError {
                name,
                existing: existing.span,
                duplicate: span,
            });
        }

        let id = SymbolId(self.symbols.len());
        let symbol = Symbol::new(name.clone(), ty, kind, span, self.current_scope);
        self.symbols.push(symbol);
        self.scopes[self.current_scope.0]
            .type_symbols
            .insert(name, id);

        Ok(id)
    }

    /// Look up a value symbol, walking up the scope chain.
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        let mut scope_id = Some(self.current_scope);

        while let Some(id) = scope_id {
            let scope = &self.scopes[id.0];
            if let Some(&symbol_id) = scope.symbols.get(name) {
                return Some(&self.symbols[symbol_id.0]);
            }
            scope_id = scope.parent;
        }

        None
    }

    /// Look up a type symbol, walking up the scope chain.
    pub fn lookup_type(&self, name: &str) -> Option<&Symbol> {
        let mut scope_id = Some(self.current_scope);

        while let Some(id) = scope_id {
            let scope = &self.scopes[id.0];
            if let Some(&symbol_id) = scope.type_symbols.get(name) {
                return Some(&self.symbols[symbol_id.0]);
            }
            scope_id = scope.parent;
        }

        None
    }

    pub fn resolve(&self, name: &str, span: Span) -> Result<&Symbol, UndefinedSymbolError> {
        self.lookup(name).ok_or_else(|| UndefinedSymbolError {
            name: name.to_string(),
            span,
            is_type: false,
        })
    }

    pub fn resolve_type(&self, name: &str, span: Span) -> Result<&Symbol, UndefinedSymbolError> {
        self.lookup_type(name).ok_or_else(|| UndefinedSymbolError {
            name: name.to_string(),
            span,
            is_type: true,
        })
    }

    pub fn get(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.0]
    }

    pub fn current(&self) -> &Scope {
        &self.scopes[self.current_scope.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: u32, end: u32) -> Span {
        Span::new(start, end)
    }

    #[test]
    fn test_define_and_lookup() {
        let mut table = SymbolTable::new();
        table
            .define("x", Type::Number, SymbolKind::Variable, span(0, 10))
            .unwrap();

        let sym = table.lookup("x").unwrap();
        assert_eq!(sym.name, "x");
        assert_eq!(sym.ty, Type::Number);
    }

    #[test]
    fn test_duplicate_detection() {
        let mut table = SymbolTable::new();
        table
            .define("x", Type::Number, SymbolKind::Variable, span(0, 10))
            .unwrap();

        let result = table.define("x", Type::String, SymbolKind::Variable, span(20, 30));
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.name, "x");
        assert_eq!(err.existing, span(0, 10));
        assert_eq!(err.duplicate, span(20, 30));
    }

    #[test]
    fn test_scope_chain_lookup() {
        let mut table = SymbolTable::new();
        table
            .define("x", Type::Number, SymbolKind::Variable, span(0, 10))
            .unwrap();

        table.push_scope(ScopeKind::Function);
        assert!(table.lookup("x").is_some());

        table
            .define("y", Type::String, SymbolKind::Variable, span(20, 30))
            .unwrap();
        assert!(table.lookup("y").is_some());

        table.pop_scope();
        assert!(table.lookup("x").is_some());
        assert!(table.lookup("y").is_none());
    }

    #[test]
    fn test_shadowing() {
        let mut table = SymbolTable::new();
        table
            .define("x", Type::Number, SymbolKind::Variable, span(0, 10))
            .unwrap();

        table.push_scope(ScopeKind::Function);
        table
            .define("x", Type::String, SymbolKind::Variable, span(20, 30))
            .unwrap();

        assert_eq!(table.lookup("x").unwrap().ty, Type::String);

        table.pop_scope();
        assert_eq!(table.lookup("x").unwrap().ty, Type::Number);
    }

    #[test]
    fn test_type_namespace() {
        let mut table = SymbolTable::new();

        table
            .define_type("User", Type::object(vec![]), SymbolKind::Interface, span(0, 20))
            .unwrap();

        table
            .define("User", Type::object(vec![]), SymbolKind::Variable, span(30, 50))
            .unwrap();

        // Both namespaces have User
        assert!(table.lookup_type("User").is_some());
        assert!(table.lookup("User").is_some());

        assert_eq!(table.lookup_type("User").unwrap().kind, SymbolKind::Interface);
        assert_eq!(table.lookup("User").unwrap().kind, SymbolKind::Variable);
    }

    #[test]
    fn test_undefined_reference() {
        let table = SymbolTable::new();
        let result = table.resolve("undefined_var", span(0, 10));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().name, "undefined_var");
    }

    #[test]
    fn test_nested_scopes() {
        let mut table = SymbolTable::new();

        table
            .define("a", Type::Number, SymbolKind::Variable, span(0, 5))
            .unwrap();

        table.push_scope(ScopeKind::Function);
        table
            .define("b", Type::Number, SymbolKind::Variable, span(10, 15))
            .unwrap();

        table.push_scope(ScopeKind::Block);
        table
            .define("c", Type::Number, SymbolKind::Variable, span(20, 25))
            .unwrap();

        // All visible from innermost
        assert!(table.lookup("a").is_some());
        assert!(table.lookup("b").is_some());
        assert!(table.lookup("c").is_some());

        table.pop_scope();
        assert!(table.lookup("a").is_some());
        assert!(table.lookup("b").is_some());
        assert!(table.lookup("c").is_none());

        table.pop_scope();
        assert!(table.lookup("a").is_some());
        assert!(table.lookup("b").is_none());
        assert!(table.lookup("c").is_none());
    }
}
