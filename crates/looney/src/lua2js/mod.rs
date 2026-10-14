//! lua2js lowering: full_moon AST -> LunaIR, plus Lua source utilities
//! (string unescaping, number parsing, name mangling).
//!
//! SCOPE TRACKING: the lowering keeps a stack of local scopes (source names).
//! A reference to a declared local is mangled if the name collides with a JS
//! reserved word or runtime helper; a reference to an *undeclared* name is a
//! runtime global (`print`, `assert`, ...) and passes through unmangled so it
//! resolves against the seeded runtime scope. This is what lets name-based
//! mangling coexist with global references.

pub mod emit;

use crate::error::CompileError;
use crate::ir::{IrBinOp, IrChunk, IrExpr, IrStmt, IrTarget, IrUnOp, Span};
use full_moon::ast::{
    Assignment, BinOp, Call, Expression, FunctionArgs, FunctionCall, LastStmt, LocalAssignment,
    Prefix, Return, Stmt, Suffix, UnOp, Var,
};
use full_moon::node::Node;
use full_moon::tokenizer::{TokenReference, TokenType};
use std::collections::HashSet;

/// Convert a full_moon node range into our frontend-neutral span.
fn span_of(node: &impl Node) -> Span {
    match node.range() {
        Some((start, end)) => Span {
            lo: start.bytes() as u32,
            hi: end.bytes() as u32,
        },
        None => Span::DUMMY,
    }
}

fn err_at(node: &impl Node, message: impl Into<String>) -> CompileError {
    match node.start_position() {
        Some(pos) => CompileError::at(pos.line() as u32, pos.character() as u32 + 1, message),
        None => CompileError::new(message),
    }
}

/// Parse a full_moon number token into f64, restricted to the subset's common
/// grammar (decimal integer/float/exponent and hex integer; no hex floats).
pub fn parse_lua_number(token: &TokenReference) -> Result<f64, CompileError> {
    let text = token.token().to_string();
    let text = text.trim();
    if let Some(hex) = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
    {
        if hex.contains(['.', 'p', 'P']) {
            return Err(err_at(token, "hex float literals are outside the subset"));
        }
        return i64::from_str_radix(hex, 16)
            .map(|v| v as f64)
            .map_err(|_| err_at(token, format!("invalid hex literal `{text}`")));
    }
    text.parse::<f64>()
        .map_err(|_| err_at(token, format!("invalid number literal `{text}`")))
}

/// Unescape a full_moon string literal token (`literal` text, quotes already
/// excluded by the tokenizer). Supports the subset escape set; multi-line
/// `[[...]]` is handled with the leading-newline skip rule. `\z` whitespace
/// skipping is NOT supported.
pub fn unescape_lua_string(token: &TokenReference) -> Result<String, CompileError> {
    let TokenType::StringLiteral {
        literal, quote_type, ..
    } = token.token().token_type()
    else {
        return Err(err_at(token, "expected a string literal"));
    };
    let raw = literal.as_str();
    if *quote_type == full_moon::tokenizer::StringLiteralQuoteType::Brackets {
        // The first character of a multi-line literal's content is skipped if
        // it is a newline.
        let mut chars = raw.chars();
        if matches!(chars.next(), Some('\n')) {
            return Ok(chars.collect());
        }
        return Ok(raw.to_string());
    }
    unescape_short_string(token, raw)
}

fn unescape_short_string(token: &TokenReference, raw: &str) -> Result<String, CompileError> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let esc = chars
            .next()
            .ok_or_else(|| err_at(token, "unterminated escape sequence"))?;
        match esc {
            'a' => out.push('\x07'),
            'b' => out.push('\x08'),
            'f' => out.push('\x0c'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'v' => out.push('\x0b'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            '\n' => out.push('\n'),
            'x' => {
                let hi = chars
                    .next()
                    .and_then(|c| c.to_digit(16))
                    .ok_or_else(|| err_at(token, "invalid \\x escape"))?;
                let lo = chars
                    .next()
                    .and_then(|c| c.to_digit(16))
                    .ok_or_else(|| err_at(token, "invalid \\x escape"))?;
                out.push(char::from_u32(hi * 16 + lo).unwrap_or('\u{fffd}'));
            }
            'u' => {
                if chars.next() != Some('{') {
                    return Err(err_at(token, "invalid \\u escape (expected `{`)"));
                }
                let mut value: u32 = 0;
                loop {
                    let c = chars
                        .next()
                        .ok_or_else(|| err_at(token, "unterminated \\u escape"))?;
                    if c == '}' {
                        break;
                    }
                    let digit = c
                        .to_digit(16)
                        .ok_or_else(|| err_at(token, "invalid \\u escape digit"))?;
                    value = value
                        .checked_mul(16)
                        .and_then(|v| v.checked_add(digit))
                        .ok_or_else(|| err_at(token, "\\u escape out of range"))?;
                }
                out.push(
                    char::from_u32(value).ok_or_else(|| err_at(token, "\\u escape out of range"))?,
                );
            }
            'z' => {
                return Err(err_at(token, "\\z escapes are outside the subset"));
            }
            d if d.is_ascii_digit() => {
                let mut value = d.to_digit(10).unwrap();
                for _ in 0..2 {
                    match chars.peek().copied().and_then(|c| c.to_digit(10)) {
                        Some(digit) => {
                            chars.next();
                            value = value * 10 + digit;
                        }
                        None => break,
                    }
                }
                if value > 255 {
                    return Err(err_at(token, "decimal escape out of range"));
                }
                out.push(char::from_u32(value).unwrap());
            }
            other => {
                return Err(err_at(token, format!("unsupported escape `\\{other}`")));
            }
        }
    }
    Ok(out)
}

/// JS reserved words plus the names the emitted JS references from the `lure`
/// runtime or generates internally. Lua locals with these names are mangled
/// with a `lua$` prefix.
const JS_RESERVED: &[&str] = &[
    "arguments", "await", "break", "case", "catch", "class", "const", "continue", "debugger",
    "default", "delete", "do", "else", "enum", "eval", "export", "extends", "false", "finally",
    "for", "function", "if", "implements", "import", "in", "instanceof", "interface", "let",
    "new", "null", "package", "private", "protected", "public", "return", "static", "super",
    "switch", "this", "throw", "true", "try", "typeof", "undefined", "var", "void", "while",
    "with", "yield",
];

/// Names the emitted JS expects to find in scope (runtime helpers, seeded
/// globals). Any Lua *local* colliding with one is mangled; globals pass
/// through unmangled (a global `print` IS the runtime's print).
const JS_RUNTIME_NAMES: &[&str] = &[
    "assert", "error", "ipairs", "luaAdd", "luaAnd", "luaArith", "luaConcat", "luaEq", "luaGe",
    "luaGt", "luaLe", "luaLen", "luaLt", "luaMod", "luaMul", "luaNe", "luaNeg", "luaNot", "luaOr",
    "luaPow", "luaSub", "luaToNumber", "luaToStr", "luaTruthy", "pairs", "pcall", "print",
    "tonumber", "tostring", "type",
];

fn needs_mangle(name: &str) -> bool {
    JS_RESERVED.contains(&name) || JS_RUNTIME_NAMES.contains(&name)
}

/// The Lua-side lowering state: a stack of local scopes holding *source* names.
#[derive(Default)]
pub struct LuaLower {
    scopes: Vec<HashSet<String>>,
}

impl LuaLower {
    pub fn new() -> Self {
        LuaLower {
            scopes: vec![HashSet::new()],
        }
    }

    fn is_local(&self, source_name: &str) -> bool {
        self.scopes
            .iter()
            .rev()
            .any(|scope| scope.contains(source_name))
    }

    /// Resolve a *reference*: locals are mangled (if reserved), globals pass
    /// through unmangled.
    fn resolve_name(&self, source_name: &str) -> String {
        if self.is_local(source_name) && needs_mangle(source_name) {
            format!("lua${source_name}")
        } else {
            source_name.to_string()
        }
    }

    /// Declare a local in the current scope. The returned name is what the
    /// binding is emitted as; the source name is recorded for later
    /// references. Call AFTER lowering the initializer (Lua scoping: the new
    /// binding is not visible in its own initializer).
    fn declare_local(&mut self, source_name: &str) -> String {
        self.scopes
            .last_mut()
            .expect("scope stack is never empty")
            .insert(source_name.to_string());
        if needs_mangle(source_name) {
            format!("lua${source_name}")
        } else {
            source_name.to_string()
        }
    }

    fn lower_binop(&self, node: &impl Node, op: &BinOp) -> Result<IrBinOp, CompileError> {
        Ok(match op {
            BinOp::Plus(_) => IrBinOp::Add,
            BinOp::Minus(_) => IrBinOp::Sub,
            BinOp::Star(_) => IrBinOp::Mul,
            BinOp::Slash(_) => IrBinOp::Div,
            BinOp::Percent(_) => IrBinOp::Mod,
            BinOp::Caret(_) => IrBinOp::Pow,
            BinOp::TwoDots(_) => IrBinOp::Concat,
            BinOp::TwoEqual(_) => IrBinOp::Eq,
            BinOp::TildeEqual(_) => IrBinOp::Ne,
            BinOp::LessThan(_) => IrBinOp::Lt,
            BinOp::LessThanEqual(_) => IrBinOp::Le,
            BinOp::GreaterThan(_) => IrBinOp::Gt,
            BinOp::GreaterThanEqual(_) => IrBinOp::Ge,
            BinOp::And(_) => IrBinOp::And,
            BinOp::Or(_) => IrBinOp::Or,
            other => {
                return Err(err_at(
                    node,
                    format!(
                        "operator `{other}` is outside the subset (bitwise ops and `//` are Phase 6+)"
                    ),
                ));
            }
        })
    }

    fn lower_unop(&self, node: &impl Node, op: &UnOp) -> Result<IrUnOp, CompileError> {
        Ok(match op {
            UnOp::Minus(_) => IrUnOp::Neg,
            UnOp::Not(_) => IrUnOp::Not,
            UnOp::Hash(_) => IrUnOp::Len,
            other => {
                return Err(err_at(
                    node,
                    format!("unary operator `{other}` is outside the subset"),
                ));
            }
        })
    }

    fn lower_call(&self, call: &FunctionCall) -> Result<IrExpr, CompileError> {
        let span = span_of(call);
        let func = match call.prefix() {
            Prefix::Name(name) => IrExpr::Var(self.resolve_name(name.token().to_string().trim())),
            Prefix::Expression(expr) => self.lower_expr(expr)?,
            other => return Err(err_at(call, format!("unsupported call prefix `{other}`"))),
        };
        let mut call_args: Option<Vec<IrExpr>> = None;
        for suffix in call.suffixes() {
            match suffix {
                Suffix::Call(Call::AnonymousCall(FunctionArgs::Parentheses { arguments, .. })) => {
                    if call_args.is_some() {
                        return Err(err_at(
                            suffix,
                            "chained calls are outside the subset for now",
                        ));
                    }
                    call_args = Some(
                        arguments
                            .iter()
                            .map(|arg| self.lower_expr(arg))
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                }
                Suffix::Call(Call::AnonymousCall(FunctionArgs::String(_))) => {
                    return Err(err_at(
                        suffix,
                        "string call sugar `f\"...\"` is outside the subset",
                    ));
                }
                Suffix::Call(Call::AnonymousCall(FunctionArgs::TableConstructor(_))) => {
                    return Err(err_at(
                        suffix,
                        "table call sugar `f{...}` is outside the subset",
                    ));
                }
                Suffix::Call(Call::MethodCall(_)) => {
                    return Err(err_at(
                        suffix,
                        "method calls (`a:m(...)`) arrive in Phase 3",
                    ));
                }
                Suffix::Index(_) => {
                    return Err(err_at(suffix, "indexing arrives in Phase 3"));
                }
                other => {
                    return Err(err_at(suffix, format!("unsupported call suffix `{other}`")));
                }
            }
        }
        let args = call_args.ok_or_else(|| err_at(call, "expected a call"))?;
        Ok(IrExpr::Call {
            func: Box::new(func),
            args,
            span,
        })
    }

    fn lower_expr(&self, expr: &Expression) -> Result<IrExpr, CompileError> {
        match expr {
            Expression::Number(token) => Ok(IrExpr::Num(parse_lua_number(token)?)),
            Expression::String(token) => Ok(IrExpr::Str(unescape_lua_string(token)?)),
            Expression::Symbol(token) => match token.token().to_string().as_str() {
                "nil" => Ok(IrExpr::Nil),
                "true" => Ok(IrExpr::Bool(true)),
                "false" => Ok(IrExpr::Bool(false)),
                "..." => Err(err_at(token, "varargs arrive in Phase 4")),
                other => Err(err_at(token, format!("unsupported symbol `{other}`"))),
            },
            Expression::Parentheses { expression, .. } => self.lower_expr(expression),
            Expression::BinaryOperator { lhs, binop, rhs } => {
                let span = span_of(expr);
                Ok(IrExpr::Bin {
                    op: self.lower_binop(expr, binop)?,
                    lhs: Box::new(self.lower_expr(lhs)?),
                    rhs: Box::new(self.lower_expr(rhs)?),
                    span,
                })
            }
            Expression::UnaryOperator { unop, expression } => {
                let span = span_of(expr);
                Ok(IrExpr::Un {
                    op: self.lower_unop(expr, unop)?,
                    expr: Box::new(self.lower_expr(expression)?),
                    span,
                })
            }
            Expression::FunctionCall(call) => self.lower_call(call),
            Expression::Function(_) => Err(err_at(expr, "functions arrive in Phase 2")),
            Expression::TableConstructor(_) => {
                Err(err_at(expr, "table constructors arrive in Phase 3"))
            }
            Expression::Var(var) => match var {
                Var::Name(token) => {
                    Ok(IrExpr::Var(self.resolve_name(token.token().to_string().trim())))
                }
                Var::Expression(_) => Err(err_at(expr, "indexing arrives in Phase 3")),
                other => Err(err_at(expr, format!("unsupported variable form `{other}`"))),
            },
            other => Err(err_at(
                expr,
                format!("expression `{other}` is outside the subset"),
            )),
        }
    }

    fn lower_local_assignment(&mut self, stmt: &LocalAssignment) -> Result<IrStmt, CompileError> {
        let span = span_of(stmt);
        let names: Vec<_> = stmt.names().iter().collect();
        if names.len() != 1 {
            return Err(err_at(
                stmt,
                "multiple assignment arrives in Phase 4 (`local a, b = ...`)",
            ));
        }
        let exprs: Vec<_> = stmt.expressions().iter().collect();
        if exprs.len() > 1 {
            return Err(err_at(
                stmt,
                "multiple assignment arrives in Phase 4 (`local a = x, y`)",
            ));
        }
        let source_name = names[0].token().to_string();
        let value = exprs
            .first()
            .map(|e| self.lower_expr(e))
            .transpose()?;
        // Lua scoping: the binding becomes visible only after its initializer.
        let name = self.declare_local(source_name.trim());
        Ok(IrStmt::Local { name, value, span })
    }

    fn lower_assignment(&self, stmt: &Assignment) -> Result<IrStmt, CompileError> {
        let span = span_of(stmt);
        let vars: Vec<_> = stmt.variables().iter().collect();
        if vars.len() != 1 {
            return Err(err_at(stmt, "multiple assignment arrives in Phase 4"));
        }
        let exprs: Vec<_> = stmt.expressions().iter().collect();
        if exprs.len() != 1 {
            return Err(err_at(stmt, "multiple assignment arrives in Phase 4"));
        }
        let target = match &vars[0] {
            Var::Name(token) => IrTarget::Var(self.resolve_name(token.token().to_string().trim())),
            Var::Expression(_) => return Err(err_at(stmt, "index assignment arrives in Phase 3")),
            other => return Err(err_at(stmt, format!("unsupported assignment target `{other}`"))),
        };
        Ok(IrStmt::Assign {
            target,
            value: self.lower_expr(exprs[0])?,
            span,
        })
    }

    fn lower_return(&self, ret: &Return) -> Result<IrStmt, CompileError> {
        let span = span_of(ret);
        let mut values = Vec::new();
        for expr in ret.returns() {
            values.push(self.lower_expr(expr)?);
        }
        Ok(IrStmt::Return { values, span })
    }

    fn lower_stmt(&mut self, stmt: &Stmt, out: &mut Vec<IrStmt>) -> Result<(), CompileError> {
        match stmt {
            Stmt::LocalAssignment(local) => {
                out.push(self.lower_local_assignment(local)?);
                Ok(())
            }
            Stmt::Assignment(assign) => {
                out.push(self.lower_assignment(assign)?);
                Ok(())
            }
            Stmt::FunctionCall(call) => {
                out.push(IrStmt::Expr {
                    expr: self.lower_call(call)?,
                    span: span_of(call),
                });
                Ok(())
            }
            Stmt::Do(do_stmt) => {
                out.push(IrStmt::Do {
                    body: self.lower_block(do_stmt.block())?,
                    span: span_of(stmt),
                });
                Ok(())
            }
            Stmt::If(_) => Err(err_at(stmt, "`if` arrives in Phase 2")),
            Stmt::While(_) => Err(err_at(stmt, "`while` arrives in Phase 2")),
            Stmt::NumericFor(_) => Err(err_at(stmt, "numeric `for` arrives in Phase 2")),
            Stmt::FunctionDeclaration(_) | Stmt::LocalFunction(_) => {
                Err(err_at(stmt, "functions arrive in Phase 2"))
            }
            Stmt::GenericFor(_) => Err(err_at(stmt, "generic `for` arrives in Phase 5")),
            Stmt::Repeat(_) => Err(err_at(stmt, "`repeat` is outside the subset (v0)")),
            other => Err(err_at(
                stmt,
                format!("statement `{other}` is outside the subset"),
            )),
        }
    }

    fn lower_block(&mut self, block: &full_moon::ast::Block) -> Result<Vec<IrStmt>, CompileError> {
        self.scopes.push(HashSet::new());
        let result = (|| {
            let mut out = Vec::new();
            for stmt in block.stmts() {
                self.lower_stmt(stmt, &mut out)?;
            }
            if let Some(last) = block.last_stmt() {
                match last {
                    LastStmt::Return(ret) => out.push(self.lower_return(ret)?),
                    LastStmt::Break(token) => {
                        return Err(err_at(token, "`break` arrives in Phase 2 (with loops)"));
                    }
                    other => {
                        return Err(err_at(last, format!("`{other}` is outside the subset")));
                    }
                }
            }
            Ok(out)
        })();
        self.scopes.pop();
        result
    }
}

/// Parse Lua source and lower it to LunaIR (the shared subset).
pub fn lower_lua(source: &str) -> Result<IrChunk, CompileError> {
    let ast = full_moon::parse_fallible(source, Default::default())
        .into_result()
        .map_err(|errs| {
            let msg = errs
                .iter()
                .map(|e| e.error_message().into_owned())
                .collect::<Vec<_>>()
                .join("; ");
            let pos = errs.first().map(|e| e.range().0);
            match pos {
                Some(pos) => CompileError::at(
                    pos.line() as u32,
                    pos.character() as u32 + 1,
                    format!("lua parse error: {msg}"),
                ),
                None => CompileError::new(format!("lua parse error: {msg}")),
            }
        })?;
    let mut lower = LuaLower::new();
    let body = lower.lower_block(ast.nodes())?;
    Ok(IrChunk { body })
}
