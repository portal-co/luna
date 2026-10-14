//! LunaIR -> Lua emission (hand-rolled; full_moon is a parser, not a codegen).
//!
//! Direction-paired with the JS frontend: assumes JS-origin IR and lowers all
//! JS semantics into js.lua runtime helper calls (`jsAdd`, `jsTruthy`, ...).

use crate::error::CompileError;
use crate::ir::{IrBinOp, IrChunk, IrExpr, IrStmt, IrTarget, IrUnOp};

fn bin_helper(op: IrBinOp) -> &'static str {
    match op {
        IrBinOp::Add => "jsAdd",
        IrBinOp::Sub => "jsSub",
        IrBinOp::Mul => "jsMul",
        IrBinOp::Div => "jsDiv",
        // JS `%` is *truncated* remainder, not Lua's floored `%`: never emit raw.
        IrBinOp::Mod => "jsRem",
        IrBinOp::Pow => "jsPow",
        IrBinOp::Concat => "jsConcat",
        IrBinOp::Eq => "jsStrictEq",
        IrBinOp::Lt => "jsLt",
        IrBinOp::Le => "jsLe",
        IrBinOp::Gt => "jsGt",
        IrBinOp::Ge => "jsGe",
        IrBinOp::Ne | IrBinOp::And | IrBinOp::Or => {
            unreachable!("handled specially by the emitter")
        }
    }
}

/// Format an f64 as a Lua expression literal with shortest round-trip
/// semantics. NaN/Infinity cannot be written as Lua literals.
fn fmt_num(value: f64) -> String {
    if value.is_nan() {
        return "(0/0)".into();
    }
    if value == f64::INFINITY {
        return "math.huge".into();
    }
    if value == f64::NEG_INFINITY {
        return "(-math.huge)".into();
    }
    // Rust's `{}` for f64 is shortest-round-trip decimal and never emits an
    // exponent for the ranges we care about; `-0` prints as `-0` which Lua
    // parses fine.
    format!("{value}")
}

fn quote_lua_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                out.push_str(&format!("\\{}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn emit_expr(expr: &IrExpr, out: &mut String) {
    match expr {
        IrExpr::Nil => out.push_str("nil"),
        IrExpr::Bool(true) => out.push_str("true"),
        IrExpr::Bool(false) => out.push_str("false"),
        IrExpr::Num(value) => out.push_str(&fmt_num(*value)),
        IrExpr::Str(value) => out.push_str(&quote_lua_string(value)),
        IrExpr::Var(name) => out.push_str(name),
        IrExpr::Call { func, args, .. } => {
            match &**func {
                IrExpr::Var(_) | IrExpr::Call { .. } => emit_expr(func, out),
                // A non-atom callee (e.g. a helper-call result of a binop) is
                // still callable; parenthesize to keep Lua's grammar happy.
                other => {
                    out.push('(');
                    emit_expr(other, out);
                    out.push(')');
                }
            }
            out.push('(');
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                emit_expr(arg, out);
            }
            out.push(')');
        }
        IrExpr::Un { op, expr, .. } => {
            let arg = |out: &mut String| match &**expr {
                IrExpr::Var(_) | IrExpr::Num(_) | IrExpr::Str(_) | IrExpr::Call { .. } => {
                    emit_expr(expr, out)
                }
                _ => {
                    out.push('(');
                    emit_expr(expr, out);
                    out.push(')');
                }
            };
            match op {
                // Raw unary minus: JS coerces via jsNeg only if needed; raw is
                // exact for numbers, and the subset's operands are numbers.
                IrUnOp::Neg => {
                    out.push('-');
                    arg(out);
                }
                IrUnOp::Not => {
                    out.push_str("jsNot(");
                    emit_expr(expr, out);
                    out.push(')');
                }
                IrUnOp::Len => {
                    out.push_str("jsLen(");
                    emit_expr(expr, out);
                    out.push(')');
                }
            }
        }
        IrExpr::Bin { op, lhs, rhs, .. } => match op {
            IrBinOp::And | IrBinOp::Or => {
                out.push_str(match op {
                    IrBinOp::And => "jsAnd(",
                    IrBinOp::Or => "jsOr(",
                    _ => unreachable!(),
                });
                out.push_str("function() return ");
                emit_expr(lhs, out);
                out.push_str(" end, function() return ");
                emit_expr(rhs, out);
                out.push_str(" end)");
            }
            IrBinOp::Ne => {
                out.push_str("not jsStrictEq(");
                emit_expr(lhs, out);
                out.push_str(", ");
                emit_expr(rhs, out);
                out.push(')');
            }
            other => {
                out.push_str(bin_helper(*other));
                out.push('(');
                emit_expr(lhs, out);
                out.push_str(", ");
                emit_expr(rhs, out);
                out.push(')');
            }
        },
    }
}

fn emit_stmt(stmt: &IrStmt, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match stmt {
        IrStmt::Local { name, value, .. } => {
            out.push_str(&pad);
            out.push_str("local ");
            out.push_str(name);
            if let Some(value) = value {
                out.push_str(" = ");
                emit_expr(value, out);
            }
            out.push('\n');
        }
        IrStmt::Assign { target, value, .. } => {
            out.push_str(&pad);
            match target {
                IrTarget::Var(name) => out.push_str(name),
            }
            out.push_str(" = ");
            emit_expr(value, out);
            out.push('\n');
        }
        IrStmt::Expr { expr, .. } => {
            out.push_str(&pad);
            emit_expr(expr, out);
            out.push('\n');
        }
        IrStmt::Do { body, .. } => {
            out.push_str(&pad);
            out.push_str("do\n");
            for inner in body {
                emit_stmt(inner, indent + 1, out);
            }
            out.push_str(&pad);
            out.push_str("end\n");
        }
        IrStmt::Return { values, .. } => {
            out.push_str(&pad);
            out.push_str("return");
            if let Some(value) = values.first() {
                out.push(' ');
                emit_expr(value, out);
            }
            out.push('\n');
        }
    }
}

/// Emit a complete Lua chunk for a validated chunk. The output expects the
/// js.lua runtime (`looney::JS_LUA_RUNTIME`) to be prepended or preloaded.
pub fn emit_lua(chunk: &IrChunk) -> Result<String, CompileError> {
    let mut out = String::new();
    for stmt in &chunk.body {
        emit_stmt(stmt, 0, &mut out);
    }
    Ok(out)
}
