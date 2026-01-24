//! Walking expressions to find undefined variable references.
//!
//! For each identifier in an expression, we check if it's defined in the current scope chain.
//! Compound expressions like `x + y` recursively check each part.

use oxc_ast::ast::*;

use crate::symbols::UndefinedSymbolError;

use super::{is_builtin_global, BindingError, Binder};

impl Binder {
    /// Walk an expression to find undefined variable references.
    pub(super) fn bind_expression(&mut self, expr: &Expression) {
        match expr {
            // Identifiers - check if defined
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

            // Call expressions
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

            // Member expressions (oxc splits into separate variants)
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

            // Binary and unary
            Expression::BinaryExpression(binary) => {
                self.bind_expression(&binary.left);
                self.bind_expression(&binary.right);
            }
            Expression::UnaryExpression(unary) => {
                self.bind_expression(&unary.argument);
            }

            // Assignment
            Expression::AssignmentExpression(assign) => {
                self.bind_assignment_target(&assign.left);
                self.bind_expression(&assign.right);
            }

            // Array literal
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

            // Object literal
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

            // Functions (create their own scope)
            Expression::ArrowFunctionExpression(arrow) => self.bind_arrow_function(arrow),
            Expression::FunctionExpression(func) => self.bind_function_expression(func),

            // Conditional (ternary)
            Expression::ConditionalExpression(cond) => {
                self.bind_expression(&cond.test);
                self.bind_expression(&cond.consequent);
                self.bind_expression(&cond.alternate);
            }

            // Logical operators
            Expression::LogicalExpression(logical) => {
                self.bind_expression(&logical.left);
                self.bind_expression(&logical.right);
            }

            // Sequence (comma operator)
            Expression::SequenceExpression(seq) => {
                for expr in &seq.expressions {
                    self.bind_expression(expr);
                }
            }

            // Template literals
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

            // New expression
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

            // Await/yield
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

    /// Check assignment targets for undefined references.
    pub(super) fn bind_assignment_target(&mut self, target: &AssignmentTarget) {
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
}
