//! Speran: A safe, token-efficient language designed for LLM agents.

pub mod ast;
pub mod builtins;
pub mod checker;
pub mod error;
pub mod interp;
pub mod lexer;
pub mod parser;
pub mod tools;
pub mod value;

use checker::CheckOptions;
use error::{SResult, SperanError};
use interp::{ExecutionResult, Interpreter, Limits, TraceEvent};
use serde_json::{json, Value as Json};
use tools::ToolRegistry;
use value::Value;

#[derive(Debug, Clone)]
pub struct RunReport {
    pub ok: bool,
    pub value: Value,
    pub emits: Vec<Value>,
    pub error: Option<SperanError>,
    pub trace: Vec<TraceEvent>,
    pub steps_used: usize,
    pub tool_calls_used: usize,
}

impl RunReport {
    pub fn to_json(&self) -> Json {
        let emits_json: Vec<Json> = self.emits.iter().map(Value::to_json).collect();
        let trace_json: Vec<Json> = self.trace.iter().map(TraceEvent::to_json).collect();
        let err_json = match &self.error {
            Some(e) => e.to_json(),
            None => Json::Null,
        };
        json!({
            "ok": self.ok,
            "value": self.value.to_json(),
            "emits": emits_json,
            "error": err_json,
            "trace": trace_json,
            "usage": {
                "steps": self.steps_used,
                "tool_calls": self.tool_calls_used,
            }
        })
    }
}

pub struct SperanRunner {
    pub tools: ToolRegistry,
    pub grants: Option<Vec<String>>,
    pub limits: Limits,
}

impl Default for SperanRunner {
    fn default() -> Self {
        SperanRunner {
            tools: ToolRegistry::new(),
            grants: None,
            limits: Limits::default(),
        }
    }
}

impl SperanRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_grants(mut self, grants: Vec<String>) -> Self {
        self.grants = Some(grants);
        self
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    pub fn execute(&mut self, source: &str) -> RunReport {
        let parsed = match parser::parse(source) {
            Ok(stmts) => stmts,
            Err(mut err) => {
                err.attach_source(source);
                return RunReport {
                    ok: false,
                    value: Value::Null,
                    emits: Vec::new(),
                    error: Some(err),
                    trace: Vec::new(),
                    steps_used: 0,
                    tool_calls_used: 0,
                };
            }
        };

        let check_opts = CheckOptions {
            registry: Some(&self.tools),
            grants: self.grants.as_deref(),
        };

        if let Err(mut err) = checker::check(&parsed, check_opts) {
            err.attach_source(source);
            return RunReport {
                ok: false,
                value: Value::Null,
                emits: Vec::new(),
                error: Some(err),
                trace: Vec::new(),
                steps_used: 0,
                tool_calls_used: 0,
            };
        }

        let interp = Interpreter::new(&mut self.tools, self.limits.clone());
        match interp.execute(&parsed) {
            Ok(ExecutionResult {
                value,
                emits,
                trace,
                steps_used,
                tool_calls_used,
            }) => RunReport {
                ok: true,
                value,
                emits,
                error: None,
                trace,
                steps_used,
                tool_calls_used,
            },
            Err(mut err) => {
                err.attach_source(source);
                RunReport {
                    ok: false,
                    value: Value::Null,
                    emits: Vec::new(),
                    error: Some(err),
                    trace: Vec::new(),
                    steps_used: 0,
                    tool_calls_used: 0,
                }
            }
        }
    }
}

/// Simple convenience function to run source code without custom tools or grants.
pub fn run(source: &str) -> RunReport {
    let mut runner = SperanRunner::new();
    runner.execute(source)
}

/// Parse and check source code statically without executing.
pub fn check(source: &str, tools: Option<&ToolRegistry>, grants: Option<&[String]>) -> SResult<()> {
    let stmts = parser::parse(source)?;
    checker::check(&stmts, CheckOptions { registry: tools, grants })
}
