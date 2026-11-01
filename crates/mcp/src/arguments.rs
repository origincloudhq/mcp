//! A tool's arguments, checked against its input schema before they are
//! decoded.
//!
//! The SDK's decoding skips fields a message doesn't have, so a misspelt
//! field, or one the method doesn't take although its message has it,
//! would be dropped without a word. The check refuses both, naming the
//! fields there are, and a required field that is missing; the types are
//! the decoding's to check.

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
                if value.get(required).is_none_or(Value::is_null) {
                    return Err(format!("{} is required", at(required)));
                }
            }
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
}
