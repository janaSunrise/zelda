use rustc_hash::FxHashMap;

use oxc_ast::ast::IdentifierReference;
use oxc_semantic::Scoping;
use oxc_span::Span;
use serde::Serialize;

use crate::types::{TypeArena, TypeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub enum ScopeKind {
    Global,
    Function,
    Block,
    Class,
    #[expect(
        dead_code,
        reason = "symbols.rs is replaced by oxc_semantic (cleanup plan, phase 4)"
    )]
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
    #[expect(
        dead_code,
        reason = "symbols.rs is replaced by oxc_semantic (cleanup plan, phase 4)"
    )]
    pub id: ScopeId,
    #[expect(
        dead_code,
        reason = "symbols.rs is replaced by oxc_semantic (cleanup plan, phase 4)"
    )]
    pub kind: ScopeKind,
    pub parent: Option<ScopeId>, // Parent scope. None for Global, because no parent.
    /// Type namespace: type aliases, interfaces. Uses FxHashMap for faster lookups.
    pub type_symbols: FxHashMap<String, SymbolId>,
}

impl Scope {
    pub fn new(id: ScopeId, kind: ScopeKind, parent: Option<ScopeId>) -> Self {
        Self {
            id,
            kind,
            parent,
            type_symbols: FxHashMap::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Class,
    Interface,
    TypeAlias,
    Enum,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    // Read only by tests.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "symbols.rs is replaced by oxc_semantic (cleanup plan, phase 4)"
        )
    )]
    pub name: String,
    pub ty: TypeId,
    // Read only by tests.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "symbols.rs is replaced by oxc_semantic (cleanup plan, phase 4)"
        )
    )]
    pub kind: SymbolKind,
    /// Location in source code.
    pub span: Span,
    /// The scope it's declared in.
    #[expect(
        dead_code,
        reason = "symbols.rs is replaced by oxc_semantic (cleanup plan, phase 4)"
    )]
    pub scope: ScopeId,
}

impl Symbol {
    pub fn new(
        name: impl Into<String>,
        ty: TypeId,
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
    pub arena: TypeArena,
    /// Value types of this file's declarations, keyed by oxc's symbol.
    values: FxHashMap<oxc_semantic::SymbolId, TypeId>,
    /// Value types of root-scope declarations by name: lib.d.ts globals and
    /// this file's top-level declarations. Used for references oxc leaves
    /// unresolved (globals) and for lookups that only have a name.
    global_values: FxHashMap<String, TypeId>,
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
            arena: TypeArena::new(),
            values: FxHashMap::default(),
            global_values: FxHashMap::default(),
        }
    }

    /// Record the value type of a declared symbol.
    pub fn define_value(&mut self, scoping: &Scoping, symbol: oxc_semantic::SymbolId, ty: TypeId) {
        self.values.insert(symbol, ty);
        if scoping.symbol_scope_id(symbol) == scoping.root_scope_id() {
            self.global_values
                .insert(scoping.symbol_name(symbol).to_string(), ty);
        }
    }

    /// The value type an identifier refers to, following oxc's resolution of
    /// the reference. Unresolved references are globals, looked up by name.
    pub fn value_of(&self, scoping: &Scoping, ident: &IdentifierReference) -> Option<TypeId> {
        let symbol = ident
            .reference_id
            .get()
            .and_then(|r| scoping.get_reference(r).symbol_id());
        match symbol {
            Some(symbol) => self.values.get(&symbol).copied(),
            None => self.global_value(ident.name.as_str()),
        }
    }

    /// The value type of one of this file's declarations.
    pub fn value_of_symbol(&self, symbol: oxc_semantic::SymbolId) -> Option<TypeId> {
        self.values.get(&symbol).copied()
    }

    /// Make a value visible by name to this file, like a lib.d.ts global.
    pub fn add_global_value(&mut self, name: impl Into<String>, ty: TypeId) {
        self.global_values.insert(name.into(), ty);
    }

    /// The value type of a root-scope declaration or lib.d.ts global.
    pub fn global_value(&self, name: &str) -> Option<TypeId> {
        self.global_values.get(name).copied()
    }

    /// Drop symbol-keyed values, keeping only the by-name globals. Symbol ids
    /// are per file, so a table shared with other files (lib.d.ts) must not
    /// carry them.
    pub fn clear_local_values(&mut self) {
        self.values.clear();
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

    /// Define a type symbol in the current scope.
    /// Returns error if name already exists in the type namespace of the same scope.
    pub fn define_type(
        &mut self,
        name: impl Into<String>,
        ty: TypeId,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: u32, end: u32) -> Span {
        Span::new(start, end)
    }

    #[test]
    fn test_define_type_and_lookup() {
        let mut table = SymbolTable::new();
        let ty = table.arena.object(vec![]);
        table
            .define_type("User", ty, SymbolKind::Interface, span(0, 10))
            .unwrap();

        let sym = table.lookup_type("User").unwrap();
        assert_eq!(sym.name, "User");
        assert_eq!(sym.ty, ty);
        assert_eq!(sym.kind, SymbolKind::Interface);
    }

    #[test]
    fn test_duplicate_type_detection() {
        let mut table = SymbolTable::new();
        table
            .define_type("T", TypeId::NUMBER, SymbolKind::TypeAlias, span(0, 10))
            .unwrap();

        let err = table
            .define_type("T", TypeId::STRING, SymbolKind::TypeAlias, span(20, 30))
            .unwrap_err();
        assert_eq!(err.name, "T");
        assert_eq!(err.existing, span(0, 10));
        assert_eq!(err.duplicate, span(20, 30));
    }

    #[test]
    fn test_type_scope_chain_and_shadowing() {
        let mut table = SymbolTable::new();
        table
            .define_type("T", TypeId::NUMBER, SymbolKind::TypeAlias, span(0, 10))
            .unwrap();

        table.push_scope(ScopeKind::Function);
        assert_eq!(table.lookup_type("T").unwrap().ty, TypeId::NUMBER);
        table
            .define_type("T", TypeId::STRING, SymbolKind::TypeAlias, span(20, 30))
            .unwrap();
        table
            .define_type("U", TypeId::BOOLEAN, SymbolKind::TypeAlias, span(40, 50))
            .unwrap();
        assert_eq!(table.lookup_type("T").unwrap().ty, TypeId::STRING);

        table.pop_scope();
        assert_eq!(table.lookup_type("T").unwrap().ty, TypeId::NUMBER);
        assert!(table.lookup_type("U").is_none());
    }

    #[test]
    fn test_global_values() {
        let mut table = SymbolTable::new();
        assert!(table.global_value("x").is_none());
        table.add_global_value("x", TypeId::NUMBER);
        assert_eq!(table.global_value("x"), Some(TypeId::NUMBER));
    }

    #[test]
    fn test_undefined_reference() {
        let table = SymbolTable::new();
        assert!(table.global_value("undefined_var").is_none());
        assert!(table.lookup_type("undefined_var").is_none());
    }
}
