use serde_json::Value;

pub(crate) fn validate_form(schema: &Value, content: &Value) -> Result<(), &'static str> {
    let properties = schema["properties"].as_object().ok_or("Invalid form properties")?;
    let values = content.as_object().ok_or("Form content must be an object")?;
    if schema["type"] != "object" || properties.len() > 16 || schema.to_string().len() > 65_536 || content.to_string().len() > 65_536 {
        return Err("Form exceeds supported limits");
    }
    if let Some(required) = schema.get("required") {
        let required = required.as_array().ok_or("Invalid required fields")?;
        for key in required {
            let key = key.as_str().ok_or("Invalid required field")?;
            if !properties.contains_key(key) || !values.contains_key(key) {
                return Err("Missing required form field");
            }
        }
    }
    for (key, value) in values {
        let field = properties.get(key).ok_or("Unknown form field")?;
        if !matches_field(field, value) {
            return Err("Form value does not match its schema");
        }
    }
    Ok(())
}

fn matches_field(schema: &Value, value: &Value) -> bool {
    let valid = match schema["type"].as_str() {
        | None if schema.get("anyOf").is_some() || schema.get("oneOf").is_some() => value.is_string(),
        | Some("string") => value.as_str().is_some_and(|text| {
            let length = text.chars().count() as u64;
            !schema["minLength"].as_u64().is_some_and(|min| length < min) && !schema["maxLength"].as_u64().is_some_and(|max| length > max)
        }),
        | Some(kind @ ("integer" | "number")) => value.as_f64().is_some_and(|number| {
            (kind == "number" || number.fract() == 0.0)
                && !schema["minimum"].as_f64().is_some_and(|min| number < min)
                && !schema["maximum"].as_f64().is_some_and(|max| number > max)
        }),
        | Some("boolean") => value.is_boolean(),
        | Some("array") => value.as_array().is_some_and(|values| {
            let length = values.len() as u64;
            length <= 32
                && !schema["minItems"].as_u64().is_some_and(|min| length < min)
                && !schema["maxItems"].as_u64().is_some_and(|max| length > max)
                && values
                    .iter()
                    .enumerate()
                    .all(|(index, value)| value.is_string() && !values[.. index].contains(value) && matches_field(&schema["items"], value))
        }),
        | _ => false,
    };
    valid
        && schema.get("anyOf").is_none_or(|options| {
            options
                .as_array()
                .is_some_and(|options| options.iter().any(|option| option.get("const") == Some(value)))
        })
        && schema
            .get("enum")
            .is_none_or(|options| options.as_array().is_some_and(|options| options.contains(value)))
        && schema.get("oneOf").is_none_or(|options| {
            options
                .as_array()
                .is_some_and(|options| options.iter().filter(|option| option.get("const") == Some(value)).count() == 1)
        })
}
