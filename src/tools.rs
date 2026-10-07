//! Tool registry: the only way a Speran program can affect the outside world.
//! Hosts register tools (later: from MCP servers); programs must declare the
//! tools they use with `use`, and the host decides which ones are granted.

use crate::value::{Map, Value};
use indexmap::IndexMap;
use serde_json::Value as Json;

pub type ToolFn = Box<dyn FnMut(&Map) -> Result<Value, String>>;

#[derive(Default)]
pub struct ToolRegistry {
    tools: IndexMap<String, ToolFn>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool. Names must be namespaced, e.g. `github.get_issue`.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        f: impl FnMut(&Map) -> Result<Value, String> + 'static,
    ) -> &mut Self {
        self.tools.insert(name.into(), Box::new(f));
        self
    }

    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    pub fn call(&mut self, name: &str, args: &Map) -> Result<Value, String> {
        match self.tools.get_mut(name) {
            Some(f) => f(args),
            None => Err(format!("tool '{}' is not registered", name)),
        }
    }

    /// Build a registry of mock tools from JSON. Each key is a tool name and
    /// each value is the response. Special forms:
    /// - `{"$error": "message"}` makes the tool fail
    /// - `{"$match": "arg", "cases": {"1": ..., "2": ...}, "default": ...}`
    ///   picks a response based on an argument's value
    pub fn from_mock_json(j: &Json) -> Result<Self, String> {
        let obj = j
            .as_object()
            .ok_or("mock file must be a JSON object mapping tool names to responses")?;
        let mut reg = ToolRegistry::new();
        for (name, spec) in obj {
            if !name.contains('.') {
                return Err(format!(
                    "mock tool '{}' must be namespaced, e.g. 'demo.{}'",
                    name, name
                ));
            }
            let spec = spec.clone();
            reg.register(name.clone(), move |args| mock_response(&spec, args));
        }
        Ok(reg)
    }
}

fn mock_response(spec: &Json, args: &Map) -> Result<Value, String> {
    if let Some(obj) = spec.as_object() {
        if let Some(e) = obj.get("$error") {
            return Err(e.as_str().map(String::from).unwrap_or_else(|| e.to_string()));
        }
        if let Some(key) = obj.get("$match").and_then(Json::as_str) {
            let arg = args.get(key).map(Value::display).unwrap_or_default();
            if let Some(case) = obj.get("cases").and_then(Json::as_object).and_then(|c| c.get(&arg)) {
                return mock_response(case, args);
            }
            if let Some(d) = obj.get("default") {
                return mock_response(d, args);
            }
            return Err(format!("no mock response for {}={}", key, arg));
        }
    }
    Ok(Value::from_json(spec))
}

/// `None` grants every registered tool. Patterns: exact name, `ns.*`, or `*`.
pub fn is_granted(name: &str, grants: Option<&[String]>) -> bool {
    match grants {
        None => true,
        Some(g) => g.iter().any(|p| {
            p == "*"
                || p == name
                || (p.ends_with(".*") && name.starts_with(&p[..p.len() - 1]))
        }),
    }
}
