//! JSON Schema (draft 2020-12) for decoded records, matching `Node::to_json`.

use serde_json::{Map, Value, json};

use crate::{CondValue, Item, PicCategory, Usage};

/// Schema for one record. Every property carries an `x-cobol` extension
/// (picture, usage, offset, length) so an API gateway can map JSON back to
/// bytes without re-reading the copybook.
pub fn json_schema(record: &Item) -> Value {
    let mut schema = item_schema(record);
    if let Value::Object(map) = &mut schema {
        map.insert(
            "$schema".into(),
            json!("https://json-schema.org/draft/2020-12/schema"),
        );
        map.insert("title".into(), json!(record.display_name()));
    }
    schema
}

fn item_schema(item: &Item) -> Value {
    let mut s = if item.is_group() {
        let mut props = Map::new();
        let mut required = Vec::new();
        for c in &item.children {
            if c.name.is_none() || c.redefines.is_some() {
                continue;
            }
            let child = match &c.occurs {
                Some(o) => {
                    // The OCCURS metadata describes the array, so x-cobol moves up to it.
                    let mut items = item_schema(c);
                    let mut ext = items
                        .as_object_mut()
                        .and_then(|m| m.remove("x-cobol"))
                        .unwrap_or_else(|| json!({}));
                    ext["occurs"] = json!({ "min": o.min, "max": o.max });
                    if let Some(d) = &o.depending_on {
                        ext["dependingOn"] = json!(d);
                    }
                    json!({
                        "type": "array",
                        "items": items,
                        "minItems": if o.depending_on.is_some() { o.min } else { o.max },
                        "maxItems": o.max,
                        "x-cobol": ext,
                    })
                }
                None => item_schema(c),
            };
            props.insert(c.display_name().to_string(), child);
            required.push(json!(c.display_name()));
        }
        json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
    } else {
        elementary_schema(item)
    };
    let mut ext = Map::new();
    ext.insert("level".into(), json!(item.level));
    ext.insert("offset".into(), json!(item.offset));
    ext.insert("length".into(), json!(item.size));
    if !item.is_group() {
        ext.insert("type".into(), json!(item.type_label()));
        ext.insert("usage".into(), json!(item.usage.label()));
    }
    if !item.conditions.is_empty() {
        let conds: Map<String, Value> = item
            .conditions
            .iter()
            .map(|c| {
                let vals: Vec<Value> = c
                    .values
                    .iter()
                    .map(|v| match v {
                        CondValue::Single(l) => json!(l.display()),
                        CondValue::Range(a, b) => {
                            json!(format!("{} THRU {}", a.display(), b.display()))
                        }
                    })
                    .collect();
                (c.name.clone(), Value::Array(vals))
            })
            .collect();
        ext.insert("conditions".into(), Value::Object(conds));
    }
    if let Value::Object(map) = &mut s {
        map.insert("x-cobol".into(), Value::Object(ext));
    }
    s
}

fn elementary_schema(item: &Item) -> Value {
    let pic = item.pic.as_ref();
    let numeric = matches!(item.usage, Usage::Packed | Usage::Binary | Usage::Comp5)
        || (item.usage == Usage::Display
            && pic.is_some_and(|p| p.category == PicCategory::Numeric));
    if matches!(item.usage, Usage::Comp1 | Usage::Comp2) {
        return json!({ "type": "number" });
    }
    if !numeric {
        let max = pic.map_or(item.size, |p| p.length);
        return json!({ "type": "string", "maxLength": max });
    }
    let p = pic.expect("numeric items have a PICTURE");
    let sign = if p.signed { "-?" } else { "" };
    if p.scale > 0 {
        let int_digits = (p.digits as i32 - p.scale).max(0);
        let int = if int_digits == 0 {
            "0".to_string()
        } else {
            format!("\\d{{1,{int_digits}}}")
        };
        return json!({
            "type": "string",
            "pattern": format!("^{sign}{int}\\.\\d{{{}}}$", p.scale),
            "description": "exact decimal as a string",
        });
    }
    let digits = p.digits as i32 - p.scale; // trailing P positions add digits
    if digits <= 15 {
        let max = 10i64.pow(digits as u32) - 1;
        json!({ "type": "integer", "minimum": if p.signed { -max } else { 0 }, "maximum": max })
    } else {
        json!({ "type": "string", "pattern": format!("^{sign}\\d{{1,{digits}}}$") })
    }
}
