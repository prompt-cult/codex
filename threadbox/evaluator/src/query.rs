//! A bounded JSON query language.
//!
//! This is the deterministic data-routing facility the DSL uses for selection,
//! projection, and assertions. It is deliberately not a general jq: there is no
//! arithmetic, no string manipulation, no function definition, no variable
//! binding, and no way to reach anything except the value it was handed.
//!
//! It is a purpose-built subset rather than an adopted dependency because the
//! required operation list is short enough that bounding and auditing it here
//! is cheaper than proving a general engine has no shell, file, or network
//! escape. See `DSL.md` — Expressions.
//!
//! Evaluation is value-in, value-out. `select` yields *absence* rather than a
//! boolean, which is what lets `map(select(...))` filter.

use serde_json::{Map, Value};
use std::fmt;

/// Maximum *structural* nesting depth accepted by the parser: parentheses,
/// brackets, braces, and function arguments. Precedence levels do not count,
/// because they are a fixed cost per term rather than a property of the
/// expression an author wrote. A hostile or generated expression still cannot
/// drive the recursive-descent parser into a stack overflow.
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `.`
    Identity,
    /// `.foo` / `["foo"]` applied to the value on the left
    Field(Box<Expr>, String),
    /// `[3]`
    Index(Box<Expr>, i64),
    /// `[]` — iterate, producing an array of the element values
    Iterate(Box<Expr>),
    Pipe(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Compare(Box<Expr>, CompareOp, Box<Expr>),
    Map(Box<Expr>),
    Select(Box<Expr>),
    Length,
    TypeOf,
    Has(String),
    Literal(Value),
    Object(Vec<(String, Expr)>),
    Array(Vec<Expr>),
    /// `$vars`, `$memo`, `$parents` — an *absolute* reference to a top-level
    /// key of the evaluation context, unaffected by piping or by `map`
    /// rebinding the input. Without these, a filter inside `map(select(...))`
    /// could not see the loop variable it needs to filter on.
    Root(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    pub message: String,
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

fn err<T>(message: impl Into<String>) -> Result<T, QueryError> {
    Err(QueryError {
        message: message.into(),
    })
}

// ---------------------------------------------------------------- lexer

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Dot,
    LBracket,
    RBracket,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Pipe,
    Ident(String),
    Str(String),
    Num(f64),
    Op(CompareOp),
    Root(String),
}

fn lex(src: &str) -> Result<Vec<Token>, QueryError> {
    let bytes: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '.' => {
                out.push(Token::Dot);
                i += 1;
            }
            '[' => {
                out.push(Token::LBracket);
                i += 1;
            }
            ']' => {
                out.push(Token::RBracket);
                i += 1;
            }
            '(' => {
                out.push(Token::LParen);
                i += 1;
            }
            ')' => {
                out.push(Token::RParen);
                i += 1;
            }
            '{' => {
                out.push(Token::LBrace);
                i += 1;
            }
            '}' => {
                out.push(Token::RBrace);
                i += 1;
            }
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            ':' => {
                out.push(Token::Colon);
                i += 1;
            }
            '|' => {
                out.push(Token::Pipe);
                i += 1;
            }
            '=' if i + 1 < bytes.len() && bytes[i + 1] == '=' => {
                out.push(Token::Op(CompareOp::Eq));
                i += 2;
            }
            '!' if i + 1 < bytes.len() && bytes[i + 1] == '=' => {
                out.push(Token::Op(CompareOp::Ne));
                i += 2;
            }
            '>' if i + 1 < bytes.len() && bytes[i + 1] == '=' => {
                out.push(Token::Op(CompareOp::Ge));
                i += 2;
            }
            '<' if i + 1 < bytes.len() && bytes[i + 1] == '=' => {
                out.push(Token::Op(CompareOp::Le));
                i += 2;
            }
            '>' => {
                out.push(Token::Op(CompareOp::Gt));
                i += 1;
            }
            '<' => {
                out.push(Token::Op(CompareOp::Lt));
                i += 1;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                loop {
                    if i >= bytes.len() {
                        return err("unterminated string literal in expression");
                    }
                    match bytes[i] {
                        '"' => {
                            i += 1;
                            break;
                        }
                        '\\' => {
                            i += 1;
                            if i >= bytes.len() {
                                return err("unterminated escape in expression string");
                            }
                            match bytes[i] {
                                'n' => s.push('\n'),
                                't' => s.push('\t'),
                                'r' => s.push('\r'),
                                '"' => s.push('"'),
                                '\\' => s.push('\\'),
                                other => {
                                    return err(format!(
                                        "escape '\\{other}' is not recognized in an expression string"
                                    ))
                                }
                            }
                            i += 1;
                        }
                        ch => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push(Token::Str(s));
            }
            c if c.is_ascii_digit() || c == '-' => {
                let start = i;
                if bytes[i] == '-' {
                    i += 1;
                }
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == '.') {
                    // A '.' directly after digits is a decimal point only when a
                    // digit follows; otherwise it is a field access.
                    if bytes[i] == '.' && !(i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()) {
                        break;
                    }
                    i += 1;
                }
                let text: String = bytes[start..i].iter().collect();
                match text.parse::<f64>() {
                    Ok(n) => out.push(Token::Num(n)),
                    Err(_) => return err(format!("'{text}' is not a valid number in an expression")),
                }
            }
            '$' => {
                i += 1;
                let start = i;
                while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
                    i += 1;
                }
                if start == i {
                    return err("'$' must be followed by a name, as in $vars or $memo");
                }
                out.push(Token::Root(bytes[start..i].iter().collect()));
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
                    i += 1;
                }
                out.push(Token::Ident(bytes[start..i].iter().collect()));
            }
            other => return err(format!("character '{other}' is not valid in an expression")),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- parser

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn eat(&mut self, want: &Token) -> Result<(), QueryError> {
        match self.peek() {
            Some(t) if t == want => {
                self.pos += 1;
                Ok(())
            }
            Some(t) => err(format!("expected {want:?} but found {t:?}")),
            None => err(format!("expected {want:?} but the expression ended")),
        }
    }

    fn parse_pipe(&mut self, depth: usize) -> Result<Expr, QueryError> {
        if depth > MAX_DEPTH {
            return err(format!("expression nests deeper than the limit of {MAX_DEPTH}"));
        }
        let mut left = self.parse_or(depth)?;
        while matches!(self.peek(), Some(Token::Pipe)) {
            self.pos += 1;
            let right = self.parse_or(depth)?;
            left = Expr::Pipe(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_or(&mut self, depth: usize) -> Result<Expr, QueryError> {
        let mut left = self.parse_and(depth)?;
        while matches!(self.peek(), Some(Token::Ident(w)) if w == "or") {
            self.pos += 1;
            let right = self.parse_and(depth)?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self, depth: usize) -> Result<Expr, QueryError> {
        let mut left = self.parse_compare(depth)?;
        while matches!(self.peek(), Some(Token::Ident(w)) if w == "and") {
            self.pos += 1;
            let right = self.parse_compare(depth)?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_compare(&mut self, depth: usize) -> Result<Expr, QueryError> {
        let left = self.parse_unary(depth)?;
        if let Some(Token::Op(op)) = self.peek().cloned() {
            self.pos += 1;
            let right = self.parse_unary(depth)?;
            return Ok(Expr::Compare(Box::new(left), op, Box::new(right)));
        }
        Ok(left)
    }

    fn parse_unary(&mut self, depth: usize) -> Result<Expr, QueryError> {
        self.parse_postfix(depth)
    }

    fn parse_postfix(&mut self, depth: usize) -> Result<Expr, QueryError> {
        let mut expr = self.parse_primary(depth)?;
        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    self.pos += 1;
                    match self.peek().cloned() {
                        Some(Token::Ident(name)) => {
                            self.pos += 1;
                            expr = Expr::Field(Box::new(expr), name);
                        }
                        other => return err(format!("expected a field name after '.' but found {other:?}")),
                    }
                }
                Some(Token::LBracket) => {
                    self.pos += 1;
                    match self.peek().cloned() {
                        Some(Token::RBracket) => {
                            self.pos += 1;
                            expr = Expr::Iterate(Box::new(expr));
                        }
                        Some(Token::Num(n)) => {
                            self.pos += 1;
                            self.eat(&Token::RBracket)?;
                            expr = Expr::Index(Box::new(expr), n as i64);
                        }
                        Some(Token::Str(s)) => {
                            self.pos += 1;
                            self.eat(&Token::RBracket)?;
                            expr = Expr::Field(Box::new(expr), s);
                        }
                        other => {
                            return err(format!(
                                "expected an index, a quoted key, or ']' but found {other:?}"
                            ))
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self, depth: usize) -> Result<Expr, QueryError> {
        if depth > MAX_DEPTH {
            return err(format!("expression nests deeper than the limit of {MAX_DEPTH}"));
        }
        match self.peek().cloned() {
            Some(Token::Dot) => {
                // `.` alone, or `.name` — the postfix loop handles the rest.
                self.pos += 1;
                match self.peek().cloned() {
                    Some(Token::Ident(name)) => {
                        self.pos += 1;
                        Ok(Expr::Field(Box::new(Expr::Identity), name))
                    }
                    _ => Ok(Expr::Identity),
                }
            }
            Some(Token::Root(name)) => {
                self.pos += 1;
                match name.as_str() {
                    "parents" | "vars" | "memo" => Ok(Expr::Root(name)),
                    other => err(format!(
                        "'${other}' is not a context reference; available: $parents, $vars, $memo"
                    )),
                }
            }
            Some(Token::Num(n)) => {
                self.pos += 1;
                Ok(Expr::Literal(
                    serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null),
                ))
            }
            Some(Token::Str(s)) => {
                self.pos += 1;
                Ok(Expr::Literal(Value::String(s)))
            }
            Some(Token::LParen) => {
                self.pos += 1;
                let inner = self.parse_pipe(depth + 1)?;
                self.eat(&Token::RParen)?;
                Ok(inner)
            }
            Some(Token::LBracket) => {
                self.pos += 1;
                let mut items = Vec::new();
                if !matches!(self.peek(), Some(Token::RBracket)) {
                    loop {
                        items.push(self.parse_pipe(depth + 1)?);
                        if matches!(self.peek(), Some(Token::Comma)) {
                            self.pos += 1;
                            continue;
                        }
                        break;
                    }
                }
                self.eat(&Token::RBracket)?;
                Ok(Expr::Array(items))
            }
            Some(Token::LBrace) => {
                self.pos += 1;
                let mut pairs = Vec::new();
                if !matches!(self.peek(), Some(Token::RBrace)) {
                    loop {
                        let key = match self.peek().cloned() {
                            Some(Token::Ident(k)) => k,
                            Some(Token::Str(k)) => k,
                            other => return err(format!("expected an object key but found {other:?}")),
                        };
                        self.pos += 1;
                        self.eat(&Token::Colon)?;
                        pairs.push((key, self.parse_pipe(depth + 1)?));
                        if matches!(self.peek(), Some(Token::Comma)) {
                            self.pos += 1;
                            continue;
                        }
                        break;
                    }
                }
                self.eat(&Token::RBrace)?;
                Ok(Expr::Object(pairs))
            }
            Some(Token::Ident(word)) => {
                self.pos += 1;
                match word.as_str() {
                    "true" => Ok(Expr::Literal(Value::Bool(true))),
                    "false" => Ok(Expr::Literal(Value::Bool(false))),
                    "null" => Ok(Expr::Literal(Value::Null)),
                    "length" => Ok(Expr::Length),
                    "type" => Ok(Expr::TypeOf),
                    // `not` negates its *input*, as in jq: `.matches | not`.
                    // There is no prefix form, so `not .a` is a parse error
                    // rather than a silent reinterpretation.
                    "not" => Ok(Expr::Not(Box::new(Expr::Identity))),
                    "map" => {
                        self.eat(&Token::LParen)?;
                        let inner = self.parse_pipe(depth + 1)?;
                        self.eat(&Token::RParen)?;
                        Ok(Expr::Map(Box::new(inner)))
                    }
                    "select" => {
                        self.eat(&Token::LParen)?;
                        let inner = self.parse_pipe(depth + 1)?;
                        self.eat(&Token::RParen)?;
                        Ok(Expr::Select(Box::new(inner)))
                    }
                    "has" => {
                        self.eat(&Token::LParen)?;
                        let key = match self.peek().cloned() {
                            Some(Token::Str(k)) => k,
                            other => {
                                return err(format!("has() takes a quoted key but found {other:?}"))
                            }
                        };
                        self.pos += 1;
                        self.eat(&Token::RParen)?;
                        Ok(Expr::Has(key))
                    }
                    other => err(format!(
                        "'{other}' is not part of the expression language; \
                         available: map, select, length, type, has, not, and, or, true, false, null"
                    )),
                }
            }
            other => err(format!("unexpected {other:?} at the start of an expression")),
        }
    }
}

/// Parse an expression. Parsing is separate from evaluation so a graph can be
/// checked for expression validity before it runs — see `IR.md` validator 7.
pub fn parse(src: &str) -> Result<Expr, QueryError> {
    let tokens = lex(src)?;
    if tokens.is_empty() {
        return err("expression is empty");
    }
    let mut parser = Parser { tokens, pos: 0 };
    let expr = parser.parse_pipe(0)?;
    if parser.pos != parser.tokens.len() {
        return err(format!(
            "expression has trailing tokens from position {}; the whole string must be one expression",
            parser.pos
        ));
    }
    Ok(expr)
}

// ---------------------------------------------------------------- eval

/// `None` means *absent*: what `select` produces when its condition is false.
/// `map` drops absent elements, which is how filtering works without streams.
type Outcome = Result<Option<Value>, QueryError>;

fn truthy(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
}

/// Structural equality that compares numbers by value rather than by storage.
/// `serde_json` distinguishes the integer `1` from the float `1.0`, but a
/// literal in an expression is parsed as a float, so a document's `1` must
/// still equal the expression's `1`.
fn equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => a == b,
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| equal(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).map(|other| equal(v, other)).unwrap_or(false))
        }
        _ => left == right,
    }
}

fn compare(left: &Value, op: CompareOp, right: &Value) -> Result<bool, QueryError> {
    match op {
        CompareOp::Eq => return Ok(equal(left, right)),
        CompareOp::Ne => return Ok(!equal(left, right)),
        _ => {}
    }
    let ordering = match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap_or(f64::NAN), b.as_f64().unwrap_or(f64::NAN));
            a.partial_cmp(&b)
        }
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        _ => {
            return err(format!(
                "cannot order {} against {}; only numbers and strings are ordered",
                type_name(left),
                type_name(right)
            ))
        }
    };
    let Some(ordering) = ordering else {
        return err("cannot order values that are not comparable".to_string());
    };
    Ok(match op {
        CompareOp::Lt => ordering.is_lt(),
        CompareOp::Le => ordering.is_le(),
        CompareOp::Gt => ordering.is_gt(),
        CompareOp::Ge => ordering.is_ge(),
        CompareOp::Eq | CompareOp::Ne => unreachable!("handled above"),
    })
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn required(outcome: Outcome, what: &str) -> Result<Value, QueryError> {
    match outcome? {
        Some(v) => Ok(v),
        None => err(format!("{what} received an absent value; select() is only meaningful inside map()")),
    }
}

fn eval_inner(expr: &Expr, input: &Value, root: &Value) -> Outcome {
    match expr {
        Expr::Identity => Ok(Some(input.clone())),
        Expr::Root(name) => Ok(Some(
            root.get(name).cloned().unwrap_or(Value::Null),
        )),
        Expr::Literal(v) => Ok(Some(v.clone())),
        Expr::Field(base, name) => {
            let base = required(eval_inner(base, input, root), "a field access")?;
            match base {
                Value::Object(map) => Ok(Some(map.get(name).cloned().unwrap_or(Value::Null))),
                Value::Null => Ok(Some(Value::Null)),
                other => err(format!(
                    "cannot read field \"{name}\" from {}; a field access needs an object",
                    type_name(&other)
                )),
            }
        }
        Expr::Index(base, idx) => {
            let base = required(eval_inner(base, input, root), "an index")?;
            match base {
                Value::Array(items) => {
                    let len = items.len() as i64;
                    let resolved = if *idx < 0 { len + idx } else { *idx };
                    if resolved < 0 || resolved >= len {
                        Ok(Some(Value::Null))
                    } else {
                        Ok(Some(items[resolved as usize].clone()))
                    }
                }
                Value::Null => Ok(Some(Value::Null)),
                other => err(format!(
                    "cannot index {} with [{idx}]; indexing needs an array",
                    type_name(&other)
                )),
            }
        }
        Expr::Iterate(base) => {
            let base = required(eval_inner(base, input, root), "an iteration")?;
            match base {
                Value::Array(items) => Ok(Some(Value::Array(items))),
                Value::Object(map) => Ok(Some(Value::Array(map.values().cloned().collect()))),
                other => err(format!(
                    "cannot iterate {}; [] needs an array or an object",
                    type_name(&other)
                )),
            }
        }
        Expr::Pipe(left, right) => match eval_inner(left, input, root)? {
            Some(value) => eval_inner(right, &value, root),
            None => Ok(None),
        },
        Expr::And(a, b) => {
            let left = required(eval_inner(a, input, root), "'and'")?;
            if !truthy(&left) {
                return Ok(Some(Value::Bool(false)));
            }
            let right = required(eval_inner(b, input, root), "'and'")?;
            Ok(Some(Value::Bool(truthy(&right))))
        }
        Expr::Or(a, b) => {
            let left = required(eval_inner(a, input, root), "'or'")?;
            if truthy(&left) {
                return Ok(Some(Value::Bool(true)));
            }
            let right = required(eval_inner(b, input, root), "'or'")?;
            Ok(Some(Value::Bool(truthy(&right))))
        }
        Expr::Not(inner) => {
            let value = required(eval_inner(inner, input, root), "'not'")?;
            Ok(Some(Value::Bool(!truthy(&value))))
        }
        Expr::Compare(a, op, b) => {
            let left = required(eval_inner(a, input, root), "a comparison")?;
            let right = required(eval_inner(b, input, root), "a comparison")?;
            Ok(Some(Value::Bool(compare(&left, *op, &right)?)))
        }
        Expr::Length => match input {
            Value::Array(items) => Ok(Some(Value::from(items.len()))),
            Value::Object(map) => Ok(Some(Value::from(map.len()))),
            Value::String(s) => Ok(Some(Value::from(s.chars().count()))),
            Value::Null => Ok(Some(Value::from(0))),
            other => err(format!("length is not defined for {}", type_name(other))),
        },
        Expr::TypeOf => Ok(Some(Value::String(type_name(input).to_string()))),
        Expr::Has(key) => match input {
            Value::Object(map) => Ok(Some(Value::Bool(map.contains_key(key)))),
            other => err(format!(
                "has(\"{key}\") is not defined for {}; it needs an object",
                type_name(other)
            )),
        },
        Expr::Select(cond) => {
            let keep = required(eval_inner(cond, input, root), "select()")?;
            if truthy(&keep) {
                Ok(Some(input.clone()))
            } else {
                Ok(None)
            }
        }
        Expr::Map(inner) => {
            let items = match input {
                Value::Array(items) => items.clone(),
                other => {
                    return err(format!(
                        "map() needs an array but received {}",
                        type_name(other)
                    ))
                }
            };
            let mut out = Vec::with_capacity(items.len());
            for item in &items {
                if let Some(value) = eval_inner(inner, item, root)? {
                    out.push(value);
                }
            }
            Ok(Some(Value::Array(out)))
        }
        Expr::Array(exprs) => {
            let mut out = Vec::with_capacity(exprs.len());
            for e in exprs {
                if let Some(v) = eval_inner(e, input, root)? {
                    out.push(v);
                }
            }
            Ok(Some(Value::Array(out)))
        }
        Expr::Object(pairs) => {
            let mut map = Map::new();
            for (key, e) in pairs {
                if let Some(v) = eval_inner(e, input, root)? {
                    map.insert(key.clone(), v);
                }
            }
            Ok(Some(Value::Object(map)))
        }
    }
}

/// Evaluate `expr` against `input`. An absent result (a bare `select` that
/// rejected its input) is reported as `null`, because a node value must exist.
pub fn eval(expr: &Expr, input: &Value) -> Result<Value, QueryError> {
    Ok(eval_inner(expr, input, input)?.unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(src: &str, input: Value) -> Value {
        let expr = parse(src).unwrap_or_else(|e| panic!("parse {src}: {e}"));
        eval(&expr, &input).unwrap_or_else(|e| panic!("eval {src}: {e}"))
    }

    #[test]
    fn identity_and_paths() {
        assert_eq!(run(".", json!({ "a": 1 })), json!({ "a": 1 }));
        assert_eq!(run(".a", json!({ "a": 1 })), json!(1));
        assert_eq!(run(".a.b", json!({ "a": { "b": 2 } })), json!(2));
        assert_eq!(run(".parents[0].ok", json!({ "parents": [{ "ok": true }] })), json!(true));
        assert_eq!(run(".memo[\"observe-page\"].step", json!({ "memo": { "observe-page": { "step": 2 } } })), json!(2));
    }

    #[test]
    fn missing_field_is_null_not_an_error() {
        assert_eq!(run(".nope", json!({ "a": 1 })), Value::Null);
        assert_eq!(run(".a.b.c", json!({})), Value::Null);
    }

    #[test]
    fn index_out_of_range_is_null() {
        assert_eq!(run(".[5]", json!([1, 2])), Value::Null);
        assert_eq!(run(".[-1]", json!([1, 2, 3])), json!(3));
    }

    #[test]
    fn map_select_filters() {
        let files = json!([
            { "semanticKind": "image", "name": "a.png" },
            { "semanticKind": "json", "name": "b.json" },
            { "semanticKind": "image", "name": "c.png" }
        ]);
        assert_eq!(
            run(". | map(select(.semanticKind == \"image\")) | length", files.clone()),
            json!(2)
        );
        assert_eq!(
            run("map(select(.semanticKind == \"image\")) | map(.name)", files),
            json!(["a.png", "c.png"])
        );
    }

    #[test]
    fn comparisons_and_booleans() {
        assert_eq!(run(".n >= 3", json!({ "n": 3 })), json!(true));
        assert_eq!(run(".n < 3", json!({ "n": 3 })), json!(false));
        assert_eq!(run(".a and .b", json!({ "a": true, "b": false })), json!(false));
        assert_eq!(run(".a or .b", json!({ "a": false, "b": true })), json!(true));
        assert_eq!(run("not", json!(false)), json!(true));
        assert_eq!(run(".matches == true", json!({ "matches": true })), json!(true));
    }

    #[test]
    fn numbers_compare_by_value_not_by_storage() {
        // A document carries the integer 1; an expression literal parses as
        // the float 1.0. They must still be equal.
        assert_eq!(run(".step == 1", json!({ "step": 1 })), json!(true));
        assert_eq!(run(".n == 3", json!({ "n": 3.0 })), json!(true));
        assert_eq!(run(". == [1, 2]", json!([1.0, 2.0])), json!(true));
        assert_eq!(run(". == { a: 1 }", json!({ "a": 1 })), json!(true));
        assert_eq!(run(".step != 2", json!({ "step": 1 })), json!(true));
    }

    #[test]
    fn absolute_context_references_survive_pipes_and_map() {
        // The case that forced these to exist: filtering a list on a loop
        // variable. Inside `map`, `.` is the element, so the loop variable is
        // only reachable through an absolute reference.
        let ctx = json!({
            "vars": { "step": { "index": 2, "count": 2 } },
            "memo": { "load": { "document": { "answers": [
                { "id": "a", "step": 1 }, { "id": "b", "step": 2 }, { "id": "c", "step": 2 }
            ] } } }
        });
        assert_eq!(
            run("$memo[\"load\"].document.answers | map(select(.step == $vars.step.index)) | length", ctx.clone()),
            json!(2)
        );
        // And after a pipe, where `.` is no longer the context object.
        assert_eq!(run(".memo | $vars.step.index", ctx), json!(2));
    }

    #[test]
    fn only_the_three_context_keys_are_addressable() {
        assert!(parse("$env").is_err());
        assert!(parse("$ENV.PATH").is_err());
        assert!(parse("$").is_err());
        for good in ["$parents", "$vars", "$memo"] {
            assert!(parse(good).is_ok(), "{good} should parse");
        }
    }

    #[test]
    fn constructors() {
        assert_eq!(
            run("{ target: .t, value: .v }", json!({ "t": "x", "v": "y" })),
            json!({ "target": "x", "value": "y" })
        );
        assert_eq!(run("[.a, .b]", json!({ "a": 1, "b": 2 })), json!([1, 2]));
    }

    #[test]
    fn type_length_has() {
        assert_eq!(run("type", json!([])), json!("array"));
        assert_eq!(run("length", json!("abcd")), json!(4));
        assert_eq!(run("has(\"a\")", json!({ "a": null })), json!(true));
        assert_eq!(run("has(\"b\")", json!({ "a": null })), json!(false));
    }

    #[test]
    fn the_worked_guard_from_the_docs() {
        // "non-empty array of images" — the assertion passes the value through
        // rather than replacing it with a boolean.
        let ctx = json!({ "parents": [{ "files": { "items": [
            { "semanticKind": "image" }, { "semanticKind": "audio" }
        ] } }] });
        assert_eq!(
            run(".parents[0].files.items | map(select(.semanticKind == \"image\")) | length > 0", ctx),
            json!(true)
        );
    }

    #[test]
    fn unknown_function_is_rejected_by_name() {
        let e = parse("env").unwrap_err();
        assert!(e.message.contains("is not part of the expression language"), "{}", e.message);
    }

    #[test]
    fn no_escape_hatches_exist() {
        for hostile in ["input", "$ENV", "include \"x\"", "import", "getpath(1)", "`ls`"] {
            assert!(parse(hostile).is_err(), "{hostile} should not parse");
        }
    }

    #[test]
    fn depth_is_bounded() {
        let deep = format!("{}.a{}", "(".repeat(64), ")".repeat(64));
        let e = parse(&deep).unwrap_err();
        assert!(e.message.contains("nests deeper"), "{}", e.message);
    }

    #[test]
    fn trailing_tokens_are_rejected() {
        // Whitespace is not significant, so `.a .b` is legitimately `.a.b`.
        // A genuinely unconsumed token is the error case.
        assert!(parse(".a}").is_err());
        assert!(parse(".a)").is_err());
        assert_eq!(run(".a .b", json!({ "a": { "b": 7 } })), json!(7));
    }

    #[test]
    fn not_negates_its_input() {
        assert_eq!(run(".matches | not", json!({ "matches": false })), json!(true));
        // There is no prefix form. `not .a` lexes as a field access applied to
        // the negation of the input, which is caught at evaluation with a
        // message naming the actual type mismatch.
        let expr = parse("not .a").expect("parses as (not).a");
        let e = eval(&expr, &json!({ "a": 1 })).unwrap_err();
        assert!(e.message.contains("from boolean"), "{}", e.message);
    }

    #[test]
    fn errors_name_the_problem() {
        let expr = parse(".a").unwrap();
        let e = eval(&expr, &json!([1, 2])).unwrap_err();
        assert_eq!(
            e.message,
            "cannot read field \"a\" from array; a field access needs an object"
        );
    }
}
