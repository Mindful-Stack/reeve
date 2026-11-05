/// The OpenAPI document is generated from the served routers. A change here is a deliberate wire change:
/// run with UPDATE_SNAPSHOTS=1 to accept it, and the generated client types in plan B are rebuilt from it.
#[test]
fn openapi_matches_snapshot() {
    let doc = serde_json::to_string_pretty(&reeve_server::openapi_json()).unwrap();
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/openapi.json");
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        std::fs::write(&path, &doc).unwrap();
        return;
    }
    let stored =
        std::fs::read_to_string(&path).expect("snapshot exists; run with UPDATE_SNAPSHOTS=1 once");
    assert_eq!(
        stored, doc,
        "OpenAPI document changed; if intended, rerun with UPDATE_SNAPSHOTS=1"
    );
}

/// Every `$ref` in the document must resolve to a component; a generic instantiation that utoipa cannot name shows up here first.
#[test]
fn openapi_refs_resolve() {
    let doc = reeve_server::openapi_json();
    let names: std::collections::HashSet<String> = doc["components"]["schemas"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    fn walk(
        v: &serde_json::Value,
        names: &std::collections::HashSet<String>,
        missing: &mut Vec<String>,
    ) {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(r) = m.get("$ref").and_then(|r| r.as_str()) {
                    let n = r.rsplit('/').next().unwrap();
                    if !names.contains(n) {
                        missing.push(n.to_string());
                    }
                }
                for x in m.values() {
                    walk(x, names, missing);
                }
            }
            serde_json::Value::Array(a) => {
                for x in a {
                    walk(x, names, missing);
                }
            }
            _ => {}
        }
    }
    let mut missing = vec![];
    walk(&doc, &names, &mut missing);
    assert!(missing.is_empty(), "dangling $ref: {missing:?}");
}

/// Every operation requires the bearer scheme and documents the 401 envelope.
#[test]
fn every_operation_is_secured_and_documents_401() {
    let doc = reeve_server::openapi_json();
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            assert!(
                op["security"].as_array().is_some_and(|s| !s.is_empty()),
                "{method} {path} has no security requirement"
            );
            assert!(
                op["responses"]["401"].is_object(),
                "{method} {path} has no 401"
            );
        }
    }
}

/// List filters are query parameters: a path without a `{placeholder}` has no path parameters, and every `/stories`
/// filter is declared `in: query` (the custom `ApiQuery` extractor gives utoipa no location to infer).
#[test]
fn list_filters_are_query_parameters() {
    let doc = reeve_server::openapi_json();
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            for p in op["parameters"].as_array().into_iter().flatten() {
                if p["in"] == "path" {
                    let name = p["name"].as_str().unwrap();
                    assert!(
                        path.contains(&format!("{{{name}}}")),
                        "{method} {path} declares path parameter {name} without a placeholder"
                    );
                }
            }
        }
    }
    let stories = doc["paths"]["/api/v1/stories"]["get"]["parameters"]
        .as_array()
        .expect("GET /stories has parameters");
    assert!(!stories.is_empty());
    for p in stories {
        assert_eq!(p["in"], "query", "{p}");
    }
}

/// Generated clients key operations by id; a shared id gives one path another path's types (web/src/lib/api/schema.d.ts).
#[test]
fn operation_ids_are_unique() {
    let doc = reeve_server::openapi_json();
    let mut ids = std::collections::HashMap::<String, String>::new();
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            let id = op["operationId"]
                .as_str()
                .unwrap_or_else(|| panic!("{method} {path} has no operationId"));
            if let Some(prev) = ids.insert(id.to_string(), format!("{method} {path}")) {
                panic!("operationId {id} is shared by {prev} and {method} {path}");
            }
        }
    }
}

/// A non-admin's `accepted` check is refused with 403 and the error envelope; the document says so, so a client built
/// from it can read the envelope. (The global modifier in routes/mod.rs adds 400, 401, 404, 409 and 422, never 403.)
#[test]
fn the_check_route_documents_403() {
    let doc = reeve_server::openapi_json();
    let r = &doc["paths"]["/api/v1/decisions/{id}/check"]["post"]["responses"]["403"];
    assert_eq!(
        r["content"]["application/json"]["schema"]["$ref"], "#/components/schemas/Response_Empty",
        "{r}"
    );
}
