//! A mock Jenkins server built from `openapi/jenkins.yaml`.
//!
//! For every `GET` operation in the spec, the mock serves the operation's
//! `200` example when the request carries the right basic-auth credentials.
//! Requests without them get the spec's `401` response, and paths that are
//! not in the spec get a 404, like Jenkins.
//!
//! Path parameters match one URL segment, except parameters marked
//! `x-jenkins-job-path: true`, which also match folder paths such as
//! `team-a/job/api-service`. An example with `x-mock-match` is only served
//! when the request's path parameters have those values.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde_json::Value;
use wiremock::matchers::{method, path_regex};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

pub const USERNAME: &str = "alice";
pub const PASSWORD: &str = "11d2c0ffee";

/// Matches one or more job names joined by `/job/`.
const JOB_PATH_PATTERN: &str = "[^/]+(?:/job/[^/]+)*";
const SEGMENT_PATTERN: &str = "[^/]+";

pub fn spec_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("openapi/jenkins.yaml")
}

pub fn load_spec() -> Value {
    let text = std::fs::read_to_string(spec_path()).expect("read OpenAPI spec");
    serde_norway::from_str(&text).expect("parse OpenAPI spec")
}

/// Follows a local `$ref` such as `#/components/parameters/tree`.
pub fn resolve<'a>(spec: &'a Value, value: &'a Value) -> &'a Value {
    let mut current = value;
    while let Some(reference) = current.get("$ref").and_then(Value::as_str) {
        let pointer = reference
            .strip_prefix('#')
            .unwrap_or_else(|| panic!("only local $refs are supported: {reference}"));
        current = spec
            .pointer(pointer)
            .unwrap_or_else(|| panic!("dangling $ref {reference}"));
    }
    current
}

/// A `GET` operation from the spec.
#[derive(Debug, Clone)]
pub struct Operation {
    pub id: String,
    pub path: String,
    /// Path parameter name and the regex it matches.
    pub params: Vec<(String, &'static str)>,
    pub content_type: String,
    pub examples: Vec<Example>,
}

#[derive(Debug, Clone)]
pub struct Example {
    pub name: String,
    pub value: Value,
    /// Path parameter values this example is limited to.
    pub matches: Vec<(String, String)>,
}

impl Operation {
    /// Regex for the operation's path, with fixed values for some parameters.
    pub fn path_regex(&self, prefix: &str, fixed: &[(String, String)]) -> String {
        let mut regex = format!("^{}", regex::escape(prefix));
        let mut rest = self.path.as_str();
        while let Some(start) = rest.find('{') {
            let end = rest[start..].find('}').expect("unclosed path parameter") + start;
            regex.push_str(&regex::escape(&rest[..start]));
            let name = &rest[start + 1..end];
            let pattern = match fixed.iter().find(|(n, _)| n == name) {
                Some((_, value)) => regex::escape(value),
                None => self
                    .params
                    .iter()
                    .find(|(n, _)| n == name)
                    .unwrap_or_else(|| panic!("{}: undeclared path parameter {name}", self.id))
                    .1
                    .to_string(),
            };
            regex.push_str(&format!("(?:{pattern})"));
            rest = &rest[end + 1..];
        }
        regex.push_str(&regex::escape(rest));
        regex.push('$');
        regex
    }
}

/// Reads every `GET` operation from the spec.
pub fn operations(spec: &Value) -> Vec<Operation> {
    let paths = spec["paths"].as_object().expect("spec has paths");
    let mut ops = Vec::new();
    for (path, item) in paths {
        let Some(get) = item.get("get") else { continue };
        let id = get["operationId"]
            .as_str()
            .unwrap_or_else(|| panic!("GET {path} has no operationId"))
            .to_string();

        let declared = item
            .get("parameters")
            .into_iter()
            .chain(get.get("parameters"))
            .flat_map(|list| list.as_array().expect("parameters is a list"))
            .map(|p| resolve(spec, p));
        let params = declared
            .filter(|p| p["in"] == "path")
            .map(|p| {
                let pattern = if p["x-jenkins-job-path"] == true {
                    JOB_PATH_PATTERN
                } else {
                    SEGMENT_PATTERN
                };
                (p["name"].as_str().unwrap().to_string(), pattern)
            })
            .collect();

        let ok = resolve(spec, &get["responses"]["200"]);
        let (content_type, media) = ok["content"]
            .as_object()
            .and_then(|c| c.iter().next())
            .unwrap_or_else(|| panic!("{id}: 200 response has no content"));

        let mut examples = Vec::new();
        if let Some(value) = media.get("example") {
            examples.push(Example {
                name: "example".into(),
                value: value.clone(),
                matches: Vec::new(),
            });
        }
        for (name, example) in media
            .get("examples")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let example = resolve(spec, example);
            let matches = example
                .get("x-mock-match")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().expect("string match").to_string()))
                        .collect()
                })
                .unwrap_or_default();
            examples.push(Example {
                name: name.clone(),
                value: example["value"].clone(),
                matches,
            });
        }
        assert!(
            examples.iter().any(|e| e.matches.is_empty()),
            "{id}: needs a 200 example without x-mock-match"
        );

        ops.push(Operation {
            id,
            path: path.clone(),
            params,
            content_type: content_type.clone(),
            examples,
        });
    }
    ops
}

/// Finds an operation's default example (the one without `x-mock-match`),
/// or a named one.
pub fn example(spec: &Value, operation_id: &str, name: Option<&str>) -> Value {
    let op = operations(spec)
        .into_iter()
        .find(|op| op.id == operation_id)
        .unwrap_or_else(|| panic!("no operation {operation_id}"));
    op.examples
        .into_iter()
        .find(|e| match name {
            Some(name) => e.name == name,
            None => e.matches.is_empty(),
        })
        .unwrap_or_else(|| panic!("{operation_id}: no example {name:?}"))
        .value
}

/// True when the request does not carry the expected basic-auth credentials.
struct WrongCredentials {
    expected: String,
}

impl Match for WrongCredentials {
    fn matches(&self, request: &Request) -> bool {
        request
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            != Some(self.expected.as_str())
    }
}

fn basic_auth_header(username: &str, password: &str) -> String {
    use base64::Engine as _;
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
    format!("Basic {encoded}")
}

fn response(content_type: &str, body: &Value) -> ResponseTemplate {
    let template = ResponseTemplate::new(200);
    if content_type == "application/json" {
        template.set_body_json(body)
    } else {
        let text = body.as_str().expect("non-JSON example must be a string");
        template.set_body_raw(
            text.as_bytes().to_vec(),
            &format!("{content_type}; charset=utf-8"),
        )
    }
}

/// A running mock server built from the spec.
pub struct JenkinsMock {
    pub server: MockServer,
    pub spec: Value,
    /// Path the API is served under, like a CloudBees controller's `/my-controller`.
    pub prefix: String,
}

impl JenkinsMock {
    /// Starts a mock that serves the spec at the server root.
    pub async fn start() -> Self {
        Self::start_at("").await
    }

    /// Starts a mock that serves the spec under `prefix`, for example `/my-controller`.
    pub async fn start_at(prefix: &str) -> Self {
        let prefix = prefix.trim_end_matches('/').to_string();
        let spec = load_spec();
        let server = MockServer::start().await;

        let unauthorized = resolve(&spec, &spec["components"]["responses"]["Unauthorized"]);
        let unauthorized_body = unauthorized["content"]["text/html"]["example"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Mock::given(WrongCredentials {
            expected: basic_auth_header(USERNAME, PASSWORD),
        })
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("WWW-Authenticate", "Basic realm=\"Jenkins\"")
                .set_body_raw(unauthorized_body, "text/html"),
        )
        .with_priority(1)
        .mount(&server)
        .await;

        for op in operations(&spec) {
            for example in &op.examples {
                // Examples tied to specific parameter values win over the default.
                let priority = if example.matches.is_empty() { 5 } else { 2 };
                Mock::given(method("GET"))
                    .and(path_regex(op.path_regex(&prefix, &example.matches)))
                    .respond_with(response(&op.content_type, &example.value))
                    .with_priority(priority)
                    .named(format!("{} ({})", op.id, example.name))
                    .mount(&server)
                    .await;
            }
        }

        Self {
            server,
            spec,
            prefix,
        }
    }

    /// Base URL to give the CLI.
    pub fn uri(&self) -> String {
        format!("{}{}/", self.server.uri(), self.prefix)
    }

    /// The default example for an operation.
    pub fn example(&self, operation_id: &str) -> Value {
        example(&self.spec, operation_id, None)
    }

    /// A named example for an operation.
    pub fn named_example(&self, operation_id: &str, name: &str) -> Value {
        example(&self.spec, operation_id, Some(name))
    }

    /// Requests the mock has received.
    pub async fn requests(&self) -> Vec<wiremock::Request> {
        self.server.received_requests().await.unwrap_or_default()
    }
}
