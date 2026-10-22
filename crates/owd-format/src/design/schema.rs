//! The published JSON Schema (`schema/owd.schema.json`) and a small validator for the part of
//! JSON Schema 2020-12 that it uses: `type`, `const`, `enum`, `required`, `properties`,
//! `additionalProperties`, `items`, `minItems`, `maxItems`, `minLength`, `maxLength`, `minimum`,
//! `maximum`, `oneOf` and local `$ref`.

use serde_json::Value;

/// The schema, exactly as published in the repository.
pub const SCHEMA_TEXT: &str = include_str!("../../../../schema/owd.schema.json");

/// The parsed schema.
pub fn schema() -> Value {
    serde_json::from_str(SCHEMA_TEXT).expect("schema/owd.schema.json is valid JSON")
}

/// Every way `instance` breaks `schema`, as `path: message` lines. Empty when it conforms.
pub fn validate(schema: &Value, instance: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    check(schema, schema, instance, "", &mut errors);
    errors
}

fn type_matches(name: &str, v: &Value) -> bool {
    match name {
        "null" => v.is_null(),
        "boolean" => v.is_boolean(),
        "string" => v.is_string(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "number" => v.is_number(),
        "integer" => match v {
            Value::Number(n) => n.as_i64().is_some() || n.as_u64().is_some(),
            _ => false,
        },
        _ => false,
    }
}

fn describe(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn at(path: &str) -> &str {
    if path.is_empty() { "/" } else { path }
}

fn check(root: &Value, schema: &Value, v: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(s) = schema.as_object() else {
        return;
    };
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        let target = r
            .strip_prefix("#/")
            .and_then(|p| p.split('/').try_fold(root, |node, key| node.get(key)));
        match target {
            Some(t) => check(root, t, v, path, errors),
            None => errors.push(format!("{}: schema reference `{r}` not found", at(path))),
        }
        return;
    }
    if let Some(t) = s.get("type") {
        let names: Vec<&str> = match t {
            Value::String(n) => vec![n.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !names.iter().any(|n| type_matches(n, v)) {
            errors.push(format!(
                "{}: expected {}, found {}",
                at(path),
                names.join(" or "),
                describe(v)
            ));
            return;
        }
    }
    if let Some(c) = s.get("const")
        && c != v
    {
        errors.push(format!("{}: must be {c}", at(path)));
    }
    if let Some(Value::Array(e)) = s.get("enum")
        && !e.contains(v)
    {
        errors.push(format!("{}: not one of the allowed values", at(path)));
    }
    if let Some(alts) = s.get("oneOf").and_then(Value::as_array) {
        let passing = alts
            .iter()
            .filter(|a| {
                let mut e = Vec::new();
                check(root, a, v, path, &mut e);
                e.is_empty()
            })
            .count();
        if passing != 1 {
            // Say why the closest alternative failed, so the message points at the field: the
            // alternative of the instance's own type if there is one, else the fewest breaks.
            let mut best: Option<(bool, Vec<String>)> = None;
            for a in alts {
                let mut e = Vec::new();
                check(root, a, v, path, &mut e);
                let same_type = a.get("type").is_some_and(|t| match t {
                    Value::String(n) => type_matches(n, v),
                    Value::Array(l) => l
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|n| type_matches(n, v)),
                    _ => false,
                });
                let better = match &best {
                    None => true,
                    Some((bt, be)) => {
                        (same_type && !bt) || (same_type == *bt && e.len() < be.len())
                    }
                };
                if better {
                    best = Some((same_type, e));
                }
            }
            let best = best.map(|(_, e)| e);
            match (passing, best) {
                (0, Some(b)) => errors.extend(b),
                _ => errors.push(format!("{}: matches more than one alternative", at(path))),
            }
        }
    }
    if let Value::Object(obj) = v {
        if let Some(Value::Array(req)) = s.get("required") {
            for k in req.iter().filter_map(Value::as_str) {
                if !obj.contains_key(k) {
                    errors.push(format!("{}: missing `{k}`", at(path)));
                }
            }
        }
        let props = s.get("properties").and_then(Value::as_object);
        for (k, child) in obj {
            let child_path = format!("{path}/{k}");
            match props.and_then(|p| p.get(k)) {
                Some(sub) => check(root, sub, child, &child_path, errors),
                None => match s.get("additionalProperties") {
                    Some(Value::Bool(false)) => {
                        errors.push(format!("{child_path}: not allowed here"));
                    }
                    Some(sub @ Value::Object(_)) => check(root, sub, child, &child_path, errors),
                    _ => {}
                },
            }
        }
    }
    if let Value::Array(items) = v {
        if let Some(n) = s.get("minItems").and_then(Value::as_u64)
            && (items.len() as u64) < n
        {
            errors.push(format!("{}: needs at least {n} items", at(path)));
        }
        if let Some(n) = s.get("maxItems").and_then(Value::as_u64)
            && (items.len() as u64) > n
        {
            errors.push(format!("{}: allows at most {n} items", at(path)));
        }
        if let Some(sub) = s.get("items") {
            for (i, item) in items.iter().enumerate() {
                check(root, sub, item, &format!("{path}/{i}"), errors);
            }
        }
    }
    if let Value::String(text) = v {
        let len = text.chars().count() as u64;
        if let Some(n) = s.get("minLength").and_then(Value::as_u64)
            && len < n
        {
            errors.push(format!("{}: needs at least {n} characters", at(path)));
        }
        if let Some(n) = s.get("maxLength").and_then(Value::as_u64)
            && len > n
        {
            errors.push(format!("{}: allows at most {n} characters", at(path)));
        }
    }
    if let Some(x) = v.as_f64() {
        if let Some(m) = s.get("minimum").and_then(Value::as_f64)
            && x < m
        {
            errors.push(format!("{}: must be at least {m}", at(path)));
        }
        if let Some(m) = s.get("maximum").and_then(Value::as_f64)
            && x > m
        {
            errors.push(format!("{}: must be at most {m}", at(path)));
        }
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use serde_json::json;

    #[test]
    fn the_validator_reports_each_kind_of_break_with_its_path() {
        let s = json!({
            "type": "object", "required": ["a", "b"], "additionalProperties": false,
            "properties": {
                "a": {"type": "integer", "minimum": 1},
                "b": {"type": "array", "minItems": 2, "items": {"type": "string"}},
                "c": {"const": 7}
            }
        });
        assert!(validate(&s, &json!({"a": 1, "b": ["x", "y"], "c": 7})).is_empty());
        let e = validate(&s, &json!({"a": 0, "b": ["x", 2], "z": 1}));
        assert!(e.contains(&"/a: must be at least 1".to_string()), "{e:?}");
        assert!(!e.iter().any(|m| m.starts_with("/b:")), "{e:?}"); // two items: the length is fine
        assert!(
            e.iter().any(|m| m.starts_with("/b/1: expected string")),
            "{e:?}"
        );
        assert!(e.contains(&"/z: not allowed here".to_string()), "{e:?}");
        assert!(validate(&s, &json!({"b": ["x", "y"]})).contains(&"/: missing `a`".to_string()));
    }

    #[test]
    fn one_of_names_the_field_that_broke_the_closest_alternative() {
        let s = json!({"oneOf": [{"type": "null"}, {"type": "object", "required": ["k"]}]});
        assert!(validate(&s, &json!(null)).is_empty());
        let e = validate(&s, &json!({}));
        assert_eq!(e, vec!["/: missing `k`".to_string()]);
    }

    #[test]
    fn the_published_schema_is_valid_json_with_the_four_stages() {
        let s = schema();
        // Version 3 since spec 0007; an older document is refused.
        assert_eq!(s["properties"]["format_version"]["const"], json!("0.4"));
        assert_eq!(
            s["properties"]["stages"]["required"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }
}
