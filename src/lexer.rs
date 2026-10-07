//! Tokenizer. Newlines end statements, except inside `(...)` and `[...]`.

use crate::error::{ErrorKind, SResult, Span, SperanError};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Num(f64),
    Str(Vec<StrPart>),
    Ident(String),
    // keywords
    Use,
    Fn,
    Return,
    Emit,
    If,
    Else,
    For,
    In,
    While,
    Break,
    Continue,
    Par,
    Try,
    And,
    Or,
    Not,
    True,
    False,
    Null,
    // punctuation
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
    Semi,
    // operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    EqEq,
    NotEq,
    Lt,
    Gt,
    Le,
    Ge,
    Newline,
    Eof,
}

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Num(n) => format!("number {}", n),
            Tok::Str(_) => "a string".into(),
            Tok::Ident(s) => format!("'{}'", s),
            Tok::Newline => "end of line".into(),
            Tok::Eof => "end of program".into(),
            other => format!("'{}'", other.symbol()),
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            Tok::Use => "use",
            Tok::Fn => "fn",
            Tok::Return => "return",
            Tok::Emit => "emit",
            Tok::If => "if",
            Tok::Else => "else",
            Tok::For => "for",
            Tok::In => "in",
            Tok::While => "while",
            Tok::Break => "break",
            Tok::Continue => "continue",
            Tok::Par => "par",
            Tok::Try => "try",
            Tok::And => "and",
            Tok::Or => "or",
            Tok::Not => "not",
            Tok::True => "true",
            Tok::False => "false",
            Tok::Null => "null",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::Comma => ",",
            Tok::Colon => ":",
            Tok::Dot => ".",
            Tok::Semi => ";",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Percent => "%",
            Tok::Assign => "=",
            Tok::PlusAssign => "+=",
            Tok::MinusAssign => "-=",
            Tok::StarAssign => "*=",
            Tok::SlashAssign => "/=",
            Tok::EqEq => "==",
            Tok::NotEq => "!=",
            Tok::Lt => "<",
            Tok::Gt => ">",
            Tok::Le => "<=",
            Tok::Ge => ">=",
            Tok::Num(_) | Tok::Str(_) | Tok::Ident(_) | Tok::Newline | Tok::Eof => "",
        }
    }
}

/// Part of a string literal: plain text or an `{expression}` to interpolate.
#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    Expr(String, Span),
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

pub fn lex(src: &str) -> SResult<Vec<Token>> {
    lex_at(src, Span::new(1, 1))
}

/// Lex starting at a given position (used for string interpolations).
pub fn lex_at(src: &str, origin: Span) -> SResult<Vec<Token>> {
    let mut lx = Lexer {
        chars: src.chars().collect(),
        pos: 0,
        line: origin.line,
        col: origin.col,
        toks: Vec::new(),
        brackets: Vec::new(),
    };
    lx.run()?;
    Ok(lx.toks)
}

fn syntax(msg: impl Into<String>, span: Span) -> SperanError {
    SperanError::new(ErrorKind::SyntaxError, msg).at(span)
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    toks: Vec<Token>,
    brackets: Vec<char>,
}

impl Lexer {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn span(&self) -> Span {
        Span::new(self.line, self.col)
    }

    fn push(&mut self, tok: Tok, span: Span) {
        self.toks.push(Token { tok, span });
    }

    fn newlines_significant(&self) -> bool {
        !matches!(self.brackets.last(), Some('(') | Some('['))
    }

    fn run(&mut self) -> SResult<()> {
        while let Some(c) = self.peek() {
            let start = self.span();
            match c {
                ' ' | '\t' | '\r' => {
                    self.bump();
                }
                '\n' => {
                    self.bump();
                    let last_is_break =
                        matches!(self.toks.last().map(|t| &t.tok), None | Some(Tok::Newline));
                    if self.newlines_significant() && !last_is_break {
                        self.push(Tok::Newline, start);
                    }
                }
                '#' => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                '"' => {
                    let parts = self.string()?;
                    self.push(Tok::Str(parts), start);
                }
                '\'' => {
                    return Err(syntax("Strings use double quotes", start)
                        .with_hint("Write \"text\" instead of 'text'."))
                }
                '0'..='9' => {
                    let n = self.number();
                    self.push(Tok::Num(n), start);
                }
                c if c.is_alphabetic() || c == '_' => {
                    let mut s = String::new();
                    while let Some(c) = self.peek() {
                        if c.is_alphanumeric() || c == '_' {
                            s.push(c);
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    let tok = match s.as_str() {
                        "use" => Tok::Use,
                        "fn" => Tok::Fn,
                        "return" => Tok::Return,
                        "emit" => Tok::Emit,
                        "if" => Tok::If,
                        "else" => Tok::Else,
                        "for" => Tok::For,
                        "in" => Tok::In,
                        "while" => Tok::While,
                        "break" => Tok::Break,
                        "continue" => Tok::Continue,
                        "par" => Tok::Par,
                        "try" => Tok::Try,
                        "and" => Tok::And,
                        "or" => Tok::Or,
                        "not" => Tok::Not,
                        "true" => Tok::True,
                        "false" => Tok::False,
                        "null" => Tok::Null,
                        _ => Tok::Ident(s),
                    };
                    self.push(tok, start);
                }
                _ => self.symbol(start)?,
            }
        }
        let end = self.span();
        self.push(Tok::Eof, end);
        Ok(())
    }

    fn number(&mut self) -> f64 {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                if c != '_' {
                    s.push(c);
                }
                self.bump();
            } else {
                break;
            }
        }
        let has_fraction = self.peek() == Some('.')
            && self.chars.get(self.pos + 1).is_some_and(|c| c.is_ascii_digit());
        if has_fraction {
            s.push('.');
            self.bump();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    s.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
        }
        s.parse().unwrap_or(0.0)
    }

    fn string(&mut self) -> SResult<Vec<StrPart>> {
        let start = self.span();
        self.bump(); // opening quote
        let unterminated = || {
            syntax("Unterminated string", start).with_hint("Close the string with a double quote \".")
        };
        let mut parts = Vec::new();
        let mut lit = String::new();
        loop {
            let c = self.bump().ok_or_else(unterminated)?;
            match c {
                '"' => break,
                '\\' => {
                    let esc_span = self.span();
                    let e = self.bump().ok_or_else(unterminated)?;
                    lit.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        '"' => '"',
                        '\\' => '\\',
                        '{' => '{',
                        '}' => '}',
                        other => {
                            return Err(syntax(format!("Unknown escape '\\{}'", other), esc_span)
                                .with_hint("Valid escapes: \\n \\t \\r \\\" \\\\ \\{ \\}"))
                        }
                    });
                }
                '{' => {
                    if !lit.is_empty() {
                        parts.push(StrPart::Lit(std::mem::take(&mut lit)));
                    }
                    let espan = self.span();
                    let unclosed = || {
                        syntax("Unclosed '{' in string", espan)
                            .with_hint("Close the interpolation with '}', or write \\{ for a literal brace.")
                    };
                    let mut depth = 1;
                    let mut src = String::new();
                    loop {
                        let c = self.bump().ok_or_else(unclosed)?;
                        match c {
                            '{' => {
                                depth += 1;
                                src.push(c);
                            }
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                src.push(c);
                            }
                            '\\' if self.peek() == Some('"') => {
                                self.bump(); // consume '"'
                                src.push('"');
                                loop {
                                    let d = self.bump().ok_or_else(unclosed)?;
                                    if d == '\\' && self.peek() == Some('"') {
                                        self.bump();
                                        src.push('"');
                                        break;
                                    }
                                    src.push(d);
                                    if d == '\\' {
                                        if let Some(e) = self.bump() {
                                            src.push(e);
                                        }
                                        continue;
                                    }
                                    if d == '"' {
                                        break;
                                    }
                                }
                            }
                            '"' => {
                                // nested string literal inside the interpolation
                                src.push(c);
                                loop {
                                    let d = self.bump().ok_or_else(unclosed)?;
                                    src.push(d);
                                    if d == '\\' {
                                        if let Some(e) = self.bump() {
                                            src.push(e);
                                        }
                                        continue;
                                    }
                                    if d == '"' {
                                        break;
                                    }
                                }
                            }
                            _ => src.push(c),
                        }
                    }
                    if src.trim().is_empty() {
                        return Err(syntax("Empty interpolation '{}' in string", espan)
                            .with_hint("Put an expression inside, e.g. \"{name}\", or write \\{ for a literal brace."));
                    }
                    parts.push(StrPart::Expr(src, espan));
                }
                _ => lit.push(c),
            }
        }
        if !lit.is_empty() || parts.is_empty() {
            parts.push(StrPart::Lit(lit));
        }
        Ok(parts)
    }

    fn symbol(&mut self, start: Span) -> SResult<()> {
        let c = self.bump().unwrap_or('\0');
        let next = self.peek();
        let two = |lx: &mut Lexer, t: Tok| {
            lx.bump();
            t
        };
        let tok = match (c, next) {
            ('(', _) => {
                self.brackets.push('(');
                Tok::LParen
            }
            ('[', _) => {
                self.brackets.push('[');
                Tok::LBracket
            }
            ('{', _) => {
                self.brackets.push('{');
                Tok::LBrace
            }
            (')', _) => {
                self.brackets.pop();
                Tok::RParen
            }
            (']', _) => {
                self.brackets.pop();
                Tok::RBracket
            }
            ('}', _) => {
                self.brackets.pop();
                Tok::RBrace
            }
            (',', _) => Tok::Comma,
            (':', _) => Tok::Colon,
            ('.', _) => Tok::Dot,
            (';', _) => Tok::Semi,
            ('+', Some('=')) => two(self, Tok::PlusAssign),
            ('+', _) => Tok::Plus,
            ('-', Some('=')) => two(self, Tok::MinusAssign),
            ('-', _) => Tok::Minus,
            ('*', Some('=')) => two(self, Tok::StarAssign),
            ('*', Some('*')) => {
                return Err(syntax("There is no '**' operator", start).with_hint("Use pow(base, exponent)."))
            }
            ('*', _) => Tok::Star,
            ('/', Some('=')) => two(self, Tok::SlashAssign),
            ('/', Some('/')) | ('/', Some('*')) => {
                return Err(syntax("Comments start with '#'", start).with_hint("Write # comment instead of // or /* */."))
            }
            ('/', _) => Tok::Slash,
            ('%', _) => Tok::Percent,
            ('=', Some('=')) => two(self, Tok::EqEq),
            ('=', Some('>')) => {
                return Err(syntax("There are no arrow functions", start)
                    .with_hint("Define a named function with fn name(args) { ... }, or use a comprehension."))
            }
            ('=', _) => Tok::Assign,
            ('!', Some('=')) => two(self, Tok::NotEq),
            ('!', _) => return Err(syntax("Use 'not' instead of '!'", start).with_hint("Write: not x")),
            ('<', Some('=')) => two(self, Tok::Le),
            ('<', _) => Tok::Lt,
            ('>', Some('=')) => two(self, Tok::Ge),
            ('>', _) => Tok::Gt,
            ('&', Some('&')) => return Err(syntax("Use 'and' instead of '&&'", start).with_hint("Write: a and b")),
            ('|', Some('|')) => return Err(syntax("Use 'or' instead of '||'", start).with_hint("Write: a or b")),
            (other, _) => return Err(syntax(format!("Unexpected character '{}'", other), start)),
        };
        self.push(tok, start);
        Ok(())
    }
}
