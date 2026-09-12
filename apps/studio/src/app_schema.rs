//! A deliberately small OpenAPI 3 JSON adapter. Unsupported contracts are visible,
//! but never silently invoked with guessed serialization or authentication.
use crate::err;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io};

#[derive(Clone, Serialize, Deserialize)]
pub struct Operation {
    pub operation: String,
    pub method: String,
    pub path: String,
    pub summary: String,
    pub parameters: Vec<Value>,
    pub body_required: bool,
    pub body_schema: Value,
    pub unsupported: Vec<String>,
}

fn resolve<'a>(doc: &'a Value, mut value: &'a Value) -> io::Result<&'a Value> {
    for _ in 0..16 {
        let Some(reference) = value.get("$ref") else {
            return Ok(value);
        };
        let pointer = reference
            .as_str()
            .and_then(|s| s.strip_prefix('#'))
            .filter(|s| s.starts_with('/'))
            .ok_or_else(|| err("Only document-local references are supported"))?;
        value = doc
            .pointer(pointer)
            .ok_or_else(|| err("Unresolved OpenAPI reference"))?;
    }
    Err(err("Cyclic or over-deep OpenAPI reference"))
}

pub fn discover(doc: &Value, base: &Url) -> io::Result<Vec<Operation>> {
    if !doc["openapi"]
        .as_str()
        .is_some_and(|v| v.starts_with("3.0.") || v.starts_with("3.1."))
    {
        return Err(err("Expected an OpenAPI 3.0 or 3.1 JSON document"));
    }
    let paths = doc["paths"]
        .as_object()
        .ok_or_else(|| err("OpenAPI paths are missing"))?;
    let mut operations = Vec::new();
    for (path, item) in paths {
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.len() > 2048
            || path.contains(['\\', '?', '#', '%'])
        {
            return Err(err("Unsupported OpenAPI path"));
        }
        let item = resolve(doc, item)?;
        for method in ["get", "post", "put", "patch", "delete"] {
            let Some(operation) = item.get(method) else {
                continue;
            };
            if !operation.is_object() {
                return Err(err("Invalid OpenAPI operation"));
            }
            if operations.len() >= 256 {
                return Err(err("API exceeds 256-operation limit"));
            }
            let mut unsupported = Vec::new();
            let servers = operation
                .get("servers")
                .or_else(|| item.get("servers"))
                .or_else(|| doc.get("servers"));
            if let Some(servers) = servers {
                // The connection origin is authoritative. Never follow schema hosts.
                if !servers.as_array().is_some_and(|entries| {
                    entries.is_empty()
                        || entries.iter().any(|entry| {
                            entry["url"]
                                .as_str()
                                .and_then(|s| base.join(s).ok())
                                .is_some_and(|url| url == *base)
                        })
                }) {
                    unsupported.push("Server URL differs from the saved origin".into());
                }
            }
            if let Some(security) = operation.get("security").or_else(|| doc.get("security"))
                && !security.as_array().is_some_and(|v| {
                    v.is_empty()
                        || v.iter()
                            .any(|s| s.as_object().is_some_and(|s| s.is_empty()))
                })
            {
                unsupported.push("Authentication must be implemented for this operation".into());
            }
            let mut parameters = BTreeMap::new();
            for owner in [item, operation] {
                if let Some(params) = owner.get("parameters") {
                    for param in params
                        .as_array()
                        .ok_or_else(|| err("Invalid parameter array"))?
                    {
                        let param = resolve(doc, param)?;
                        let location = param["in"]
                            .as_str()
                            .ok_or_else(|| err("Parameter location missing"))?;
                        let name = param["name"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 128)
                            .ok_or_else(|| err("Invalid parameter name"))?;
                        let schema = resolve(doc, &param["schema"])?;
                        let kind = schema["type"].as_str().unwrap_or("");
                        if !matches!(location, "path" | "query")
                            || !matches!(kind, "string" | "integer" | "number" | "boolean")
                            || param.get("content").is_some()
                            || param["style"].as_str().is_some_and(|s| {
                                s != if location == "path" { "simple" } else { "form" }
                            })
                        {
                            unsupported.push(format!(
                                "Unsupported serialization for {location} parameter {name}"
                            ));
                        }
                        parameters.insert((location.to_owned(),name.to_owned()),json!({"name":name,"in":location,"required":location == "path" || param["required"] == true,"type":kind}));
                    }
                }
            }
            let request = resolve(doc, &operation["requestBody"])?;
            let mut body_schema = Value::Null;
            if !request.is_null() {
                if let Some(media) = request["content"].get("application/json") {
                    body_schema = resolve(doc, &media["schema"])?.clone();
                } else {
                    unsupported.push("Only JSON request bodies are supported".into());
                }
            }
            if body_schema.to_string().len() > 2048 {
                body_schema = json!({"note":"Body schema is too large for the operation summary; inspect the original API document."});
            }
            for segment in path.split('/') {
                if segment.contains(['{', '}'])
                    && !(segment.starts_with('{')
                        && segment.ends_with('}')
                        && segment.matches('{').count() == 1
                        && segment.matches('}').count() == 1)
                {
                    unsupported.push("Path placeholders must occupy an entire segment".into());
                }
            }
            let compiled = Operation {
                operation: format!("{} {path}", method.to_uppercase()),
                method: method.to_uppercase(),
                path: path.clone(),
                summary: operation["summary"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(240)
                    .collect(),
                parameters: parameters.into_values().collect(),
                body_required: request["required"] == true,
                body_schema,
                unsupported,
            };
            if serde_json::to_string(&compiled).map_err(err)?.len() > 9000 {
                return Err(err(
                    "Operation metadata exceeds 9 KiB; inspect the original API document",
                ));
            }
            operations.push(compiled);
        }
    }
    if operations.is_empty() {
        return Err(err("No supported HTTP methods found in schema"));
    }
    Ok(operations)
}

pub fn request(operation: &Operation, base: &Url, action: &Value) -> io::Result<Value> {
    if !operation.unsupported.is_empty() {
        return Err(err(operation.unsupported.join("; ")));
    }
    let empty = serde_json::Map::new();
    let supplied = match action.get("parameters") {
        None => &empty,
        Some(value) => value
            .as_object()
            .ok_or_else(|| err("parameters must be an object with path/query objects"))?,
    };
    for (location, values) in supplied {
        if !matches!(location.as_str(), "path" | "query") {
            return Err(err("Unsupported parameter location"));
        }
        for name in values
            .as_object()
            .ok_or_else(|| err("Parameter locations must contain objects"))?
            .keys()
        {
            if !operation
                .parameters
                .iter()
                .any(|p| p["in"] == *location && p["name"] == *name)
            {
                return Err(err(format!("Unknown {location} parameter {name}")));
            }
        }
    }
    let mut values = BTreeMap::new();
    for parameter in &operation.parameters {
        let location = parameter["in"]
            .as_str()
            .ok_or_else(|| err("Invalid saved parameter"))?;
        let name = parameter["name"]
            .as_str()
            .ok_or_else(|| err("Invalid saved parameter"))?;
        let value = supplied.get(location).and_then(|v| v.get(name));
        let Some(value) = value else {
            if parameter["required"] == true {
                return Err(err(format!("Missing {location} parameter {name}")));
            }
            continue;
        };
        let valid = match parameter["type"].as_str() {
            Some("string") => value.is_string(),
            Some("integer") => value.is_i64() || value.is_u64(),
            Some("number") => value.is_number(),
            Some("boolean") => value.is_boolean(),
            _ => false,
        };
        if !valid {
            return Err(err(format!("Wrong type for {location} parameter {name}")));
        }
        let text = value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string());
        if text.len() > 2048 || (location == "path" && matches!(text.as_str(), "" | "." | "..")) {
            return Err(err("Invalid path/parameter value"));
        }
        values.insert((location, name), text);
    }
    let mut url = base.clone();
    {
        let mut segments = url.path_segments_mut().map_err(|_| err("Invalid origin"))?;
        segments.clear();
        for segment in operation
            .path
            .strip_prefix('/')
            .ok_or_else(|| err("Invalid operation path"))?
            .split('/')
        {
            if let Some(name) = segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                segments.push(
                    values
                        .get(&("path", name))
                        .ok_or_else(|| err("Missing path parameter declaration or value"))?,
                );
            } else {
                if matches!(segment, "." | "..") {
                    return Err(err("Dot path segments are unsupported"));
                }
                segments.push(segment);
            }
        }
    }
    for ((location, name), value) in values {
        if location == "query" {
            url.query_pairs_mut().append_pair(name, &value);
        }
    }
    if operation.body_required && action.get("body").is_none() {
        return Err(err("Missing JSON request body"));
    }
    let mut result = json!({"method":operation.method,"path":url.path().to_owned()+&url.query().map(|q| format!("?{q}")).unwrap_or_default()});
    if let Some(body) = action.get("body") {
        result["body"] = body.clone();
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> Value {
        json!({"openapi":"3.0.3","paths":{"/items/{id}":{"parameters":[{"$ref":"#/components/parameters/id"}],"post":{"parameters":[{"name":"search","in":"query","required":true,"schema":{"type":"string"}}],"requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object"}}}}}}},"components":{"parameters":{"id":{"name":"id","in":"path","required":true,"schema":{"type":"string"}}}}})
    }
    #[test]
    fn required_types_and_encoding_are_enforced() {
        let base = Url::parse("http://127.0.0.1:1234/").unwrap();
        let op = discover(&document(), &base).unwrap().remove(0);
        let action = json!({"parameters":{"path":{"id":"a/b?#"},"query":{"search":"a&admin=true"}},"body":{}});
        let call = request(&op, &base, &action).unwrap();
        assert_eq!(call["path"], "/items/a%2Fb%3F%23?search=a%26admin%3Dtrue");
        assert_eq!(call["method"], "POST");
        for bad in [
            json!({}),
            json!({"parameters":{"path":{"id":1}}}),
            json!({"parameters":{"path":{"id":".."},"query":{"search":"x"}},"body":{}}),
            json!({"parameters":{"path":{"id":"x"},"query":{"search":"x","undeclared":"x"}},"body":{}}),
            json!({"parameters":{"path":{"id":"x"},"query":{"search":"x"}}}),
        ] {
            assert!(request(&op, &base, &bad).is_err());
        }
    }
    #[test]
    fn unsupported_security_servers_and_serialization_cannot_be_invoked() {
        let base = Url::parse("http://127.0.0.1:1234/").unwrap();
        for extra in [
            json!({"security":[{"token":[]}]}),
            json!({"servers":[{"url":"https://example.com"}]}),
            json!({"servers":[{"url":"/api"}]}),
            json!({"parameters":[{"name":"token","in":"header","schema":{"type":"string"}}]}),
        ] {
            let mut doc = json!({"openapi":"3.1.0","paths":{"/ping":{"get":{}}}});
            doc["paths"]["/ping"]["get"] = extra;
            let op = discover(&doc, &base).unwrap().remove(0);
            assert!(!op.unsupported.is_empty());
            assert!(request(&op, &base, &json!({})).is_err());
        }
    }
    #[test]
    fn remote_and_cyclic_references_fail_without_fetching() {
        let base = Url::parse("http://127.0.0.1:1234/").unwrap();
        for reference in ["http://127.0.0.1:1/secret", "#/paths/~1ping"] {
            let doc = json!({"openapi":"3.0.3","paths":{"/ping":{"$ref":reference}}});
            assert!(discover(&doc, &base).is_err());
        }
    }
}
