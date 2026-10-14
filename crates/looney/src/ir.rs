//! LunaIR: the shared subset AST. This module *is* the executable definition of
//! the Luna Subset (see docs/SUBSET.md): both frontends lower into these nodes
//! and both backends emit from them.
//!
//! IMPORTANT (direction-pairing): IR ops are abstract subset operations. The JS
//! emitter (`lua2js::emit`) assumes Lua-origin IR and emits `lua*` runtime
//! helpers; the Lua emitter (`js2lua::emit`) assumes JS-origin IR and emits
//! `js*` runtime helpers. We never emit JS from JS-origin IR or vice versa.
//!
//! Style rule for this file and both emitters: NO wildcard match arms over IR
//! enums. Adding a variant must break the build until validator and both
//! emitters handle it (see tests/ir_shape.rs).

use crate::error::CompileError;

/// Byte offsets into the source text (half-open, frontend-normalized).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub const DUMMY: Span = Span { lo: 0, hi: 0 };
}

/// Binary operators in the subset. Comparison/logical ops have
/// source-language semantics; emitters map them to direction-specific
/// runtime helpers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrBinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl IrBinOp {
    /// Lua-ish surface spelling, for diagnostics only.
    pub fn name(self) -> &'static str {
        match self {
            IrBinOp::Add => "+",
            IrBinOp::Sub => "-",
            IrBinOp::Mul => "*",
            IrBinOp::Div => "/",
            IrBinOp::Mod => "%",
            IrBinOp::Pow => "^",
            IrBinOp::Concat => "..",
            IrBinOp::Eq => "==",
            IrBinOp::Ne => "~=",
            IrBinOp::Lt => "<",
            IrBinOp::Le => "<=",
            IrBinOp::Gt => ">",
            IrBinOp::Ge => ">=",
            IrBinOp::And => "and",
            IrBinOp::Or => "or",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrUnOp {
    Neg,
    Not,
    Len,
}

impl IrUnOp {
    pub fn name(self) -> &'static str {
        match self {
            IrUnOp::Neg => "-",
            IrUnOp::Not => "not",
            IrUnOp::Len => "#",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum IrExpr {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    /// A post-mangling identifier. Frontends own mangling; emitters trust it.
    Var(String),
    Call {
        func: Box<IrExpr>,
        args: Vec<IrExpr>,
        span: Span,
    },
    Un {
        op: IrUnOp,
        expr: Box<IrExpr>,
        span: Span,
    },
    Bin {
        op: IrBinOp,
        lhs: Box<IrExpr>,
        rhs: Box<IrExpr>,
        span: Span,
    },
    /// Conditional expression: Lua `c and a or b` idiom / JS ternary.
    /// Conditions are truthiness-checked by the direction-paired runtime.
    Cond {
        cond: Box<IrExpr>,
        then: Box<IrExpr>,
        else_: Box<IrExpr>,
        span: Span,
    },
    /// Function value: `function(...)` / `function f(...) {}` / arrow.
    /// Varargs/rest params are Phase 4.
    Function {
        params: Vec<String>,
        body: Vec<IrStmt>,
        span: Span,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum IrTarget {
    /// Assign to a (local or global) variable.
    Var(String),
    // Phase 3: Index { obj: IrExpr, key: IrExpr }
}

#[derive(Clone, Debug, PartialEq)]
pub enum IrStmt {
    Local {
        name: String,
        value: Option<IrExpr>,
        span: Span,
    },
    Assign {
        target: IrTarget,
        value: IrExpr,
        span: Span,
    },
    /// Expression statement. Validator restricts this to calls.
    Expr {
        expr: IrExpr,
        span: Span,
    },
    Do {
        body: Vec<IrStmt>,
        span: Span,
    },
    Return {
        values: Vec<IrExpr>,
        span: Span,
    },
    /// Chain of condition/block arms; `else_` is the final fallthrough.
    If {
        arms: Vec<(IrExpr, Vec<IrStmt>)>,
        else_: Option<Vec<IrStmt>>,
        span: Span,
    },
    While {
        cond: IrExpr,
        body: Vec<IrStmt>,
        span: Span,
    },
    /// Lua numeric `for i = start, stop[, step]` — the loop variable is
    /// immutable inside the body and fresh per iteration (JS: `let` in a block
    /// per iteration, so closures capture per-iteration values, matching Lua).
    NumericFor {
        var: String,
        start: IrExpr,
        stop: IrExpr,
        step: Option<IrExpr>,
        body: Vec<IrStmt>,
        span: Span,
    },
    Break {
        span: Span,
    },
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct IrChunk {
    pub body: Vec<IrStmt>,
}

/// True when `name` is a legal identifier in IR. The portable *source* grammar
/// is ASCII `[A-Za-z_][A-Za-z0-9_]*` (enforced at lowering); IR additionally
/// allows `$` because the lua2js name mangler produces `lua$`-prefixed names.
/// (The js2lua lowering rejects `$` in source identifiers, so `$` never flows
/// toward the Lua emitter in practice.)
pub fn valid_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn check_ident(name: &str) -> Result<(), CompileError> {
    if valid_ident(name) {
        Ok(())
    } else {
        Err(CompileError::new(format!(
            "identifier `{name}` is outside the portable subset grammar"
        )))
    }
}

fn validate_expr(expr: &IrExpr) -> Result<(), CompileError> {
    match expr {
        IrExpr::Nil | IrExpr::Bool(_) | IrExpr::Num(_) | IrExpr::Str(_) => Ok(()),
        IrExpr::Var(name) => check_ident(name),
        IrExpr::Call { func, args, .. } => {
            validate_expr(func)?;
            for arg in args {
                validate_expr(arg)?;
            }
            Ok(())
        }
        IrExpr::Un { expr, .. } => validate_expr(expr),
        IrExpr::Bin { lhs, rhs, .. } => {
            validate_expr(lhs)?;
            validate_expr(rhs)
        }
        IrExpr::Cond { cond, then, else_, .. } => {
            validate_expr(cond)?;
            validate_expr(then)?;
            validate_expr(else_)
        }
        IrExpr::Function { params, body, .. } => {
            for param in params {
                check_ident(param)?;
            }
            validate_stmts(body)
        }
    }
}

fn validate_stmt(stmt: &IrStmt) -> Result<(), CompileError> {
    match stmt {
        IrStmt::Local { name, value, .. } => {
            check_ident(name)?;
            if let Some(value) = value {
                validate_expr(value)?;
            }
            Ok(())
        }
        IrStmt::Assign { target, value, .. } => {
            match target {
                IrTarget::Var(name) => check_ident(name)?,
            }
            validate_expr(value)
        }
        IrStmt::Expr { expr, .. } => {
            if !matches!(expr, IrExpr::Call { .. }) {
                return Err(CompileError::new(
                    "expression statements must be calls in the subset",
                ));
            }
            validate_expr(expr)
        }
        IrStmt::Do { body, .. } => validate_stmts(body),
        IrStmt::Return { values, .. } => {
            if values.len() > 1 {
                return Err(CompileError::new(
                    "multiple return values arrive in Phase 4",
                ));
            }
            for value in values {
                validate_expr(value)?;
            }
            Ok(())
        }
        IrStmt::If { arms, else_, .. } => {
            for (cond, body) in arms {
                validate_expr(cond)?;
                validate_stmts(body)?;
            }
            if let Some(else_) = else_ {
                validate_stmts(else_)?;
            }
            Ok(())
        }
        IrStmt::While { cond, body, .. } => {
            validate_expr(cond)?;
            validate_stmts(body)
        }
        IrStmt::NumericFor {
            var,
            start,
            stop,
            step,
            body,
            ..
        } => {
            check_ident(var)?;
            validate_expr(start)?;
            validate_expr(stop)?;
            if let Some(step) = step {
                validate_expr(step)?;
            }
            validate_stmts(body)
        }
        IrStmt::Break { .. } => Ok(()),
    }
}

fn validate_stmts(stmts: &[IrStmt]) -> Result<(), CompileError> {
    for stmt in stmts {
        validate_stmt(stmt)?;
    }
    Ok(())
}

/// Validate a lowered chunk. This is what makes "same subset" checkable:
/// anything a backend can't emit (or the subset forbids) is rejected here or
/// at lowering time.
pub fn validate(chunk: &IrChunk) -> Result<(), CompileError> {
    validate_stmts(&chunk.body)
}
