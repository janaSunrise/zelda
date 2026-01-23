use std::fmt;

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

    // Compound
    Array(Box<Type>),
    Tuple(Vec<Type>),
    Union(Vec<Type>),
    Intersection(Vec<Type>),

    // Objects
    Object {
        properties: Vec<Property>,
        index_signature: Option<IndexSignature>,
    },

    // Functions
    Function {
        params: Vec<Param>,
        return_type: Box<Type>,
        type_params: Vec<TypeParam>,
    },

    // References
    TypeRef {
        name: String,
        type_args: Vec<Type>,
    },

    // Type parameter (used in generic definitions)
    TypeParameter {
        name: String,
        constraint: Option<Box<Type>>,
        default: Option<Box<Type>>,
    },
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
            // Primitives
            Type::String => write!(f, "string"),
            Type::Number => write!(f, "number"),
            Type::Boolean => write!(f, "boolean"),
            Type::Null => write!(f, "null"),
            Type::Undefined => write!(f, "undefined"),
            Type::Void => write!(f, "void"),
            Type::Any => write!(f, "any"),
            Type::Unknown => write!(f, "unknown"),
            Type::Never => write!(f, "never"),

            // Literals
            Type::StringLiteral(s) => write!(f, "\"{}\"", s),
            Type::NumberLiteral(n) => write!(f, "{}", n),
            Type::BooleanLiteral(b) => write!(f, "{}", b),

            // Compound
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
                    // Wrap function types in () for clarity
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
                    // Wrap union/function types in () for clarity
                    if matches!(ty, Type::Union(_) | Type::Function { .. }) {
                        write!(f, "({})", ty)?;
                    } else {
                        write!(f, "{}", ty)?;
                    }
                }
                Ok(())
            }

            // Objects
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

            // Functions
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

            // References
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct IndexSignature {
    pub key_type: Box<Type>, // string or number
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
