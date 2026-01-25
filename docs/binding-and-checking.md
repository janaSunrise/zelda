# Binding and Checking

This doc explains binding and checking, and how they connect.

### The Pipeline

Here's the flow of a type checker:
Source code → Parser → AST → Binder → Symbol Table → Checker → Errors

The parser turns text into an AST (done by oxc for us). Then:
1. Binder walks the AST and builds a symbol table
2. Checker uses that symbol table to verify type correctness

You can't check types until you know what names exist and what types they have. The binder figures that out.
The checker does the actual type checking.

### What the Binder Does

The binder walks through the code and writes down every name it sees being declared, along with enough information to look it up later.

The binder has two jobs:
1. Register declarations: when you write `const x = 5`, the binder records that `x` exists, its type is `number`, and where it was declared
2. Detect early errors: duplicate declarations (`const x = 1; const x = 2;`) and undefined references (`const y = unknownVar;`)


#### Walking the AST

The binder recursively walks every statement in the program:

```rs
pub fn bind_program(&mut self, program: &Program) {
    for stmt in &program.body {
        self.bind_statement(stmt);
    }
}

fn bind_statement(&mut self, stmt: &Statement) {
    match stmt {
        Statement::VariableDeclaration(decl) => self.bind_variable_declaration(decl),
        Statement::FunctionDeclaration(decl) => self.bind_function_declaration(decl),
        Statement::BlockStatement(block) => self.bind_block_statement(block),
        // ...
    }
}
```

When it encounters a declaration, it extracts the name and type, then adds it to the symbol table.

#### Binding a Variable

Let's trace through `const x: number = 42;`:
1. Parser gives us a `VariableDeclaration` node with one `VariableDeclarator`
2. We extract the name: `x`
3. We get the type from the annotation: `number`
4. We call `symbols.define("x", Type::Number, SymbolKind::Variable, span)`
5. If `x` already exists in this scope, we emit a duplicate error
6. We walk the initializer expression to check for undefined references

```rs
fn bind_variable_declarator(&mut self, declarator: &VariableDeclarator) {
    if let BindingPattern::BindingIdentifier(ident) = &declarator.id {
        let name = ident.name.as_str();
        let span = ident.span;
        let ty = self.resolve_binding_type(declarator);

        if let Err(err) = self.symbols.define(name, ty, SymbolKind::Variable, span) {
            self.errors.push(err.into());
        }
    }

    // Check initializer for undefined references
    if let Some(init) = &declarator.init {
        self.bind_expression(init);
    }
}
```

#### Binding Functions

Functions are interesting because they create scope and can reference themselves.

```ts
function factorial(n: number): number {
    if (n <= 1) return 1;
    return n * factorial(n - 1);  // recursive call
}
```

The order matters:
1. Add the function name to the current scope (before entering the body)
2. Push a new function scope
3. Add parameters to the function scope
4. Bind the body statements
5. Pop back to the parent scope

Step 1 happens first so the recursive `factorial(n - 1)` call inside the body can find the function.

```rs
fn bind_function_declaration(&mut self, decl: &Function) {
    // 1. Add function to current scope (for recursion)
    if let Some(ident) = &decl.id {
        let ty = self.build_function_type(decl);
        self.symbols.define(name, ty, SymbolKind::Function, span);
    }

    // 2. New scope for function body
    self.symbols.push_scope(ScopeKind::Function);

    // 3. Add parameters
    for param in &decl.params.items {
        self.bind_formal_parameter(param);
    }

    // 4. Bind body
    if let Some(body) = &decl.body {
        for stmt in &body.statements {
            self.bind_statement(stmt);
        }
    }

    // 5. Pop scope
    self.symbols.pop_scope();
}
```

#### Block Scopes

Blocks create new scopes too. This is why `let` and `const` are block-scoped:

```ts
{
    const x = 1;
}
const y = x;  // x is not defined
```

When we enter a block, we push a scope. When we exit, we pop it. Any symbols defined inside disappear.

```rs
fn bind_block_statement(&mut self, block: &BlockStatement) {
    self.symbols.push_scope(ScopeKind::Block);
    for stmt in &block.body {
        self.bind_statement(stmt);
    }
    self.symbols.pop_scope();
}
```

### Handoff to the Checker

After binding, we have a symbol table with every declared name and its type. The binder passes this to the checker:

```rs
// Binding phase
let mut binder = Binder::new();
binder.bind_program(&program);

// Checking phase receives the symbol table
let mut checker = Checker::new(&binder.symbols);
checker.check_program(&program);
```

The checker takes a reference to the symbol table. It doesn't modify it, it just reads from the table to verify types.

### What the Checker Does

The checker walks the AST again, but this time it's verifying type correctness:

1. Type inference - compute the type of expressions
2. Type checking - verify that types are compatible where required
3. Error collection - gather all type errors

#### Walking Statements

Similar to the binder, the checker walks statements:

```rs
fn check_statement(&mut self, stmt: &Statement) {
    match stmt {
        Statement::VariableDeclaration(decl) => self.check_variable_declaration(decl),
        Statement::FunctionDeclaration(func) => self.check_function_declaration(func),
        Statement::ExpressionStatement(expr) => self.check_expression(&expr.expression),
        // ...
    }
}
```

But it's doing different things. When it sees a variable declaration, it's checking that the initializer type matches the declared type.

#### Checking a Variable Declaration

For `const x: number = "hello";`:
1. Infer the initializer type: `"hello"` as `string`
2. Get the declared type from annotation: `number`
3. Check if `string` is assignable to `number` (no!)
4. Emit error: Type 'string' is not assignable to type 'number'

```rs
fn check_variable_declaration(&mut self, decl: &VariableDeclaration) {
    for declarator in &decl.declarations {
        if let Some(init) = &declarator.init {
            let init_type = self.infer_expression(init);

            if let Some(annotation) = &declarator.type_annotation {
                let declared_type = self.resolve_type(&annotation.type_annotation);
                if !self.is_assignable(&init_type, &declared_type) {
                    self.errors.push(TypeError::not_assignable(
                        &init_type,
                        &declared_type,
                        declarator.span,
                    ));
                }
            }
        }
    }
}
```

#### Checking Function Calls

For a call like `add(1, "hello")` where `add(a: number, b: number)`:
1. Look up `add` in the symbol table
2. Get its type: `(a: number, b: number) => number`
3. Check argument count: 2 expected, 2 provided.
4. Check first argument: `1` is `number`, param wants `number`.
5. Check second argument: `"hello"` is `string`, param wants `number`.
6. Emit error: Argument of type 'string' is not assignable to parameter of type 'number'

```rs
fn check_call_expression(&mut self, call: &CallExpression) {
    let callee_type = self.infer_expression(&call.callee);

    if let Type::Function { params, .. } = callee_type {
        // Check argument count
        if call.arguments.len() < params.len() {
            self.errors.push(TypeError::wrong_argument_count(...));
        }

        // Check argument types
        for (i, arg) in call.arguments.iter().enumerate() {
            let arg_type = self.infer_expression(arg);
            if !self.is_assignable(&arg_type, &params[i].ty) {
                self.errors.push(TypeError::argument_not_assignable(...));
            }
        }
    }
}
```

#### Using the Symbol Table

The checker constantly queries the symbol table:

```rs
// Look up variable type
fn infer_expression(&self, expr: &Expression) -> Type {
    match expr {
        Expression::Identifier(ident) => {
            self.symbols
                .lookup(ident.name.as_str())
                .map(|s| s.ty.clone())
                .unwrap_or(Type::Any)
        }
        // ...
    }
}
```

When it sees `x + 1`, it needs to know what type `x` is. It looks up `x` in the symbol table, finds the symbol the binder registered,
and gets its type.

### The Full Flow

```ts
const x: number = 42;
const y = x + 1;
const z: string = y; // error
```

Binding phase:
1. Bind `const x: number = 42`
   - Define symbol: `x`, type: `number`, scope: global
2. Bind `const y = x + 1`
   - Check `x` is defined using symbol table (it is)
   - No type annotation, so we don't know `y`'s type yet
   - Define symbol: `y`, type will be inferred, scope: global
3. Bind `const z: string = y`
   - Check `y` is defined (yes)
   - Define symbol: `z`, type: `string`, scope: global

Checking phase:
1. Check `const x: number = 42`
   - Infer `42` as `number` (literal)
   - Declared type: `number`
   - Is `number` assignable to `number`? (yes)
2. Check `const y = x + 1`
   - Infer `x + 1` as `number` (need to infer `x` first)
   - Look up `x` in symbol table (type is `number`)
   - `number + number` is `number`
   - No annotation, so no check needed
3. Check `const z: string = y`
   - Infer `y`: look up in symbol table (type is `number`)
   - Declared type: `string`
   - Is `number` assignable to `string`? (no)
   - Emit error: Type 'number' is not assignable to type 'string'

### Why Separate Phases?

Why we don't just do everything in one pass. A few reasons:

Forward references: In TypeScript, you can reference things before they're declared in some cases:

```ts
console.log(x);  // hoisting allows this for var
var x = 1;
```

The binder handles this by doing a first pass to register everything, so the checker can find any name.

Recursion and mutual recursion:
```ts
function isEven(n: number): boolean {
    return n === 0 || isOdd(n - 1);
}

function isOdd(n: number): boolean {
    return n !== 0 && isEven(n - 1);
}
```

Both functions call each other. If we tried to check while binding, `isEven` would fail because `isOdd` isn't registered yet.
