//! Recursive-descent parser.

use crate::ast::*;
use crate::error::{ErrorKind, SResult, Span, SperanError};
use crate::lexer::{lex, lex_at, StrPart, Tok, Token};
use std::rc::Rc;

pub fn parse(src: &str) -> SResult<Vec<Stmt>> {
    let toks = lex(src)?;
    Parser { toks, pos: 0 }.program()
}

const BRACE_HINT: &str = "Inside strings, {expr} interpolates a value. Write \\{ for a literal brace.";

fn syntax(msg: impl Into<String>, span: Span) -> SperanError {
    SperanError::new(ErrorKind::SyntaxError, msg).at(span)
}

/// Keywords from other languages that LLMs commonly emit, with the fix.
fn foreign_keyword_hint(word: &str) -> Option<&'static str> {
    Some(match word {
        "def" | "function" | "func" => "Define functions with: fn name(a, b) { ... }",
        "let" | "var" | "const" | "val" => "Speran has no declarations. Just assign: x = 1",
        "elif" | "elsif" | "elseif" => "Use 'else if'.",
        "import" | "from" | "require" | "include" => {
            "Speran has no imports. Declare tools at the top with: use namespace.tool"
        }
        "lambda" => "Speran has no lambdas. Define a named fn, or use a comprehension.",
        "class" | "struct" | "interface" => "Speran has no classes. Use maps, e.g. {name: \"x\", age: 3}",
        "async" | "await" => "Tool calls are synchronous. Use 'par x in xs { ... }' for concurrency.",
        "throw" | "raise" => "Speran has no exceptions. Return an error value, e.g. return {error: \"...\"}",
        "switch" | "match" | "case" => "Use if / else if / else.",
        "do" | "loop" | "until" => "Use 'while cond { ... }' or 'for x in xs { ... }'.",
        _ => return None,
    })
}

fn foreign_value_hint(word: &str) -> Option<&'static str> {
    Some(match word {
        "True" => "Use lowercase: true",
        "False" => "Use lowercase: false",
        "None" | "nil" | "undefined" | "NULL" | "Null" => "Use: null",
        _ => return None,
    })
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

impl Parser {
    // ----- token helpers -------------------------------------------------

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        let i = (self.pos + n).min(self.toks.len() - 1);
        &self.toks[i].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn advance(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn check(&self, t: &Tok) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(t)
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.check(t) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn unexpected(&self, expected: &str) -> SperanError {
        syntax(format!("Expected {}, found {}", expected, self.peek().describe()), self.span())
    }

    fn expect(&mut self, t: &Tok, what: &str) -> SResult<Token> {
        if self.check(t) {
            Ok(self.advance())
        } else {
            Err(self.unexpected(what))
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Tok::Newline) {
            self.advance();
        }
    }

    fn skip_separators(&mut self) {
        while matches!(self.peek(), Tok::Newline | Tok::Semi) {
            self.advance();
        }
    }

    fn end_stmt(&mut self) -> SResult<()> {
        match self.peek() {
            Tok::Newline | Tok::Semi => {
                self.advance();
                Ok(())
            }
            Tok::Eof | Tok::RBrace => Ok(()),
            Tok::Colon => Err(self
                .unexpected("end of statement")
                .with_hint("Speran uses braces for blocks, e.g. if x > 1 { ... }")),
            _ => Err(self
                .unexpected("end of statement")
                .with_hint("Put each statement on its own line, or separate them with ';'.")),
        }
    }

    fn ident(&mut self, what: &str) -> SResult<String> {
        match self.peek().clone() {
            Tok::Ident(n) => {
                self.advance();
                Ok(n)
            }
            t if !t.symbol().is_empty() && t.symbol().chars().all(char::is_alphabetic) => {
                Err(syntax(format!("'{}' is a reserved word and cannot be used as {}", t.symbol(), what), self.span()))
            }
            _ => Err(self.unexpected(what)),
        }
    }

    /// Field names after '.' may be any word, including keywords (e.g. `r.error`).
    fn field_name(&mut self) -> SResult<String> {
        match self.peek().clone() {
            Tok::Ident(n) => {
                self.advance();
                Ok(n)
            }
            t if !t.symbol().is_empty() && t.symbol().chars().all(char::is_alphabetic) => {
                self.advance();
                Ok(t.symbol().to_string())
            }
            _ => Err(self.unexpected("a field name after '.'")),
        }
    }

    fn loop_vars(&mut self) -> SResult<Vec<String>> {
        let mut vars = vec![self.ident("a loop variable")?];
        while self.eat(&Tok::Comma) {
            vars.push(self.ident("a loop variable")?);
        }
        Ok(vars)
    }

    // ----- statements ----------------------------------------------------

    fn program(&mut self) -> SResult<Vec<Stmt>> {
        let mut stmts = Vec::new();
        loop {
            self.skip_separators();
            if self.check(&Tok::Eof) {
                break;
            }
            if self.check(&Tok::RBrace) {
                return Err(syntax("Unexpected '}'", self.span()).with_hint("Check that every '{' has exactly one matching '}'."));
            }
            stmts.push(self.statement()?);
            self.end_stmt()?;
        }
        Ok(stmts)
    }

    fn block(&mut self) -> SResult<Vec<Stmt>> {
        if self.check(&Tok::Colon) {
            return Err(self.unexpected("'{'").with_hint("Speran uses braces for blocks, e.g. if x > 1 { ... }"));
        }
        self.expect(&Tok::LBrace, "'{'")?;
        let mut stmts = Vec::new();
        loop {
            self.skip_separators();
            if self.eat(&Tok::RBrace) {
                break;
            }
            if self.check(&Tok::Eof) {
                return Err(self.unexpected("'}'").with_hint("A block was opened with '{' but never closed."));
            }
            stmts.push(self.statement()?);
            self.end_stmt()?;
        }
        Ok(stmts)
    }

    fn statement(&mut self) -> SResult<Stmt> {
        let span = self.span();
        let kind = match self.peek().clone() {
            Tok::Use => {
                self.advance();
                let mut tools = Vec::new();
                loop {
                    let s = self.span();
                    let mut name = self.ident("a tool name")?;
                    while self.eat(&Tok::Dot) {
                        name.push('.');
                        name.push_str(&self.field_name()?);
                    }
                    tools.push(ToolRef { name, span: s });
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                    self.skip_newlines();
                }
                StmtKind::Use(tools)
            }
            Tok::Fn => {
                self.advance();
                let name = self.ident("a function name")?;
                self.expect(&Tok::LParen, "'(' after the function name")?;
                let mut params = Vec::new();
                while !self.check(&Tok::RParen) {
                    let p_span = self.span();
                    let p = self.ident("a parameter name")?;
                    if params.contains(&p) {
                        return Err(syntax(format!("Parameter '{}' is listed twice", p), p_span));
                    }
                    params.push(p);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(&Tok::RParen, "')'")?;
                let body = self.block()?;
                StmtKind::Fn(Rc::new(FnDef { name, params, body, span }))
            }
            Tok::Return => {
                self.advance();
                if matches!(self.peek(), Tok::Newline | Tok::Semi | Tok::Eof | Tok::RBrace) {
                    StmtKind::Return(None)
                } else {
                    StmtKind::Return(Some(self.expr()?))
                }
            }
            Tok::Emit => {
                self.advance();
                StmtKind::Emit(self.expr()?)
            }
            Tok::If => self.if_stmt()?,
            Tok::For => {
                self.advance();
                let vars = self.loop_vars()?;
                self.expect(&Tok::In, "'in'")?;
                let iter = self.expr()?;
                let body = self.block()?;
                StmtKind::For { vars, iter, body }
            }
            Tok::While => {
                self.advance();
                let cond = self.expr()?;
                let body = self.block()?;
                StmtKind::While { cond, body }
            }
            Tok::Break => {
                self.advance();
                StmtKind::Break
            }
            Tok::Continue => {
                self.advance();
                StmtKind::Continue
            }
            Tok::Ident(word)
                if foreign_keyword_hint(&word).is_some()
                    && !matches!(
                        self.peek_at(1),
                        Tok::Assign | Tok::PlusAssign | Tok::MinusAssign | Tok::StarAssign | Tok::SlashAssign
                    ) =>
            {
                return Err(syntax(format!("'{}' is not a Speran keyword", word), span)
                    .with_hint(foreign_keyword_hint(&word).unwrap_or_default()));
            }
            _ => {
                let target = self.expr()?;
                let op = match self.peek() {
                    Tok::Assign => Some(None),
                    Tok::PlusAssign => Some(Some(BinOp::Add)),
                    Tok::MinusAssign => Some(Some(BinOp::Sub)),
                    Tok::StarAssign => Some(Some(BinOp::Mul)),
                    Tok::SlashAssign => Some(Some(BinOp::Div)),
                    _ => None,
                };
                match op {
                    None => StmtKind::Expr(target),
                    Some(op) => {
                        let op_span = self.span();
                        self.advance();
                        self.skip_newlines();
                        let value = self.expr()?;
                        match target.kind {
                            ExprKind::Ident(name) => StmtKind::Assign { name, op, value },
                            ExprKind::Index { .. } | ExprKind::Member { .. } => {
                                return Err(SperanError::new(
                                    ErrorKind::ImmutableValue,
                                    "Lists and maps cannot be modified in place",
                                )
                                .at(op_span)
                                .with_hint("Values are immutable. Build a new value instead: m = m + {key: value} or xs = xs + [item]"))
                            }
                            _ => {
                                return Err(syntax("Invalid assignment target", target.span)
                                    .with_hint("Only plain variable names can be assigned, e.g. x = 1"))
                            }
                        }
                    }
                }
            }
        };
        Ok(Stmt { kind, span })
    }

    fn if_stmt(&mut self) -> SResult<StmtKind> {
        self.advance(); // 'if'
        let cond = self.expr()?;
        let then = self.block()?;
        // `else` may be on the line after the closing brace
        let save = self.pos;
        self.skip_newlines();
        let els = if self.eat(&Tok::Else) {
            if self.check(&Tok::If) {
                let span = self.span();
                let kind = self.if_stmt()?;
                Some(vec![Stmt { kind, span }])
            } else {
                Some(self.block()?)
            }
        } else {
            self.pos = save;
            None
        };
        Ok(StmtKind::If { cond, then, els })
    }

    // ----- expressions ---------------------------------------------------

    pub fn expr(&mut self) -> SResult<Expr> {
        self.or_expr()
    }

    fn binary(op: BinOp, left: Expr, right: Expr, span: Span) -> Expr {
        Expr::new(ExprKind::Binary { op, left: Box::new(left), right: Box::new(right) }, span)
    }

    fn or_expr(&mut self) -> SResult<Expr> {
        let mut left = self.and_expr()?;
        while self.check(&Tok::Or) {
            let span = self.span();
            self.advance();
            self.skip_newlines();
            let right = self.and_expr()?;
            left = Self::binary(BinOp::Or, left, right, span);
        }
        Ok(left)
    }

    fn and_expr(&mut self) -> SResult<Expr> {
        let mut left = self.not_expr()?;
        while self.check(&Tok::And) {
            let span = self.span();
            self.advance();
            self.skip_newlines();
            let right = self.not_expr()?;
            left = Self::binary(BinOp::And, left, right, span);
        }
        Ok(left)
    }

    fn not_expr(&mut self) -> SResult<Expr> {
        if self.check(&Tok::Not) {
            let span = self.span();
            self.advance();
            let e = self.not_expr()?;
            return Ok(Expr::new(ExprKind::Unary { op: UnOp::Not, expr: Box::new(e) }, span));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> SResult<Expr> {
        let mut left = self.additive()?;
        loop {
            let op = match self.peek() {
                Tok::EqEq => BinOp::Eq,
                Tok::NotEq => BinOp::Ne,
                Tok::Lt => BinOp::Lt,
                Tok::Gt => BinOp::Gt,
                Tok::Le => BinOp::Le,
                Tok::Ge => BinOp::Ge,
                Tok::In => BinOp::In,
                Tok::Not if matches!(self.peek_at(1), Tok::In) => BinOp::NotIn,
                _ => break,
            };
            let span = self.span();
            self.advance();
            if op == BinOp::NotIn {
                self.advance();
            }
            self.skip_newlines();
            let right = self.additive()?;
            left = Self::binary(op, left, right, span);
        }
        Ok(left)
    }

    fn additive(&mut self) -> SResult<Expr> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            let span = self.span();
            self.advance();
            self.skip_newlines();
            let right = self.multiplicative()?;
            left = Self::binary(op, left, right, span);
        }
        Ok(left)
    }

    fn multiplicative(&mut self) -> SResult<Expr> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                _ => break,
            };
            let span = self.span();
            self.advance();
            self.skip_newlines();
            let right = self.unary()?;
            left = Self::binary(op, left, right, span);
        }
        Ok(left)
    }

    fn unary(&mut self) -> SResult<Expr> {
        let span = self.span();
        match self.peek() {
            Tok::Minus => {
                self.advance();
                let e = self.unary()?;
                Ok(Expr::new(ExprKind::Unary { op: UnOp::Neg, expr: Box::new(e) }, span))
            }
            Tok::Try => {
                self.advance();
                if self.check(&Tok::LBrace) {
                    return Err(syntax("'try' wraps an expression, not a block", span)
                        .with_hint("Write: r = try tool.call(arg: 1), then check r.error"));
                }
                let e = self.unary()?;
                Ok(Expr::new(ExprKind::Try(Box::new(e)), span))
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> SResult<Expr> {
        let mut e = self.primary()?;
        loop {
            let span = self.span();
            match self.peek() {
                Tok::LParen => {
                    self.advance();
                    let args = self.args()?;
                    e = Expr::new(ExprKind::Call { callee: Box::new(e), args }, span);
                }
                Tok::LBracket => {
                    self.advance();
                    let obj = Box::new(e);
                    if self.eat(&Tok::Colon) {
                        let end = self.slice_end()?;
                        e = Expr::new(ExprKind::Slice { obj, start: None, end }, span);
                    } else {
                        let idx = self.expr()?;
                        if self.eat(&Tok::Colon) {
                            let end = self.slice_end()?;
                            e = Expr::new(ExprKind::Slice { obj, start: Some(Box::new(idx)), end }, span);
                        } else {
                            self.expect(&Tok::RBracket, "']'")?;
                            e = Expr::new(ExprKind::Index { obj, index: Box::new(idx) }, span);
                        }
                    }
                }
                Tok::Dot => {
                    self.advance();
                    let name = self.field_name()?;
                    e = Expr::new(ExprKind::Member { obj: Box::new(e), name }, span);
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn slice_end(&mut self) -> SResult<Option<Box<Expr>>> {
        let end = if self.check(&Tok::RBracket) { None } else { Some(Box::new(self.expr()?)) };
        self.expect(&Tok::RBracket, "']'")?;
        Ok(end)
    }

    fn args(&mut self) -> SResult<Vec<Arg>> {
        let mut args: Vec<Arg> = Vec::new();
        while !self.check(&Tok::RParen) {
            let span = self.span();
            let mut name = None;
            if let Tok::Ident(n) = self.peek().clone() {
                match self.peek_at(1) {
                    Tok::Colon => {
                        self.advance();
                        self.advance();
                        name = Some(n);
                    }
                    Tok::Assign => {
                        return Err(syntax("Named arguments use ':', not '='", span)
                            .with_hint(format!("Write {}: value instead of {}=value", n, n)))
                    }
                    _ => {}
                }
            }
            let value = self.expr()?;
            if name.is_none() && args.iter().any(|a| a.name.is_some()) {
                return Err(syntax("Positional argument after a named argument", span)
                    .with_hint("Put positional arguments first, then named ones."));
            }
            args.push(Arg { name, value, span });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RParen, "')' or ','")?;
        Ok(args)
    }

    fn primary(&mut self) -> SResult<Expr> {
        let span = self.span();
        let kind = match self.peek().clone() {
            Tok::Num(n) => {
                self.advance();
                ExprKind::Num(n)
            }
            Tok::Str(parts) => {
                self.advance();
                ExprKind::Str(self.string_parts(parts)?)
            }
            Tok::True => {
                self.advance();
                ExprKind::Bool(true)
            }
            Tok::False => {
                self.advance();
                ExprKind::Bool(false)
            }
            Tok::Null => {
                self.advance();
                ExprKind::Null
            }
            Tok::Ident(name) => {
                if let Some(h) = foreign_value_hint(&name) {
                    return Err(syntax(format!("'{}' is not a Speran value", name), span).with_hint(h));
                }
                if let Some(h) = foreign_keyword_hint(&name) {
                    if !matches!(self.peek_at(1), Tok::Dot | Tok::LBracket | Tok::Newline | Tok::Eof | Tok::RParen | Tok::Comma) {
                        return Err(syntax(format!("'{}' is not a Speran keyword", name), span).with_hint(h));
                    }
                }
                self.advance();
                ExprKind::Ident(name)
            }
            Tok::LParen => {
                self.advance();
                let e = self.expr()?;
                self.expect(&Tok::RParen, "')'")?;
                return Ok(e);
            }
            Tok::LBracket => {
                self.advance();
                return self.list_or_comp(span);
            }
            Tok::LBrace => {
                self.advance();
                self.map_literal()?
            }
            Tok::Par => {
                self.advance();
                let vars = self.loop_vars()?;
                self.expect(&Tok::In, "'in'")?;
                let iter = self.expr()?;
                let body = self.block()?;
                ExprKind::Par { vars, iter: Box::new(iter), body }
            }
            _ => return Err(self.unexpected("an expression")),
        };
        Ok(Expr::new(kind, span))
    }

    fn string_parts(&self, parts: Vec<StrPart>) -> SResult<Vec<StrSeg>> {
        parts
            .into_iter()
            .map(|p| match p {
                StrPart::Lit(s) => Ok(StrSeg::Lit(s)),
                StrPart::Expr(src, span) => {
                    let toks = lex_at(&src, span).map_err(|e| e.or_hint(BRACE_HINT))?;
                    let mut sub = Parser { toks, pos: 0 };
                    let e = sub.expr().map_err(|e| e.or_hint(BRACE_HINT))?;
                    sub.skip_newlines();
                    if !sub.check(&Tok::Eof) {
                        return Err(sub.unexpected("'}' to end the interpolation").with_hint(BRACE_HINT));
                    }
                    Ok(StrSeg::Expr(e))
                }
            })
            .collect()
    }

    fn list_or_comp(&mut self, span: Span) -> SResult<Expr> {
        if self.eat(&Tok::RBracket) {
            return Ok(Expr::new(ExprKind::List(vec![]), span));
        }
        let first = self.expr()?;
        if self.eat(&Tok::For) {
            let vars = self.loop_vars()?;
            self.expect(&Tok::In, "'in'")?;
            let iter = self.expr()?;
            let cond = if self.eat(&Tok::If) { Some(Box::new(self.expr()?)) } else { None };
            self.expect(&Tok::RBracket, "']'")?;
            return Ok(Expr::new(
                ExprKind::Comp { expr: Box::new(first), vars, iter: Box::new(iter), cond },
                span,
            ));
        }
        let mut items = vec![first];
        while self.eat(&Tok::Comma) {
            if self.check(&Tok::RBracket) {
                break;
            }
            items.push(self.expr()?);
        }
        self.expect(&Tok::RBracket, "']' or ','")?;
        Ok(Expr::new(ExprKind::List(items), span))
    }

    fn map_literal(&mut self) -> SResult<ExprKind> {
        let mut entries = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&Tok::RBrace) {
                break;
            }
            let kspan = self.span();
            let key = match self.peek().clone() {
                Tok::Ident(n) => {
                    self.advance();
                    n
                }
                Tok::Str(parts) => {
                    self.advance();
                    match parts.as_slice() {
                        [StrPart::Lit(s)] => s.clone(),
                        _ => return Err(syntax("Map keys cannot contain interpolation", kspan)),
                    }
                }
                t if !t.symbol().is_empty() && t.symbol().chars().all(char::is_alphabetic) => {
                    self.advance();
                    t.symbol().to_string()
                }
                _ => return Err(self.unexpected("a map key (a name or \"string\")")),
            };
            self.skip_newlines();
            if self.check(&Tok::Assign) {
                return Err(self.unexpected("':'").with_hint("Map entries use ':', e.g. {name: \"x\"}"));
            }
            self.expect(&Tok::Colon, "':' after the map key")?;
            self.skip_newlines();
            let v = self.expr()?;
            entries.push((key, v));
            self.skip_newlines();
            if self.eat(&Tok::Comma) {
                continue;
            }
            self.expect(&Tok::RBrace, "',' or '}'")?;
            break;
        }
        Ok(ExprKind::Map(entries))
    }
}
