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

/// Atomic for the purposes of a call/function position: no parens needed.
fn is_atom(expr: &IrExpr) -> bool {
    matches!(
        expr,
        IrExpr::Nil
            | IrExpr::Bool(_)
            | IrExpr::Num(_)
            | IrExpr::Str(_)
            | IrExpr::Var(_)
            | IrExpr::Call { .. }
            | IrExpr::Function { .. } // multi-line, self-contained
    )
}

/// `at_line_start` must be true when emitting at the beginning of a logical
/// line (after indentation): a parenthesized expression there would be parsed
/// as a continuation of the previous line's call, so we use `do ... end`
/// expression-wrapping instead (only reachable for multi-line function exprs).
fn emit_expr_ctx(expr: &IrExpr, indent: usize, at_line_start: bool, out: &mut String) {
    match expr {
        IrExpr::Nil => out.push_str("nil"),
        IrExpr::Bool(true) => out.push_str("true"),
        IrExpr::Bool(false) => out.push_str("false"),
        IrExpr::Num(value) => out.push_str(&fmt_num(*value)),
        IrExpr::Str(value) => out.push_str(&quote_lua_string(value)),
        IrExpr::Var(name) => out.push_str(name),
        IrExpr::Call { func, args, .. } => {
            match &**func {
                IrExpr::Function { .. } if at_line_start => {
                    // `(function ... end)(...)` at line start would chain off
                    // the previous statement — wrap with a do-block instead.
                    out.push_str("(function()\n");
                    out.push_str(&"  ".repeat(indent + 1));
                    out.push_str("return ");
                    emit_expr_ctx(func, indent + 1, false, out);
                    out.push('\n');
                    out.push_str(&"  ".repeat(indent));
                    out.push_str("end)()");
                }
                f if is_atom(f) => emit_expr_ctx(f, indent, false, out),
                other => {
                    out.push('(');
                    emit_expr_ctx(other, indent, false, out);
                    out.push(')');
                }
            }
            if !matches!(&**func, IrExpr::Function { .. }) || !at_line_start {
                out.push('(');
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    emit_expr_ctx(arg, indent, false, out);
                }
                out.push(')');
            }
        }
        IrExpr::Un { op, expr, .. } => {
            match op {
                // Raw unary minus: JS coerces via jsNeg only if needed; raw is
                // exact for numbers, and the subset's operands are numbers.
                IrUnOp::Neg => {
                    out.push('-');
                    match &**expr {
                        e if is_atom(e) => emit_expr_ctx(e, indent, false, out),
                        _ => {
                            out.push('(');
                            emit_expr_ctx(expr, indent, false, out);
                            out.push(')');
                        }
                    }
                }
                IrUnOp::Not => {
                    out.push_str("jsNot(");
                    emit_expr_ctx(expr, indent, false, out);
                    out.push(')');
                }
                IrUnOp::Len => {
                    out.push_str("jsLen(");
                    emit_expr_ctx(expr, indent, false, out);
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
                emit_expr_ctx(lhs, indent, false, out);
                out.push_str(" end, function() return ");
                emit_expr_ctx(rhs, indent, false, out);
                out.push_str(" end)");
            }
            IrBinOp::Ne => {
                out.push_str("not jsStrictEq(");
                emit_expr_ctx(lhs, indent, false, out);
                out.push_str(", ");
                emit_expr_ctx(rhs, indent, false, out);
                out.push(')');
            }
            other => {
                out.push_str(bin_helper(*other));
                out.push('(');
                emit_expr_ctx(lhs, indent, false, out);
                out.push_str(", ");
                emit_expr_ctx(rhs, indent, false, out);
                out.push(')');
            }
        },
        IrExpr::Cond { cond, then, else_, .. } => {
            out.push_str("jsCond(");
            emit_expr_ctx(cond, indent, false, out);
            out.push_str(", function() return ");
            emit_expr_ctx(then, indent, false, out);
            out.push_str(" end, function() return ");
            emit_expr_ctx(else_, indent, false, out);
            out.push_str(" end)");
        }
        IrExpr::Function { params, body, .. } => {
            emit_function(params, body, indent, out);
        }
    }
}

/// Emit an anonymous Lua function value. Params normalize `nil` to
/// `jsUndefined` (JS pads missing args with undefined, never nil). The
/// fallthrough return is `jsUndefined` (JS functions return undefined when
/// they fall off the end); an explicit `return;` also lowers to that.
fn emit_function(params: &[String], body: &[IrStmt], indent: usize, out: &mut String) {
    out.push_str("function(");
    out.push_str(&params.join(", "));
    out.push_str(")\n");
    let pad = "  ".repeat(indent + 1);
    for param in params {
        out.push_str(&pad);
        out.push_str("if ");
        out.push_str(param);
        out.push_str(" == nil then ");
        out.push_str(param);
        out.push_str(" = jsUndefined end\n");
    }
    for stmt in body {
        emit_stmt(stmt, indent + 1, out);
    }
    // JS fallthrough returns `undefined`. Lua forbids statements after
    // `return`, so only emit the fallthrough if the body doesn't end in one.
    if !matches!(body.last(), Some(IrStmt::Return { .. })) {
        out.push_str(&pad);
        out.push_str("return jsUndefined\n");
    }
    out.push_str(&"  ".repeat(indent));
    out.push_str("end");
}

fn emit_stmt(stmt: &IrStmt, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match stmt {
        IrStmt::Local { name, value, .. } => {
            out.push_str(&pad);
            // Local function values lower to `local name` + `name =
            // function(...)`: recursion works via the upvalue (Lua has no
            // named function expressions like JS does).
            match value {
                Some(IrExpr::Function { params, body, .. }) => {
                    out.push_str("local ");
                    out.push_str(name);
                    out.push('\n');
                    out.push_str(&pad);
                    out.push_str(name);
                    out.push_str(" = ");
                    emit_function(params, body, indent, out);
                    out.push('\n');
                }
                Some(value) => {
                    out.push_str("local ");
                    out.push_str(name);
                    out.push_str(" = ");
                    emit_expr_ctx(value, indent, false, out);
                    out.push('\n');
                }
                None => {
                    out.push_str("local ");
                    out.push_str(name);
                    out.push('\n');
                }
            }
        }
        IrStmt::Assign { target, value, .. } => {
            out.push_str(&pad);
            match target {
                IrTarget::Var(name) => out.push_str(name),
            }
            out.push_str(" = ");
            emit_expr_ctx(value, indent, false, out);
            out.push('\n');
        }
        IrStmt::Expr { expr, .. } => {
            out.push_str(&pad);
            emit_expr_ctx(expr, indent, true, out);
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
            match values.first() {
                Some(value) => {
                    out.push(' ');
                    emit_expr_ctx(value, indent, false, out);
                }
                // JS `return;` returns `undefined`; a bare `return` would
                // return zero values instead.
                None => out.push_str(" jsUndefined"),
            }
            out.push('\n');
        }
        IrStmt::If { arms, else_, .. } => {
            for (i, (cond, body)) in arms.iter().enumerate() {
                if i == 0 {
                    out.push_str(&pad);
                    out.push_str("if ");
                } else {
                    out.push_str(&pad);
                    out.push_str("elseif ");
                }
                out.push_str("jsTruthy(");
                emit_expr_ctx(cond, indent, false, out);
                out.push_str(") then\n");
                for inner in body {
                    emit_stmt(inner, indent + 1, out);
                }
            }
            if let Some(else_) = else_ {
                out.push_str(&pad);
                out.push_str("else\n");
                for inner in else_ {
                    emit_stmt(inner, indent + 1, out);
                }
            }
            out.push_str(&pad);
            out.push_str("end\n");
        }
        IrStmt::While { cond, body, .. } => {
            out.push_str(&pad);
            out.push_str("while jsTruthy(");
            emit_expr_ctx(cond, indent, false, out);
            out.push_str(") do\n");
            for inner in body {
                emit_stmt(inner, indent + 1, out);
            }
            out.push_str(&pad);
            out.push_str("end\n");
        }
        IrStmt::NumericFor {
            var,
            start,
            stop,
            step,
            body,
            ..
        } => {
            // Reached from js2lua only for JS-origin IR built directly (tests)
            // — JS classic-for semantics differ; use raw Lua numeric for here.
            out.push_str(&pad);
            out.push_str("for ");
            out.push_str(var);
            out.push_str(" = ");
            emit_expr_ctx(start, indent, false, out);
            out.push_str(", ");
            emit_expr_ctx(stop, indent, false, out);
            if let Some(step) = step {
                out.push_str(", ");
                emit_expr_ctx(step, indent, false, out);
            }
            out.push_str(" do\n");
            for inner in body {
                emit_stmt(inner, indent + 1, out);
            }
            out.push_str(&pad);
            out.push_str("end\n");
        }
        IrStmt::Break { .. } => {
            out.push_str(&pad);
            out.push_str("break\n");
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
