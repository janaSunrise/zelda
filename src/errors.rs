//! TypeScript error codes and messages.
//!
//! Reference: https://github.com/microsoft/TypeScript/blob/main/src/compiler/diagnosticMessages.json

#[derive(Debug, Clone, Copy)]
pub struct Diagnostic {
    pub code: u32,
    pub message: &'static str,
}

impl Diagnostic {
    pub const fn new(code: u32, message: &'static str) -> Self {
        Self { code, message }
    }

    pub fn format(&self, args: &[&str]) -> String {
        let mut result = self.message.to_string();
        for (i, arg) in args.iter().enumerate() {
            result = result.replace(&format!("{{{}}}", i), arg);
        }
        result
    }

    /// "TS{code}: {message}"
    pub fn format_full(&self, args: &[&str]) -> String {
        format!("TS{}: {}", self.code, self.format(args))
    }
}

// ============================================
// 1000-1999: Syntax Errors
// ============================================

pub const EXPECTED: Diagnostic = Diagnostic::new(1005, "'{0}' expected.");
pub const SUPER_IN_NON_DERIVED: Diagnostic =
    Diagnostic::new(1013, "'super' can only be referenced in a derived class.");
pub const REST_PARAM_MUST_BE_LAST: Diagnostic =
    Diagnostic::new(1014, "A rest parameter must be last in a parameter list.");
pub const REQUIRED_AFTER_OPTIONAL: Diagnostic = Diagnostic::new(
    1016,
    "A required parameter cannot follow an optional parameter.",
);
pub const CONTINUE_OUTSIDE_LOOP: Diagnostic = Diagnostic::new(
    1104,
    "A 'continue' statement can only be used within an enclosing iteration statement.",
);
pub const BREAK_OUTSIDE_LOOP: Diagnostic = Diagnostic::new(
    1105,
    "A 'break' statement can only be used within an enclosing iteration or switch statement.",
);
pub const RETURN_OUTSIDE_FUNCTION: Diagnostic = Diagnostic::new(
    1108,
    "A 'return' statement can only be used within a function body.",
);
pub const ABSTRACT_WITH_BODY: Diagnostic = Diagnostic::new(
    1245,
    "Method '{0}' cannot have an implementation because it is marked abstract.",
);

// ============================================
// 2300-2399: Declaration Errors
// ============================================

pub const DUPLICATE_IDENTIFIER: Diagnostic = Diagnostic::new(2300, "Duplicate identifier '{0}'.");
pub const CANNOT_FIND_NAME: Diagnostic = Diagnostic::new(2304, "Cannot find name '{0}'.");
pub const NO_EXPORTED_MEMBER: Diagnostic =
    Diagnostic::new(2305, "Module '{0}' has no exported member '{1}'.");
pub const CANNOT_FIND_MODULE: Diagnostic = Diagnostic::new(
    2307,
    "Cannot find module '{0}' or its corresponding type declarations.",
);
pub const NO_DEFAULT_EXPORT: Diagnostic =
    Diagnostic::new(2308, "Module '{0}' has no default export.");
pub const CIRCULAR_BASE_TYPE: Diagnostic = Diagnostic::new(
    2310,
    "Type '{0}' recursively references itself as a base type.",
);
pub const EXTENDS_NON_CLASS: Diagnostic =
    Diagnostic::new(2311, "A class may only extend another class.");
pub const IMPLEMENTS_NON_CLASS: Diagnostic = Diagnostic::new(
    2312,
    "A class may only implement another class or interface.",
);
pub const CIRCULAR_CONSTRAINT: Diagnostic =
    Diagnostic::new(2313, "Type parameter '{0}' has a circular constraint.");
pub const GENERIC_ARITY: Diagnostic =
    Diagnostic::new(2314, "Generic type '{0}' requires {1} type argument(s).");
pub const NOT_GENERIC: Diagnostic = Diagnostic::new(2315, "Type '{0}' is not generic.");

// ============================================
// 2320-2349: Type Compatibility Errors
// ============================================

pub const NOT_ASSIGNABLE: Diagnostic =
    Diagnostic::new(2322, "Type '{0}' is not assignable to type '{1}'.");
pub const PROPERTY_NOT_EXIST: Diagnostic =
    Diagnostic::new(2339, "Property '{0}' does not exist on type '{1}'.");
pub const PRIVATE_PROPERTY: Diagnostic = Diagnostic::new(
    2341,
    "Property '{0}' is private and only accessible within class '{1}'.",
);
pub const CONSTRAINT_NOT_SATISFIED: Diagnostic =
    Diagnostic::new(2344, "Type '{0}' does not satisfy the constraint '{1}'.");
pub const ARG_NOT_ASSIGNABLE: Diagnostic = Diagnostic::new(
    2345,
    "Argument of type '{0}' is not assignable to parameter of type '{1}'.",
);

// ============================================
// 2350-2399: Object/Function Type Errors
// ============================================

pub const NOT_CALLABLE: Diagnostic = Diagnostic::new(2349, "This expression is not callable.");
pub const TYPE_CONVERSION_MISTAKE: Diagnostic = Diagnostic::new(
    2352,
    "Conversion of type '{0}' to type '{1}' may be a mistake because neither type sufficiently overlaps with the other.",
);
pub const LEFT_ARITHMETIC_ANY_NUMBER: Diagnostic = Diagnostic::new(
    2362,
    "The left-hand side of an arithmetic operation must be of type 'any', 'number', 'bigint' or an enum type.",
);
pub const RIGHT_ARITHMETIC_ANY_NUMBER: Diagnostic = Diagnostic::new(
    2363,
    "The right-hand side of an arithmetic operation must be of type 'any', 'number', 'bigint' or an enum type.",
);
pub const OPERATOR_CANNOT_BE_APPLIED: Diagnostic = Diagnostic::new(
    2365,
    "Operator '{0}' cannot be applied to types '{1}' and '{2}'.",
);
pub const NOT_CONSTRUCTABLE: Diagnostic =
    Diagnostic::new(2351, "This expression is not constructable.");
pub const EXCESS_PROPERTY: Diagnostic = Diagnostic::new(
    2353,
    "Object literal may only specify known properties, and '{0}' does not exist in type '{1}'.",
);
pub const MUST_RETURN_VALUE: Diagnostic = Diagnostic::new(
    2355,
    "A function whose declared type is neither 'undefined', 'void', nor 'any' must return a value.",
);
pub const MISSING_RETURN: Diagnostic = Diagnostic::new(
    2366,
    "Function lacks ending return statement and return type does not include 'undefined'.",
);
pub const SUPER_BEFORE_THIS: Diagnostic = Diagnostic::new(
    2376,
    "'super' must be called before accessing 'this' in the constructor of a derived class.",
);
pub const DERIVED_NEEDS_SUPER: Diagnostic = Diagnostic::new(
    2377,
    "Constructors for derived classes must contain a 'super' call.",
);
pub const GETTER_MUST_RETURN: Diagnostic =
    Diagnostic::new(2378, "A 'get' accessor must return a value.");
pub const ACCESSOR_TYPE_MISMATCH: Diagnostic = Diagnostic::new(
    2380,
    "The return type of a 'get' accessor must be assignable to its 'set' accessor type.",
);
pub const OVERLOAD_NOT_COMPATIBLE: Diagnostic = Diagnostic::new(
    2394,
    "This overload signature is not compatible with its implementation signature.",
);

// ============================================
// 2400-2449: Class Errors
// ============================================

pub const INCORRECTLY_EXTENDS: Diagnostic =
    Diagnostic::new(2415, "Class '{0}' incorrectly extends base class '{1}'.");
pub const STATIC_INCORRECTLY_EXTENDS: Diagnostic = Diagnostic::new(
    2417,
    "Class static side '{0}' incorrectly extends base class static side '{1}'.",
);
pub const INCORRECTLY_IMPLEMENTS: Diagnostic =
    Diagnostic::new(2420, "Class '{0}' incorrectly implements interface '{1}'.");
pub const IMPLEMENTS_INVALID_TYPE: Diagnostic = Diagnostic::new(
    2422,
    "A class can only implement an object type or intersection of object types with statically known members.",
);
pub const PROTECTED_PROPERTY: Diagnostic = Diagnostic::new(
    2445,
    "Property '{0}' is protected and only accessible within class '{1}' and its subclasses.",
);
pub const USED_BEFORE_ASSIGNED: Diagnostic =
    Diagnostic::new(2454, "Variable '{0}' is used before being assigned.");

// ============================================
// 2500-2549: Function Call Errors
// ============================================

pub const ABSTRACT_INSTANTIATE: Diagnostic =
    Diagnostic::new(2511, "Cannot create an instance of an abstract class.");
pub const ABSTRACT_IN_NON_ABSTRACT: Diagnostic = Diagnostic::new(
    2512,
    "Abstract methods can only appear within an abstract class.",
);
pub const MISSING_ABSTRACT_MEMBER: Diagnostic = Diagnostic::new(
    2515,
    "Non-abstract class '{0}' does not implement inherited abstract member '{1}' from class '{2}'.",
);
pub const POSSIBLY_NULL: Diagnostic = Diagnostic::new(2531, "Object is possibly 'null'.");
pub const POSSIBLY_UNDEFINED: Diagnostic = Diagnostic::new(2532, "Object is possibly 'undefined'.");
pub const POSSIBLY_NULL_UNDEFINED: Diagnostic =
    Diagnostic::new(2533, "Object is possibly 'null' or 'undefined'.");
pub const INVALID_INDEX_TYPE: Diagnostic =
    Diagnostic::new(2538, "Type '{0}' cannot be used as an index type.");
pub const READONLY_INDEX: Diagnostic =
    Diagnostic::new(2542, "Index signature in type '{0}' only permits reading.");
pub const WRONG_ARG_COUNT: Diagnostic =
    Diagnostic::new(2554, "Expected {0} arguments, but got {1}.");
pub const WRONG_ARG_COUNT_RANGE: Diagnostic =
    Diagnostic::new(2555, "Expected {0}-{1} arguments, but got {2}.");
pub const TOO_FEW_ARGS: Diagnostic =
    Diagnostic::new(2556, "Expected at least {0} arguments, but got {1}.");
pub const NO_COMMON_PROPERTIES: Diagnostic = Diagnostic::new(
    2559,
    "Type '{0}' has no properties in common with type '{1}'.",
);
pub const PROPERTY_USED_BEFORE_ASSIGNED: Diagnostic = Diagnostic::new(
    2565,
    "Property '{0}' is used before being assigned in the constructor.",
);
pub const STATIC_MEMBER_SUGGESTION: Diagnostic = Diagnostic::new(
    2576,
    "Property '{0}' does not exist on type '{1}'. Did you mean to access the static member '{2}.{0}' instead?",
);

// ============================================
// 2550-2599: Suggestion Errors
// ============================================

pub const PROPERTY_NOT_EXIST_SUGGESTION: Diagnostic = Diagnostic::new(
    2551,
    "Property '{0}' does not exist on type '{1}'. Did you mean '{2}'?",
);
pub const CANNOT_FIND_NAME_SUGGESTION: Diagnostic =
    Diagnostic::new(2552, "Cannot find name '{0}'. Did you mean '{1}'?");

// ============================================
// 2600-2699: This/Context Errors
// ============================================

pub const IMPLICIT_ANY_THIS: Diagnostic = Diagnostic::new(
    2683,
    "'this' implicitly has type 'any' because it does not have a type annotation.",
);
pub const TYPE_USED_AS_VALUE: Diagnostic = Diagnostic::new(
    2693,
    "'{0}' only refers to a type, but is being used as a value here.",
);
pub const VALUE_USED_AS_TYPE: Diagnostic = Diagnostic::new(
    2749,
    "'{0}' refers to a value, but is being used as a type here. Did you mean 'typeof {0}'?",
);
pub const THIS_NOT_ASSIGNABLE: Diagnostic = Diagnostic::new(
    2684,
    "The 'this' context of type '{0}' is not assignable to method's 'this' of type '{1}'.",
);

// ============================================
// 2700-2799: Null/Invoke Errors
// ============================================

pub const INVOKE_POSSIBLY_NULL: Diagnostic =
    Diagnostic::new(2721, "Cannot invoke an object which is possibly 'null'.");
pub const INVOKE_POSSIBLY_UNDEFINED: Diagnostic = Diagnostic::new(
    2722,
    "Cannot invoke an object which is possibly 'undefined'.",
);
pub const INVOKE_POSSIBLY_NULL_UNDEFINED: Diagnostic = Diagnostic::new(
    2723,
    "Cannot invoke an object which is possibly 'null' or 'undefined'.",
);
pub const MULTIPLE_PROPERTIES_MISSING: Diagnostic = Diagnostic::new(
    2739,
    "Type '{0}' is missing the following properties from type '{1}': {2}",
);
pub const MANY_PROPERTIES_MISSING: Diagnostic = Diagnostic::new(
    2740,
    "Type '{0}' is missing the following properties from type '{1}': {2}, and {3} more.",
);
pub const PROPERTY_MISSING: Diagnostic = Diagnostic::new(
    2741,
    "Property '{0}' is missing in type '{1}' but required in type '{2}'.",
);
pub const NO_OVERLOAD_MATCH: Diagnostic = Diagnostic::new(2769, "No overload matches this call.");
pub const LAST_OVERLOAD_ERROR: Diagnostic =
    Diagnostic::new(2770, "The last overload gave the following error.");

// ============================================
// 7000-7999: Strict Mode / Implicit Any
// ============================================

pub const IMPLICIT_ANY_VAR: Diagnostic =
    Diagnostic::new(7005, "Variable '{0}' implicitly has an 'any' type.");
pub const IMPLICIT_ANY_PARAM: Diagnostic =
    Diagnostic::new(7006, "Parameter '{0}' implicitly has an 'any' type.");
pub const IMPLICIT_ANY_MEMBER: Diagnostic =
    Diagnostic::new(7008, "Member '{0}' implicitly has an 'any' type.");
pub const IMPLICIT_ANY_RETURN: Diagnostic = Diagnostic::new(
    7010,
    "Function expression, which lacks return-type annotation, implicitly has an 'any' return type.",
);
pub const NO_INDEX_SIGNATURE: Diagnostic = Diagnostic::new(
    7017,
    "Element implicitly has an 'any' type because type '{0}' has no index signature.",
);
pub const IMPLICIT_ANY_SOME_LOCATIONS: Diagnostic = Diagnostic::new(
    7034,
    "Variable '{0}' implicitly has type 'any' in some locations where its type cannot be determined.",
);
pub const IMPLICIT_ANY_INDEX: Diagnostic = Diagnostic::new(
    7053,
    "Element implicitly has an 'any' type because expression of type '{0}' can't be used to index type '{1}'.",
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_no_args() {
        assert_eq!(NOT_CALLABLE.format(&[]), "This expression is not callable.");
    }

    #[test]
    fn test_format_one_arg() {
        assert_eq!(CANNOT_FIND_NAME.format(&["foo"]), "Cannot find name 'foo'.");
    }

    #[test]
    fn test_format_two_args() {
        assert_eq!(
            NOT_ASSIGNABLE.format(&["string", "number"]),
            "Type 'string' is not assignable to type 'number'."
        );
    }

    #[test]
    fn test_format_full() {
        assert_eq!(
            NOT_ASSIGNABLE.format_full(&["string", "number"]),
            "TS2322: Type 'string' is not assignable to type 'number'."
        );
    }

    #[test]
    fn test_diagnostic_code() {
        assert_eq!(DUPLICATE_IDENTIFIER.code, 2300);
        assert_eq!(CANNOT_FIND_NAME.code, 2304);
        assert_eq!(NOT_ASSIGNABLE.code, 2322);
    }
}
