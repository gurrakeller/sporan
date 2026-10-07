//! Errors are a core feature of Speran: they are structured, precise, and come
//! with a hint so that an LLM can fix its program in a single retry.

use serde_json::{json, Value as Json};
use std::fmt;

/// 1-based source location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

impl Span {
    pub fn new(line: usize, col: usize) -> Self {
        Span { line, col }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    SyntaxError,
    UnknownName,
    UnknownTool,
    UndeclaredTool,
    CapabilityDenied,
    TypeError,
    ValueError,
    IndexError,
    KeyError,
    ArgumentError,
    ToolError,
    ImmutableValue,
    DivisionByZero,
    StepLimitExceeded,
    ToolCallLimitExceeded,
    Timeout,
    RecursionLimit,
}

impl ErrorKind {
    pub fn name(self) -> &'static str {
        match self {
            ErrorKind::SyntaxError => "SyntaxError",
            ErrorKind::UnknownName => "UnknownName",
            ErrorKind::UnknownTool => "UnknownTool",
            ErrorKind::UndeclaredTool => "UndeclaredTool",
            ErrorKind::CapabilityDenied => "CapabilityDenied",
            ErrorKind::TypeError => "TypeError",
            ErrorKind::ValueError => "ValueError",
            ErrorKind::IndexError => "IndexError",
            ErrorKind::KeyError => "KeyError",
            ErrorKind::ArgumentError => "ArgumentError",
            ErrorKind::ToolError => "ToolError",
            ErrorKind::ImmutableValue => "ImmutableValue",
            ErrorKind::DivisionByZero => "DivisionByZero",
            ErrorKind::StepLimitExceeded => "StepLimitExceeded",
            ErrorKind::ToolCallLimitExceeded => "ToolCallLimitExceeded",
            ErrorKind::Timeout => "Timeout",
            ErrorKind::RecursionLimit => "RecursionLimit",
        }
    }

    /// Fatal errors are resource-limit violations. They can never be caught
    /// by `try`, so a program cannot escape its sandbox limits.
    pub fn is_fatal(self) -> bool {
        matches!(
            self,
            ErrorKind::StepLimitExceeded
                | ErrorKind::ToolCallLimitExceeded
                | ErrorKind::Timeout
                | ErrorKind::RecursionLimit
        )
    }
}

#[derive(Debug, Clone)]
pub struct SperanError {
    pub kind: ErrorKind,
    pub message: String,
    pub span: Option<Span>,
    pub hint: Option<String>,
    /// The offending source line, attached once the full source is known.
    pub code: Option<String>,
}

pub type SResult<T> = Result<T, SperanError>;

impl SperanError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        SperanError { kind, message: message.into(), span: None, hint: None, code: None }
    }

    /// Set the location, unless a more precise (inner) location is already set.
    pub fn at(mut self, span: Span) -> Self {
        if self.span.is_none() {
            self.span = Some(span);
        }
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Set the hint only if none is set yet.
    pub fn or_hint(mut self, hint: impl Into<String>) -> Self {
        if self.hint.is_none() {
            self.hint = Some(hint.into());
        }
        self
    }

    pub fn maybe_hint(mut self, hint: Option<String>) -> Self {
        if hint.is_some() {
            self.hint = hint;
        }
        self
    }

    pub fn attach_source(&mut self, src: &str) {
        if let (Some(span), None) = (self.span, &self.code) {
            self.code = src
                .lines()
                .nth(span.line.saturating_sub(1))
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty());
        }
    }

    pub fn to_json(&self) -> Json {
        let mut m = serde_json::Map::new();
        m.insert("error".into(), json!(self.kind.name()));
        if let Some(s) = self.span {
            m.insert("line".into(), json!(s.line));
            m.insert("col".into(), json!(s.col));
        }
        if let Some(c) = &self.code {
            m.insert("code".into(), json!(c));
        }
        m.insert("message".into(), json!(self.message));
        if let Some(h) = &self.hint {
            m.insert("hint".into(), json!(h));
        }
        Json::Object(m)
    }
}

impl fmt::Display for SperanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind.name())?;
        if let Some(s) = self.span {
            write!(f, " at {}:{}", s.line, s.col)?;
        }
        write!(f, ": {}", self.message)?;
        if let Some(h) = &self.hint {
            write!(f, " (hint: {})", h)?;
        }
        Ok(())
    }
}

impl std::error::Error for SperanError {}

pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Closest candidate within a small edit distance, if any.
pub fn suggest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let max = (name.chars().count() / 3).max(1);
    candidates
        .into_iter()
        .filter(|c| *c != name)
        .map(|c| (levenshtein(name, c), c))
        .filter(|(d, _)| *d <= max)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c.to_string())
}

pub fn did_you_mean<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    suggest(name, candidates).map(|s| format!("Did you mean '{}'?", s))
}
