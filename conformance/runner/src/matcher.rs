//! JSON matching for expectations.
//!
//! - Plain values must be equal (numbers compare as f64).
//! - `"$any"`, `"$string"`, `"$nonempty"`, `"$number"`, `"$int"`,
//!   `"$bool"`, `"$null"`, `"$array"`, `"$object"` match by type.
//! - `"$prefix:<p>"` matches a string starting with `<p>`;
//!   `"$substr:<s>"` one containing `<s>`.
//! - Objects match key by key and must have the same key set, unless
//!   the expectation has `"$open": true`. An expected value of
//!   `"$absent"` requires the key to be missing.
//! - An object with `"$each"`, `"$len"`, `"$min_len"` or `"$contains"`
//!   matches an array: every element, its length, or that each listed
//!   matcher matches some element.

use serde_json::Value;

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 200 { format!("{}…", &s[..200]) } else { s }
}

/// Matches `got` against `want`; the error names the JSON path.
pub fn matches(want: &Value, got: &Value) -> Result<(), String> {
    at("", want, got)
}

fn at(path: &str, want: &Value, got: &Value) -> Result<(), String> {
    let fail = |m: String| Err(format!("{}: {m}", if path.is_empty() { "$" } else { path }));
    if let Value::String(s) = want {
        let ok = match s.as_str() {
            "$any" => Some(true),
            "$string" => Some(got.is_string()),
            "$nonempty" => Some(got.as_str().is_some_and(|g| !g.is_empty())),
            "$number" => Some(got.is_number()),
            "$int" => Some(got.is_i64() || got.is_u64()),
            "$bool" => Some(got.is_boolean()),
            "$null" => Some(got.is_null()),
            "$array" => Some(got.is_array()),
            "$object" => Some(got.is_object()),
            _ => s
                .strip_prefix("$prefix:")
                .map(|p| got.as_str().is_some_and(|g| g.starts_with(p)))
                .or_else(|| s.strip_prefix("$substr:").map(|p| got.as_str().is_some_and(|g| g.contains(p)))),
        };
        if let Some(ok) = ok {
            return if ok { Ok(()) } else { fail(format!("want {s}, got {}", short(got))) };
        }
    }
    match want {
        Value::Object(w) if w.keys().any(|k| matches!(k.as_str(), "$each" | "$len" | "$min_len" | "$contains")) => {
            let Value::Array(g) = got else { return fail(format!("want array, got {}", type_name(got))) };
            if let Some(n) = w.get("$len").and_then(Value::as_u64)
                && g.len() as u64 != n
            {
                return fail(format!("want length {n}, got {}", g.len()));
            }
            if let Some(n) = w.get("$min_len").and_then(Value::as_u64)
                && (g.len() as u64) < n
            {
                return fail(format!("want length >= {n}, got {}", g.len()));
            }
            if let Some(each) = w.get("$each") {
                for (i, el) in g.iter().enumerate() {
                    at(&format!("{path}[{i}]"), each, el)?;
                }
            }
            if let Some(Value::Array(needles)) = w.get("$contains") {
                for (n, needle) in needles.iter().enumerate() {
                    if !g.iter().any(|el| matches(needle, el).is_ok()) {
                        return fail(format!("no element matches $contains[{n}] {}", short(needle)));
                    }
                }
            }
            Ok(())
        }
        Value::Object(w) => {
            let Value::Object(g) = got else { return fail(format!("want object, got {}", short(got))) };
            let open = w.get("$open").and_then(Value::as_bool).unwrap_or(false);
            for (k, wv) in w {
                if k == "$open" {
                    continue;
                }
                let p = format!("{path}.{k}");
                match (wv.as_str(), g.get(k)) {
                    (Some("$absent"), None) => {}
                    (Some("$absent"), Some(v)) => return Err(format!("{p}: want absent, got {}", short(v))),
                    (_, None) => return Err(format!("{p}: missing")),
                    (_, Some(gv)) => at(&p, wv, gv)?,
                }
            }
            if !open {
                for k in g.keys() {
                    if !w.contains_key(k) {
                        return Err(format!("{path}.{k}: unexpected key (value {})", short(&g[k])));
                    }
                }
            }
            Ok(())
        }
        Value::Array(w) => {
            let Value::Array(g) = got else { return fail(format!("want array, got {}", short(got))) };
            if w.len() != g.len() {
                return fail(format!("want {} elements, got {}: {}", w.len(), g.len(), short(got)));
            }
            for (i, (wv, gv)) in w.iter().zip(g).enumerate() {
                at(&format!("{path}[{i}]"), wv, gv)?;
            }
            Ok(())
        }
        Value::Number(w) => match got.as_f64() {
            Some(g) if Some(g) == w.as_f64() => Ok(()),
            _ => fail(format!("want {w}, got {}", short(got))),
        },
        _ => {
            if want == got {
                Ok(())
            } else {
                fail(format!("want {}, got {}", short(want), short(got)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matchers() {
        assert!(matches(&json!({"a": "$string", "b": "$absent"}), &json!({"a": "x"})).is_ok());
        assert!(matches(&json!({"a": "$string"}), &json!({"a": "x", "c": 1})).is_err());
        assert!(matches(&json!("$substr:ses=1&t"), &json!("seg.m4s?ses=1&t=x")).is_ok());
        assert!(matches(&json!("$substr:nope"), &json!("seg.m4s")).is_err());
        assert!(matches(&json!({"a": "$string", "$open": true}), &json!({"a": "x", "c": 1})).is_ok());
        assert!(
            matches(&json!({"$each": {"id": "$prefix:lib-"}, "$len": 2}), &json!([{"id": "lib-1"}, {"id": "lib-2"}]))
                .is_ok()
        );
        assert!(
            matches(
                &json!({"$contains": [{"id": "b", "$open": true}]}),
                &json!([{"id": "a", "x": 1}, {"id": "b", "x": 2}])
            )
            .is_ok()
        );
        assert!(matches(&json!(1), &json!(1.0)).is_ok());
        assert!(matches(&json!("$int"), &json!(1.5)).is_err());
        assert!(matches(&json!([1, 2]), &json!([1])).is_err());
        let e = matches(&json!({"a": {"b": 1}}), &json!({"a": {"b": 2}})).unwrap_err();
        assert!(e.starts_with(".a.b:"), "{e}");
    }
}
