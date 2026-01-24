//! The binder walks the AST and populates the symbol table.

use oxc_allocator::Box as OxcBox;
use oxc_ast::ast::*;

use crate::symbols::{
    DuplicateSymbolError, ScopeKind, SymbolKind, SymbolTable, UndefinedSymbolError,
};
use crate::types::{Param, Property, Type, TypeParam};

#[derive(Debug, Clone)]
pub enum BindingError {
    DuplicateSymbol(DuplicateSymbolError),
    UndefinedSymbol(UndefinedSymbolError),
}

impl From<DuplicateSymbolError> for BindingError {
    fn from(err: DuplicateSymbolError) -> Self {
        BindingError::DuplicateSymbol(err)
    }
}

impl From<UndefinedSymbolError> for BindingError {
    fn from(err: UndefinedSymbolError) -> Self {
        BindingError::UndefinedSymbol(err)
    }
}

pub struct Binder {
    pub symbols: SymbolTable,
    pub errors: Vec<BindingError>,
}

impl Default for Binder {
    fn default() -> Self {
        Self::new()
    }
}

impl Binder {
    pub fn new() -> Self {
        Self {
            symbols: SymbolTable::new(),
            errors: Vec::new(),
        }
    }

    pub fn bind_program(&mut self, program: &Program) {
        for stmt in &program.body {
            self.bind_statement(stmt);
        }
    }

    fn bind_statement(&mut self, stmt: &Statement) {
        match stmt {
            Statement::VariableDeclaration(decl) => self.bind_variable_declaration(decl),
            Statement::FunctionDeclaration(decl) => self.bind_function_declaration(decl),
            Statement::ClassDeclaration(decl) => self.bind_class_declaration(decl),
            Statement::TSInterfaceDeclaration(decl) => self.bind_interface_declaration(decl),
            Statement::TSTypeAliasDeclaration(decl) => self.bind_type_alias_declaration(decl),
            Statement::BlockStatement(block) => self.bind_block_statement(block),
            Statement::IfStatement(if_stmt) => self.bind_if_statement(if_stmt),
            Statement::WhileStatement(while_stmt) => {
                self.bind_statement(&while_stmt.body);
            }
            Statement::ForStatement(for_stmt) => self.bind_for_statement(for_stmt),
            Statement::ReturnStatement(ret) => {
                if let Some(arg) = &ret.argument {
                    self.bind_expression(arg);
                }
            }
            Statement::ExpressionStatement(expr_stmt) => {
                self.bind_expression(&expr_stmt.expression);
            }
            _ => {}
        }
    }

    fn bind_variable_declaration(&mut self, decl: &VariableDeclaration) {
        for declarator in &decl.declarations {
            self.bind_variable_declarator(declarator);
        }
    }

    /// 1. Extract the name
    /// 2. Get type number from annotation
    /// 3. Add symbol to current scope
    /// 4. Check value for undefined references
    fn bind_variable_declarator(&mut self, declarator: &VariableDeclarator) {
        if let BindingPatternKind::BindingIdentifier(ident) = &declarator.id.kind {
            let name = ident.name.as_str();
            let span = ident.span;

            let ty = self.resolve_binding_type(&declarator.id, &declarator.init);

            if let Err(err) = self.symbols.define(name, ty, SymbolKind::Variable, span) {
                self.errors.push(err.into());
            }
        }

        if let Some(init) = &declarator.init {
            self.bind_expression(init);
        }
    }

    /// 1. Add function to global scope
    /// 2. Push new function scope
    /// 3. Add parameters to function scope
    /// 4. Bind body and check inner variables defined
    /// 5. Pop back to global scope
    fn bind_function_declaration(&mut self, decl: &Function) {
        if let Some(ident) = &decl.id {
            let name = ident.name.as_str();
            let span = ident.span;
            let ty = self.build_function_type(decl);

            if let Err(err) = self.symbols.define(name, ty, SymbolKind::Function, span) {
                self.errors.push(err.into());
            }
        }

        self.symbols.push_scope(ScopeKind::Function);

        for param in &decl.params.items {
            self.bind_formal_parameter(param);
        }

        if let Some(body) = &decl.body {
            for stmt in &body.statements {
                self.bind_statement(stmt);
            }
        }

        self.symbols.pop_scope();
    }

    fn bind_formal_parameter(&mut self, param: &FormalParameter) {
        if let BindingPatternKind::BindingIdentifier(ident) = &param.pattern.kind {
            let name = ident.name.as_str();
            let span = ident.span;
            let ty = self.resolve_type_annotation_oxc(&param.pattern.type_annotation);

            if let Err(err) = self.symbols.define(name, ty, SymbolKind::Parameter, span) {
                self.errors.push(err.into());
            }
        }
    }

    /// Classes exist in both value and type namespaces.
    /// `class Foo {}` creates both a constructor function `Foo` (value)
    /// and a type `Foo` (the instance type).
    fn bind_class_declaration(&mut self, decl: &Class) {
        if let Some(ident) = &decl.id {
            let name = ident.name.as_str();
            let span = ident.span;
            let ty = Type::type_ref(name, vec![]);

            if let Err(err) = self.symbols.define(name, ty.clone(), SymbolKind::Class, span) {
                self.errors.push(err.into());
            }
            if let Err(err) = self.symbols.define_type(name, ty, SymbolKind::Class, span) {
                self.errors.push(err.into());
            }
        }

        self.symbols.push_scope(ScopeKind::Class);

        for element in &decl.body.body {
            self.bind_class_element(element);
        }

        self.symbols.pop_scope();
    }

    fn bind_class_element(&mut self, element: &ClassElement) {
        match element {
            ClassElement::MethodDefinition(method) => {
                let func = &method.value;
                self.symbols.push_scope(ScopeKind::Function);

                for param in &func.params.items {
                    self.bind_formal_parameter(param);
                }

                if let Some(body) = &func.body {
                    for stmt in &body.statements {
                        self.bind_statement(stmt);
                    }
                }

                self.symbols.pop_scope();
            }
            ClassElement::PropertyDefinition(prop) => {
                if let Some(value) = &prop.value {
                    self.bind_expression(value);
                }
            }
            _ => {}
        }
    }

    /// Interfaces only exist in the type namespace
    fn bind_interface_declaration(&mut self, decl: &TSInterfaceDeclaration) {
        let name = decl.id.name.as_str();
        let span = decl.id.span;
        let ty = self.build_interface_type(decl);

        if let Err(err) = self.symbols.define_type(name, ty, SymbolKind::Interface, span) {
            self.errors.push(err.into());
        }
    }

    /// Type aliases only exist in the type namespace.
    fn bind_type_alias_declaration(&mut self, decl: &TSTypeAliasDeclaration) {
        let name = decl.id.name.as_str();
        let span = decl.id.span;
        let ty = self.resolve_ts_type(&decl.type_annotation);

        if let Err(err) = self.symbols.define_type(name, ty, SymbolKind::TypeAlias, span) {
            self.errors.push(err.into());
        }
    }

    fn bind_block_statement(&mut self, block: &BlockStatement) {
        self.symbols.push_scope(ScopeKind::Block);
        for stmt in &block.body {
            self.bind_statement(stmt);
        }
        self.symbols.pop_scope();
    }

    fn bind_if_statement(&mut self, if_stmt: &IfStatement) {
        self.bind_expression(&if_stmt.test);
        self.bind_statement(&if_stmt.consequent);
        if let Some(alt) = &if_stmt.alternate {
            self.bind_statement(alt);
        }
    }

    /// For loops create a block scope for their initializer.
    /// `for (let i = 0; ...)` - the `i` is scoped to the loop.
    fn bind_for_statement(&mut self, for_stmt: &ForStatement) {
        self.symbols.push_scope(ScopeKind::Block);

        if let Some(init) = &for_stmt.init {
            match init {
                ForStatementInit::VariableDeclaration(decl) => {
                    self.bind_variable_declaration(decl);
                }
                _ => {
                    if let Some(expr) = init.as_expression() {
                        self.bind_expression(expr);
                    }
                }
            }
        }

        if let Some(test) = &for_stmt.test {
            self.bind_expression(test);
        }

        if let Some(update) = &for_stmt.update {
            self.bind_expression(update);
        }

        self.bind_statement(&for_stmt.body);

        self.symbols.pop_scope();
    }

    /// Walk expressions to find undefined variable references.
    /// For compound expressions like x + y, recursively check each part.
    fn bind_expression(&mut self, expr: &Expression) {
        match expr {
            Expression::Identifier(ident) => {
                let name = ident.name.as_str();
                if !is_builtin_global(name) && self.symbols.lookup(name).is_none() {
                    self.errors.push(BindingError::UndefinedSymbol(UndefinedSymbolError {
                        name: name.to_string(),
                        span: ident.span,
                        is_type: false,
                    }));
                }
            }
            Expression::CallExpression(call) => {
                self.bind_expression(&call.callee);
                for arg in &call.arguments {
                    if let Argument::SpreadElement(spread) = arg {
                        self.bind_expression(&spread.argument);
                    } else if let Some(expr) = arg.as_expression() {
                        self.bind_expression(expr);
                    }
                }
            }
            // oxc splits member expressions into separate variants
            Expression::StaticMemberExpression(member) => {
                self.bind_expression(&member.object);
            }
            Expression::ComputedMemberExpression(member) => {
                self.bind_expression(&member.object);
                self.bind_expression(&member.expression);
            }
            Expression::PrivateFieldExpression(member) => {
                self.bind_expression(&member.object);
            }
            Expression::BinaryExpression(binary) => {
                self.bind_expression(&binary.left);
                self.bind_expression(&binary.right);
            }
            Expression::UnaryExpression(unary) => {
                self.bind_expression(&unary.argument);
            }
            Expression::AssignmentExpression(assign) => {
                self.bind_assignment_target(&assign.left);
                self.bind_expression(&assign.right);
            }
            Expression::ArrayExpression(arr) => {
                for elem in &arr.elements {
                    match elem {
                        ArrayExpressionElement::SpreadElement(spread) => {
                            self.bind_expression(&spread.argument);
                        }
                        ArrayExpressionElement::Elision(_) => {}
                        _ => {
                            if let Some(expr) = elem.as_expression() {
                                self.bind_expression(expr);
                            }
                        }
                    }
                }
            }
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPropertyKind::ObjectProperty(p) => {
                            self.bind_expression(&p.value);
                        }
                        ObjectPropertyKind::SpreadProperty(spread) => {
                            self.bind_expression(&spread.argument);
                        }
                    }
                }
            }
            Expression::ArrowFunctionExpression(arrow) => self.bind_arrow_function(arrow),
            Expression::FunctionExpression(func) => self.bind_function_expression(func),
            Expression::ConditionalExpression(cond) => {
                self.bind_expression(&cond.test);
                self.bind_expression(&cond.consequent);
                self.bind_expression(&cond.alternate);
            }
            Expression::LogicalExpression(logical) => {
                self.bind_expression(&logical.left);
                self.bind_expression(&logical.right);
            }
            Expression::SequenceExpression(seq) => {
                for expr in &seq.expressions {
                    self.bind_expression(expr);
                }
            }
            Expression::TemplateLiteral(template) => {
                for expr in &template.expressions {
                    self.bind_expression(expr);
                }
            }
            Expression::TaggedTemplateExpression(tagged) => {
                self.bind_expression(&tagged.tag);
                for expr in &tagged.quasi.expressions {
                    self.bind_expression(expr);
                }
            }
            Expression::NewExpression(new_expr) => {
                self.bind_expression(&new_expr.callee);
                for arg in &new_expr.arguments {
                    if let Argument::SpreadElement(spread) = arg {
                        self.bind_expression(&spread.argument);
                    } else if let Some(expr) = arg.as_expression() {
                        self.bind_expression(expr);
                    }
                }
            }
            Expression::AwaitExpression(await_expr) => {
                self.bind_expression(&await_expr.argument);
            }
            Expression::YieldExpression(yield_expr) => {
                if let Some(arg) = &yield_expr.argument {
                    self.bind_expression(arg);
                }
            }
            // Literals and `this` have no references to check
            Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::RegExpLiteral(_)
            | Expression::ThisExpression(_) => {}
            _ => {}
        }
    }

    fn bind_assignment_target(&mut self, target: &AssignmentTarget) {
        match target {
            AssignmentTarget::AssignmentTargetIdentifier(ident) => {
                let name = ident.name.as_str();
                if !is_builtin_global(name) && self.symbols.lookup(name).is_none() {
                    self.errors.push(BindingError::UndefinedSymbol(UndefinedSymbolError {
                        name: name.to_string(),
                        span: ident.span,
                        is_type: false,
                    }));
                }
            }
            AssignmentTarget::StaticMemberExpression(member) => {
                self.bind_expression(&member.object);
            }
            AssignmentTarget::ComputedMemberExpression(member) => {
                self.bind_expression(&member.object);
                self.bind_expression(&member.expression);
            }
            _ => {}
        }
    }

    /// Arrow functions: `() => expr` or `() => { stmts }`.
    /// In oxc, the body is always a FunctionBody struct. When `arrow.expression` is true,
    /// it contains a single ExpressionStatement wrapping the expression.
    fn bind_arrow_function(&mut self, arrow: &ArrowFunctionExpression) {
        self.symbols.push_scope(ScopeKind::Function);

        for param in &arrow.params.items {
            self.bind_formal_parameter(param);
        }

        for stmt in &arrow.body.statements {
            self.bind_statement(stmt);
        }

        self.symbols.pop_scope();
    }

    /// Named function expressions bind their name inside their own scope.
    /// `const f = function foo() { foo(); }` - `foo` is only visible inside.
    fn bind_function_expression(&mut self, func: &Function) {
        self.symbols.push_scope(ScopeKind::Function);

        if let Some(ident) = &func.id {
            let name = ident.name.as_str();
            let span = ident.span;
            let ty = self.build_function_type(func);
            let _ = self.symbols.define(name, ty, SymbolKind::Function, span);
        }

        for param in &func.params.items {
            self.bind_formal_parameter(param);
        }

        if let Some(body) = &func.body {
            for stmt in &body.statements {
                self.bind_statement(stmt);
            }
        }

        self.symbols.pop_scope();
    }

    /// Get type from annotation if present, otherwise infer from initializer.
    /// Falls back to `any` if neither exists.
    fn resolve_binding_type(&self, pattern: &BindingPattern, init: &Option<Expression>) -> Type {
        if let Some(annotation) = &pattern.type_annotation {
            return self.resolve_ts_type(&annotation.type_annotation);
        }

        if let Some(init) = init {
            return self.infer_expression_type(init);
        }

        Type::Any
    }

    /// Helper for oxc's arena-allocated Box<TSTypeAnnotation>.
    fn resolve_type_annotation_oxc(&self, annotation: &Option<OxcBox<TSTypeAnnotation>>) -> Type {
        match annotation {
            Some(ann) => self.resolve_ts_type(&ann.type_annotation),
            None => Type::Any,
        }
    }

    /// Convert oxc's TSType AST node to our Type representation.
    fn resolve_ts_type(&self, ts_type: &TSType) -> Type {
        match ts_type {
            TSType::TSStringKeyword(_) => Type::String,
            TSType::TSNumberKeyword(_) => Type::Number,
            TSType::TSBooleanKeyword(_) => Type::Boolean,
            TSType::TSNullKeyword(_) => Type::Null,
            TSType::TSUndefinedKeyword(_) => Type::Undefined,
            TSType::TSVoidKeyword(_) => Type::Void,
            TSType::TSAnyKeyword(_) => Type::Any,
            TSType::TSUnknownKeyword(_) => Type::Unknown,
            TSType::TSNeverKeyword(_) => Type::Never,

            TSType::TSLiteralType(lit) => match &lit.literal {
                TSLiteral::StringLiteral(s) => Type::StringLiteral(s.value.to_string()),
                TSLiteral::NumericLiteral(n) => Type::NumberLiteral(n.value),
                TSLiteral::BooleanLiteral(b) => Type::BooleanLiteral(b.value),
                _ => Type::Any,
            },

            TSType::TSArrayType(arr) => {
                Type::Array(Box::new(self.resolve_ts_type(&arr.element_type)))
            }

            TSType::TSTupleType(tuple) => {
                let types: Vec<Type> = tuple
                    .element_types
                    .iter()
                    .map(|elem| self.resolve_tuple_element(elem))
                    .collect();
                Type::Tuple(types)
            }

            TSType::TSUnionType(union) => {
                let types: Vec<Type> = union
                    .types
                    .iter()
                    .map(|t| self.resolve_ts_type(t))
                    .collect();
                Type::Union(types)
            }

            TSType::TSIntersectionType(inter) => {
                let types: Vec<Type> = inter
                    .types
                    .iter()
                    .map(|t| self.resolve_ts_type(t))
                    .collect();
                Type::Intersection(types)
            }

            TSType::TSTypeReference(type_ref) => {
                let name = match &type_ref.type_name {
                    TSTypeName::IdentifierReference(ident) => ident.name.to_string(),
                    TSTypeName::QualifiedName(qual) => qual.right.name.to_string(),
                };

                let type_args: Vec<Type> = type_ref
                    .type_parameters
                    .as_ref()
                    .map(|params| {
                        params
                            .params
                            .iter()
                            .map(|t| self.resolve_ts_type(t))
                            .collect()
                    })
                    .unwrap_or_default();

                Type::TypeRef { name, type_args }
            }

            TSType::TSFunctionType(func) => {
                let params: Vec<Param> = func
                    .params
                    .items
                    .iter()
                    .map(|p| {
                        let name = match &p.pattern.kind {
                            BindingPatternKind::BindingIdentifier(ident) => {
                                ident.name.to_string()
                            }
                            _ => "_".to_string(),
                        };
                        let ty = self.resolve_type_annotation_oxc(&p.pattern.type_annotation);
                        Param::new(name, ty)
                    })
                    .collect();

                let return_type = self.resolve_ts_type(&func.return_type.type_annotation);

                Type::Function {
                    params,
                    return_type: Box::new(return_type),
                    type_params: vec![],
                }
            }

            TSType::TSTypeLiteral(lit) => {
                let properties: Vec<Property> = lit
                    .members
                    .iter()
                    .filter_map(|member| {
                        if let TSSignature::TSPropertySignature(prop) = member {
                            let name = match &prop.key {
                                PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
                                PropertyKey::StringLiteral(s) => s.value.to_string(),
                                _ => return None,
                            };
                            let ty = prop
                                .type_annotation
                                .as_ref()
                                .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                                .unwrap_or(Type::Any);
                            let mut property = Property::new(name, ty);
                            if prop.optional {
                                property = property.optional();
                            }
                            if prop.readonly {
                                property = property.readonly();
                            }
                            Some(property)
                        } else {
                            None
                        }
                    })
                    .collect();

                Type::Object {
                    properties,
                    index_signature: None,
                }
            }

            TSType::TSParenthesizedType(paren) => self.resolve_ts_type(&paren.type_annotation),

            _ => Type::Any,
        }
    }

    /// Tuple elements forms:
    /// - Regular: `[string, number]`
    /// - Optional: `[string, number?]`
    /// - Rest: `[string, ...number[]]`
    /// - Named: `[name: string, age: number]`
    fn resolve_tuple_element(&self, elem: &TSTupleElement) -> Type {
        match elem {
            TSTupleElement::TSOptionalType(opt) => self.resolve_ts_type(&opt.type_annotation),
            TSTupleElement::TSRestType(rest) => self.resolve_ts_type(&rest.type_annotation),
            TSTupleElement::TSNamedTupleMember(named) => {
                self.resolve_tuple_element(&named.element_type)
            }
            _ => {
                if let Some(ty) = elem.as_ts_type() {
                    self.resolve_ts_type(ty)
                } else {
                    Type::Any
                }
            }
        }
    }

    fn build_function_type(&self, func: &Function) -> Type {
        let params: Vec<Param> = func
            .params
            .items
            .iter()
            .map(|p| {
                let name = match &p.pattern.kind {
                    BindingPatternKind::BindingIdentifier(ident) => ident.name.to_string(),
                    _ => "_".to_string(),
                };
                let ty = self.resolve_type_annotation_oxc(&p.pattern.type_annotation);
                let mut param = Param::new(name, ty);
                if p.pattern.optional {
                    param = param.optional();
                }
                param
            })
            .collect();

        let return_type = func
            .return_type
            .as_ref()
            .map(|ann| self.resolve_ts_type(&ann.type_annotation))
            .unwrap_or(Type::Void);

        let type_params: Vec<TypeParam> = func
            .type_parameters
            .as_ref()
            .map(|params| {
                params
                    .params
                    .iter()
                    .map(|p| {
                        let mut tp = TypeParam::new(p.name.name.to_string());
                        if let Some(constraint) = &p.constraint {
                            tp = tp.with_constraint(self.resolve_ts_type(constraint));
                        }
                        tp
                    })
                    .collect()
            })
            .unwrap_or_default();

        Type::Function {
            params,
            return_type: Box::new(return_type),
            type_params,
        }
    }

    fn build_interface_type(&self, decl: &TSInterfaceDeclaration) -> Type {
        let properties: Vec<Property> = decl
            .body
            .body
            .iter()
            .filter_map(|member| {
                if let TSSignature::TSPropertySignature(prop) = member {
                    let name = match &prop.key {
                        PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
                        PropertyKey::StringLiteral(s) => s.value.to_string(),
                        _ => return None,
                    };
                    let ty = prop
                        .type_annotation
                        .as_ref()
                        .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                        .unwrap_or(Type::Any);
                    let mut property = Property::new(name, ty);
                    if prop.optional {
                        property = property.optional();
                    }
                    if prop.readonly {
                        property = property.readonly();
                    }
                    Some(property)
                } else {
                    None
                }
            })
            .collect();

        Type::Object {
            properties,
            index_signature: None,
        }
    }

    fn infer_expression_type(&self, expr: &Expression) -> Type {
        match expr {
            Expression::StringLiteral(s) => Type::StringLiteral(s.value.to_string()),
            Expression::NumericLiteral(n) => Type::NumberLiteral(n.value),
            Expression::BooleanLiteral(b) => Type::BooleanLiteral(b.value),
            Expression::NullLiteral(_) => Type::Null,
            Expression::ArrayExpression(_) => Type::Array(Box::new(Type::Any)),
            Expression::ObjectExpression(_) => Type::Object {
                properties: vec![],
                index_signature: None,
            },
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
                Type::Function {
                    params: vec![],
                    return_type: Box::new(Type::Any),
                    type_params: vec![],
                }
            }
            Expression::Identifier(ident) => {
                // Look up the identifier's type
                self.symbols
                    .lookup(ident.name.as_str())
                    .map(|s| s.ty.clone())
                    .unwrap_or(Type::Any)
            }
            _ => Type::Any,
        }
    }
}

/// Built-in globals that we skip for now (no lib.d.ts yet).
/// Once we load lib.d.ts, these will be properly defined.
fn is_builtin_global(name: &str) -> bool {
    matches!(
        name,
        "console"
            | "window"
            | "document"
            | "global"
            | "globalThis"
            | "process"
            | "require"
            | "module"
            | "exports"
            | "__dirname"
            | "__filename"
            | "setTimeout"
            | "setInterval"
            | "clearTimeout"
            | "clearInterval"
            | "Promise"
            | "Array"
            | "Object"
            | "String"
            | "Number"
            | "Boolean"
            | "Map"
            | "Set"
            | "WeakMap"
            | "WeakSet"
            | "Symbol"
            | "Error"
            | "TypeError"
            | "RangeError"
            | "SyntaxError"
            | "ReferenceError"
            | "JSON"
            | "Math"
            | "Date"
            | "RegExp"
            | "parseInt"
            | "parseFloat"
            | "isNaN"
            | "isFinite"
            | "undefined"
            | "NaN"
            | "Infinity"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    fn parse_and_bind(source: &str) -> Binder {
        let allocator = Allocator::default();
        let source_type = SourceType::ts();
        let result = Parser::new(&allocator, source, source_type).parse();
        assert!(!result.panicked);

        let mut binder = Binder::new();
        binder.bind_program(&result.program);
        binder
    }

    #[test]
    fn test_bind_variable() {
        let binder = parse_and_bind("const x: number = 42;");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.lookup("x").is_some());
    }

    #[test]
    fn test_bind_function() {
        let binder = parse_and_bind("function add(a: number, b: number): number { return a + b; }");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.lookup("add").is_some());
    }

    #[test]
    fn test_bind_interface() {
        let binder = parse_and_bind("interface User { name: string; age: number; }");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.lookup_type("User").is_some());
    }

    #[test]
    fn test_duplicate_variable() {
        let binder = parse_and_bind("const x = 1; const x = 2;");
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::DuplicateSymbol(err) => assert_eq!(err.name, "x"),
            _ => panic!("Expected DuplicateSymbol error"),
        }
    }

    #[test]
    fn test_undefined_reference() {
        let binder = parse_and_bind("const x = y;");
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::UndefinedSymbol(err) => assert_eq!(err.name, "y"),
            _ => panic!("Expected UndefinedSymbol error"),
        }
    }

    #[test]
    fn test_function_scope() {
        let binder = parse_and_bind(
            r#"
            const x = 1;
            function foo() {
                const y = 2;
                return x + y;
            }
            "#,
        );
        assert!(binder.errors.is_empty());
    }

    #[test]
    fn test_shadowing_allowed() {
        let binder = parse_and_bind(
            r#"
            const x = 1;
            function foo() {
                const x = 2;
                return x;
            }
            "#,
        );
        assert!(binder.errors.is_empty());
    }

    #[test]
    fn test_type_alias() {
        let binder = parse_and_bind("type StringOrNumber = string | number;");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.lookup_type("StringOrNumber").is_some());
    }

    #[test]
    fn test_class_declaration() {
        let binder = parse_and_bind(
            r#"
            class Person {
                name: string;
                constructor(name: string) {
                    this.name = name;
                }
            }
            "#,
        );
        assert!(binder.errors.is_empty());
        // Class is in both namespaces
        assert!(binder.symbols.lookup("Person").is_some());
        assert!(binder.symbols.lookup_type("Person").is_some());
    }

    #[test]
    fn test_arrow_function() {
        let binder = parse_and_bind("const add = (a: number, b: number) => a + b;");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.lookup("add").is_some());
    }

    #[test]
    fn test_block_scope() {
        let binder = parse_and_bind(
            r#"
            {
                const x = 1;
            }
            const y = x;
            "#,
        );
        // x should not be visible outside the block
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::UndefinedSymbol(err) => assert_eq!(err.name, "x"),
            _ => panic!("Expected UndefinedSymbol error"),
        }
    }
}
