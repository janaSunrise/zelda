use rustc_hash::FxHashMap;

use oxc_ast::ast::IdentifierReference;
use oxc_semantic::Scoping;
use oxc_span::Span;
use serde::Serialize;

use crate::types::{TypeArena, TypeId};

#[derive(Debug, Clone, Copy)]
struct ScopeId(usize);

/// A lexical scope of the type namespace. Values are resolved by oxc_semantic;
/// types are still resolved by name through this scope chain.
#[derive(Debug, Clone, Default)]
struct Scope {
    parent: Option<ScopeId>,
    types: FxHashMap<String, TypeId>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UndefinedSymbolError {
    pub name: String,
    #[serde(serialize_with = "crate::errors::serialize_span")]
    pub span: Span,
    pub is_type: bool,
}

/// The symbol table tracks all declared symbols and their scopes.
///
/// Scopes form a tree via parent pointers. Name lookup walks up the tree
/// from the current scope until a match is found (lexical scoping).
#[derive(Debug, Clone)]
pub struct SymbolTable {
    scopes: Vec<Scope>,
    current_scope: ScopeId,
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
        Self {
            scopes: vec![Scope::default()],
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

    pub fn push_scope(&mut self) {
        let id = ScopeId(self.scopes.len());
        self.scopes.push(Scope {
            parent: Some(self.current_scope),
            types: FxHashMap::default(),
        });
        self.current_scope = id;
    }

    pub fn pop_scope(&mut self) {
        match self.scopes[self.current_scope.0].parent {
            Some(parent) => self.current_scope = parent,
            None => panic!("pop_scope called at global scope - binder bug"),
        }
    }

    /// Define a type in the current scope. The first definition of a name
    /// wins; redeclarations are reported from oxc's semantic analysis.
    pub fn define_type(&mut self, name: impl Into<String>, ty: TypeId) {
        self.scopes[self.current_scope.0]
            .types
            .entry(name.into())
            .or_insert(ty);
    }

    /// Look up a type, walking up the scope chain.
    pub fn lookup_type(&self, name: &str) -> Option<TypeId> {
        let mut scope_id = Some(self.current_scope);
        while let Some(id) = scope_id {
            let scope = &self.scopes[id.0];
            if let Some(&ty) = scope.types.get(name) {
                return Some(ty);
            }
            scope_id = scope.parent;
        }
        None
    }

    /// Every type defined in any scope, by name.
    pub fn all_types(&self) -> impl Iterator<Item = (&str, TypeId)> {
        self.scopes
            .iter()
            .flat_map(|scope| scope.types.iter().map(|(name, &ty)| (name.as_str(), ty)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_type_definition_wins() {
        let mut table = SymbolTable::new();
        table.define_type("T", TypeId::NUMBER);
        table.define_type("T", TypeId::STRING);
        assert_eq!(table.lookup_type("T"), Some(TypeId::NUMBER));
    }

    #[test]
    fn inner_scope_shadows_types() {
        let mut table = SymbolTable::new();
        table.define_type("T", TypeId::NUMBER);

        table.push_scope();
        assert_eq!(table.lookup_type("T"), Some(TypeId::NUMBER));
        table.define_type("T", TypeId::STRING);
        table.define_type("U", TypeId::BOOLEAN);
        assert_eq!(table.lookup_type("T"), Some(TypeId::STRING));

        table.pop_scope();
        assert_eq!(table.lookup_type("T"), Some(TypeId::NUMBER));
        assert_eq!(table.lookup_type("U"), None);
    }

    #[test]
    fn global_values_by_name() {
        let mut table = SymbolTable::new();
        assert_eq!(table.global_value("x"), None);
        table.add_global_value("x", TypeId::NUMBER);
        assert_eq!(table.global_value("x"), Some(TypeId::NUMBER));
    }
}
