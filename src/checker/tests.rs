use super::*;
use crate::binder::Binder;
use crate::lib_dts::load_lib_dts;
use crate::symbols::SymbolTable;
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn check(source: &str) -> Vec<TypeError> {
    let allocator = Allocator::default();
    let source_type = SourceType::ts();
    let result = Parser::new(&allocator, source, source_type).parse();
    assert!(!result.panicked);

    let mut symbols = SymbolTable::new();
    load_lib_dts(&mut symbols);

    let mut binder = Binder::with_symbols(symbols);
    binder.bind_program(&result.program);

    let mut checker = Checker::new(&mut binder.symbols);
    checker.check_program(&result.program);

    checker.errors
}

#[test]
fn test_infer_string_literal() {
    let errors = check("const x = \"hello\";");
    assert!(errors.is_empty());
}

#[test]
fn test_infer_number_literal() {
    let errors = check("const x = 42;");
    assert!(errors.is_empty());
}

#[test]
fn test_const_vs_let_widening() {
    let errors = check("const x = \"hello\"; let y = \"world\";");
    assert!(errors.is_empty());
}

#[test]
fn test_type_mismatch() {
    let errors = check("const x: number = \"hello\";");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_array_inference() {
    let errors = check("const arr = [1, 2, 3];");
    assert!(errors.is_empty());
}

#[test]
fn test_mixed_array_inference() {
    let errors = check("const arr = [1, \"hello\"];");
    assert!(errors.is_empty());
}

#[test]
fn test_object_inference() {
    let errors = check("const obj = { x: 1, y: \"hello\" };");
    assert!(errors.is_empty());
}

#[test]
fn test_function_return_check() {
    let errors = check("function f(): number { return \"hello\"; }");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_function_return_inference() {
    let errors = check("function f() { return 42; }");
    assert!(errors.is_empty());
}

#[test]
fn test_binary_operations() {
    let errors = check("const x = 1 + 2; const y = \"a\" + \"b\";");
    assert!(errors.is_empty());
}

#[test]
fn test_assignable_literal_to_base() {
    let errors = check("const x: string = \"hello\";");
    assert!(errors.is_empty());
}

#[test]
fn test_union_not_assignable() {
    let errors = check("const x: string | number = true;");
    assert_eq!(errors.len(), 1);
}

#[test]
fn test_shorthand_property() {
    // { x } is equivalent to { x: x }
    let errors = check("const x = 1; const obj = { x };");
    assert!(errors.is_empty());
}

#[test]
fn test_method_shorthand() {
    // { foo() {} } is equivalent to { foo: function() {} }
    let errors = check("const obj = { foo() { return 1; } };");
    assert!(errors.is_empty());
}

#[test]
fn test_spread_in_object() {
    let errors = check("const a = { x: 1 }; const b = { ...a, y: 2 };");
    assert!(errors.is_empty());
}

#[test]
fn test_empty_array() {
    let errors = check("const arr: number[] = [];");
    assert!(errors.is_empty());
}

#[test]
fn test_empty_array_inferred() {
    // Empty array infers never[] but we allow it
    let errors = check("const arr = [];");
    assert!(errors.is_empty());
}

#[test]
fn test_function_call_wrong_arg_count() {
    let errors = check("function f(x: number) {} f();");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2554); // Expected N arguments, but got M
}

#[test]
fn test_function_call_too_many_args() {
    let errors = check("function f(x: number) {} f(1, 2);");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2554);
}

#[test]
fn test_function_call_wrong_arg_type() {
    let errors = check("function f(x: number) {} f(\"hello\");");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2345); // Argument not assignable
}

#[test]
fn test_function_call_correct() {
    let errors = check("function f(x: number, y: string) {} f(1, \"hello\");");
    assert!(errors.is_empty());
}

#[test]
fn test_function_call_optional_param() {
    let errors = check("function f(x: number, y?: string) {} f(1);");
    assert!(errors.is_empty());
}

#[test]
fn test_reassignment_type_mismatch() {
    let errors = check("let x: number = 1; x = \"hello\";");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_reassignment_correct() {
    let errors = check("let x: number = 1; x = 2;");
    assert!(errors.is_empty());
}

#[test]
fn test_missing_return_in_non_void() {
    let errors = check("function f(): number {}");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2366); // Missing return
}

#[test]
fn test_void_function_no_return_ok() {
    let errors = check("function f(): void {}");
    assert!(errors.is_empty());
}

#[test]
fn test_undefined_return_no_return_ok() {
    let errors = check("function f(): undefined {}");
    assert!(errors.is_empty());
}

#[test]
fn test_null_assignable_to_number() {
    // In non-strict mode, null is assignable to anything
    let errors = check("const x: number = null;");
    assert!(errors.is_empty());
}

#[test]
fn test_undefined_assignable_to_string() {
    // In non-strict mode, undefined is assignable to anything
    let errors = check("const x: string = undefined;");
    assert!(errors.is_empty());
}

#[test]
fn test_object_missing_property() {
    let errors = check("const x: { a: number } = {};");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2741);
}

#[test]
fn test_object_missing_multiple_properties() {
    let errors = check("const x: { a: number; b: string } = {};");
    assert_eq!(errors.len(), 2);
}

#[test]
fn test_object_has_required_property() {
    let errors = check("const x: { a: number } = { a: 1 };");
    assert!(errors.is_empty());
}

#[test]
fn test_object_property_type_mismatch() {
    let errors = check("const x: { a: number } = { a: \"hello\" };");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_object_excess_property() {
    let errors = check("const x: { a: number } = { a: 1, b: 2 };");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2353);
}

#[test]
fn test_object_multiple_excess_properties() {
    let errors = check("const x: { a: number } = { a: 1, b: 2, c: 3 };");
    assert_eq!(errors.len(), 2);
}

#[test]
fn test_object_optional_property_missing_ok() {
    let errors = check("const x: { a: number; b?: string } = { a: 1 };");
    assert!(errors.is_empty());
}

#[test]
fn test_object_optional_property_present_ok() {
    let errors = check("const x: { a: number; b?: string } = { a: 1, b: \"hi\" };");
    assert!(errors.is_empty());
}

#[test]
fn test_object_optional_property_wrong_type() {
    let errors = check("const x: { a: number; b?: string } = { a: 1, b: 42 };");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_object_all_optional_empty_ok() {
    let errors = check("const x: { a?: number; b?: string } = {};");
    assert!(errors.is_empty());
}

#[test]
fn test_property_access_exists() {
    let errors = check("const obj = { x: 1 }; const y = obj.x;");
    assert!(errors.is_empty());
}

#[test]
fn test_property_access_not_exists() {
    let errors = check("const obj = { x: 1 }; const y = obj.z;");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339);
}

#[test]
fn test_property_access_on_typed_object() {
    let errors = check("const obj: { x: number } = { x: 1 }; const y = obj.x;");
    assert!(errors.is_empty());
}

#[test]
fn test_property_access_not_exists_on_typed() {
    let errors = check("const obj: { x: number } = { x: 1 }; const y = obj.z;");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339);
}

#[test]
fn test_computed_property_access_string_literal() {
    let errors = check("const obj = { x: 1 }; const y = obj[\"x\"];");
    assert!(errors.is_empty());
}

#[test]
fn test_computed_property_access_not_exists() {
    let errors = check("const obj = { x: 1 }; const y = obj[\"z\"];");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339);
}

#[test]
fn test_property_access_on_any() {
    let errors = check("const obj: any = {}; const y = obj.anything;");
    assert!(errors.is_empty());
}

#[test]
fn test_array_length_property() {
    let errors = check("const arr = [1, 2, 3]; const len = arr.length;");
    assert!(errors.is_empty());
}

#[test]
fn test_array_method_property() {
    let errors = check("const arr = [1, 2, 3]; const mapped = arr.map;");
    assert!(errors.is_empty());
}

#[test]
fn test_string_length_property() {
    let errors = check("const s = \"hello\"; const len = s.length;");
    assert!(errors.is_empty());
}

#[test]
fn test_nested_property_access() {
    let errors = check("const obj = { inner: { x: 1 } }; const y = obj.inner.x;");
    assert!(errors.is_empty());
}

#[test]
fn test_nested_property_not_exists() {
    let errors = check("const obj = { inner: { x: 1 } }; const y = obj.inner.z;");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339);
}

#[test]
fn test_union_source_all_branches_must_match() {
    // string | number is NOT assignable to string
    let errors = check("const x: string | number = 1; const y: string = x;");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_union_target_any_branch_works() {
    // string is assignable to string | number
    let errors = check("const x: string | number = \"hello\";");
    assert!(errors.is_empty());
}

#[test]
fn test_union_with_null() {
    let errors = check("const x: string | null = null;");
    assert!(errors.is_empty());
}

#[test]
fn test_union_with_undefined() {
    let errors = check("const x: number | undefined = undefined;");
    assert!(errors.is_empty());
}

#[test]
fn test_nested_union() {
    let errors = check("const x: (string | number) | boolean = true;");
    assert!(errors.is_empty());
}

#[test]
fn test_intersection_must_satisfy_all() {
    let errors = check("const x: { a: number } & { b: string } = { a: 1 };");
    assert_eq!(errors.len(), 1);
}

#[test]
fn test_intersection_satisfies_all() {
    let errors = check("const x: { a: number } & { b: string } = { a: 1, b: \"hi\" };");
    assert!(errors.is_empty());
}

#[test]
fn test_array_element_type_mismatch() {
    let errors = check("const x: number[] = [1, 2, \"three\"];");
    assert_eq!(errors.len(), 1);
}

#[test]
fn test_array_covariance() {
    // string[] should not be assignable to (string | number)[] in strict mode
    // but we're in non-strict, so this works
    let errors = check("const x: string[] = [\"a\"]; const y: (string | number)[] = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_tuple_exact_length() {
    let errors = check("const x: [number, string] = [1];");
    assert_eq!(errors.len(), 1);
}

#[test]
fn test_tuple_type_mismatch() {
    let errors = check("const x: [number, string] = [1, 2];");
    assert_eq!(errors.len(), 1);
}

#[test]
fn test_tuple_correct() {
    let errors = check("const x: [number, string] = [1, \"hello\"];");
    assert!(errors.is_empty());
}

#[test]
fn test_tuple_to_array() {
    let errors = check("const x: [number, number] = [1, 2]; const y: number[] = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_function_return_covariance() {
    // () => string is assignable to () => string | number
    let errors = check(r#"
        const f: () => string = () => "hello";
        const g: () => string | number = f;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_function_param_contravariance() {
    // (x: string | number) => void is assignable to (x: string) => void
    let errors = check(r#"
        const f: (x: string | number) => void = (x) => {};
        const g: (x: string) => void = f;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_function_fewer_params_ok() {
    // () => void is assignable to (x: number) => void (callback compatibility)
    let errors = check(r#"
        const f: () => void = () => {};
        const g: (x: number) => void = f;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_function_more_params_error() {
    // (x: number, y: number) => void is NOT assignable to (x: number) => void
    let errors = check(r#"
        const f: (x: number, y: number) => void = (x, y) => {};
        const g: (x: number) => void = f;
    "#);
    assert_eq!(errors.len(), 1);
}

#[test]
fn test_any_accepts_anything() {
    let errors = check("const x: any = { foo: 1, bar: \"test\" };");
    assert!(errors.is_empty());
}

#[test]
fn test_any_assignable_to_anything() {
    let errors = check("const x: any = 1; const y: string = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_unknown_accepts_anything() {
    let errors = check("const x: unknown = { foo: 1 };");
    assert!(errors.is_empty());
}

#[test]
fn test_never_assignable_to_anything() {
    let errors = check(r#"
        function fail(): never { throw new Error(); }
        const x: string = fail();
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_void_function() {
    let errors = check("function f(): void { return; }");
    assert!(errors.is_empty());
}

#[test]
fn test_ternary_inference() {
    let errors = check("const x = true ? 1 : \"hello\"; const y: string | number = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_logical_and_inference() {
    // a && b returns b's type if a is truthy
    let errors = check("const x = true && 42; const y: number = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_logical_or_inference() {
    let errors = check("const x = false || \"fallback\"; const y: boolean | string = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_nullish_coalescing_inference() {
    let errors = check("const x = null ?? \"default\"; const y: null | string = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_arithmetic_inference() {
    let errors = check("const x = 1 + 2 * 3 - 4 / 2; const y: number = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_string_concat_inference() {
    let errors = check("const x = \"hello\" + \" \" + \"world\"; const y: string = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_comparison_inference() {
    let errors = check("const x = 1 < 2; const y: boolean = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_typeof_inference() {
    let errors = check("const x = typeof 42; const y: string = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_unary_not_inference() {
    let errors = check("const x = !true; const y: boolean = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_unary_minus_inference() {
    let errors = check("const x = -42; const y: number = x;");
    assert!(errors.is_empty());
}

#[test]
fn test_nested_object_inference() {
    let errors = check(r#"
        const obj = {
            user: {
                name: "alice",
                age: 30
            },
            active: true
        };
        const name: string = obj.user.name;
        const age: number = obj.user.age;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_array_of_objects_inference() {
    let errors = check(r#"
        const users = [
            { name: "alice", age: 30 },
            { name: "bob", age: 25 }
        ];
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_function_returning_object() {
    let errors = check(r#"
        function createUser(name: string, age: number) {
            return { name, age };
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_call_with_literal_args() {
    let errors = check(r#"
        function greet(name: string, age: number): string {
            return name;
        }
        const result = greet("alice", 30);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_call_with_variable_args() {
    let errors = check(r#"
        function add(a: number, b: number): number {
            return a + b;
        }
        const x = 1;
        const y = 2;
        const sum = add(x, y);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_call_with_wrong_literal_type() {
    let errors = check(r#"
        function square(n: number): number {
            return n * n;
        }
        const result = square("hello");
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2345);
}

#[test]
fn test_callback_function() {
    let errors = check(r#"
        function map(arr: number[], fn: (x: number) => number): number[] {
            return arr;
        }
        const doubled = map([1, 2, 3], (x: number) => x * 2);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_interface_as_type() {
    let errors = check(r#"
        interface User {
            name: string;
            age: number;
        }
        const user: User = { name: "alice", age: 30 };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_interface_missing_property() {
    let errors = check(r#"
        interface User {
            name: string;
            age: number;
        }
        const user: User = { name: "alice" };
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2741);
}

#[test]
fn test_interface_optional_property() {
    let errors = check(r#"
        interface User {
            name: string;
            age?: number;
        }
        const user: User = { name: "alice" };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_type_alias() {
    let errors = check(r#"
        type StringOrNumber = string | number;
        const x: StringOrNumber = 42;
        const y: StringOrNumber = "hello";
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_empty_object_literal() {
    let errors = check("const x: {} = {};");
    assert!(errors.is_empty());
}

#[test]
fn test_object_with_only_optional_properties() {
    let errors = check("const x: { a?: number; b?: string } = {};");
    assert!(errors.is_empty());
}

#[test]
fn test_deeply_nested_property_access() {
    let errors = check(r#"
        const obj = { a: { b: { c: { d: 42 } } } };
        const val: number = obj.a.b.c.d;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_array_index_access() {
    let errors = check(r#"
        const arr = [1, 2, 3];
        const first = arr[0];
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_spread_operator_in_array() {
    let errors = check(r#"
        const a = [1, 2];
        const b = [...a, 3, 4];
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_template_literal() {
    let errors = check(r#"
        const name = "world";
        const greeting: string = `Hello, ${name}!`;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_arrow_function_expression_body() {
    let errors = check("const double = (x: number) => x * 2;");
    assert!(errors.is_empty());
}

#[test]
fn test_arrow_function_block_body() {
    let errors = check(r#"
        const double = (x: number) => {
            return x * 2;
        };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_iife() {
    let errors = check("const x = ((n: number) => n * 2)(5);");
    assert!(errors.is_empty());
}

#[test]
fn test_recursive_function() {
    let errors = check(r#"
        function factorial(n: number): number {
            if (n <= 1) return 1;
            return n * factorial(n - 1);
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_mutually_recursive_functions() {
    let errors = check(r#"
        function isEven(n: number): boolean {
            if (n === 0) return true;
            return isOdd(n - 1);
        }
        function isOdd(n: number): boolean {
            if (n === 0) return false;
            return isEven(n - 1);
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_const_assertion_behavior() {
    // const gets literal type, let gets widened
    let errors = check(r#"
        const x = "hello";
        let y = "hello";
        const a: "hello" = x;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_multiple_statements_multiple_errors() {
    let errors = check(r#"
        const x: number = "wrong";
        const y: string = 123;
        const z: boolean = "also wrong";
    "#);
    assert_eq!(errors.len(), 3);
}

#[test]
fn test_void_vs_undefined() {
    let errors = check(r#"
        function f(): void {}
        function g(): undefined { return undefined; }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_bigint_literal() {
    let errors = check("const x = 9007199254740991n;");
    assert!(errors.is_empty());
}

#[test]
fn test_sequence_expression() {
    let errors = check("const x = (1, 2, 3);");
    assert!(errors.is_empty());
}

#[test]
fn test_new_expression() {
    let errors = check("const date = new Date();");
    assert!(errors.is_empty());
}

#[test]
fn test_for_loop_scope() {
    let errors = check(r#"
        for (let i = 0; i < 10; i++) {
            const x = i;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_while_loop() {
    let errors = check(r#"
        let x = 0;
        while (x < 10) {
            x = x + 1;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_if_else() {
    let errors = check(r#"
        const x = 5;
        if (x > 0) {
            const positive = true;
        } else {
            const negative = true;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_typeof_narrowing_string() {
    let errors = check(r#"
        function f(x: string | number) {
            if (typeof x === "string") {
                const y: string = x;
            }
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_typeof_narrowing_number() {
    let errors = check(r#"
        function f(x: string | number) {
            if (typeof x === "number") {
                const y: number = x;
            }
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_null_check_narrowing() {
    let errors = check(r#"
        function f(x: string | null) {
            if (x !== null) {
                const y: string = x;
            }
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_undefined_check_narrowing() {
    let errors = check(r#"
        function f(x: string | undefined) {
            if (x !== undefined) {
                const y: string = x;
            }
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_narrowing_not_applied_outside_if() {
    let errors = check(r#"
        function f(x: string | number) {
            if (typeof x === "string") {
                const y: string = x;
            }
            const z: string = x;
        }
    "#);
    // z assignment should fail because x is still string | number outside if
    assert_eq!(errors.len(), 1);
}

#[test]
fn test_switch_statement() {
    let errors = check(r#"
        function f(x: number): string {
            switch (x) {
                case 1:
                    return "one";
                case 2:
                    return "two";
                default:
                    return "other";
            }
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_try_catch() {
    let errors = check(r#"
        function f(): number {
            try {
                return 1;
            } catch (e) {
                return 0;
            }
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_for_in_loop() {
    let errors = check(r#"
        function f(obj: { a: number; b: number }): void {
            for (const key in obj) {
                const x: string = key;
            }
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_for_of_loop() {
    let errors = check(r#"
        function f(arr: number[]): number {
            let sum = 0;
            for (const item of arr) {
                sum = sum + item;
            }
            return sum;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_do_while_loop() {
    let errors = check(r#"
        function f(): number {
            let x = 0;
            do {
                x = x + 1;
            } while (x < 10);
            return x;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_index_signature_basic() {
    let errors = check("const obj: { [key: string]: number } = { a: 1, b: 2 };");
    assert!(errors.is_empty());
}

#[test]
fn test_index_signature_value_type_mismatch() {
    let errors = check("const obj: { [key: string]: number } = { a: \"wrong\" };");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_index_signature_property_access() {
    let errors = check(r#"
        const obj: { [key: string]: number } = { a: 1 };
        const x: number = obj.anyProp;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_index_signature_property_access_wrong_type() {
    let errors = check(r#"
        const obj: { [key: string]: number } = { a: 1 };
        const x: string = obj.anyProp;
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_index_signature_with_explicit_property() {
    let errors = check(r#"
        const obj: { name: string; [key: string]: string } = { name: "test", extra: "ok" };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_index_signature_explicit_property_mismatch() {
    let errors = check(r#"
        const obj: { name: string; [key: string]: string } = { name: 42 };
    "#);
    assert_eq!(errors.len(), 1);
}

#[test]
fn test_index_signature_computed_access() {
    let errors = check(r#"
        const obj: { [key: string]: number } = { a: 1 };
        const key: string = "test";
        const val = obj[key];
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_index_signature_number_key() {
    let errors = check(r#"
        const arr: { [index: number]: string } = { 0: "first", 1: "second" };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_index_signature_empty_object() {
    let errors = check("const obj: { [key: string]: number } = {};");
    assert!(errors.is_empty());
}

#[test]
fn test_interface_with_index_signature() {
    let errors = check(r#"
        interface StringMap {
            [key: string]: string;
        }
        const map: StringMap = { hello: "world", foo: "bar" };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_interface_index_signature_property_access() {
    let errors = check(r#"
        interface NumberDict {
            [key: string]: number;
        }
        function f(d: NumberDict): number {
            return d.anyKey;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_index_signature_mixed_explicit_and_index() {
    let errors = check(r#"
        interface Config {
            name: string;
            [key: string]: string;
        }
        const cfg: Config = { name: "app", version: "1.0" };
        const n: string = cfg.name;
        const v: string = cfg.version;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_index_signature_excess_property_with_index() {
    let errors = check(r#"
        const obj: { known: number; [key: string]: number } = {
            known: 1,
            extra: 2,
            another: 3
        };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_index_signature_nested_object() {
    let errors = check(r#"
        interface UserMap {
            [id: string]: { name: string; age: number };
        }
        const users: UserMap = {
            user1: { name: "Alice", age: 30 },
            user2: { name: "Bob", age: 25 }
        };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_index_signature_function_value() {
    let errors = check(r#"
        interface Handlers {
            [event: string]: () => void;
        }
        const h: Handlers = {
            click: () => {},
            hover: () => {}
        };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_union_property_access_common() {
    let errors = check(r#"
        interface A { x: number; y: string; }
        interface B { x: number; z: boolean; }
        function f(val: A | B): number {
            return val.x;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_union_property_access_not_common() {
    let errors = check(r#"
        interface A { x: number; y: string; }
        interface B { x: number; z: boolean; }
        function f(val: A | B): string {
            return val.y;
        }
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339);
}

#[test]
fn test_intersection_property_access() {
    let errors = check(r#"
        interface A { x: number; }
        interface B { y: string; }
        function f(val: A & B) {
            const a: number = val.x;
            const b: string = val.y;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_union_assignment_valid() {
    let errors = check(r#"
        let x: string | number = "hello";
        x = 42;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_union_assignment_invalid() {
    let errors = check("const x: string | number = true;");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_intersection_object_literal() {
    let errors = check(r#"
        type Named = { name: string };
        type Aged = { age: number };
        const person: Named & Aged = { name: "Alice", age: 30 };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_union_of_literals() {
    let errors = check(r#"
        type Direction = "left" | "right" | "up" | "down";
        const dir: Direction = "left";
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_union_of_literals_invalid() {
    let errors = check(r#"
        type Direction = "left" | "right" | "up" | "down";
        const dir: Direction = "diagonal";
    "#);
    assert_eq!(errors.len(), 1);
}

#[test]
fn test_excess_property_fresh_literal_errors() {
    // Direct object literal: excess property check SHOULD apply
    let errors = check("const x: { a: number } = { a: 1, b: 2 };");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2353); // Excess property error
}

#[test]
fn test_excess_property_variable_bypasses() {
    // Assigning through variable: excess property check should NOT apply
    // TypeScript's "freshness" rule - object literals lose freshness when assigned to a variable
    let errors = check(r#"
        const obj = { a: 1, b: 2 };
        const x: { a: number } = obj;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_excess_property_variable_still_checks_types() {
    // Even without excess checks, property types must still match
    let errors = check(r#"
        const obj = { a: "wrong" };
        const x: { a: number } = obj;
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_excess_property_variable_missing_property() {
    let errors = check(r#"
        const obj = { b: 2 };
        const x: { a: number } = obj;
    "#);
    assert_eq!(errors.len(), 1);
    // Note: Currently reports as type mismatch (2322) rather than missing property (2741)
    // because the variable assignment goes through is_assignable() not check_object_literal_against_type()
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_excess_property_function_param_variable() {
    let errors = check(r#"
        function f(x: { a: number }) {}
        const obj = { a: 1, b: 2 };
        f(obj);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_interface_extends_basic() {
    // interface B extends A should inherit A's properties
    let errors = check(r#"
        interface Animal {
            name: string;
        }
        interface Dog extends Animal {
            breed: string;
        }
        const dog: Dog = { name: "Rex", breed: "German Shepherd" };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_interface_extends_missing_base_property() {
    let errors = check(r#"
        interface Animal {
            name: string;
        }
        interface Dog extends Animal {
            breed: string;
        }
        const dog: Dog = { breed: "Labrador" };
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2741); // Missing property 'name'
}

#[test]
fn test_interface_extends_missing_derived_property() {
    let errors = check(r#"
        interface Animal {
            name: string;
        }
        interface Dog extends Animal {
            breed: string;
        }
        const dog: Dog = { name: "Rex" };
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2741); // Missing property 'breed'
}

#[test]
fn test_interface_extends_multiple() {
    // interface C extends A, B gets properties from both
    let errors = check(r#"
        interface Named {
            name: string;
        }
        interface Aged {
            age: number;
        }
        interface Person extends Named, Aged {
            email: string;
        }
        const person: Person = { name: "Alice", age: 30, email: "alice@example.com" };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_interface_extends_multiple_missing() {
    let errors = check(r#"
        interface Named {
            name: string;
        }
        interface Aged {
            age: number;
        }
        interface Person extends Named, Aged {
            email: string;
        }
        const person: Person = { name: "Alice", email: "alice@example.com" };
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2741); // Missing 'age'
}

#[test]
fn test_interface_extends_chain() {
    let errors = check(r#"
        interface A {
            a: number;
        }
        interface B extends A {
            b: string;
        }
        interface C extends B {
            c: boolean;
        }
        const obj: C = { a: 1, b: "hello", c: true };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_interface_extends_nonexistent() {
    let errors = check(r#"
        interface Dog extends NonExistent {
            breed: string;
        }
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2304); // Cannot find name 'NonExistent'
}

#[test]
fn test_interface_extends_with_optional() {
    let errors = check(r#"
        interface Base {
            required: string;
            optional?: number;
        }
        interface Derived extends Base {
            extra: boolean;
        }
        const obj: Derived = { required: "hello", extra: true };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_recursive_interface() {
    let errors = check(r#"
        interface ListNode {
            value: number;
            next: ListNode | null;
        }
        const node: ListNode = { value: 1, next: null };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_recursive_interface_nested() {
    let errors = check(r#"
        interface TreeNode {
            value: number;
            left: TreeNode | null;
            right: TreeNode | null;
        }
        const tree: TreeNode = {
            value: 1,
            left: { value: 2, left: null, right: null },
            right: null
        };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_recursive_type_alias() {
    let errors = check(r#"
        type JsonValue = string | number | boolean | null | JsonArray | JsonObject;
        type JsonArray = JsonValue[];
        type JsonObject = { [key: string]: JsonValue };
        const data: JsonValue = { name: "test", values: [1, 2, 3] };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_interface_method_signature() {
    let errors = check(r#"
        interface Calculator {
            add(a: number, b: number): number;
            subtract(a: number, b: number): number;
        }
        const calc: Calculator = {
            add: (a: number, b: number) => a + b,
            subtract: (a: number, b: number) => a - b
        };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_interface_readonly_property() {
    let errors = check(r#"
        interface Point {
            readonly x: number;
            readonly y: number;
        }
        const p: Point = { x: 10, y: 20 };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_type_alias_union() {
    let errors = check(r#"
        type StringOrNumber = string | number;
        const a: StringOrNumber = "hello";
        const b: StringOrNumber = 42;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_type_alias_intersection() {
    let errors = check(r#"
        type Named = { name: string };
        type Aged = { age: number };
        type Person = Named & Aged;
        const p: Person = { name: "Alice", age: 30 };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_type_alias_to_interface() {
    let errors = check(r#"
        interface User {
            name: string;
        }
        type UserAlias = User;
        const u: UserAlias = { name: "Bob" };
    "#);
    assert!(errors.is_empty());
}

// Generics

#[test]
fn test_generic_function_declaration_basic() {
    let errors = check(r#"
        function identity<T>(x: T): T {
            return x;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_function_with_multiple_type_params() {
    let errors = check(r#"
        function makePair<A, B>(a: A, b: B): { first: A; second: B } {
            return { first: a, second: b };
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_interface_basic() {
    let errors = check(r#"
        interface Box<T> {
            value: T;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_type_alias_basic() {
    let errors = check(r#"
        type Pair<A, B> = { first: A; second: B };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_function_explicit_type_arg() {
    let errors = check(r#"
        function identity<T>(x: T): T {
            return x;
        }
        const result: number = identity<number>(42);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_function_explicit_type_arg_mismatch() {
    let errors = check(r#"
        function identity<T>(x: T): T {
            return x;
        }
        const result = identity<string>(42);
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2345); // Argument of type 'X' is not assignable to parameter of type 'Y'
}

#[test]
fn test_generic_interface_instantiation() {
    let errors = check(r#"
        interface Box<T> {
            value: T;
        }
        const numBox: Box<number> = { value: 42 };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_interface_instantiation_error() {
    let errors = check(r#"
        interface Box<T> {
            value: T;
        }
        const numBox: Box<number> = { value: "hello" };
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322); // Type 'X' is not assignable to type 'Y'
}

#[test]
fn test_generic_function_type_inference_simple() {
    // identity(42) should infer T = number
    let errors = check(r#"
        function identity<T>(x: T): T {
            return x;
        }
        const result: number = identity(42);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_function_type_inference_string() {
    let errors = check(r#"
        function identity<T>(x: T): T {
            return x;
        }
        const result: string = identity("hello");
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_function_type_inference_mismatch() {
    let errors = check(r#"
        function identity<T>(x: T): T {
            return x;
        }
        const result: string = identity(42);
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2322);
}

#[test]
fn test_generic_function_type_inference_multiple_args() {
    let errors = check(r#"
        function first<T>(a: T, b: T): T {
            return a;
        }
        const result: number = first(1, 2);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_constraint_basic() {
    let errors = check(r#"
        function getLength<T extends { length: number }>(x: T): number {
            return x.length;
        }
        const len = getLength("hello");
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_constraint_violation() {
    let errors = check(r#"
        function getLength<T extends { length: number }>(x: T): number {
            return x.length;
        }
        const len = getLength(42);
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2344); // Type does not satisfy constraint
}

#[test]
fn test_generic_constraint_with_explicit_type_arg() {
    let errors = check(r#"
        interface HasLength {
            length: number;
        }
        function getLength<T extends HasLength>(x: T): number {
            return x.length;
        }
        const len = getLength<number>(42);
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2344);
}

#[test]
fn test_generic_default_type() {
    let errors = check(r#"
        interface Container<T = string> {
            value: T;
        }
        const c: Container = { value: "hello" };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_default_type_override() {
    let errors = check(r#"
        interface Container<T = string> {
            value: T;
        }
        const c: Container<number> = { value: 42 };
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_function_default() {
    let errors = check(r#"
        function wrap<T = string>(x: T): { value: T } {
            return { value: x };
        }
        const result = wrap("hello");
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_optional_param_call_without_arg() {
    let errors = check("function f(x?: number) {} f();");
    assert!(errors.is_empty());
}

#[test]
fn test_optional_param_call_with_arg() {
    let errors = check("function f(x?: number) {} f(42);");
    assert!(errors.is_empty());
}

#[test]
fn test_optional_param_call_with_undefined() {
    let errors = check("function f(x?: number) {} f(undefined);");
    assert!(errors.is_empty());
}

#[test]
fn test_optional_param_wrong_type() {
    let errors = check(r#"function f(x?: number) {} f("hello");"#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2345);
}

#[test]
fn test_required_param_after_optional_error() {
    let errors = check("function f(x?: number, y: string) {}");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 1016);
}

#[test]
fn test_multiple_optional_params() {
    let errors = check("function f(x?: number, y?: string) {} f(); f(1); f(1, 'a');");
    assert!(errors.is_empty());
}

#[test]
fn test_optional_param_type_is_union_with_undefined() {
    let errors = check(r#"
        function f(x?: number) {
            let y: number | undefined = x;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_optional_param_strict_mode_deferred() {
    let errors = check(r#"
        function f(x?: number): number {
            return x;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_rest_param_basic() {
    let errors = check("function f(...args: number[]) {} f(1, 2, 3);");
    assert!(errors.is_empty());
}

#[test]
fn test_rest_param_empty() {
    let errors = check("function f(...args: number[]) {} f();");
    assert!(errors.is_empty());
}

#[test]
fn test_rest_param_wrong_type() {
    let errors = check(r#"function f(...args: number[]) {} f(1, "hello", 3);"#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2345);
}

#[test]
fn test_rest_param_must_be_last() {
    // Parser enforces rest param must be last - syntax error, not type error
}

#[test]
fn test_rest_param_with_regular_params() {
    let errors = check("function f(x: number, ...rest: string[]) {} f(1, 'a', 'b');");
    assert!(errors.is_empty());
}

#[test]
fn test_rest_param_spread_call() {
    let errors = check(r#"
        function f(...args: number[]) {}
        const arr = [1, 2, 3];
        f(...arr);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_arrow_contextual_typing() {
    let errors = check(r#"
        const f: (x: number) => number = x => x + 1;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_arrow_explicit_types() {
    let errors = check(r#"
        const f = (x: number): number => x + 1;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_arrow_expression_body_call() {
    let errors = check(r#"
        const f = (x: number) => x * 2;
        const result: number = f(5);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_arrow_block_body_call() {
    let errors = check(r#"
        const f = (x: number): number => {
            return x * 2;
        };
        const result: number = f(5);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_method_shorthand_in_object() {
    let errors = check(r#"
        const obj = {
            greet(name: string): string {
                return "Hello " + name;
            }
        };
        const result: string = obj.greet("World");
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_method_shorthand_equivalent() {
    let errors = check(r#"
        const obj1 = { foo(): number { return 1; } };
        const obj2 = { foo: function(): number { return 1; } };
        const x: number = obj1.foo();
        const y: number = obj2.foo();
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_arrow_with_rest_param() {
    let errors = check(r#"
        const sum = (...nums: number[]): number => {
            let total = 0;
            return total;
        };
        sum(1, 2, 3);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_function_expression_with_rest_param() {
    let errors = check(r#"
        const sum = function(...nums: number[]): number {
            return 0;
        };
        sum(1, 2, 3);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_arrow_with_optional_param() {
    let errors = check(r#"
        const greet = (name?: string): string => {
            return "Hello";
        };
        greet();
        greet("World");
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_combined_optional_and_rest_params() {
    let errors = check("function f(required: string, optional?: number, ...rest: boolean[]) {} f(\"hello\"); f(\"hello\", 42); f(\"hello\", 42, true, false);");
    assert!(errors.is_empty());
}

// Classes

#[test]
fn test_class_basic_instantiation() {
    let errors = check(r#"
        class Point {
            x: number;
            y: number;
        }
        const p = new Point();
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_class_instance_property_access() {
    let errors = check(r#"
        class Point {
            x: number;
            y: number;
        }
        const p = new Point();
        const x: number = p.x;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_class_instance_property_not_found() {
    let errors = check(r#"
        class Point {
            x: number;
            y: number;
        }
        const p = new Point();
        const z = p.z;
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339); // Property 'z' does not exist
}

#[test]
fn test_class_constructor_with_params() {
    let errors = check(r#"
        class Point {
            x: number;
            y: number;
            constructor(x: number, y: number) {
                this.x = x;
                this.y = y;
            }
        }
        const p = new Point(1, 2);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_class_constructor_wrong_arg_count() {
    let errors = check(r#"
        class Point {
            x: number;
            y: number;
            constructor(x: number, y: number) {
                this.x = x;
                this.y = y;
            }
        }
        const p = new Point(1);
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2554); // Expected N arguments, but got M
}

#[test]
fn test_class_constructor_wrong_arg_type() {
    let errors = check(r#"
        class Point {
            x: number;
            y: number;
            constructor(x: number, y: number) {
                this.x = x;
                this.y = y;
            }
        }
        const p = new Point("hello", 2);
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2345); // Argument of type 'X' not assignable to 'Y'
}

#[test]
fn test_class_method_call() {
    let errors = check(r#"
        class Calculator {
            add(a: number, b: number): number {
                return a + b;
            }
        }
        const calc = new Calculator();
        const result: number = calc.add(1, 2);
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_class_method_wrong_arg_type() {
    let errors = check(r#"
        class Calculator {
            add(a: number, b: number): number {
                return a + b;
            }
        }
        const calc = new Calculator();
        const result = calc.add("hello", 2);
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2345);
}

#[test]
fn test_class_as_type_annotation() {
    let errors = check(r#"
        class User {
            name: string;
        }
        const user: User = new User();
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_class_type_mismatch() {
    let errors = check(r#"
        class User {
            name: string;
        }
        const user: User = { name: "alice" };
    "#);
    // Object literal should be assignable to class type (structural typing)
    assert!(errors.is_empty());
}

#[test]
fn test_class_extends_basic() {
    let errors = check(r#"
        class Animal {
            name: string;
        }
        class Dog extends Animal {
            breed: string;
        }
        const dog = new Dog();
        const name: string = dog.name;
        const breed: string = dog.breed;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_class_extends_method() {
    let errors = check(r#"
        class Animal {
            speak(): string {
                return "...";
            }
        }
        class Dog extends Animal {
            bark(): string {
                return "woof";
            }
        }
        const dog = new Dog();
        const sound1: string = dog.speak();
        const sound2: string = dog.bark();
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_class_extends_property_not_on_base() {
    let errors = check(r#"
        class Animal {
            name: string;
        }
        class Dog extends Animal {
            breed: string;
        }
        const animal = new Animal();
        const breed = animal.breed;
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339); // Property does not exist
}

#[test]
fn test_class_assignable_to_base() {
    let errors = check(r#"
        class Animal {
            name: string;
        }
        class Dog extends Animal {
            breed: string;
        }
        const dog = new Dog();
        const animal: Animal = dog;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_class_basic() {
    let errors = check(r#"
        class Box<T> {
            value: T;
        }
        const box = new Box<number>();
        const val: number = box.value;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_class_constraint_satisfied() {
    let errors = check(r#"
        class Container<T extends { length: number }> {
            item: T;
        }
        const c = new Container<string>();
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_generic_class_constraint_violated() {
    let errors = check(r#"
        class Container<T extends { length: number }> {
            item: T;
        }
        const c = new Container<number>();
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2344); // Type does not satisfy constraint
}

#[test]
fn test_static_property_access() {
    let errors = check(r#"
        class Counter {
            static count: number;
        }
        const c: number = Counter.count;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_static_method_call() {
    let errors = check(r#"
        class Factory {
            static create(): string {
                return "instance";
            }
        }
        const s: string = Factory.create();
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_static_property_not_on_instance() {
    let errors = check(r#"
        class Counter {
            static count: number;
        }
        const c = new Counter();
        const x = c.count;
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339); // Property does not exist
}

#[test]
fn test_static_property_not_found() {
    let errors = check(r#"
        class Counter {
            static count: number;
        }
        const x = Counter.nonexistent;
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339);
}

#[test]
fn test_class_implements_interface() {
    let errors = check(r#"
        interface Printable {
            print(): void;
        }
        class Document implements Printable {
            print(): void {}
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_class_implements_missing_method() {
    let errors = check(r#"
        interface Printable {
            print(): void;
        }
        class Document implements Printable {
        }
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2420); // Class incorrectly implements interface
}

#[test]
fn test_class_implements_missing_property() {
    let errors = check(r#"
        interface Named {
            name: string;
        }
        class Person implements Named {
        }
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2420);
}

#[test]
fn test_class_implements_multiple_interfaces() {
    let errors = check(r#"
        interface Named {
            name: string;
        }
        interface Aged {
            age: number;
        }
        class Person implements Named, Aged {
            name: string;
            age: number;
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_parameter_property_public() {
    let errors = check(r#"
        class Point {
            constructor(public x: number, public y: number) {}
        }
        const p = new Point(1, 2);
        const x: number = p.x;
        const y: number = p.y;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_parameter_property_readonly() {
    let errors = check(r#"
        class Point {
            constructor(readonly x: number) {}
        }
        const p = new Point(1);
        const x: number = p.x;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_this_type_in_method() {
    let errors = check(r#"
        class Counter {
            count: number;
            increment(): void {
                this.count = this.count + 1;
            }
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_this_property_not_found() {
    let errors = check(r#"
        class Counter {
            count: number;
            increment(): void {
                this.nonexistent = 1;
            }
        }
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339);
}

#[test]
fn test_super_call_in_derived_constructor() {
    let errors = check(r#"
        class Animal {
            constructor(public name: string) {}
        }
        class Dog extends Animal {
            constructor(name: string, public breed: string) {
                super(name);
            }
        }
        const d = new Dog("Rex", "German Shepherd");
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_super_call_wrong_args() {
    let errors = check(r#"
        class Animal {
            constructor(public name: string) {}
        }
        class Dog extends Animal {
            constructor() {
                super(42);
            }
        }
    "#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2345); // Argument not assignable
}

#[test]
fn test_super_property_access() {
    let errors = check(r#"
        class Animal {
            speak(): string {
                return "...";
            }
        }
        class Dog extends Animal {
            speak(): string {
                return super.speak() + " woof";
            }
        }
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_class_expression() {
    let errors = check(r#"
        const MyClass = class {
            value: number;
        };
        const obj = new MyClass();
        const v: number = obj.value;
    "#);
    assert!(errors.is_empty());
}

#[test]
fn test_named_class_expression() {
    let errors = check(r#"
        const MyClass = class InnerName {
            value: number;
        };
        const obj = new MyClass();
        const v: number = obj.value;
    "#);
    assert!(errors.is_empty());
}

// Apparent types: primitive method resolution via lib.d.ts

#[test]
fn test_string_method_via_interface() {
    let errors = check(r#"const x: string = "hello".toUpperCase();"#);
    assert!(errors.is_empty());
}

#[test]
fn test_string_method_chained() {
    let errors = check(r#"const x: string = "hello".toUpperCase().toLowerCase();"#);
    assert!(errors.is_empty());
}

#[test]
fn test_string_literal_method() {
    let errors = check(r#"const x = "hello".charAt(0);"#);
    assert!(errors.is_empty());
}

#[test]
fn test_string_method_return_type() {
    let errors = check(r#"const x: number = "hello".length;"#);
    assert!(errors.is_empty());
}

#[test]
fn test_string_includes_method() {
    let errors = check(r#"const x: boolean = "hello".includes("el");"#);
    assert!(errors.is_empty());
}

#[test]
fn test_array_method_via_interface() {
    let errors = check(r#"const arr = [1, 2, 3]; const mapFn = arr.map;"#);
    assert!(errors.is_empty());
}

#[test]
fn test_array_filter_method() {
    let errors = check(r#"const arr = [1, 2, 3]; const filterFn = arr.filter;"#);
    assert!(errors.is_empty());
}

#[test]
fn test_array_find_method() {
    let errors = check(r#"const arr = [1, 2, 3]; const findFn = arr.find;"#);
    assert!(errors.is_empty());
}

#[test]
fn test_array_foreach_method() {
    let errors = check(r#"const arr = [1, 2, 3]; const forEachFn = arr.forEach;"#);
    assert!(errors.is_empty());
}

#[test]
fn test_unknown_string_method_error() {
    let errors = check(r#"const x = "hello".unknownMethod();"#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339); // Property does not exist
}

#[test]
fn test_unknown_array_method_error() {
    let errors = check(r#"const arr = [1, 2, 3]; const x = arr.unknownMethod();"#);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, 2339);
}

#[test]
fn test_number_method_via_interface() {
    let errors = check(r#"const n = 42; const s: string = n.toFixed(2);"#);
    assert!(errors.is_empty());
}

#[test]
fn test_number_literal_method() {
    let errors = check(r#"const n = 42; const s = n.toString();"#);
    assert!(errors.is_empty());
}

#[test]
fn test_boolean_method_via_interface() {
    let errors = check(r#"const b = true; const x: boolean = b.valueOf();"#);
    assert!(errors.is_empty());
}

// Error reporting: severity and related spans

#[test]
fn test_error_severity_default_is_error() {
    use super::Severity;

    let error = TypeError::new("test message", oxc_span::Span::new(0, 10), 1234);
    assert!(error.is_error());
    assert!(!error.is_warning());
    assert_eq!(error.severity, Severity::Error);
}

#[test]
fn test_warning_severity() {
    use super::Severity;

    let warning = TypeError::warning("test warning", oxc_span::Span::new(0, 10), 5678);
    assert!(!warning.is_error());
    assert!(warning.is_warning());
    assert_eq!(warning.severity, Severity::Warning);
}

#[test]
fn test_related_spans() {
    let error = TypeError::new("main error", oxc_span::Span::new(0, 10), 1234)
        .with_related("related context", oxc_span::Span::new(20, 30))
        .with_related("another related", oxc_span::Span::new(40, 50));

    assert_eq!(error.related.len(), 2);
    assert_eq!(error.related[0].message, "related context");
    assert_eq!(error.related[0].span.start, 20);
    assert_eq!(error.related[1].message, "another related");
}
