use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    // Primitives
    String,
    Number,
    Boolean,
    Null,
    Undefined,
    Void,
    Any,
    Unknown,
    Never,

    // Literals
    StringLiteral(String),
    NumberLiteral(f64),
    BooleanLiteral(bool),

    // Compound types
    Array(Box<Type>),
    Tuple(Vec<Type>),
    Union(Vec<Type>),
    Intersection(Vec<Type>),

    // Structural types
    Object {
        properties: Vec<Property>,
        index_signature: Option<IndexSignature>,
    },
    Function {
        params: Vec<Param>,
        return_type: Box<Type>,
        type_params: Vec<TypeParam>,
    },

    // Named type reference with `type`: Array<string>, Map<K,V>, User, etc.
    TypeRef {
        name: String,
        type_args: Vec<Type>,
    },

    // Type parameter in a generic definition: the T in <T extends Foo>
    TypeParameter {
        name: String,
        constraint: Option<Box<Type>>,
        default: Option<Box<Type>>,
    },
}

/// We can't derive `Eq` because `f64` doesn't implement it (NaN != NaN violates reflexivity).
/// This is safe for our use case since TypeScript number literals are always concrete values
/// and we never construct NaN literals in the type system.
impl Eq for Type {}

/// We can't derive `Hash` because `f64` doesn't implement it. Different bit patterns
/// (like +0.0 vs -0.0) would need special handling, and NaN has no meaningful hash.
/// We solve this by converting f64 to its raw bits via `to_bits()`, giving us a
/// consistent u64 that we can hash normally.
impl Hash for Type {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Type::String | Type::Number | Type::Boolean | Type::Null
            | Type::Undefined | Type::Void | Type::Any | Type::Unknown | Type::Never => {}
            Type::StringLiteral(s) => s.hash(state),
            Type::NumberLiteral(n) => n.to_bits().hash(state),
            Type::BooleanLiteral(b) => b.hash(state),
            Type::Array(elem) => elem.hash(state),
            Type::Tuple(types) => types.hash(state),
            Type::Union(types) => types.hash(state),
            Type::Intersection(types) => types.hash(state),
            Type::Object { properties, index_signature } => {
                properties.hash(state);
                index_signature.hash(state);
            }
            Type::Function { params, return_type, type_params } => {
                params.hash(state);
                return_type.hash(state);
                type_params.hash(state);
            }
            Type::TypeRef { name, type_args } => {
                name.hash(state);
                type_args.hash(state);
            }
            Type::TypeParameter { name, constraint, default } => {
                name.hash(state);
                constraint.hash(state);
                default.hash(state);
            }
        }
    }
}

/// We can't derive `Ord` because `f64` only implements `PartialOrd`. NaN is unordered
/// (NaN < x, NaN > x, and NaN == x are all false). We use `total_cmp()` which defines
/// a total ordering: -NaN < -∞ < ... < -0 < +0 < ... < +∞ < +NaN.
impl PartialOrd for Type {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// See `PartialOrd` above, this uses `total_cmp()` for the `NumberLiteral` variant.
impl Ord for Type {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_disc = std::mem::discriminant(self);
        let other_disc = std::mem::discriminant(other);

        // First compare by discriminant
        match format!("{:?}", self_disc).cmp(&format!("{:?}", other_disc)) {
            Ordering::Equal => {}
            ord => return ord,
        }

        // Then compare by content
        match (self, other) {
            (Type::String, Type::String) => Ordering::Equal,
            (Type::Number, Type::Number) => Ordering::Equal,
            (Type::Boolean, Type::Boolean) => Ordering::Equal,
            (Type::Null, Type::Null) => Ordering::Equal,
            (Type::Undefined, Type::Undefined) => Ordering::Equal,
            (Type::Void, Type::Void) => Ordering::Equal,
            (Type::Any, Type::Any) => Ordering::Equal,
            (Type::Unknown, Type::Unknown) => Ordering::Equal,
            (Type::Never, Type::Never) => Ordering::Equal,
            (Type::StringLiteral(a), Type::StringLiteral(b)) => a.cmp(b),
            (Type::NumberLiteral(a), Type::NumberLiteral(b)) => a.total_cmp(b),
            (Type::BooleanLiteral(a), Type::BooleanLiteral(b)) => a.cmp(b),
            (Type::Array(a), Type::Array(b)) => a.cmp(b),
            (Type::Tuple(a), Type::Tuple(b)) => a.cmp(b),
            (Type::Union(a), Type::Union(b)) => a.cmp(b),
            (Type::Intersection(a), Type::Intersection(b)) => a.cmp(b),
            (Type::Object { properties: pa, index_signature: ia },
             Type::Object { properties: pb, index_signature: ib }) => {
                pa.cmp(pb).then_with(|| ia.cmp(ib))
            }
            (Type::Function { params: pa, return_type: ra, type_params: ta },
             Type::Function { params: pb, return_type: rb, type_params: tb }) => {
                pa.cmp(pb).then_with(|| ra.cmp(rb)).then_with(|| ta.cmp(tb))
            }
            (Type::TypeRef { name: na, type_args: aa },
             Type::TypeRef { name: nb, type_args: ab }) => {
                na.cmp(nb).then_with(|| aa.cmp(ab))
            }
            (Type::TypeParameter { name: na, constraint: ca, default: da },
             Type::TypeParameter { name: nb, constraint: cb, default: db }) => {
                na.cmp(nb).then_with(|| ca.cmp(cb)).then_with(|| da.cmp(db))
            }
            _ => Ordering::Equal, // Same discriminant, shouldn't happen
        }
    }
}

impl Type {
    pub fn object(properties: Vec<Property>) -> Self {
        Self::Object {
            properties,
            index_signature: None,
        }
    }

    pub fn function(params: Vec<Param>, return_type: Type) -> Self {
        Self::Function {
            params,
            return_type: Box::new(return_type),
            type_params: Vec::new(),
        }
    }

    pub fn generic_function(
        type_params: Vec<TypeParam>,
        params: Vec<Param>,
        return_type: Type,
    ) -> Self {
        Self::Function {
            params,
            return_type: Box::new(return_type),
            type_params,
        }
    }

    pub fn array(element_type: Type) -> Self {
        Self::Array(Box::new(element_type))
    }

    pub fn union(types: Vec<Type>) -> Self {
        Self::Union(types)
    }

    pub fn intersection(types: Vec<Type>) -> Self {
        Self::Intersection(types)
    }

    pub fn type_ref(name: impl Into<String>, type_args: Vec<Type>) -> Self {
        Self::TypeRef {
            name: name.into(),
            type_args,
        }
    }

    pub fn is_primitive(&self) -> bool {
        matches!(
            self,
            Type::String | Type::Number | Type::Boolean | Type::Null | Type::Undefined | Type::Void
        )
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::String => write!(f, "string"),
            Type::Number => write!(f, "number"),
            Type::Boolean => write!(f, "boolean"),
            Type::Null => write!(f, "null"),
            Type::Undefined => write!(f, "undefined"),
            Type::Void => write!(f, "void"),
            Type::Any => write!(f, "any"),
            Type::Unknown => write!(f, "unknown"),
            Type::Never => write!(f, "never"),

            Type::StringLiteral(s) => write!(f, "\"{}\"", s),
            Type::NumberLiteral(n) => write!(f, "{}", n),
            Type::BooleanLiteral(b) => write!(f, "{}", b),

            Type::Array(elem) => write!(f, "{}[]", elem),
            Type::Tuple(types) => {
                write!(f, "[")?;
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", ty)?;
                }
                write!(f, "]")
            }
            Type::Union(types) => {
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    // Wrap function types: ((x: number) => void) | string
                    if matches!(ty, Type::Function { .. }) {
                        write!(f, "({})", ty)?;
                    } else {
                        write!(f, "{}", ty)?;
                    }
                }
                Ok(())
            }
            Type::Intersection(types) => {
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, " & ")?;
                    }
                    // Wrap unions and functions with parentheses
                    if matches!(ty, Type::Union(_) | Type::Function { .. }) {
                        write!(f, "({})", ty)?;
                    } else {
                        write!(f, "{}", ty)?;
                    }
                }
                Ok(())
            }

            Type::Object {
                properties,
                index_signature,
            } => {
                write!(f, "{{ ")?;
                let mut first = true;
                for prop in properties {
                    if !first {
                        write!(f, "; ")?;
                    }
                    first = false;
                    if prop.readonly {
                        write!(f, "readonly ")?;
                    }
                    write!(f, "{}", prop.name)?;
                    if prop.optional {
                        write!(f, "?")?;
                    }
                    write!(f, ": {}", prop.ty)?;
                }
                if let Some(idx) = index_signature {
                    if !first {
                        write!(f, "; ")?;
                    }
                    write!(f, "[key: {}]: {}", idx.key_type, idx.value_type)?;
                }
                write!(f, " }}")
            }

            Type::Function {
                params,
                return_type,
                type_params,
            } => {
                if !type_params.is_empty() {
                    write!(f, "<")?;
                    for (i, tp) in type_params.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", tp.name)?;
                        if let Some(constraint) = &tp.constraint {
                            write!(f, " extends {}", constraint)?;
                        }
                        if let Some(default) = &tp.default {
                            write!(f, " = {}", default)?;
                        }
                    }
                    write!(f, ">")?;
                }
                write!(f, "(")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if param.rest {
                        write!(f, "...")?;
                    }
                    write!(f, "{}", param.name)?;
                    if param.optional {
                        write!(f, "?")?;
                    }
                    write!(f, ": {}", param.ty)?;
                }
                write!(f, ") => {}", return_type)
            }

            Type::TypeRef { name, type_args } => {
                write!(f, "{}", name)?;
                if !type_args.is_empty() {
                    write!(f, "<")?;
                    for (i, arg) in type_args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", arg)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }

            Type::TypeParameter {
                name,
                constraint,
                default,
            } => {
                write!(f, "{}", name)?;
                if let Some(constraint) = constraint {
                    write!(f, " extends {}", constraint)?;
                }
                if let Some(default) = default {
                    write!(f, " = {}", default)?;
                }
                Ok(())
            }
        }
    }
}

/// Property in an object type: { name: string, age?: number }
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Property {
    pub name: String,
    pub ty: Type,
    pub optional: bool,
    pub readonly: bool,
}

impl Property {
    pub fn new(name: impl Into<String>, ty: Type) -> Self {
        Self {
            name: name.into(),
            ty,
            optional: false,
            readonly: false,
        }
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub fn readonly(mut self) -> Self {
        self.readonly = true;
        self
    }
}

/// Parameter in a function type: (x: number, ...rest: string[]) => void
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub optional: bool,
    pub rest: bool,
}

impl Param {
    pub fn new(name: impl Into<String>, ty: Type) -> Self {
        Self {
            name: name.into(),
            ty,
            optional: false,
            rest: false,
        }
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub fn rest(mut self) -> Self {
        self.rest = true;
        self
    }
}

/// Type parameter in a generic: <T extends Constraint = Default>
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeParam {
    pub name: String,
    pub constraint: Option<Box<Type>>,
    pub default: Option<Box<Type>>,
}

impl TypeParam {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            constraint: None,
            default: None,
        }
    }

    pub fn with_constraint(mut self, constraint: Type) -> Self {
        self.constraint = Some(Box::new(constraint));
        self
    }

    pub fn with_default(mut self, default: Type) -> Self {
        self.default = Some(Box::new(default));
        self
    }
}

/// Index signature: { [key: string]: number }
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IndexSignature {
    pub key_type: Box<Type>,
    pub value_type: Box<Type>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitive_display() {
        assert_eq!(Type::String.to_string(), "string");
        assert_eq!(Type::Number.to_string(), "number");
        assert_eq!(Type::Boolean.to_string(), "boolean");
        assert_eq!(Type::Null.to_string(), "null");
        assert_eq!(Type::Undefined.to_string(), "undefined");
        assert_eq!(Type::Void.to_string(), "void");
        assert_eq!(Type::Any.to_string(), "any");
        assert_eq!(Type::Unknown.to_string(), "unknown");
        assert_eq!(Type::Never.to_string(), "never");
    }

    #[test]
    fn test_literal_display() {
        assert_eq!(Type::StringLiteral("hello".into()).to_string(), "\"hello\"");
        assert_eq!(Type::NumberLiteral(42.0).to_string(), "42");
        assert_eq!(Type::BooleanLiteral(true).to_string(), "true");
    }

    #[test]
    fn test_array_display() {
        assert_eq!(Type::array(Type::String).to_string(), "string[]");
        assert_eq!(Type::array(Type::Number).to_string(), "number[]");
        assert_eq!(
            Type::array(Type::array(Type::String)).to_string(),
            "string[][]"
        );
    }

    #[test]
    fn test_tuple_display() {
        assert_eq!(
            Type::Tuple(vec![Type::String, Type::Number]).to_string(),
            "[string, number]"
        );
    }

    #[test]
    fn test_union_display() {
        assert_eq!(
            Type::union(vec![Type::String, Type::Number]).to_string(),
            "string | number"
        );
        assert_eq!(
            Type::union(vec![Type::String, Type::Null, Type::Undefined]).to_string(),
            "string | null | undefined"
        );
    }

    #[test]
    fn test_intersection_display() {
        let a = Type::object(vec![Property::new("a", Type::String)]);
        let b = Type::object(vec![Property::new("b", Type::Number)]);
        assert_eq!(
            Type::intersection(vec![a, b]).to_string(),
            "{ a: string } & { b: number }"
        );
    }

    #[test]
    fn test_object_display() {
        let obj = Type::object(vec![
            Property::new("name", Type::String),
            Property::new("age", Type::Number).optional(),
        ]);
        assert_eq!(obj.to_string(), "{ name: string; age?: number }");

        let readonly_obj = Type::object(vec![Property::new("id", Type::Number).readonly()]);
        assert_eq!(readonly_obj.to_string(), "{ readonly id: number }");
    }

    #[test]
    fn test_function_display() {
        let func = Type::function(
            vec![Param::new("x", Type::Number), Param::new("y", Type::Number)],
            Type::Number,
        );
        assert_eq!(func.to_string(), "(x: number, y: number) => number");

        let func_optional = Type::function(
            vec![
                Param::new("x", Type::Number),
                Param::new("y", Type::Number).optional(),
            ],
            Type::Number,
        );
        assert_eq!(
            func_optional.to_string(),
            "(x: number, y?: number) => number"
        );

        let func_rest = Type::function(
            vec![Param::new("args", Type::array(Type::Number)).rest()],
            Type::Number,
        );
        assert_eq!(func_rest.to_string(), "(...args: number[]) => number");
    }

    #[test]
    fn test_generic_function_display() {
        let func = Type::generic_function(
            vec![TypeParam::new("T")],
            vec![Param::new("x", Type::type_ref("T", vec![]))],
            Type::type_ref("T", vec![]),
        );
        assert_eq!(func.to_string(), "<T>(x: T) => T");

        let func_constrained = Type::generic_function(
            vec![TypeParam::new("T").with_constraint(Type::String)],
            vec![Param::new("x", Type::type_ref("T", vec![]))],
            Type::type_ref("T", vec![]),
        );
        assert_eq!(
            func_constrained.to_string(),
            "<T extends string>(x: T) => T"
        );
    }

    #[test]
    fn test_type_ref_display() {
        assert_eq!(
            Type::type_ref("Array", vec![Type::String]).to_string(),
            "Array<string>"
        );
        assert_eq!(
            Type::type_ref("Map", vec![Type::String, Type::Number]).to_string(),
            "Map<string, number>"
        );
        assert_eq!(Type::type_ref("User", vec![]).to_string(), "User");
    }

    #[test]
    fn test_is_primitive() {
        assert!(Type::String.is_primitive());
        assert!(Type::Number.is_primitive());
        assert!(Type::Boolean.is_primitive());
        assert!(!Type::Any.is_primitive());
        assert!(!Type::array(Type::String).is_primitive());
    }

    #[test]
    fn test_object_with_index_signature() {
        let obj = Type::Object {
            properties: vec![],
            index_signature: Some(IndexSignature {
                key_type: Box::new(Type::String),
                value_type: Box::new(Type::Number),
            }),
        };
        assert_eq!(obj.to_string(), "{ [key: string]: number }");
    }
}
