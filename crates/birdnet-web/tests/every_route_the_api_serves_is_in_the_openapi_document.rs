//! Every route the API serves under `/api/v2` is in `openapi.json`, every path
//! `openapi.json` names is served, and the document is structurally sound.
//!
//! # What was wrong
//!
//! `openapi.json` is hand-maintained and its only completeness check ran one
//! way: `every_documented_path_is_routed` asks whether each *documented* path
//! is routed. Nothing asked the other question, so ten routed paths were
//! absent from the document for as long as nobody noticed — the API index,
//! effort-corrected abundance, phenology, the Raven export and the two
//! per-clip label tracks, the sound level, species tracking and both
//! web sockets — and `docs/book/reference/api.md` said so in prose, which is a
//! count nothing kept true. The one converse gate (`every_bearer_route_is_
//! documented_as_bearer_gated`) covered the eighteen token-gated routes
//! because it could read them from a table. The other ~70 had no table.
//!
//! # Where the list of routes comes from
//!
//! From the router itself. axum has no public route listing, but `Router`'s
//! `Debug` implementation prints every registered path and, for each, which
//! methods have a handler, and `nest` flattens a nested router's routes into
//! full paths. This reads that. It is not a stable API, so the first test
//! below is a check on the *reader*: it must recover every route in the
//! bearer-gated tables (`WRITE_ROUTES`, `READ_ROUTES`) — which are separate
//! source of truth — plus a handful of well-known ones, and it must recover
//! as many routes as the output has `RouteId(` entries. If axum changes the
//! format, that test fails by name instead of the gate passing on an empty
//! set.
//!
//! The router read is every router `server::build_router` merges —
//! `public_routes`, `admin_routes`, the mutating page routes and the bearer
//! router — without their layers, so a route cannot be public, admin-only or
//! token-gated and escape.
//!
//! # What this holds
//!
//! 1. Every served `/api/v2` (method, path) is documented, or is in
//!    [`NOT_DOCUMENTED`] with a reason. A stale exemption fails too.
//! 2. Every documented (method, path) is served. (A documented operation on a
//!    path that does not take that method is as wrong as a missing path.)
//! 3. The document is structurally valid OpenAPI 3.x as far as a small checker
//!    can tell: every `$ref` resolves, every path parameter in a template is
//!    declared (and every declared one is in the template), every operation has
//!    a response and every response a description, `operationId`s are unique,
//!    named security schemes and tags exist. **Redocly itself was not run** — it
//!    is not installed here — and this is not a substitute for its rules.
//! 4. The checker is not vacuous: each rule fails on a document broken in
//!    exactly that way.

use std::collections::{BTreeMap, BTreeSet};

use birdnet_web::routes;
use serde_json::{Value, json};

const SPEC: &str = include_str!("../openapi.json");

/// Methods OpenAPI path items may carry that this API uses. `HEAD` is axum's
/// implicit companion of `GET` and is not documented separately.
const METHODS: [&str; 5] = ["get", "put", "post", "delete", "patch"];

/// Routed under `/api/v2` and deliberately not in the document.
///
/// Each entry: method, the served path, and why a client should not be told.
/// Empty today: every route the API serves is documented. An entry here is a
/// decision, so it carries its reason, and a stale one (the route was removed,
/// or got documented) fails `an_exemption_is_still_needed`.
const NOT_DOCUMENTED: &[(&str, &str, &str)] = &[];

/// One served or documented operation: `GET /api/v2/health`, with every
/// path parameter name replaced by `{}` so the comparison is about the shape of
/// the path and not about what a document chose to call a placeholder.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Op {
    method: String,
    path: String,
}

impl std::fmt::Display for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.method, self.path)
    }
}

/// `/a/{b}/c/{*d}` -> `/a/{}/c/{*}`.
fn shape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars();
    while let Some(c) = chars.next() {
        if c == '{' {
            let wildcard = path_is_wildcard(&mut chars);
            out.push_str(if wildcard { "{*}" } else { "{}" });
        } else {
            out.push(c);
        }
    }
    out
}

/// Consume a placeholder's name up to its `}`; whether it was a `{*name}`.
fn path_is_wildcard(chars: &mut std::str::Chars<'_>) -> bool {
    let mut first = true;
    let mut wildcard = false;
    for c in chars.by_ref() {
        if c == '}' {
            break;
        }
        if first && c == '*' {
            wildcard = true;
        }
        first = false;
    }
    wildcard
}

// ---------------------------------------------------------------------------
// Reading the routes the router serves
// ---------------------------------------------------------------------------

/// Every router the server serves under one `Router`, with no layers.
fn everything() -> axum::Router<birdnet_web::state::AppState> {
    routes::public_routes()
        .merge(routes::admin_routes())
        .merge(routes::pages::mutating_router())
        .merge(routes::api_write::router())
}

/// What `Router`'s `Debug` output says it serves: `(path, methods)`.
///
/// `methods` is empty for an endpoint that is a nested service and carries no
/// per-method information; the caller treats that as "any method".
fn read_routes(debug: &str) -> Vec<(String, Vec<String>)> {
    // The router's own routes come before its fallback router; the fallback
    // is `/` and a catch-all, which are not API routes.
    let main = debug
        .split("fallback_router:")
        .next()
        .expect("split yields one part");
    let paths_at = main
        .find("node: Node { paths: {")
        .expect("the Debug output no longer lists the router's paths");
    let routes_at = main
        .find("routes: {")
        .expect("the Debug output no longer lists the router's routes");

    let id_of =
        |chunk: &str| -> String { chunk.split(')').next().expect("an id").trim().to_owned() };

    let mut path_of: BTreeMap<String, String> = BTreeMap::new();
    for chunk in main[paths_at..].split("RouteId(").skip(1) {
        let Some(open) = chunk.find('"') else {
            continue;
        };
        let rest = &chunk[open + 1..];
        let Some(close) = rest.find('"') else {
            continue;
        };
        path_of.insert(id_of(chunk), rest[..close].to_owned());
    }

    let mut out = Vec::new();
    for chunk in main[routes_at..paths_at].split("RouteId(").skip(1) {
        let id = id_of(chunk);
        let path = path_of
            .get(&id)
            .unwrap_or_else(|| panic!("RouteId({id}) has endpoint but no path"))
            .clone();
        let mut methods = Vec::new();
        if chunk.contains("MethodRouter(MethodRouter {") {
            for m in ["get", "post", "put", "delete", "patch"] {
                // `get: BoxedHandler` / `get: Route`; absent is `get: None`.
                if chunk.contains(&format!("{m}: BoxedHandler"))
                    || chunk.contains(&format!("{m}: Route"))
                {
                    methods.push(m.to_uppercase());
                }
            }
        }
        out.push((path, methods));
    }
    out
}

/// Every served operation under `/api/v2`, by shape.
fn served() -> BTreeSet<Op> {
    let debug = format!("{:?}", everything());
    let mut ops = BTreeSet::new();
    for (path, methods) in read_routes(&debug) {
        if !path.starts_with("/api/v2") {
            continue;
        }
        // A nested service has no method list; every method the document may
        // use is then "served" for the purpose of the reverse check, and for
        // the forward check the route is simply reported as `ANY`.
        let methods = if methods.is_empty() {
            vec!["ANY".to_owned()]
        } else {
            methods
        };
        for method in methods {
            ops.insert(Op {
                method,
                path: shape(&path),
            });
        }
    }
    ops
}

// ---------------------------------------------------------------------------
// Reading the document
// ---------------------------------------------------------------------------

fn spec() -> Value {
    serde_json::from_str(SPEC).expect("openapi.json is JSON")
}

/// The prefix a path item's URL is appended to: its own `servers[0]` if it has
/// one, else the document's.
///
/// A path item that carries its own `servers` is how the document names the API
/// root exactly: the index is `/api/v2` and `/api/v2/` is a 404, and an OpenAPI
/// path must begin with `/`, so `/` under the `/api/v2` server would name the
/// wrong URL.
fn prefix_of<'a>(doc: &'a Value, item: &'a Value) -> &'a str {
    item["servers"][0]["url"]
        .as_str()
        .or_else(|| doc["servers"][0]["url"].as_str())
        .expect("the document has a server URL")
}

/// Every documented operation, by shape, with the prefix applied.
fn documented(doc: &Value) -> BTreeSet<Op> {
    let mut ops = BTreeSet::new();
    for (path, item) in doc["paths"].as_object().expect("paths") {
        let url = format!("{}{path}", prefix_of(doc, item));
        for method in METHODS {
            if item.get(method).is_some() {
                ops.insert(Op {
                    method: method.to_uppercase(),
                    path: shape(&url),
                });
            }
        }
    }
    ops
}

// ---------------------------------------------------------------------------
// The structural checker
// ---------------------------------------------------------------------------

/// Resolve a local `$ref` (`#/components/schemas/Error`) against `doc`.
fn resolve<'a>(doc: &'a Value, reference: &str) -> Option<&'a Value> {
    let pointer = reference.strip_prefix('#')?;
    doc.pointer(pointer)
}

/// Every `$ref` string anywhere in the document, with where it was found.
fn refs(value: &Value, at: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if k == "$ref" {
                    if let Some(s) = v.as_str() {
                        out.push((at.to_owned(), s.to_owned()));
                    }
                } else {
                    refs(v, &format!("{at}/{k}"), out);
                }
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                refs(v, &format!("{at}/{i}"), out);
            }
        }
        _ => {}
    }
}

/// `{name}` placeholders in a path template, in order.
fn template_params(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        out.push(rest[open + 1..open + close].to_owned());
        rest = &rest[open + close + 1..];
    }
    out
}

/// The parameters an operation has, `$ref`s resolved, as `(name, in, required)`.
fn parameters(
    doc: &Value,
    item: &Value,
    op: &Value,
    problems: &mut Vec<String>,
    at: &str,
) -> Vec<(String, String, bool)> {
    let mut out: Vec<(String, String, bool)> = Vec::new();
    for source in [&item["parameters"], &op["parameters"]] {
        for p in source.as_array().into_iter().flatten() {
            let resolved = match p["$ref"].as_str() {
                Some(r) => match resolve(doc, r) {
                    Some(v) => v,
                    None => continue, // reported by the $ref check
                },
                None => p,
            };
            let (Some(name), Some(place)) = (resolved["name"].as_str(), resolved["in"].as_str())
            else {
                problems.push(format!("{at}: a parameter has no name or no `in`"));
                continue;
            };
            let required = resolved["required"].as_bool().unwrap_or(false);
            // An operation-level parameter overrides a path-level one.
            out.retain(|(n, i, _)| !(n == name && i == place));
            out.push((name.to_owned(), place.to_owned(), required));
        }
    }
    out
}

/// Every structural problem in `doc`, phrased with where it is.
#[allow(clippy::too_many_lines)]
fn problems_in(doc: &Value) -> Vec<String> {
    let mut problems = Vec::new();

    if !doc["openapi"].as_str().is_some_and(|v| v.starts_with("3.")) {
        problems.push("`openapi` is not a 3.x version string".to_owned());
    }
    for key in ["title", "version"] {
        if doc["info"][key].as_str().is_none_or(str::is_empty) {
            problems.push(format!("`info.{key}` is missing"));
        }
    }

    let mut all_refs = Vec::new();
    refs(doc, "#", &mut all_refs);
    for (at, reference) in &all_refs {
        if !reference.starts_with("#/") {
            problems.push(format!("{at}: `{reference}` is not a local reference"));
        } else if resolve(doc, reference).is_none() {
            problems.push(format!("{at}: `$ref` {reference} does not resolve"));
        }
    }

    let schemes: BTreeSet<&str> = doc["components"]["securitySchemes"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k.as_str())
        .collect();
    let tags: BTreeSet<&str> = doc["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str())
        .collect();

    let mut operation_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut shapes: BTreeMap<String, String> = BTreeMap::new();
    for (path, item) in doc["paths"].as_object().expect("paths") {
        if !path.starts_with('/') {
            problems.push(format!("path `{path}` does not begin with `/`"));
        }
        if path.contains('?') {
            problems.push(format!("path `{path}` carries a query string"));
        }
        if let Some(other) = shapes.insert(shape(path), path.clone()) {
            problems.push(format!(
                "paths `{other}` and `{path}` differ only in their placeholder names"
            ));
        }
        let in_template = template_params(path);

        for method in METHODS {
            let Some(op) = item.get(method) else { continue };
            let at = format!("{} {path}", method.to_uppercase());

            let declared = parameters(doc, item, op, &mut problems, &at);
            let declared_path: BTreeSet<&str> = declared
                .iter()
                .filter(|(_, place, _)| place == "path")
                .map(|(n, _, _)| n.as_str())
                .collect();
            for name in &in_template {
                if !declared_path.contains(name.as_str()) {
                    problems.push(format!(
                        "{at}: `{{{name}}}` is in the path but not declared as a path parameter"
                    ));
                }
            }
            for name in &declared_path {
                if !in_template.iter().any(|t| t == name) {
                    problems.push(format!("{at}: path parameter `{name}` is not in the path"));
                }
            }
            for (name, place, required) in &declared {
                if place == "path" && !required {
                    problems.push(format!(
                        "{at}: path parameter `{name}` is not `required: true`"
                    ));
                }
            }

            match op["responses"].as_object() {
                Some(responses) if !responses.is_empty() => {
                    for (code, response) in responses {
                        let ok_code = code == "default"
                            || (code.len() == 3
                                && code
                                    .bytes()
                                    .next()
                                    .is_some_and(|b| (b'1'..=b'5').contains(&b))
                                && code[1..].bytes().all(|b| b.is_ascii_digit() || b == b'X'));
                        if !ok_code {
                            problems.push(format!("{at}: `{code}` is not a status code"));
                        }
                        if response["$ref"].is_null() && response["description"].as_str().is_none()
                        {
                            problems.push(format!("{at}: response {code} has no description"));
                        }
                    }
                }
                _ => problems.push(format!("{at}: no responses")),
            }

            if op["summary"].as_str().is_none_or(str::is_empty) {
                problems.push(format!("{at}: no summary"));
            }
            match op["operationId"].as_str() {
                None => problems.push(format!("{at}: no operationId")),
                Some(id) => {
                    if let Some(other) = operation_ids.insert(id.to_owned(), at.clone()) {
                        problems.push(format!("{at}: operationId `{id}` is also {other}"));
                    }
                }
            }
            for tag in op["tags"].as_array().into_iter().flatten() {
                if let Some(tag) = tag.as_str()
                    && !tags.contains(tag)
                {
                    problems.push(format!("{at}: tag `{tag}` is not declared"));
                }
            }
            for requirement in op["security"].as_array().into_iter().flatten() {
                for name in requirement
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(k, _)| k)
                {
                    if !schemes.contains(name.as_str()) {
                        problems.push(format!("{at}: security scheme `{name}` is not defined"));
                    }
                }
            }
            if let Some(body) = op.get("requestBody")
                && body["$ref"].is_null()
                && body["content"]
                    .as_object()
                    .is_none_or(serde_json::Map::is_empty)
            {
                problems.push(format!("{at}: requestBody has no content"));
            }
        }
    }
    problems
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// The reader reads what the router serves — checked against sources that do
/// not depend on it.
#[test]
fn the_route_reader_sees_every_route_it_should() {
    let debug = format!("{:?}", everything());
    let found = read_routes(&debug);
    let ids = debug
        .split("fallback_router:")
        .next()
        .unwrap_or_default()
        .matches("routes: {")
        .count();
    assert_eq!(ids, 1, "the Debug output's shape changed");
    // As many (path, methods) entries as paths the output lists.
    let listed = debug
        .split("fallback_router:")
        .next()
        .unwrap_or_default()
        .split("node: Node { paths: {")
        .nth(1)
        .expect("paths")
        .matches("RouteId(")
        .count();
    assert_eq!(
        found.len(),
        listed,
        "a route was lost between the two listings"
    );
    assert!(found.len() > 150, "only {} routes found", found.len());

    let ops = served();
    // The bearer-gated routes come from tables of their own.
    for (method, path) in routes::api_write::WRITE_ROUTES
        .iter()
        .chain(routes::api_write::READ_ROUTES)
    {
        let op = Op {
            method: (*method).to_uppercase(),
            path: shape(path),
        };
        assert!(
            ops.contains(&op),
            "{op} is in the bearer route tables and the reader did not find it"
        );
    }
    // And well-known ones from each router that is nested under /api/v2.
    for (method, path) in [
        ("GET", "/api/v2/health"),
        ("GET", "/api/v2/detections"),
        ("GET", "/api/v2/species/top"),
        ("GET", "/api/v2/analytics/sessions"),
        ("GET", "/api/v2/recordings/{filename}"),
        ("GET", "/api/v2/openapi.json"),
        ("GET", "/api/v2/ws/detections"),
        ("POST", "/api/v2/detections/review"),
    ] {
        let op = Op {
            method: method.to_owned(),
            path: shape(path),
        };
        assert!(ops.contains(&op), "{op} was not found: the reader is wrong");
    }
    // Pages are not API routes.
    assert!(ops.iter().all(|o| o.path.starts_with("/api/v2")));
}

#[test]
fn every_route_the_api_serves_is_documented() {
    let doc = spec();
    let documented = documented(&doc);
    let exempt: BTreeSet<Op> = NOT_DOCUMENTED
        .iter()
        .map(|(m, p, _)| Op {
            method: (*m).to_owned(),
            path: shape(p),
        })
        .collect();
    let missing: Vec<String> = served()
        .into_iter()
        .filter(|op| !documented.contains(op) && !exempt.contains(op))
        .map(|op| op.to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "{} route(s) are served under /api/v2 and are not in openapi.json, so every generated \
         client is blind to them. Document them, or name them in NOT_DOCUMENTED with a reason:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

#[test]
fn every_documented_operation_is_served() {
    let doc = spec();
    let served = served();
    let served_any: BTreeSet<&str> = served
        .iter()
        .filter(|o| o.method == "ANY")
        .map(|o| o.path.as_str())
        .collect();
    let phantom: Vec<String> = documented(&doc)
        .into_iter()
        .filter(|op| !served.contains(op) && !served_any.contains(op.path.as_str()))
        .map(|op| op.to_string())
        .collect();
    assert!(
        phantom.is_empty(),
        "{} documented operation(s) are not served by any route:\n  {}",
        phantom.len(),
        phantom.join("\n  ")
    );
}

/// A stale exemption is a route that is gone or has since been documented: the
/// list would then excuse nothing, and the next route added there would be
/// excused by accident.
#[test]
fn an_exemption_is_still_needed() {
    let doc = spec();
    let documented = documented(&doc);
    let served = served();
    for (method, path, reason) in NOT_DOCUMENTED {
        assert!(!reason.trim().is_empty(), "{method} {path} has no reason");
        let op = Op {
            method: (*method).to_owned(),
            path: shape(path),
        };
        assert!(served.contains(&op), "{op} is exempt but no longer served");
        assert!(
            !documented.contains(&op),
            "{op} is exempt but is documented"
        );
    }
}

#[test]
fn the_document_is_structurally_valid() {
    let doc = spec();
    let problems = problems_in(&doc);
    assert!(
        problems.is_empty(),
        "{} problem(s) in openapi.json (redocly itself was not run):\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
    // The checker looked at something.
    let mut all_refs = Vec::new();
    refs(&doc, "#", &mut all_refs);
    assert!(
        all_refs.len() > 100,
        "only {} $refs checked",
        all_refs.len()
    );
    assert!(documented(&doc).len() > 80);
}

/// The checker finds each defect it claims to: a document broken in exactly
/// one way is reported in exactly that way. Without this a checker that
/// returned nothing would pass the test above.
#[test]
fn the_checker_finds_each_kind_of_defect() {
    let base = json!({
        "openapi": "3.1.0",
        "info": {"title": "t", "version": "1"},
        "servers": [{"url": "/api/v2"}],
        "tags": [{"name": "x", "description": "d"}],
        "paths": {
            "/things/{id}": {
                "get": {
                    "tags": ["x"], "summary": "s", "operationId": "getThing",
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                    "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Thing"}}}}}
                }
            }
        },
        "components": {"schemas": {"Thing": {"type": "object"}}}
    });
    assert!(problems_in(&base).is_empty(), "{:?}", problems_in(&base));

    let broken = |edit: &dyn Fn(&mut Value), needle: &str| {
        let mut doc = base.clone();
        edit(&mut doc);
        let problems = problems_in(&doc);
        assert!(
            problems.iter().any(|p| p.contains(needle)),
            "a document broken as `{needle}` was not reported: {problems:?}"
        );
    };
    broken(
        &|d| {
            d["paths"]["/things/{id}"]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"]["$ref"] = json!("#/components/schemas/Nope");
        },
        "does not resolve",
    );
    broken(
        &|d| {
            d["paths"]["/things/{id}"]["get"]["parameters"] = json!([]);
        },
        "not declared as a path parameter",
    );
    broken(
        &|d| {
            d["paths"]["/things/{id}"]["get"]["parameters"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name": "other", "in": "path", "required": true, "schema": {"type": "string"}}));
        },
        "is not in the path",
    );
    broken(
        &|d| {
            d["paths"]["/things/{id}"]["get"]["parameters"][0]["required"] = json!(false);
        },
        "not `required: true`",
    );
    broken(
        &|d| {
            d["paths"]["/things/{id}"]["get"]["responses"] = json!({});
        },
        "no responses",
    );
    broken(
        &|d| {
            d["paths"]["/things/{id}"]["get"]["responses"]["200"] = json!({"content": {}});
        },
        "has no description",
    );
    broken(
        &|d| {
            d["paths"]["/things/{id}"]["get"]["tags"] = json!(["undeclared"]);
        },
        "is not declared",
    );
    broken(
        &|d| {
            d["paths"]["/things/{id}"]["get"]["security"] = json!([{"nope": []}]);
        },
        "is not defined",
    );
    broken(
        &|d| {
            d["paths"]["/other/{id}"] = d["paths"]["/things/{id}"].clone();
            d["paths"]["/other/{id}"]["get"]["operationId"] = json!("getThing");
        },
        "is also",
    );
    broken(
        &|d| {
            d["paths"]["/things/{name}"] = d["paths"]["/things/{id}"].clone();
        },
        "differ only in their placeholder names",
    );
    broken(
        &|d| {
            d["openapi"] = json!("2.0");
        },
        "3.x",
    );
    broken(
        &|d| {
            d["paths"]["/things/{id}"]["get"]
                .as_object_mut()
                .unwrap()
                .remove("operationId");
        },
        "no operationId",
    );
}
