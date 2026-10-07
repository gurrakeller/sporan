//! Runtime values. Everything is JSON-compatible and immutable; `Rc` makes
//! copying values cheap since nothing is ever mutated in place.

use indexmap::IndexMap;
use serde_json::Value as Json;
use std::rc::Rc;

pub type Map = IndexMap<String, Value>;

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(Rc<str>),
    List(Rc<Vec<Value>>),
    Map(Rc<Map>),
}

impl Value {
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(Rc::from(s.into()))
    }

    pub fn list(v: Vec<Value>) -> Value {
        Value::List(Rc::new(v))
    }

    pub fn map(m: Map) -> Value {
        Value::Map(Rc::new(m))
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::List(_) => "list",
            Value::Map(_) => "map",
        }
    }

    /// `false`, `null`, `0`, `""`, `[]` and `{}` are falsy.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Map(m) => !m.is_empty(),
        }
    }

    pub fn to_json(&self) -> Json {
        match self {
            Value::Null => Json::Null,
            Value::Bool(b) => Json::Bool(*b),
            Value::Num(n) => num_to_json(*n),
            Value::Str(s) => Json::String(s.to_string()),
            Value::List(l) => Json::Array(l.iter().map(Value::to_json).collect()),
            Value::Map(m) => {
                Json::Object(m.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
            }
        }
    }

    pub fn from_json(j: &Json) -> Value {
        match j {
            Json::Null => Value::Null,
            Json::Bool(b) => Value::Bool(*b),
            Json::Number(n) => Value::Num(n.as_f64().unwrap_or(0.0)),
            Json::String(s) => Value::str(s.clone()),
            Json::Array(a) => Value::list(a.iter().map(Value::from_json).collect()),
            Json::Object(o) => {
                Value::map(o.iter().map(|(k, v)| (k.clone(), Value::from_json(v))).collect())
            }
        }
    }

    /// JSON representation, e.g. `"hi"`, `[1, 2]`.
    pub fn repr(&self) -> String {
        serde_json::to_string(&self.to_json()).unwrap_or_default()
    }

    /// Human representation used by `str()` and interpolation: strings are raw.
    pub fn display(&self) -> String {
        match self {
            Value::Str(s) => s.to_string(),
            _ => self.repr(),
        }
    }
}

pub fn num_to_json(n: f64) -> Json {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.0e15 {
        Json::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map(Json::Number).unwrap_or(Json::Null)
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => {
                a.len() == b.len() && a.iter().all(|(k, v)| b.get(k) == Some(v))
            }
            _ => false,
        }
    }
}
