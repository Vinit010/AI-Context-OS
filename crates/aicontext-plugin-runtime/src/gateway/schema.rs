//! The JSON Schema subset the gateway enforces before a call.
//!
//! `docs/PLUGIN_SPEC.md` §3 requires every tool to carry a Draft 2020-12 `input_schema` with
//! `additionalProperties: false`, and the gateway is the component that enforces it. A full
//! Draft 2020-12 engine is a large runtime dependency (`ARCHITECTURE.md` risk R-2), so this module
//! implements a deliberately small, closed subset and refuses everything outside it (`PLG-012`).
//! Refusing an unknown keyword is the important half: silently ignoring `oneOf` or `$ref` would
//! leave a constraint the plugin believes is enforced and the host never checked.
//!
//! # Supported subset
//!
//! Every schema is an object. The gateway accepts these keywords:
//!
//! - `type`: one type name, or an array of names. An array of more than one type may carry only
//!   `enum` and `const`; per-type constraints are refused because they would silently not apply to
//!   the other branches.
//! - `enum`, `const`.
//! - Object: `properties`, `required`, and `additionalProperties: false`. Every object schema must
//!   state `additionalProperties: false`; the default of `true` is not accepted because it would
//!   allow fields nobody declared.
//! - Array: `items`, `minItems`, `maxItems`. `items` is required.
//! - String: `minLength`, `maxLength`, `pattern`.
//! - Number and integer: `minimum`, `maximum`.
//! - The annotations `$schema`, `$id`, `$comment`, `title`, `description`, `default`, `examples`,
//!   `deprecated`, `readOnly`, and `writeOnly`, which carry no constraint and are ignored.
//!
//! `pattern` is compiled by the `regex` crate. A pattern using an ECMA-262 construct Rust's engine
//! does not support is refused at start rather than left for the plugin to interpret differently.
//! `minLength` and `maxLength` count Unicode scalar values, as Draft 2020-12 does.
//!
//! # Limits
//!
//! The schema itself is capped by [`MAX_SCHEMA_BYTES`] and the shared JSON depth cap. Arguments are
//! capped by [`MAX_ARGUMENT_BYTES`] and the same depth cap before any validation walk, so a deeply
//! nested or enormous value cannot turn a check into an unbounded traversal.

use std::collections::{BTreeMap, BTreeSet};

use regex::{Regex, RegexBuilder};
use serde_json::{Map, Number, Value};

use crate::json::{MAX_JSON_DEPTH, json_depth};

/// The largest `input_schema` the gateway accepts, in compact JSON bytes.
pub(crate) const MAX_SCHEMA_BYTES: usize = 64 * 1024;

/// The largest argument set the gateway accepts, in compact JSON bytes.
pub(crate) const MAX_ARGUMENT_BYTES: usize = 64 * 1024;

/// The largest pattern the gateway compiles, in bytes.
const MAX_PATTERN_BYTES: usize = 4 * 1024;

/// A compiled tool input schema.
#[derive(Debug)]
pub(crate) struct CompiledSchema {
    root: Node,
}

impl CompiledSchema {
    /// Compile the schema the plugin advertised.
    ///
    /// # Errors
    ///
    /// A human-readable reason when the schema is outside the subset, has an invalid keyword
    /// value, or is larger or deeper than the host accepts. The caller maps it to `PLG-012`.
    pub(crate) fn compile(schema: &Value) -> Result<Self, String> {
        if json_depth(schema) > MAX_JSON_DEPTH {
            return Err(format!(
                "the input schema nests deeper than the {MAX_JSON_DEPTH}-level limit"
            ));
        }
        let bytes = schema.to_string().len();
        if bytes > MAX_SCHEMA_BYTES {
            return Err(format!(
                "the input schema is {bytes} bytes; the maximum is {MAX_SCHEMA_BYTES}"
            ));
        }

        let root = compile_node(schema, true, "input_schema")?;
        if root.types != [JsonType::Object] {
            return Err("the root schema must have `type: object`".to_owned());
        }
        Ok(Self { root })
    }

    /// Validate one argument set against the schema.
    ///
    /// # Errors
    ///
    /// A human-readable path and reason for the first violation.
    pub(crate) fn validate(&self, arguments: &Value) -> Result<(), String> {
        if json_depth(arguments) > MAX_JSON_DEPTH {
            return Err(format!(
                "the arguments nest deeper than the {MAX_JSON_DEPTH}-level limit"
            ));
        }
        let bytes = arguments.to_string().len();
        if bytes > MAX_ARGUMENT_BYTES {
            return Err(format!(
                "the arguments are {bytes} bytes; the maximum is {MAX_ARGUMENT_BYTES}"
            ));
        }
        self.root.validate(arguments, "arguments")
    }
}

/// One type name in the subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JsonType {
    Object,
    Array,
    String,
    Number,
    Integer,
    Boolean,
    Null,
}

impl JsonType {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "object" => Some(Self::Object),
            "array" => Some(Self::Array),
            "string" => Some(Self::String),
            "number" => Some(Self::Number),
            "integer" => Some(Self::Integer),
            "boolean" => Some(Self::Boolean),
            "null" => Some(Self::Null),
            _ => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Array => "array",
            Self::String => "string",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Null => "null",
        }
    }
}

/// A compiled schema node.
#[derive(Debug)]
struct Node {
    /// An empty list means "any value"; the schema stated no `type`.
    types: Vec<JsonType>,
    enum_values: Option<Vec<Value>>,
    const_value: Option<Value>,
    object: Option<ObjectRules>,
    array: Option<ArrayRules>,
    string: Option<StringRules>,
    number: Option<NumberRules>,
}

#[derive(Debug)]
struct ObjectRules {
    properties: BTreeMap<String, Node>,
    required: BTreeSet<String>,
}

#[derive(Debug)]
struct ArrayRules {
    items: Box<Node>,
    min_items: Option<usize>,
    max_items: Option<usize>,
}

#[derive(Debug)]
struct StringRules {
    min_length: Option<usize>,
    max_length: Option<usize>,
    pattern: Option<Regex>,
}

#[derive(Debug)]
struct NumberRules {
    minimum: Option<f64>,
    maximum: Option<f64>,
}

/// Keywords that carry no constraint and are accepted for readability.
const ANNOTATIONS: &[&str] = &[
    "$schema",
    "$id",
    "$comment",
    "title",
    "description",
    "default",
    "examples",
    "deprecated",
    "readOnly",
    "writeOnly",
];

/// Keywords this module enforces.
const CONSTRAINTS: &[&str] = &[
    "type",
    "enum",
    "const",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "minItems",
    "maxItems",
    "minLength",
    "maxLength",
    "pattern",
    "minimum",
    "maximum",
];

fn compile_node(value: &Value, root: bool, path: &str) -> Result<Node, String> {
    let Some(member) = value.as_object() else {
        return Err(format!(
            "{path}: a schema must be an object; boolean schemas are unsupported"
        ));
    };

    for key in member.keys() {
        if !ANNOTATIONS.contains(&key.as_str()) && !CONSTRAINTS.contains(&key.as_str()) {
            return Err(format!(
                "{path}: `{key}` is not in the supported JSON Schema subset"
            ));
        }
    }

    let types = compile_types(member, root, path)?;
    let enum_values = compile_enum(member, path)?;
    let const_value = member.get("const").cloned();

    let single = match types.as_slice() {
        [only] => Some(*only),
        [] => None,
        _ => {
            for key in member.keys() {
                if key == "type"
                    || key == "enum"
                    || key == "const"
                    || ANNOTATIONS.contains(&key.as_str())
                {
                    continue;
                }
                return Err(format!(
                    "{path}: `{key}` cannot be enforced when `type` names more than one type"
                ));
            }
            None
        }
    };

    let mut node = Node {
        types,
        enum_values,
        const_value,
        object: None,
        array: None,
        string: None,
        number: None,
    };

    if root {
        node.object = Some(compile_object(member, path)?);
        return Ok(node);
    }

    match single {
        Some(JsonType::Object) => node.object = Some(compile_object(member, path)?),
        Some(JsonType::Array) => node.array = Some(compile_array(member, path)?),
        Some(JsonType::String) => node.string = Some(compile_string(member, path)?),
        Some(JsonType::Number | JsonType::Integer) => {
            node.number = Some(compile_number(member, path)?);
        }
        Some(JsonType::Boolean | JsonType::Null) | None => {
            reject_applicable(
                member,
                &[
                    "properties",
                    "required",
                    "additionalProperties",
                    "items",
                    "minItems",
                    "maxItems",
                    "minLength",
                    "maxLength",
                    "pattern",
                    "minimum",
                    "maximum",
                ],
                path,
            )?;
        }
    }

    Ok(node)
}

fn compile_types(
    member: &Map<String, Value>,
    root: bool,
    path: &str,
) -> Result<Vec<JsonType>, String> {
    let Some(value) = member.get("type") else {
        if root {
            return Err(format!("{path}: the root schema must have `type: object`"));
        }
        return Ok(Vec::new());
    };

    let names: Vec<&str> = match value {
        Value::String(name) => vec![name.as_str()],
        Value::Array(items) => {
            if items.is_empty() {
                return Err(format!("{path}.type: the type list is empty"));
            }
            let mut names = Vec::with_capacity(items.len());
            for item in items {
                let Some(name) = item.as_str() else {
                    return Err(format!("{path}.type: every entry must be a string"));
                };
                names.push(name);
            }
            names
        }
        _ => {
            return Err(format!(
                "{path}.type: expected a string or an array of strings"
            ));
        }
    };

    let mut types = Vec::with_capacity(names.len());
    for name in names {
        let Some(kind) = JsonType::parse(name) else {
            return Err(format!(
                "{path}.type: `{name}` is not one of object, array, string, number, integer, \
                 boolean, null"
            ));
        };
        if types.contains(&kind) {
            return Err(format!("{path}.type: `{name}` is repeated"));
        }
        types.push(kind);
    }
    if types.contains(&JsonType::Number) && types.contains(&JsonType::Integer) {
        return Err(format!(
            "{path}.type: `number` already covers `integer`; list only one"
        ));
    }
    if root && types != [JsonType::Object] {
        return Err(format!("{path}: the root schema must have `type: object`"));
    }
    Ok(types)
}

fn compile_enum(member: &Map<String, Value>, path: &str) -> Result<Option<Vec<Value>>, String> {
    let Some(value) = member.get("enum") else {
        return Ok(None);
    };
    let Some(items) = value.as_array() else {
        return Err(format!("{path}.enum: expected an array"));
    };
    if items.is_empty() {
        return Err(format!("{path}.enum: the array is empty"));
    }
    Ok(Some(items.clone()))
}

fn compile_object(member: &Map<String, Value>, path: &str) -> Result<ObjectRules, String> {
    reject_applicable(
        member,
        &[
            "items",
            "minItems",
            "maxItems",
            "minLength",
            "maxLength",
            "pattern",
            "minimum",
            "maximum",
        ],
        path,
    )?;

    match member.get("additionalProperties") {
        Some(Value::Bool(false)) => {}
        Some(_) => {
            return Err(format!(
                "{path}.additionalProperties: only `false` is supported; every object must reject \
                 undeclared properties"
            ));
        }
        None => {
            return Err(format!(
                "{path}.additionalProperties: required and must be `false`"
            ));
        }
    }

    let mut properties = BTreeMap::new();
    if let Some(value) = member.get("properties") {
        let Some(entries) = value.as_object() else {
            return Err(format!("{path}.properties: expected an object"));
        };
        for (name, schema) in entries {
            if name.chars().any(char::is_control) {
                return Err(format!(
                    "{path}.properties: the property name {name:?} contains a control character"
                ));
            }
            let child_path = format!("{path}.properties.{name}");
            properties.insert(name.clone(), compile_node(schema, false, &child_path)?);
        }
    }

    let mut required = BTreeSet::new();
    if let Some(value) = member.get("required") {
        let Some(entries) = value.as_array() else {
            return Err(format!("{path}.required: expected an array of strings"));
        };
        for entry in entries {
            let Some(name) = entry.as_str() else {
                return Err(format!("{path}.required: every entry must be a string"));
            };
            if !properties.contains_key(name) {
                return Err(format!(
                    "{path}.required: `{name}` is not declared under `properties`, and \
                     `additionalProperties: false` means it could never be supplied"
                ));
            }
            if !required.insert(name.to_owned()) {
                return Err(format!("{path}.required: `{name}` is repeated"));
            }
        }
    }

    Ok(ObjectRules {
        properties,
        required,
    })
}

fn compile_array(member: &Map<String, Value>, path: &str) -> Result<ArrayRules, String> {
    reject_applicable(
        member,
        &[
            "properties",
            "required",
            "additionalProperties",
            "minLength",
            "maxLength",
            "pattern",
            "minimum",
            "maximum",
        ],
        path,
    )?;

    let Some(items) = member.get("items") else {
        return Err(format!(
            "{path}.items: required; an array schema must declare its item shape"
        ));
    };
    let items = compile_node(items, false, &format!("{path}.items"))?;
    let min_items = optional_usize(member, "minItems", path)?;
    let max_items = optional_usize(member, "maxItems", path)?;
    ensure_order(min_items, max_items, "minItems", "maxItems", path)?;

    Ok(ArrayRules {
        items: Box::new(items),
        min_items,
        max_items,
    })
}

fn compile_string(member: &Map<String, Value>, path: &str) -> Result<StringRules, String> {
    reject_applicable(
        member,
        &[
            "properties",
            "required",
            "additionalProperties",
            "items",
            "minItems",
            "maxItems",
            "minimum",
            "maximum",
        ],
        path,
    )?;

    let min_length = optional_usize(member, "minLength", path)?;
    let max_length = optional_usize(member, "maxLength", path)?;
    ensure_order(min_length, max_length, "minLength", "maxLength", path)?;

    let pattern = match member.get("pattern") {
        Some(Value::String(pattern)) => {
            if pattern.len() > MAX_PATTERN_BYTES {
                return Err(format!(
                    "{path}.pattern: the pattern is {} bytes; the maximum is {MAX_PATTERN_BYTES}",
                    pattern.len()
                ));
            }
            let compiled = RegexBuilder::new(pattern)
                .size_limit(1 << 20)
                .dfa_size_limit(1 << 20)
                .build()
                .map_err(|error| {
                    format!("{path}.pattern: `{pattern}` is not a valid pattern: {error}")
                })?;
            Some(compiled)
        }
        Some(_) => return Err(format!("{path}.pattern: expected a string")),
        None => None,
    };

    Ok(StringRules {
        min_length,
        max_length,
        pattern,
    })
}

fn compile_number(member: &Map<String, Value>, path: &str) -> Result<NumberRules, String> {
    reject_applicable(
        member,
        &[
            "properties",
            "required",
            "additionalProperties",
            "items",
            "minItems",
            "maxItems",
            "minLength",
            "maxLength",
            "pattern",
        ],
        path,
    )?;

    let minimum = optional_number(member, "minimum", path)?;
    let maximum = optional_number(member, "maximum", path)?;
    if let (Some(minimum), Some(maximum)) = (minimum, maximum) {
        if minimum > maximum {
            return Err(format!(
                "{path}: `minimum` ({minimum}) is greater than `maximum` ({maximum})"
            ));
        }
    }

    Ok(NumberRules { minimum, maximum })
}

fn optional_usize(
    member: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Option<usize>, String> {
    let Some(value) = member.get(key) else {
        return Ok(None);
    };
    let Some(number) = value.as_u64() else {
        return Err(format!("{path}.{key}: expected a non-negative integer"));
    };
    usize::try_from(number)
        .map(Some)
        .map_err(|_| format!("{path}.{key}: {number} is too large for this host"))
}

fn optional_number(
    member: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Option<f64>, String> {
    let Some(value) = member.get(key) else {
        return Ok(None);
    };
    let Some(number) = value.as_f64() else {
        return Err(format!("{path}.{key}: expected a number"));
    };
    if !number.is_finite() {
        return Err(format!("{path}.{key}: the number is not finite"));
    }
    Ok(Some(number))
}

fn ensure_order(
    minimum: Option<usize>,
    maximum: Option<usize>,
    minimum_name: &str,
    maximum_name: &str,
    path: &str,
) -> Result<(), String> {
    if let (Some(minimum), Some(maximum)) = (minimum, maximum) {
        if minimum > maximum {
            return Err(format!(
                "{path}: `{minimum_name}` ({minimum}) is greater than `{maximum_name}` ({maximum})"
            ));
        }
    }
    Ok(())
}

fn reject_applicable(member: &Map<String, Value>, keys: &[&str], path: &str) -> Result<(), String> {
    for key in keys {
        if member.contains_key(*key) {
            return Err(format!(
                "{path}: `{key}` does not apply to this schema's `type`"
            ));
        }
    }
    Ok(())
}

impl Node {
    fn validate(&self, value: &Value, path: &str) -> Result<(), String> {
        if let Some(values) = &self.enum_values {
            if !values.iter().any(|candidate| json_equal(candidate, value)) {
                return Err(format!(
                    "{path}: the value is not one of the enumerated values"
                ));
            }
        }
        if let Some(expected) = &self.const_value {
            if !json_equal(expected, value) {
                return Err(format!(
                    "{path}: the value does not equal the `const` value"
                ));
            }
        }

        let Some(only) = self.single_type() else {
            if self.types.is_empty() {
                return Ok(());
            }
            return validate_union(&self.types, value, path);
        };

        match only {
            JsonType::Object => self.validate_object(value, path),
            JsonType::Array => self.validate_array(value, path),
            JsonType::String => self.validate_string(value, path),
            JsonType::Number | JsonType::Integer => self.validate_number(value, path, only),
            JsonType::Boolean => {
                if value.is_boolean() {
                    Ok(())
                } else {
                    Err(expected(only, value, path))
                }
            }
            JsonType::Null => {
                if value.is_null() {
                    Ok(())
                } else {
                    Err(expected(only, value, path))
                }
            }
        }
    }

    fn single_type(&self) -> Option<JsonType> {
        match self.types.as_slice() {
            [only] => Some(*only),
            _ => None,
        }
    }

    fn validate_object(&self, value: &Value, path: &str) -> Result<(), String> {
        let Some(object) = value.as_object() else {
            return Err(expected(JsonType::Object, value, path));
        };
        let Some(rules) = &self.object else {
            return Ok(());
        };

        for name in &rules.required {
            if !object.contains_key(name) {
                return Err(format!("{path}: the required property `{name}` is missing"));
            }
        }

        for (name, item) in object {
            let Some(schema) = rules.properties.get(name) else {
                return Err(format!(
                    "{path}: the property `{name}` is not declared, and the schema rejects \
                     additional properties"
                ));
            };
            schema.validate(item, &format!("{path}.{name}"))?;
        }
        Ok(())
    }

    fn validate_array(&self, value: &Value, path: &str) -> Result<(), String> {
        let Some(items) = value.as_array() else {
            return Err(expected(JsonType::Array, value, path));
        };
        let Some(rules) = &self.array else {
            return Ok(());
        };

        if let Some(minimum) = rules.min_items {
            if items.len() < minimum {
                return Err(format!(
                    "{path}: expected at least {minimum} items, found {}",
                    items.len()
                ));
            }
        }
        if let Some(maximum) = rules.max_items {
            if items.len() > maximum {
                return Err(format!(
                    "{path}: expected at most {maximum} items, found {}",
                    items.len()
                ));
            }
        }
        for (index, item) in items.iter().enumerate() {
            rules.items.validate(item, &format!("{path}[{index}]"))?;
        }
        Ok(())
    }

    fn validate_string(&self, value: &Value, path: &str) -> Result<(), String> {
        let Some(text) = value.as_str() else {
            return Err(expected(JsonType::String, value, path));
        };
        let Some(rules) = &self.string else {
            return Ok(());
        };

        let length = text.chars().count();
        if let Some(minimum) = rules.min_length {
            if length < minimum {
                return Err(format!(
                    "{path}: expected at least {minimum} characters, found {length}"
                ));
            }
        }
        if let Some(maximum) = rules.max_length {
            if length > maximum {
                return Err(format!(
                    "{path}: expected at most {maximum} characters, found {length}"
                ));
            }
        }
        if let Some(pattern) = &rules.pattern {
            if !pattern.is_match(text) {
                return Err(format!(
                    "{path}: the value does not match the pattern {}",
                    pattern.as_str()
                ));
            }
        }
        Ok(())
    }

    fn validate_number(&self, value: &Value, path: &str, kind: JsonType) -> Result<(), String> {
        let Some(number) = value.as_number() else {
            return Err(expected(kind, value, path));
        };
        if kind == JsonType::Integer && !is_integer(number) {
            return Err(format!(
                "{path}: expected an integer, found {}",
                describe(value)
            ));
        }
        let Some(number) = number.as_f64() else {
            return Err(format!(
                "{path}: the number cannot be compared on this host"
            ));
        };
        let Some(rules) = &self.number else {
            return Ok(());
        };
        if let Some(minimum) = rules.minimum {
            if number < minimum {
                return Err(format!(
                    "{path}: expected a value >= {minimum}, found {number}"
                ));
            }
        }
        if let Some(maximum) = rules.maximum {
            if number > maximum {
                return Err(format!(
                    "{path}: expected a value <= {maximum}, found {number}"
                ));
            }
        }
        Ok(())
    }
}

fn validate_union(types: &[JsonType], value: &Value, path: &str) -> Result<(), String> {
    for kind in types {
        let matches = match kind {
            JsonType::Object => value.is_object(),
            JsonType::Array => value.is_array(),
            JsonType::String => value.is_string(),
            JsonType::Number => value.is_number(),
            JsonType::Integer => value.as_number().is_some_and(is_integer),
            JsonType::Boolean => value.is_boolean(),
            JsonType::Null => value.is_null(),
        };
        if matches {
            return Ok(());
        }
    }
    let names: Vec<&str> = types.iter().map(|kind| kind.name()).collect();
    Err(format!(
        "{path}: expected one of {}, found {}",
        names.join(", "),
        describe(value)
    ))
}

fn is_integer(number: &Number) -> bool {
    if number.is_i64() || number.is_u64() {
        return true;
    }
    number
        .as_f64()
        .is_some_and(|value| value.is_finite() && value.fract() == 0.0)
}

/// JSON Schema numeric equality: `1` and `1.0` are the same number.
///
/// The comparison is intentionally exact: two parsed finite `f64` values are equal only when they
/// denote the same number, and JSON Schema equality has no tolerance. A margin would make `1` and
/// `1.0000001` interchangeable, which the schema did not say.
#[allow(clippy::float_cmp)]
fn json_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => match (left.as_f64(), right.as_f64()) {
            (Some(left), Some(right)) => left == right,
            _ => left == right,
        },
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| json_equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .all(|(key, left)| right.get(key).is_some_and(|right| json_equal(left, right)))
        }
        _ => left == right,
    }
}

fn expected(kind: JsonType, value: &Value, path: &str) -> String {
    format!(
        "{path}: expected {}, found {}",
        kind.name(),
        describe(value)
    )
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{CompiledSchema, MAX_ARGUMENT_BYTES, MAX_SCHEMA_BYTES};

    fn schema() -> serde_json::Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["instance_id"],
            "properties": {
                "instance_id": {
                    "type": "string",
                    "pattern": "^i-[0-9a-f]{8,17}$"
                },
                "reason": {
                    "type": "string",
                    "maxLength": 500
                },
                "tags": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 3,
                    "items": {"type": "string", "minLength": 1}
                }
            }
        })
    }

    #[test]
    fn the_documented_example_compiles_and_accepts_a_real_argument_set() {
        let compiled = CompiledSchema::compile(&schema()).expect("the documented schema compiles");

        compiled
            .validate(&json!({
                "instance_id": "i-0abc1234",
                "reason": "idle",
                "tags": ["prod"]
            }))
            .expect("the argument set is valid");
    }

    #[test]
    fn a_missing_required_property_names_the_property() {
        let compiled = CompiledSchema::compile(&schema()).expect("valid schema");

        let error = compiled
            .validate(&json!({"reason": "idle"}))
            .expect_err("required property missing");
        assert!(error.contains("instance_id"), "{error}");
        assert!(error.contains("required"), "{error}");
    }

    #[test]
    fn an_undeclared_property_is_refused_by_name() {
        let compiled = CompiledSchema::compile(&schema()).expect("valid schema");

        let error = compiled
            .validate(&json!({"instance_id": "i-0abc1234", "shell": "rm -rf /"}))
            .expect_err("additional property");
        assert!(error.contains("shell"), "{error}");
    }

    #[test]
    fn a_pattern_mismatch_is_refused() {
        let compiled = CompiledSchema::compile(&schema()).expect("valid schema");

        let error = compiled
            .validate(&json!({"instance_id": "not-an-instance"}))
            .expect_err("pattern mismatch");
        assert!(error.contains("pattern"), "{error}");
    }

    #[test]
    fn nested_arrays_and_lengths_are_enforced() {
        let compiled = CompiledSchema::compile(&schema()).expect("valid schema");

        assert!(
            compiled
                .validate(&json!({"instance_id": "i-0abc1234", "tags": []}))
                .is_err()
        );
        assert!(
            compiled
                .validate(&json!({"instance_id": "i-0abc1234", "tags": [""]}))
                .is_err()
        );
        assert!(
            compiled
                .validate(&json!({"instance_id": "i-0abc1234", "tags": ["a", "b", "c", "d"]}))
                .is_err()
        );
    }

    #[test]
    fn the_root_must_be_an_object_with_additional_properties_false() {
        for schema in [
            json!({"type": "string", "additionalProperties": false}),
            json!({"type": "object"}),
            json!({"type": "object", "additionalProperties": true}),
            json!({"type": "object", "additionalProperties": false, "properties": {
                "nested": {"type": "object", "properties": {}}
            }}),
        ] {
            assert!(
                CompiledSchema::compile(&schema).is_err(),
                "must be refused: {schema}"
            );
        }
    }

    #[test]
    fn an_unsupported_keyword_is_refused_rather_than_ignored() {
        for keyword in [
            "oneOf",
            "anyOf",
            "allOf",
            "$ref",
            "format",
            "patternProperties",
        ] {
            let mut schema = schema();
            schema[keyword] = json!([]);
            let error = CompiledSchema::compile(&schema).expect_err("unsupported keyword");
            assert!(error.contains(keyword), "{keyword}: {error}");
        }
    }

    #[test]
    fn a_constraint_on_the_wrong_type_is_refused() {
        for schema in [
            json!({"type": "object", "additionalProperties": false, "properties": {
                "name": {"type": "string", "minItems": 1}
            }}),
            json!({"type": "object", "additionalProperties": false, "properties": {
                "name": {"type": "number", "pattern": "^x$"}
            }}),
            json!({"type": "array", "items": {"type": "string"}, "minLength": 1}),
        ] {
            assert!(
                CompiledSchema::compile(&schema).is_err(),
                "must be refused: {schema}"
            );
        }
    }

    #[test]
    fn a_type_union_may_only_carry_enum_or_const() {
        let valid = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": {"type": ["string", "null"], "maxLength": 10}
            }
        });
        assert!(CompiledSchema::compile(&valid).is_err());

        let valid = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": {"type": ["string", "null"], "enum": ["a", null]}
            }
        });
        let compiled = CompiledSchema::compile(&valid).expect("the union compiles");
        compiled
            .validate(&json!({"name": null}))
            .expect("null is allowed");
        compiled
            .validate(&json!({"name": "a"}))
            .expect("the enum member is allowed");
        assert!(compiled.validate(&json!({"name": 1})).is_err());
    }

    #[test]
    fn integer_and_number_follow_json_schema_equality() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "count": {"type": "integer", "minimum": 1, "maximum": 3}
            }
        });
        let compiled = CompiledSchema::compile(&schema).expect("valid schema");

        compiled.validate(&json!({"count": 1})).expect("integer");
        compiled
            .validate(&json!({"count": 1.0}))
            .expect("1.0 is an integer");
        assert!(compiled.validate(&json!({"count": 1.5})).is_err());
        assert!(compiled.validate(&json!({"count": 0})).is_err());
        assert!(compiled.validate(&json!({"count": 4})).is_err());
    }

    #[test]
    fn enum_and_const_compare_numbers_by_value() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "mode": {"type": "number", "enum": [1, 2]},
                "fixed": {"const": 3}
            }
        });
        let compiled = CompiledSchema::compile(&schema).expect("valid schema");

        compiled
            .validate(&json!({"mode": 1.0, "fixed": 3.0}))
            .expect("numeric equality is by value");
        assert!(compiled.validate(&json!({"mode": 3})).is_err());
    }

    #[test]
    fn arguments_are_capped_by_size_and_depth() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "text": {"type": "string"}
            }
        });
        let compiled = CompiledSchema::compile(&schema).expect("valid schema");

        let big = "x".repeat(MAX_ARGUMENT_BYTES + 1);
        let error = compiled
            .validate(&json!({"text": big}))
            .expect_err("too large");
        assert!(error.contains("maximum"), "{error}");

        let mut deep = json!("bottom");
        for _ in 0..40 {
            deep = json!({"nested": deep});
        }
        let error = compiled
            .validate(&json!({"text": deep}))
            .expect_err("too deep");
        assert!(error.contains("deeper"), "{error}");
    }

    #[test]
    fn the_schema_itself_is_capped_by_size_and_depth() {
        let big = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "text": {"type": "string", "pattern": "x".repeat(MAX_SCHEMA_BYTES)}
            }
        });
        assert!(CompiledSchema::compile(&big).is_err());

        let mut deep = json!({"type": "string"});
        for _ in 0..40 {
            deep = json!({"type": "array", "items": deep});
        }
        assert!(CompiledSchema::compile(&deep).is_err());
    }

    #[test]
    fn the_subset_agrees_with_a_real_draft_2020_12_validator() {
        // The drift net: the hand-rolled subset and a real Draft 2020-12 engine are two readings of
        // the same contract. Every case below must get the same answer from both. The cases cover
        // each supported constraint so a divergence cannot hide behind an untested keyword.
        let numeric = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["count"],
            "properties": {
                "count": {"type": "integer", "minimum": 1, "maximum": 3},
                "mode": {"type": "number", "enum": [1, 2]},
                "fixed": {"const": 3},
                "maybe": {"type": ["string", "null"], "enum": ["a", null]}
            }
        });

        let cases = [
            (
                schema(),
                vec![
                    json!({"instance_id": "i-0abc1234", "reason": "idle", "tags": ["prod"]}),
                    json!({"reason": "idle"}),
                    json!({"instance_id": "i-0abc1234", "shell": "rm -rf /"}),
                    json!({"instance_id": "not-an-instance"}),
                    json!({"instance_id": "i-0abc1234", "tags": []}),
                    json!({"instance_id": "i-0abc1234", "tags": [""]}),
                    json!({"instance_id": "i-0abc1234", "tags": ["a", "b", "c", "d"]}),
                    json!({"instance_id": "i-0abc1234", "tags": "prod"}),
                    json!({"instance_id": "i-0abc1234", "reason": "x".repeat(501)}),
                ],
            ),
            (
                numeric,
                vec![
                    json!({"count": 1}),
                    json!({"count": 1.0}),
                    json!({"count": 1.5}),
                    json!({"count": 0}),
                    json!({"count": 4}),
                    json!({"count": 2, "mode": 1.0}),
                    json!({"count": 2, "mode": 3}),
                    json!({"count": 2, "fixed": 3.0}),
                    json!({"count": 2, "maybe": "a"}),
                    json!({"count": 2, "maybe": null}),
                    json!({"count": 2, "maybe": 1}),
                    json!({"count": 2, "undeclared": true}),
                ],
            ),
        ];

        for (schema, arguments) in cases {
            let validator =
                jsonschema::validator_for(&schema).expect("the fixture schema compiles");
            let compiled = CompiledSchema::compile(&schema).expect("the subset compiles");
            for arguments in arguments {
                assert_eq!(
                    compiled.validate(&arguments).is_ok(),
                    validator.is_valid(&arguments),
                    "the subset and Draft 2020-12 disagree on {arguments} for {schema}"
                );
            }
        }
    }
}
