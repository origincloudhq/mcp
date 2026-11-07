//! A tool's arguments, checked against its input schema before they are
//! decoded.
//!
//! The SDK's decoding skips fields a message doesn't have, so a misspelt
//! field, or one the method doesn't take although its message has it,
//! would be dropped without a word. The check refuses both, naming the
//! fields there are, a required field that is missing, and a choice of a
//! oneof given more than once, or not at all when one is needed; the types
//! are the decoding's to check. An empty string is missing: proto3 can't tell
//! it from a field left out.

use serde_json::{Map, Value};

/// Checks `arguments` against `schema`, the generated input schema of a
/// tool.
pub(crate) fn check(schema: &Value, arguments: &Map<String, Value>) -> Result<(), String> {
    object(schema, arguments, "")
}

fn object(schema: &Value, value: &Map<String, Value>, path: &str) -> Result<(), String> {
    let at = |field: &str| {
        if path.is_empty() {
            format!("`{field}`")
        } else {
            format!("`{path}.{field}`")
        }
    };
    match schema.get("properties").and_then(Value::as_object) {
        Some(properties) => {
            for (field, v) in value {
                let Some(field_schema) = properties.get(field) else {
                    let known: Vec<&str> = properties.keys().map(String::as_str).collect();
                    return Err(format!(
                        "there is no field {}; the fields there are: {}",
                        at(field),
                        known.join(", ")
                    ));
                };
                nested(field_schema, v, &at(field).replace('`', ""))?;
            }
            for required in schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                match value.get(required) {
                    None | Some(Value::Null) => {
                        return Err(format!("{} is required", at(required)));
                    }
                    Some(Value::String(s)) if s.is_empty() => {
                        return Err(format!("{} is required, and can't be empty", at(required)));
                    }
                    Some(_) => {}
                }
            }
            choices(schema, value, &at)?;
        }
        // A map: every value is of one schema.
        None => {
            if let Some(values) = schema.get("additionalProperties").filter(|v| v.is_object()) {
                for (key, v) in value {
                    nested(values, v, &at(key).replace('`', ""))?;
                }
            }
        }
    }
    Ok(())
}

/// Checks the choices of `schema`, an object's, the oneofs of its message:
/// its `oneOf`, or each of its `allOf`, has a branch requiring each member,
/// and, when no member is needed, one requiring none.
fn choices(
    schema: &Value,
    value: &Map<String, Value>,
    at: &dyn Fn(&str) -> String,
) -> Result<(), String> {
    let all = schema.get("allOf").and_then(Value::as_array);
    let one_ofs = schema
        .get("oneOf")
        .into_iter()
        .chain(all.into_iter().flatten().filter_map(|s| s.get("oneOf")))
        .filter_map(Value::as_array);
    for branches in one_ofs {
        let members: Vec<&str> = branches
            .iter()
            .filter_map(|b| b.get("required")?.as_array()?.first()?.as_str())
            .collect();
        let none = branches.iter().any(|b| b.get("not").is_some());
        let named = |fields: &[&str]| fields.iter().map(|f| at(f)).collect::<Vec<_>>().join(", ");
        let given: Vec<&str> = members
            .iter()
            .copied()
            .filter(|m| !matches!(value.get(*m), None | Some(Value::Null)))
            .collect();
        if given.len() > 1 {
            return Err(format!(
                "only one of {} can be given, not {}",
                named(&members),
                named(&given)
            ));
        }
        if given.is_empty() && !none {
            return Err(format!("one of {} is required", named(&members)));
        }
    }
    Ok(())
}

fn nested(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    match value {
        Value::Object(fields) if schema.get("type").and_then(Value::as_str) == Some("object") => {
            object(schema, fields, path)
        }
        Value::Array(items) => match schema.get("items") {
            Some(item) => items
                .iter()
                .enumerate()
                .try_for_each(|(i, v)| nested(item, v, &format!("{path}[{i}]"))),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "parent": { "type": "string" },
                "volume": {
                    "type": "object",
                    "properties": {
                        "sizeGib": { "type": "integer" },
                        "tags": { "type": "object", "additionalProperties": { "type": "string" } },
                    },
                    "required": ["sizeGib"],
                    "additionalProperties": false,
                },
                "zones": {
                    "type": "array",
                    "items": { "type": "object", "properties": { "name": { "type": "string" } } },
                },
            },
            "required": ["parent", "volume"],
            "additionalProperties": false,
        })
    }

    fn arguments(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn arguments_the_schema_describes_pass() {
        let ok = arguments(json!({
            "parent": "projects/p/zones/z",
            "volume": { "sizeGib": 10, "tags": { "origin:name": "data" } },
            "zones": [{ "name": "z" }],
        }));
        assert_eq!(check(&schema(), &ok), Ok(()));
    }

    #[test]
    fn an_unknown_field_is_refused_with_the_fields_there_are() {
        let misspelt = arguments(json!({
            "parent": "p",
            "volume": { "sizeGib": 10, "size_gib": 20 },
        }));
        assert_eq!(
            check(&schema(), &misspelt),
            Err("there is no field `volume.size_gib`; the fields there are: sizeGib, tags".into())
        );
        let in_a_list = arguments(json!({
            "parent": "p",
            "volume": { "sizeGib": 1 },
            "zones": [{ "name": "z" }, { "nmae": "y" }],
        }));
        assert_eq!(
            check(&schema(), &in_a_list),
            Err("there is no field `zones[1].nmae`; the fields there are: name".into())
        );
    }

    #[test]
    fn a_missing_required_field_is_refused() {
        assert_eq!(
            check(&schema(), &arguments(json!({ "volume": { "sizeGib": 1 } }))),
            Err("`parent` is required".into())
        );
        assert_eq!(
            check(
                &schema(),
                &arguments(json!({ "parent": "p", "volume": {} }))
            ),
            Err("`volume.sizeGib` is required".into())
        );
    }

    #[test]
    fn an_empty_required_string_is_missing() {
        assert_eq!(
            check(
                &schema(),
                &arguments(json!({ "parent": "", "volume": { "sizeGib": 1 } }))
            ),
            Err("`parent` is required, and can't be empty".into())
        );
        // An optional one may be empty.
        let schema = json!({
            "type": "object",
            "properties": { "parent": { "type": "string" }, "filter": { "type": "string" } },
            "required": ["parent"],
        });
        assert_eq!(
            check(&schema, &arguments(json!({ "parent": "p", "filter": "" }))),
            Ok(())
        );
    }

    fn choices() -> Value {
        json!({
            "type": "object",
            "properties": {
                "recordSet": {
                    "type": "object",
                    "properties": {
                        "basic": { "type": "object", "properties": {} },
                        "alias": { "type": "object", "properties": {} },
                    },
                    "oneOf": [{ "required": ["basic"] }, { "required": ["alias"] }],
                },
                "text": { "type": "string" },
                "link": { "type": "string" },
            },
            "oneOf": [
                { "required": ["text"] },
                { "required": ["link"] },
                { "not": { "anyOf": [{ "required": ["text"] }, { "required": ["link"] }] } },
            ],
        })
    }

    #[test]
    fn a_choice_takes_one_member() {
        let ok = arguments(json!({ "recordSet": { "alias": {} }, "link": "l" }));
        assert_eq!(check(&choices(), &ok), Ok(()));
        let both = arguments(json!({ "recordSet": { "basic": {}, "alias": {} } }));
        assert_eq!(
            check(&choices(), &both),
            Err("only one of `recordSet.basic`, `recordSet.alias` can be given, not `recordSet.basic`, `recordSet.alias`".into())
        );
        let neither = arguments(json!({ "recordSet": {} }));
        assert_eq!(
            check(&choices(), &neither),
            Err("one of `recordSet.basic`, `recordSet.alias` is required".into())
        );
    }

    #[test]
    fn a_choice_of_at_most_one_may_take_none() {
        let none = arguments(json!({ "recordSet": { "basic": {} } }));
        assert_eq!(check(&choices(), &none), Ok(()));
        let both = arguments(json!({ "recordSet": { "basic": {} }, "text": "t", "link": "l" }));
        assert_eq!(
            check(&choices(), &both),
            Err("only one of `text`, `link` can be given, not `text`, `link`".into())
        );
    }
}
