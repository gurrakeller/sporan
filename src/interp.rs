//! Sandboxed tree-walking interpreter with fuel limits, tracing, and safe execution.

use crate::ast::*;
use crate::builtins;
use crate::error::{did_you_mean, ErrorKind, SResult, Span, SperanError};
use crate::tools::ToolRegistry;
use crate::value::{Map, Value};
use indexmap::IndexMap;
use serde_json::{json, Value as Json};
use std::rc::Rc;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_steps: usize,
    pub max_tool_calls: usize,
    pub timeout_ms: Option<u64>,
    pub max_call_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_steps: 100_000,
            max_tool_calls: 50,
            timeout_ms: Some(30_000),
            max_call_depth: 64,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TraceEvent {
    pub step: usize,
    pub tool: String,
    pub args: Json,
    pub result: Result<Json, String>,
    pub duration_ms: u128,
}

impl TraceEvent {
    pub fn to_json(&self) -> Json {
        let (ok, out, err) = match &self.result {
            Ok(v) => (true, v.clone(), Json::Null),
            Err(e) => (false, Json::Null, json!(e)),
        };
        json!({
            "step": self.step,
            "tool": self.tool,
            "args": self.args,
            "ok": ok,
            "output": out,
            "error": err,
            "duration_ms": self.duration_ms,
        })
    }
}

pub struct ExecutionResult {
    pub value: Value,
    pub emits: Vec<Value>,
    pub trace: Vec<TraceEvent>,
    pub steps_used: usize,
    pub tool_calls_used: usize,
}

enum ControlFlow {
    None,
    Value(Value),
    Return(Value),
    Break,
    Continue,
}

pub struct Interpreter<'a> {
    env: Vec<IndexMap<String, Value>>,
    functions: IndexMap<String, Rc<FnDef>>,
    tools: &'a mut ToolRegistry,
    limits: Limits,
    steps: usize,
    tool_calls: usize,
    call_depth: usize,
    start_time: Instant,
    emits: Vec<Value>,
    trace: Vec<TraceEvent>,
    frame_markers: Vec<usize>,
}

impl<'a> Interpreter<'a> {
    pub fn new(tools: &'a mut ToolRegistry, limits: Limits) -> Self {
        Interpreter {
            env: vec![IndexMap::new()],
            functions: IndexMap::new(),
            tools,
            limits,
            steps: 0,
            tool_calls: 0,
            call_depth: 0,
            start_time: Instant::now(),
            emits: Vec::new(),
            trace: Vec::new(),
            frame_markers: vec![0],
        }
    }

    pub fn execute(mut self, stmts: &[Stmt]) -> SResult<ExecutionResult> {
        // Collect top-level functions
        for s in stmts {
            if let StmtKind::Fn(f) = &s.kind {
                self.functions.insert(f.name.clone(), f.clone());
            }
        }

        let mut last_val = Value::Null;
        for s in stmts {
            match self.exec_stmt(s)? {
                ControlFlow::Return(v) => {
                    last_val = v;
                    break;
                }
                ControlFlow::Value(v) => {
                    last_val = v;
                }
                ControlFlow::Break | ControlFlow::Continue => {
                    return Err(SperanError::new(
                        ErrorKind::SyntaxError,
                        "Loop control statement outside loop",
                    )
                    .at(s.span));
                }
                ControlFlow::None => {}
            }
        }

        Ok(ExecutionResult {
            value: last_val,
            emits: self.emits,
            trace: self.trace,
            steps_used: self.steps,
            tool_calls_used: self.tool_calls,
        })
    }

    fn tick(&mut self, span: Span) -> SResult<()> {
        self.steps += 1;
        if self.steps > self.limits.max_steps {
            return Err(SperanError::new(
                ErrorKind::StepLimitExceeded,
                format!("Exceeded step limit of {} steps", self.limits.max_steps),
            )
            .at(span)
            .with_hint("Check for accidental infinite loops or reduce input size."));
        }

        if let Some(ms) = self.limits.timeout_ms {
            if self.start_time.elapsed().as_millis() > ms as u128 {
                return Err(SperanError::new(
                    ErrorKind::Timeout,
                    format!("Execution timed out after {}ms", ms),
                )
                .at(span));
            }
        }

        Ok(())
    }

    fn push_scope(&mut self) {
        self.env.push(IndexMap::new());
    }

    fn pop_scope(&mut self) {
        self.env.pop();
    }

    fn get_var(&self, name: &str, span: Span) -> SResult<Value> {
        for scope in self.env.iter().rev() {
            if let Some(val) = scope.get(name) {
                return Ok(val.clone());
            }
        }
        let avail: Vec<String> = self.all_var_names();
        let hint = did_you_mean(name, avail.iter().map(|s| s.as_str()));
        Err(SperanError::new(
            ErrorKind::UnknownName,
            format!("Variable '{}' is not defined", name),
        )
        .at(span)
        .maybe_hint(hint))
    }

    fn set_var(&mut self, name: &str, val: Value) {
        let frame_start = self.frame_markers.last().copied().unwrap_or(0);
        for scope in self.env[frame_start..].iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), val);
                return;
            }
        }
        if let Some(scope) = self.env.last_mut() {
            scope.insert(name.to_string(), val);
        }
    }

    fn define_var(&mut self, name: &str, val: Value) {
        if let Some(scope) = self.env.last_mut() {
            scope.insert(name.to_string(), val);
        }
    }

    fn all_var_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        for scope in &self.env {
            for k in scope.keys() {
                out.push(k.clone());
            }
        }
        for f in self.functions.keys() {
            out.push(f.clone());
        }
        out
    }

    // ----- Statements -----

    fn exec_stmt(&mut self, stmt: &Stmt) -> SResult<ControlFlow> {
        self.tick(stmt.span)?;
        match &stmt.kind {
            StmtKind::Use(_) => Ok(ControlFlow::None),
            StmtKind::Fn(f) => {
                self.functions.insert(f.name.clone(), f.clone());
                Ok(ControlFlow::None)
            }
            StmtKind::Assign { name, op, value } => {
                let rhs = self.eval_expr(value)?;
                let final_val = match op {
                    None => rhs,
                    Some(b_op) => {
                        let cur = self.get_var(name, stmt.span)?;
                        self.eval_bin_op(*b_op, cur, rhs, stmt.span)?
                    }
                };
                self.set_var(name, final_val);
                Ok(ControlFlow::None)
            }
            StmtKind::Expr(e) => {
                let v = self.eval_expr(e)?;
                Ok(ControlFlow::Value(v))
            }
            StmtKind::If { cond, then, els } => {
                let c = self.eval_expr(cond)?;
                if c.truthy() {
                    self.push_scope();
                    let res = self.exec_block(then)?;
                    self.pop_scope();
                    Ok(res)
                } else if let Some(els_stmts) = els {
                    self.push_scope();
                    let res = self.exec_block(els_stmts)?;
                    self.pop_scope();
                    Ok(res)
                } else {
                    Ok(ControlFlow::None)
                }
            }
            StmtKind::For { vars, iter, body } => {
                let iter_val = self.eval_expr(iter)?;
                let items: Vec<Value> = match iter_val {
                    Value::List(l) => (*l).clone(),
                    Value::Str(s) => s.chars().map(|c| Value::str(c.to_string())).collect(),
                    Value::Map(m) => m.keys().map(|k| Value::str(k.clone())).collect(),
                    other => {
                        return Err(SperanError::new(
                            ErrorKind::TypeError,
                            format!("Cannot iterate over {}", other.type_name()),
                        )
                        .at(iter.span)
                        .with_hint("Only lists, maps, and strings can be iterated with 'for'."))
                    }
                };

                for item in items {
                    self.push_scope();
                    self.bind_vars(vars, item, stmt.span)?;
                    let res = self.exec_block(body)?;
                    self.pop_scope();
                    match res {
                        ControlFlow::Break => break,
                        ControlFlow::Continue | ControlFlow::Value(_) | ControlFlow::None => {}
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                    }
                }
                Ok(ControlFlow::None)
            }
            StmtKind::While { cond, body } => {
                loop {
                    self.tick(stmt.span)?;
                    let c = self.eval_expr(cond)?;
                    if !c.truthy() {
                        break;
                    }
                    self.push_scope();
                    let res = self.exec_block(body)?;
                    self.pop_scope();
                    match res {
                        ControlFlow::Break => break,
                        ControlFlow::Continue | ControlFlow::Value(_) | ControlFlow::None => {}
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                    }
                }
                Ok(ControlFlow::None)
            }
            StmtKind::Return(e) => {
                let val = match e {
                    Some(expr) => self.eval_expr(expr)?,
                    None => Value::Null,
                };
                Ok(ControlFlow::Return(val))
            }
            StmtKind::Emit(e) => {
                let val = self.eval_expr(e)?;
                self.emits.push(val);
                Ok(ControlFlow::None)
            }
            StmtKind::Break => Ok(ControlFlow::Break),
            StmtKind::Continue => Ok(ControlFlow::Continue),
        }
    }

    fn exec_block(&mut self, stmts: &[Stmt]) -> SResult<ControlFlow> {
        let mut last_flow = ControlFlow::None;
        for s in stmts {
            let res = self.exec_stmt(s)?;
            match res {
                ControlFlow::Return(_) | ControlFlow::Break | ControlFlow::Continue => return Ok(res),
                ControlFlow::Value(_) => last_flow = res,
                ControlFlow::None => {}
            }
        }
        Ok(last_flow)
    }

    fn bind_vars(&mut self, vars: &[String], item: Value, span: Span) -> SResult<()> {
        if vars.len() == 1 {
            self.define_var(&vars[0], item);
        } else {
            match item {
                Value::List(l) => {
                    if l.len() != vars.len() {
                        return Err(SperanError::new(
                            ErrorKind::ValueError,
                            format!(
                                "Cannot unpack {} item(s) into {} loop variables",
                                l.len(),
                                vars.len()
                            ),
                        )
                        .at(span));
                    }
                    for (v, val) in vars.iter().zip(l.iter()) {
                        self.define_var(v, val.clone());
                    }
                }
                other => {
                    return Err(SperanError::new(
                        ErrorKind::TypeError,
                        format!("Cannot unpack {} into multiple loop variables", other.type_name()),
                    )
                    .at(span));
                }
            }
        }
        Ok(())
    }

    // ----- Expressions -----

    pub fn eval_expr(&mut self, expr: &Expr) -> SResult<Value> {
        self.tick(expr.span)?;
        match &expr.kind {
            ExprKind::Num(n) => Ok(Value::Num(*n)),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Null => Ok(Value::Null),
            ExprKind::Ident(name) => self.get_var(name, expr.span),
            ExprKind::Str(segs) => {
                let mut out = String::new();
                for s in segs {
                    match s {
                        StrSeg::Lit(lit) => out.push_str(lit),
                        StrSeg::Expr(e) => {
                            let val = self.eval_expr(e)?;
                            out.push_str(&val.display());
                        }
                    }
                }
                Ok(Value::str(out))
            }
            ExprKind::List(items) => {
                let mut vals = Vec::with_capacity(items.len());
                for it in items {
                    vals.push(self.eval_expr(it)?);
                }
                Ok(Value::list(vals))
            }
            ExprKind::Map(entries) => {
                let mut m = Map::new();
                for (k, v) in entries {
                    m.insert(k.clone(), self.eval_expr(v)?);
                }
                Ok(Value::map(m))
            }
            ExprKind::Comp { expr, vars, iter, cond } => {
                let iter_val = self.eval_expr(iter)?;
                let items: Vec<Value> = match iter_val {
                    Value::List(l) => (*l).clone(),
                    Value::Str(s) => s.chars().map(|c| Value::str(c.to_string())).collect(),
                    Value::Map(m) => m.keys().map(|k| Value::str(k.clone())).collect(),
                    o => {
                        return Err(SperanError::new(
                            ErrorKind::TypeError,
                            format!("Cannot iterate over {}", o.type_name()),
                        )
                        .at(iter.span))
                    }
                };

                let mut out = Vec::new();
                for item in items {
                    self.push_scope();
                    self.bind_vars(vars, item, expr.span)?;
                    let include = match cond {
                        Some(c) => self.eval_expr(c)?.truthy(),
                        None => true,
                    };
                    if include {
                        let evaluated = self.eval_expr(expr)?;
                        out.push(evaluated);
                    }
                    self.pop_scope();
                }
                Ok(Value::list(out))
            }
            ExprKind::Unary { op, expr: inner } => {
                let v = self.eval_expr(inner)?;
                match op {
                    UnOp::Neg => match v {
                        Value::Num(n) => Ok(Value::Num(-n)),
                        o => Err(SperanError::new(
                            ErrorKind::TypeError,
                            format!("Cannot negate {}", o.type_name()),
                        )
                        .at(expr.span)),
                    },
                    UnOp::Not => Ok(Value::Bool(!v.truthy())),
                }
            }
            ExprKind::Binary { op, left, right } => {
                // Short-circuiting for and/or
                match op {
                    BinOp::And => {
                        let l = self.eval_expr(left)?;
                        if !l.truthy() {
                            return Ok(l);
                        }
                        return self.eval_expr(right);
                    }
                    BinOp::Or => {
                        let l = self.eval_expr(left)?;
                        if l.truthy() {
                            return Ok(l);
                        }
                        return self.eval_expr(right);
                    }
                    _ => {}
                }
                let l = self.eval_expr(left)?;
                let r = self.eval_expr(right)?;
                self.eval_bin_op(*op, l, r, expr.span)
            }
            ExprKind::Member { obj, name } => {
                let o = self.eval_expr(obj)?;
                match o {
                    Value::Map(m) => match m.get(name) {
                        Some(val) => Ok(val.clone()),
                        None => {
                            let keys: Vec<&str> = m.keys().map(|k| k.as_str()).collect();
                            let hint = did_you_mean(name, keys);
                            Err(SperanError::new(
                                ErrorKind::KeyError,
                                format!("Map has no key '{}'", name),
                            )
                            .at(expr.span)
                            .maybe_hint(hint))
                        }
                    },
                    other => Err(SperanError::new(
                        ErrorKind::TypeError,
                        format!("Cannot access property '.{}' on {}", name, other.type_name()),
                    )
                    .at(expr.span)
                    .with_hint("Property access is only available on maps.")),
                }
            }
            ExprKind::Index { obj, index } => {
                let o = self.eval_expr(obj)?;
                let idx = self.eval_expr(index)?;
                match (&o, &idx) {
                    (Value::List(l), Value::Num(n)) => {
                        let len = l.len() as i64;
                        let i = *n as i64;
                        let resolved = if i < 0 { i + len } else { i };
                        if resolved >= 0 && (resolved as usize) < l.len() {
                            Ok(l[resolved as usize].clone())
                        } else {
                            Err(SperanError::new(
                                ErrorKind::IndexError,
                                format!("Index {} out of bounds for list of length {}", n, l.len()),
                            )
                            .at(expr.span))
                        }
                    }
                    (Value::Str(s), Value::Num(n)) => {
                        let chars: Vec<char> = s.chars().collect();
                        let len = chars.len() as i64;
                        let i = *n as i64;
                        let resolved = if i < 0 { i + len } else { i };
                        if resolved >= 0 && (resolved as usize) < chars.len() {
                            Ok(Value::str(chars[resolved as usize].to_string()))
                        } else {
                            Err(SperanError::new(
                                ErrorKind::IndexError,
                                format!("Index {} out of bounds for string of length {}", n, len),
                            )
                            .at(expr.span))
                        }
                    }
                    (Value::Map(m), Value::Str(k)) => match m.get(&**k) {
                        Some(v) => Ok(v.clone()),
                        None => {
                            let keys: Vec<&str> = m.keys().map(|x| x.as_str()).collect();
                            let hint = did_you_mean(k, keys);
                            Err(SperanError::new(
                                ErrorKind::KeyError,
                                format!("Map has no key \"{}\"", k),
                            )
                            .at(expr.span)
                            .maybe_hint(hint))
                        }
                    },
                    (other_obj, other_idx) => Err(SperanError::new(
                        ErrorKind::TypeError,
                        format!(
                            "Cannot index {} with {}",
                            other_obj.type_name(),
                            other_idx.type_name()
                        ),
                    )
                    .at(expr.span)),
                }
            }
            ExprKind::Slice { obj, start, end } => {
                let o = self.eval_expr(obj)?;
                let s_val = match start {
                    Some(s) => match self.eval_expr(s)? {
                        Value::Num(n) => Some(n as i64),
                        other => {
                            return Err(SperanError::new(
                                ErrorKind::TypeError,
                                format!("Slice start must be a number, got {}", other.type_name()),
                            )
                            .at(s.span))
                        }
                    },
                    None => None,
                };
                let e_val = match end {
                    Some(e) => match self.eval_expr(e)? {
                        Value::Num(n) => Some(n as i64),
                        other => {
                            return Err(SperanError::new(
                                ErrorKind::TypeError,
                                format!("Slice end must be a number, got {}", other.type_name()),
                            )
                            .at(e.span))
                        }
                    },
                    None => None,
                };

                match o {
                    Value::List(l) => {
                        let len = l.len() as i64;
                        let (from, to) = resolve_slice_bounds(len, s_val, e_val);
                        Ok(Value::list(l[from..to].to_vec()))
                    }
                    Value::Str(s) => {
                        let chars: Vec<char> = s.chars().collect();
                        let len = chars.len() as i64;
                        let (from, to) = resolve_slice_bounds(len, s_val, e_val);
                        let sub: String = chars[from..to].iter().collect();
                        Ok(Value::str(sub))
                    }
                    other => Err(SperanError::new(
                        ErrorKind::TypeError,
                        format!("Cannot slice {}", other.type_name()),
                    )
                    .at(expr.span)),
                }
            }
            ExprKind::Call { callee, args } => self.eval_call(callee, args, expr.span),
            ExprKind::Try(inner) => {
                let result = self.eval_expr(inner);
                match result {
                    Ok(val) => {
                        let mut m = Map::new();
                        m.insert("value".into(), val);
                        m.insert("error".into(), Value::Null);
                        Ok(Value::map(m))
                    }
                    Err(err) => {
                        if err.kind.is_fatal() {
                            // Fatal limit errors must not be caught by try
                            return Err(err);
                        }
                        let mut m = Map::new();
                        m.insert("value".into(), Value::Null);
                        m.insert("error".into(), Value::from_json(&err.to_json()));
                        Ok(Value::map(m))
                    }
                }
            }
            ExprKind::Par { vars, iter, body } => {
                let iter_val = self.eval_expr(iter)?;
                let items: Vec<Value> = match iter_val {
                    Value::List(l) => (*l).clone(),
                    other => {
                        return Err(SperanError::new(
                            ErrorKind::TypeError,
                            format!("'par' requires a list to iterate over, got {}", other.type_name()),
                        )
                        .at(iter.span))
                    }
                };

                let mut results = Vec::new();
                for item in items {
                    self.push_scope();
                    self.bind_vars(vars, item, expr.span)?;
                    let mut body_val = Value::Null;
                    for s in body {
                        match self.exec_stmt(s)? {
                            ControlFlow::Return(v) => {
                                body_val = v;
                                break;
                            }
                            ControlFlow::Break | ControlFlow::Continue => break,
                            ControlFlow::Value(v) => {
                                body_val = v;
                            }
                            ControlFlow::None => {}
                        }
                    }
                    results.push(body_val);
                    self.pop_scope();
                }
                Ok(Value::list(results))
            }
        }
    }

    fn eval_bin_op(&mut self, op: BinOp, left: Value, right: Value, span: Span) -> SResult<Value> {
        match op {
            BinOp::Add => match (left, right) {
                (Value::Num(a), Value::Num(b)) => Ok(Value::Num(a + b)),
                (Value::Str(a), Value::Str(b)) => Ok(Value::str(format!("{}{}", a, b))),
                (Value::List(a), Value::List(b)) => {
                    let mut combined = (*a).clone();
                    combined.extend(b.iter().cloned());
                    Ok(Value::list(combined))
                }
                (Value::Map(a), Value::Map(b)) => {
                    let mut combined = (*a).clone();
                    for (k, v) in b.iter() {
                        combined.insert(k.clone(), v.clone());
                    }
                    Ok(Value::map(combined))
                }
                (l, r) => Err(SperanError::new(
                    ErrorKind::TypeError,
                    format!("Cannot add {} and {}", l.type_name(), r.type_name()),
                )
                .at(span)),
            },
            BinOp::Sub => match (left, right) {
                (Value::Num(a), Value::Num(b)) => Ok(Value::Num(a - b)),
                (l, r) => Err(SperanError::new(
                    ErrorKind::TypeError,
                    format!("Cannot subtract {} from {}", r.type_name(), l.type_name()),
                )
                .at(span)),
            },
            BinOp::Mul => match (left, right) {
                (Value::Num(a), Value::Num(b)) => Ok(Value::Num(a * b)),
                (Value::Str(s), Value::Num(n)) | (Value::Num(n), Value::Str(s)) => {
                    let count = (n as i64).max(0) as usize;
                    Ok(Value::str(s.repeat(count)))
                }
                (l, r) => Err(SperanError::new(
                    ErrorKind::TypeError,
                    format!("Cannot multiply {} by {}", l.type_name(), r.type_name()),
                )
                .at(span)),
            },
            BinOp::Div => match (left, right) {
                (Value::Num(a), Value::Num(b)) => {
                    if b == 0.0 {
                        Err(SperanError::new(ErrorKind::DivisionByZero, "Division by zero").at(span))
                    } else {
                        Ok(Value::Num(a / b))
                    }
                }
                (l, r) => Err(SperanError::new(
                    ErrorKind::TypeError,
                    format!("Cannot divide {} by {}", l.type_name(), r.type_name()),
                )
                .at(span)),
            },
            BinOp::Mod => match (left, right) {
                (Value::Num(a), Value::Num(b)) => {
                    if b == 0.0 {
                        Err(SperanError::new(ErrorKind::DivisionByZero, "Modulo by zero").at(span))
                    } else {
                        Ok(Value::Num(a % b))
                    }
                }
                (l, r) => Err(SperanError::new(
                    ErrorKind::TypeError,
                    format!("Cannot modulo {} by {}", l.type_name(), r.type_name()),
                )
                .at(span)),
            },
            BinOp::Eq => Ok(Value::Bool(left == right)),
            BinOp::Ne => Ok(Value::Bool(left != right)),
            BinOp::Lt => {
                let ord = builtins::compare("<", &left, &right).map_err(|e| e.at(span))?;
                Ok(Value::Bool(ord == std::cmp::Ordering::Less))
            }
            BinOp::Gt => {
                let ord = builtins::compare(">", &left, &right).map_err(|e| e.at(span))?;
                Ok(Value::Bool(ord == std::cmp::Ordering::Greater))
            }
            BinOp::Le => {
                let ord = builtins::compare("<=", &left, &right).map_err(|e| e.at(span))?;
                Ok(Value::Bool(ord != std::cmp::Ordering::Greater))
            }
            BinOp::Ge => {
                let ord = builtins::compare(">=", &left, &right).map_err(|e| e.at(span))?;
                Ok(Value::Bool(ord != std::cmp::Ordering::Less))
            }
            BinOp::In => self.eval_in(left, right, span),
            BinOp::NotIn => {
                let v = self.eval_in(left, right, span)?;
                match v {
                    Value::Bool(b) => Ok(Value::Bool(!b)),
                    _ => unreachable!(),
                }
            }
            BinOp::And | BinOp::Or => unreachable!("handled in eval_expr"),
        }
    }

    fn eval_in(&self, item: Value, container: Value, span: Span) -> SResult<Value> {
        match container {
            Value::List(l) => Ok(Value::Bool(l.contains(&item))),
            Value::Map(m) => match item {
                Value::Str(k) => Ok(Value::Bool(m.contains_key(&*k))),
                other => Err(SperanError::new(
                    ErrorKind::TypeError,
                    format!("Map key must be string, got {}", other.type_name()),
                )
                .at(span)),
            },
            Value::Str(s) => match item {
                Value::Str(sub) => Ok(Value::Bool(s.contains(&*sub))),
                other => Err(SperanError::new(
                    ErrorKind::TypeError,
                    format!("'in' string expects string substring, got {}", other.type_name()),
                )
                .at(span)),
            },
            other => Err(SperanError::new(
                ErrorKind::TypeError,
                format!("'in' cannot be used on {}", other.type_name()),
            )
            .at(span)),
        }
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Arg], span: Span) -> SResult<Value> {
        // Dotted path -> tool call (e.g. github.get_issue)
        if let Some(tool_name) = dotted_path(callee) {
            if tool_name.contains('.') {
                return self.call_tool(&tool_name, args, span);
            }
        }

        // Identifier -> built-in or user function
        if let ExprKind::Ident(name) = &callee.kind {
            if builtins::is_builtin(name) {
                let mut pos_args = Vec::new();
                let mut kw_args = Map::new();
                for a in args {
                    let v = self.eval_expr(&a.value)?;
                    if let Some(n) = &a.name {
                        kw_args.insert(n.clone(), v);
                    } else {
                        pos_args.push(v);
                    }
                }
                return builtins::call(name, &pos_args, &kw_args).map_err(|e| e.at(span));
            }

            if let Some(f_def) = self.functions.get(name).cloned() {
                return self.call_user_fn(&f_def, args, span);
            }
        }

        Err(SperanError::new(
            ErrorKind::TypeError,
            "Target of call is not a callable function or tool",
        )
        .at(span))
    }

    fn call_user_fn(&mut self, f_def: &FnDef, args: &[Arg], span: Span) -> SResult<Value> {
        if self.call_depth >= self.limits.max_call_depth {
            return Err(SperanError::new(
                ErrorKind::RecursionLimit,
                format!("Call stack recursion limit of {} exceeded", self.limits.max_call_depth),
            )
            .at(span));
        }

        if args.len() != f_def.params.len() {
            return Err(SperanError::new(
                ErrorKind::ArgumentError,
                format!(
                    "Function '{}' expects {} arguments, but {} were given",
                    f_def.name,
                    f_def.params.len(),
                    args.len()
                ),
            )
            .at(span));
        }

        let mut evaluated_args = Vec::with_capacity(args.len());
        for a in args {
            evaluated_args.push(self.eval_expr(&a.value)?);
        }

        self.call_depth += 1;
        self.frame_markers.push(self.env.len());
        self.push_scope();
        for (param, val) in f_def.params.iter().zip(evaluated_args) {
            self.define_var(param, val);
        }

        let mut return_val = Value::Null;
        for s in &f_def.body {
            match self.exec_stmt(s)? {
                ControlFlow::Return(v) => {
                    return_val = v;
                    break;
                }
                ControlFlow::Value(v) => {
                    return_val = v;
                }
                ControlFlow::Break | ControlFlow::Continue => {
                    self.pop_scope();
                    self.frame_markers.pop();
                    self.call_depth -= 1;
                    return Err(SperanError::new(
                        ErrorKind::SyntaxError,
                        "Loop control statement outside loop",
                    )
                    .at(s.span));
                }
                ControlFlow::None => {}
            }
        }

        self.pop_scope();
        self.frame_markers.pop();
        self.call_depth -= 1;
        Ok(return_val)
    }

    fn call_tool(&mut self, name: &str, args: &[Arg], span: Span) -> SResult<Value> {
        self.tool_calls += 1;
        if self.tool_calls > self.limits.max_tool_calls {
            return Err(SperanError::new(
                ErrorKind::ToolCallLimitExceeded,
                format!("Exceeded maximum of {} tool calls", self.limits.max_tool_calls),
            )
            .at(span));
        }

        let mut tool_args = Map::new();
        for a in args {
            let v = self.eval_expr(&a.value)?;
            if let Some(param_name) = &a.name {
                tool_args.insert(param_name.clone(), v);
            } else {
                return Err(SperanError::new(
                    ErrorKind::ArgumentError,
                    format!("Tool calls require named arguments (e.g. key: \"value\")"),
                )
                .at(a.span)
                .with_hint(format!("Write {}(param: value) instead of positional arguments.", name)));
            }
        }

        let args_json = Value::map(tool_args.clone()).to_json();
        let call_start = Instant::now();
        let call_res = self.tools.call(name, &tool_args);
        let duration = call_start.elapsed().as_millis();

        let trace_result = match &call_res {
            Ok(val) => Ok(val.to_json()),
            Err(e) => Err(e.clone()),
        };

        self.trace.push(TraceEvent {
            step: self.steps,
            tool: name.to_string(),
            args: args_json,
            result: trace_result,
            duration_ms: duration,
        });

        match call_res {
            Ok(v) => Ok(v),
            Err(e) => Err(SperanError::new(ErrorKind::ToolError, e).at(span)),
        }
    }
}

fn resolve_slice_bounds(len: i64, start: Option<i64>, end: Option<i64>) -> (usize, usize) {
    let s = start.unwrap_or(0);
    let e = end.unwrap_or(len);
    let s = if s < 0 { (s + len).max(0) } else { s.min(len) };
    let e = if e < 0 { (e + len).max(0) } else { e.min(len) };
    let from = s.min(e) as usize;
    let to = e.max(s) as usize;
    (from, to)
}
