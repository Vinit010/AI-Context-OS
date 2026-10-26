//! Bounded-structure helpers shared by the supervisor and the gateway.
//!
//! Depth is counted with the root value at 1, an object's members at 2, and so on. The supervisor
//! uses the same measurement for a plugin result and the gateway for a caller's arguments, so a
//! value that is "too deep" means one thing everywhere.

use serde_json::Value;

/// The deepest JSON structure either boundary accepts.
///
/// `docs/PLUGIN_SPEC.md` §4 bounds every result at 32 levels. The gateway applies the same cap to
/// arguments: a caller cannot hand a plugin a shape the host would refuse to read back.
pub(crate) const MAX_JSON_DEPTH: usize = 32;

/// The depth of a JSON value, with the root at 1.
pub(crate) fn json_depth(value: &Value) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(value, 1_usize)];
    while let Some((value, depth)) = stack.pop() {
        if depth > deepest {
            deepest = depth;
        }
        match value {
            Value::Array(items) => {
                for item in items {
                    stack.push((item, depth.saturating_add(1)));
                }
            }
            Value::Object(members) => {
                for item in members.values() {
                    stack.push((item, depth.saturating_add(1)));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    deepest
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{MAX_JSON_DEPTH, json_depth};

    #[test]
    fn a_scalar_is_one_level_and_an_object_holding_it_is_two() {
        assert_eq!(json_depth(&json!(1)), 1);
        assert_eq!(json_depth(&json!({"value": 1})), 2);
        assert_eq!(json_depth(&json!({"value": [1]})), 3);
    }

    #[test]
    fn the_depth_is_the_deepest_branch_not_the_first() {
        let value = json!({
            "shallow": 1,
            "branch": {"first": 2, "second": {"deep": 3}}
        });
        assert_eq!(json_depth(&value), 4);
    }

    #[test]
    fn the_cap_is_the_documented_thirty_two_levels() {
        let mut value = json!("bottom");
        for _ in 0..MAX_JSON_DEPTH - 1 {
            value = json!({ "nested": value });
        }
        assert_eq!(json_depth(&value), MAX_JSON_DEPTH);

        value = json!({ "nested": value });
        assert_eq!(json_depth(&value), MAX_JSON_DEPTH + 1);
    }
}
