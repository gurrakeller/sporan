//! Syntax tree produced by the parser.

use crate::error::Span;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    In,
    NotIn,
    And,
    Or,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
            BinOp::In => "in",
            BinOp::NotIn => "not in",
            BinOp::And => "and",
            BinOp::Or => "or",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Expr { kind, span }
    }
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Num(f64),
    Str(Vec<StrSeg>),
    Bool(bool),
    Null,
    Ident(String),
    List(Vec<Expr>),
    Map(Vec<(String, Expr)>),
    /// `[expr for a, b in iter if cond]`
    Comp { expr: Box<Expr>, vars: Vec<String>, iter: Box<Expr>, cond: Option<Box<Expr>> },
    Unary { op: UnOp, expr: Box<Expr> },
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    Call { callee: Box<Expr>, args: Vec<Arg> },
    Member { obj: Box<Expr>, name: String },
    Index { obj: Box<Expr>, index: Box<Expr> },
    Slice { obj: Box<Expr>, start: Option<Box<Expr>>, end: Option<Box<Expr>> },
    /// `try expr` -> `{value, error}`
    Try(Box<Expr>),
    /// `par x in xs { ... }` -> list of each body's last expression
    Par { vars: Vec<String>, iter: Box<Expr>, body: Vec<Stmt> },
}

#[derive(Debug, Clone)]
pub enum StrSeg {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Use(Vec<ToolRef>),
    Fn(Rc<FnDef>),
    Assign { name: String, op: Option<BinOp>, value: Expr },
    Expr(Expr),
    If { cond: Expr, then: Vec<Stmt>, els: Option<Vec<Stmt>> },
    For { vars: Vec<String>, iter: Expr, body: Vec<Stmt> },
    While { cond: Expr, body: Vec<Stmt> },
    Return(Option<Expr>),
    Emit(Expr),
    Break,
    Continue,
}

#[derive(Debug, Clone)]
pub struct ToolRef {
    pub name: String,
    pub span: Span,
}

#[derive(Debug)]
pub struct FnDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// `github.get_issue` -> Some("github.get_issue") if the expression is a pure
/// chain of identifiers.
pub fn dotted_path(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Ident(n) => Some(n.clone()),
        ExprKind::Member { obj, name } => dotted_path(obj).map(|p| format!("{}.{}", p, name)),
        _ => None,
    }
}
