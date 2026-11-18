//! Walking expressions to bind the functions nested inside them. Class
//! expressions are not walked yet.

use oxc_ast::ast::*;

use super::Binder;

impl Binder<'_> {
    pub(super) fn bind_expression(&mut self, expr: &Expression) {
        match expr {
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

            _ => {}
        }
    }

    pub(super) fn bind_assignment_target(&mut self, target: &AssignmentTarget) {
        match target {
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
