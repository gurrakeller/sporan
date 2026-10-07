//! Static checker: runs before execution to catch typos, undeclared tools,
//! capability violations, method calls, and out-of-scope variables.

use crate::ast::*;
use crate::builtins;
use crate::error::{did_you_mean, ErrorKind, SResult, Span, SperanError};
use crate::tools::{is_granted, ToolRegistry};
use std::collections::HashSet;

pub struct CheckOptions<'a> {
    pub registry: Option<&'a ToolRegistry>,
    pub grants: Option<&'a [String]>,
}

pub fn check(stmts: &[Stmt], options: CheckOptions) -> SResult<()> {
    let mut checker = Checker {
        declared_tools: HashSet::new(),
        scopes: vec![HashSet::new()],
        loop_depth: 0,
        options,
    };
    checker.run(stmts)
}

struct Checker<'a> {
    declared_tools: HashSet<String>,
    scopes: Vec<HashSet<String>>,
    loop_depth: usize,
    options: CheckOptions<'a>,
}

impl<'a> Checker<'a> {
    fn enter_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }

    fn exit_scope(&mut self) {
        self.scopes.pop();
    }

    fn define_var(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string());
        }
    }

    fn is_in_scope(&self, name: &str) -> bool {
        if builtins::is_builtin(name) {
            return true;
        }
        for scope in self.scopes.iter().rev() {
            if scope.contains(name) {
                return true;
            }
        }
        false
    }

    fn all_visible_names(&self) -> Vec<String> {
        let mut names: Vec<String> = builtins::names().map(String::from).collect();
        for scope in &self.scopes {
            for v in scope {
                names.push(v.clone());
            }
        }
        names
    }

    fn run(&mut self, stmts: &[Stmt]) -> SResult<()> {
        // First pass: collect top-level functions so order of definition doesn't matter
        for stmt in stmts {
            if let StmtKind::Fn(f) = &stmt.kind {
                self.define_var(&f.name);
            }
        }

        for stmt in stmts {
            self.check_stmt(stmt)?;
        }
        Ok(())
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> SResult<()> {
        match &stmt.kind {
            StmtKind::Use(tools) => {
                for t in tools {
                    self.check_use_tool(t)?;
                }
            }
            StmtKind::Fn(f) => {
                self.define_var(&f.name);
                self.enter_scope();
                for param in &f.params {
                    self.define_var(param);
                }
                for s in &f.body {
                    self.check_stmt(s)?;
                }
                self.exit_scope();
            }
            StmtKind::Assign { name, op: _, value } => {
                self.check_expr(value)?;
                self.define_var(name);
            }
            StmtKind::Expr(e) => {
                self.check_expr(e)?;
            }
            StmtKind::If { cond, then, els } => {
                self.check_expr(cond)?;
                self.enter_scope();
                for s in then {
                    self.check_stmt(s)?;
                }
                self.exit_scope();
                if let Some(els_stmts) = els {
                    self.enter_scope();
                    for s in els_stmts {
                        self.check_stmt(s)?;
                    }
                    self.exit_scope();
                }
            }
            StmtKind::For { vars, iter, body } => {
                self.check_expr(iter)?;
                self.loop_depth += 1;
                self.enter_scope();
                for v in vars {
                    self.define_var(v);
                }
                for s in body {
                    self.check_stmt(s)?;
                }
                self.exit_scope();
                self.loop_depth -= 1;
            }
            StmtKind::While { cond, body } => {
                self.check_expr(cond)?;
                self.loop_depth += 1;
                self.enter_scope();
                for s in body {
                    self.check_stmt(s)?;
                }
                self.exit_scope();
                self.loop_depth -= 1;
            }
            StmtKind::Break => {
                if self.loop_depth == 0 {
                    return Err(SperanError::new(
                        ErrorKind::SyntaxError,
                        "'break' outside of loop",
                    )
                    .at(stmt.span)
                    .with_hint("'break' can only be used inside a 'for' or 'while' loop."));
                }
            }
            StmtKind::Continue => {
                if self.loop_depth == 0 {
                    return Err(SperanError::new(
                        ErrorKind::SyntaxError,
                        "'continue' outside of loop",
                    )
                    .at(stmt.span)
                    .with_hint("'continue' can only be used inside a 'for' or 'while' loop."));
                }
            }
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.check_expr(e)?;
                }
            }
            StmtKind::Emit(e) => {
                self.check_expr(e)?;
            }
        }
        Ok(())
    }

    fn check_use_tool(&mut self, t: &ToolRef) -> SResult<()> {
        if !t.name.contains('.') {
            return Err(SperanError::new(
                ErrorKind::SyntaxError,
                format!("Tool '{}' must be namespaced (e.g. 'github.{}')", t.name, t.name),
            )
            .at(t.span)
            .with_hint("Tool names must be namespaced, e.g. github.search_issues or slack.post"));
        }

        // Check against capabilities / grants
        if !is_granted(&t.name, self.options.grants) {
            return Err(SperanError::new(
                ErrorKind::CapabilityDenied,
                format!("Host denied access to capability '{}'", t.name),
            )
            .at(t.span)
            .with_hint("This tool is not granted by the host's capability permissions."));
        }

        // If registry is provided, check existence
        if let Some(reg) = self.options.registry {
            if !reg.has(&t.name) {
                let avail = reg.names();
                let hint = did_you_mean(&t.name, avail.iter().map(|s| s.as_str()));
                return Err(SperanError::new(
                    ErrorKind::UnknownTool,
                    format!("Tool '{}' is not registered", t.name),
                )
                .at(t.span)
                .maybe_hint(hint));
            }
        }

        self.declared_tools.insert(t.name.clone());
        Ok(())
    }

    fn check_expr(&self, expr: &Expr) -> SResult<()> {
        match &expr.kind {
            ExprKind::Num(_) | ExprKind::Bool(_) | ExprKind::Null => Ok(()),
            ExprKind::Str(segs) => {
                for s in segs {
                    if let StrSeg::Expr(e) = s {
                        self.check_expr(e)?;
                    }
                }
                Ok(())
            }
            ExprKind::Ident(name) => {
                if !self.is_in_scope(name) {
                    let avail = self.all_visible_names();
                    let hint = did_you_mean(name, avail.iter().map(|s| s.as_str()));
                    return Err(SperanError::new(
                        ErrorKind::UnknownName,
                        format!("Variable '{}' is not defined", name),
                    )
                    .at(expr.span)
                    .maybe_hint(hint));
                }
                Ok(())
            }
            ExprKind::List(items) => {
                for item in items {
                    self.check_expr(item)?;
                }
                Ok(())
            }
            ExprKind::Map(entries) => {
                for (_, v) in entries {
                    self.check_expr(v)?;
                }
                Ok(())
            }
            ExprKind::Comp { expr, vars, iter, cond } => {
                self.check_expr(iter)?;
                // Check comp body in scope of vars
                let mut inner = Checker {
                    declared_tools: self.declared_tools.clone(),
                    scopes: self.scopes.clone(),
                    loop_depth: self.loop_depth,
                    options: CheckOptions {
                        registry: self.options.registry,
                        grants: self.options.grants,
                    },
                };
                inner.enter_scope();
                for v in vars {
                    inner.define_var(v);
                }
                inner.check_expr(expr)?;
                if let Some(c) = cond {
                    inner.check_expr(c)?;
                }
                Ok(())
            }
            ExprKind::Unary { expr, .. } => self.check_expr(expr),
            ExprKind::Binary { left, right, .. } => {
                self.check_expr(left)?;
                self.check_expr(right)
            }
            ExprKind::Call { callee, args } => {
                self.check_call(callee, args, expr.span)
            }
            ExprKind::Member { obj, .. } => self.check_expr(obj),
            ExprKind::Index { obj, index } => {
                self.check_expr(obj)?;
                self.check_expr(index)
            }
            ExprKind::Slice { obj, start, end } => {
                self.check_expr(obj)?;
                if let Some(s) = start {
                    self.check_expr(s)?;
                }
                if let Some(e) = end {
                    self.check_expr(e)?;
                }
                Ok(())
            }
            ExprKind::Try(e) => self.check_expr(e),
            ExprKind::Par { vars, iter, body } => {
                self.check_expr(iter)?;
                let mut inner = Checker {
                    declared_tools: self.declared_tools.clone(),
                    scopes: self.scopes.clone(),
                    loop_depth: self.loop_depth + 1,
                    options: CheckOptions {
                        registry: self.options.registry,
                        grants: self.options.grants,
                    },
                };
                inner.enter_scope();
                for v in vars {
                    inner.define_var(v);
                }
                for s in body {
                    inner.check_stmt(s)?;
                }
                Ok(())
            }
        }
    }

    fn check_call(&self, callee: &Expr, args: &[Arg], span: Span) -> SResult<()> {
        for a in args {
            self.check_expr(&a.value)?;
        }

        // Detect method call syntax: `obj.method(...)` where obj is NOT a tool namespace
        if let ExprKind::Member { obj, name } = &callee.kind {
            let obj_is_value = match &obj.kind {
                ExprKind::Ident(id) => self.is_in_scope(id),
                _ => true, // Expressions like [1, 2].append() or f().upper() are values, not tools
            };

            if obj_is_value {
                let obj_repr = match &obj.kind {
                    ExprKind::Ident(id) => id.clone(),
                    _ => "value".to_string(),
                };
                return Err(SperanError::new(
                    ErrorKind::SyntaxError,
                    format!("Method calls like '.{}()' are not supported", name),
                )
                .at(span)
                .with_hint(builtins::method_hint(name, &obj_repr)));
            }

            if let Some(path) = dotted_path(callee) {
                if path.contains('.') {
                    // This is a tool call like github.get_issue(...)
                    return self.check_tool_call(&path, args, span);
                }
            }
            return Err(SperanError::new(
                ErrorKind::SyntaxError,
                format!("Method calls like '.{}()' are not supported", name),
            )
            .at(span)
            .with_hint(builtins::method_hint(name, "value")));
        }

        // Direct function call: `name(...)`
        if let ExprKind::Ident(name) = &callee.kind {
            if builtins::is_builtin(name) {
                let npos = args.iter().filter(|a| a.name.is_none()).count();
                let kw_names: Vec<&str> = args.iter().filter_map(|a| a.name.as_deref()).collect();
                builtins::check_arity(name, npos, kw_names.into_iter()).map_err(|e| e.at(span))?;
                return Ok(());
            }

            if !self.is_in_scope(name) {
                // If it's not in scope, check if it was supposed to be a tool call without namespace
                let avail = self.all_visible_names();
                let hint = did_you_mean(name, avail.iter().map(|s| s.as_str()));
                return Err(SperanError::new(
                    ErrorKind::UnknownName,
                    format!("Function '{}' is not defined", name),
                )
                .at(callee.span)
                .maybe_hint(hint));
            }
            return Ok(());
        }

        self.check_expr(callee)
    }

    fn check_tool_call(&self, path: &str, _args: &[Arg], span: Span) -> SResult<()> {
        if !self.declared_tools.contains(path) {
            return Err(SperanError::new(
                ErrorKind::UndeclaredTool,
                format!("Tool '{}' was used but not declared with 'use'", path),
            )
            .at(span)
            .with_hint(format!("Add 'use {}' at the top of your program.", path)));
        }
        Ok(())
    }
}
