//! LunaIR -> JavaScript emission via the SWC AST + codegen.
//!
//! Direction-paired with the Lua frontend: assumes Lua-origin IR and lowers
//! all Lua semantics into `lure` runtime helper calls (`luaAdd`, `luaTruthy`,
//! ...). The emitted script is a classic (non-module) script so that a
//! top-level `return` stays legal, matching Lua chunk semantics.

use crate::error::CompileError;
use crate::ir::{IrBinOp, IrChunk, IrExpr, IrStmt, IrTarget, IrUnOp};
use swc_atoms::Atom;
use swc_common::{FileName, SourceMap, SyntaxContext, DUMMY_SP, GLOBALS};
use swc_common::sync::Lrc;
use swc_ecma_ast::*;
use swc_ecma_codegen::{text_writer::JsWriter, Emitter};

fn ident(name: &str) -> Ident {
    Ident::new_no_ctxt(Atom::from(name), DUMMY_SP)
}

fn binding(name: &str) -> BindingIdent {
    BindingIdent {
        id: ident(name),
        type_ann: None,
    }
}

fn null_lit() -> Expr {
    Expr::Lit(Lit::Null(Null { span: DUMMY_SP }))
}

/// `name(arg0, arg1, ...)` with a bare-identifier callee.
fn helper_call(name: &str, args: Vec<Expr>) -> Expr {
    Expr::Call(CallExpr {
        span: DUMMY_SP,
        ctxt: SyntaxContext::empty(),
        callee: Callee::Expr(Box::new(Expr::Ident(ident(name)))),
        args: args
            .into_iter()
            .map(|expr| ExprOrSpread {
                spread: None,
                expr: Box::new(expr),
            })
            .collect(),
        type_args: None,
    })
}

/// `() => body` — used to preserve short-circuit semantics for and/or.
fn thunk(body: Expr) -> Expr {
    Expr::Arrow(ArrowExpr {
        span: DUMMY_SP,
        ctxt: SyntaxContext::empty(),
        params: vec![],
        body: Box::new(BlockStmtOrExpr::Expr(Box::new(body))),
        is_async: false,
        is_generator: false,
        type_params: None,
        return_type: None,
    })
}

fn bin_helper(op: IrBinOp) -> &'static str {
    match op {
        IrBinOp::Add => "luaAdd",
        IrBinOp::Sub => "luaSub",
        IrBinOp::Mul => "luaMul",
        IrBinOp::Div => "luaDiv",
        IrBinOp::Mod => "luaMod",
        IrBinOp::Pow => "luaPow",
        IrBinOp::Concat => "luaConcat",
        IrBinOp::Eq => "luaEq",
        IrBinOp::Ne => "luaNe",
        IrBinOp::Lt => "luaLt",
        IrBinOp::Le => "luaLe",
        IrBinOp::Gt => "luaGt",
        IrBinOp::Ge => "luaGe",
        IrBinOp::And | IrBinOp::Or => unreachable!("logical ops are thunked"),
    }
}

pub fn emit_expr(expr: &IrExpr) -> Expr {
    match expr {
        IrExpr::Nil => null_lit(),
        IrExpr::Bool(value) => Expr::Lit(Lit::Bool(Bool {
            span: DUMMY_SP,
            value: *value,
        })),
        IrExpr::Num(value) => Expr::Lit(Lit::Num(Number {
            span: DUMMY_SP,
            value: *value,
            raw: None,
        })),
        IrExpr::Str(value) => Expr::Lit(Lit::Str(Str {
            span: DUMMY_SP,
            value: Atom::from(value.as_str()).into(),
            raw: None,
        })),
        IrExpr::Var(name) => Expr::Ident(ident(name)),
        IrExpr::Call { func, args, .. } => Expr::Call(CallExpr {
            span: DUMMY_SP,
            ctxt: SyntaxContext::empty(),
            callee: Callee::Expr(Box::new(emit_expr(func))),
            args: args
                .iter()
                .map(|arg| ExprOrSpread {
                    spread: None,
                    expr: Box::new(emit_expr(arg)),
                })
                .collect(),
            type_args: None,
        }),
        IrExpr::Un { op, expr, .. } => {
            let arg = emit_expr(expr);
            match op {
                // Raw unary minus: correct for numbers; Lua error semantics for
                // non-numbers arrive with the error-handling phase (Phase 5).
                IrUnOp::Neg => Expr::Unary(UnaryExpr {
                    span: DUMMY_SP,
                    op: UnaryOp::Minus,
                    arg: Box::new(arg),
                }),
                IrUnOp::Not => helper_call("luaNot", vec![arg]),
                IrUnOp::Len => helper_call("luaLen", vec![arg]),
            }
        }
        IrExpr::Bin { op, lhs, rhs, .. } => {
            let l = emit_expr(lhs);
            let r = emit_expr(rhs);
            match op {
                IrBinOp::And => helper_call("luaAnd", vec![thunk(l), thunk(r)]),
                IrBinOp::Or => helper_call("luaOr", vec![thunk(l), thunk(r)]),
                other => helper_call(bin_helper(*other), vec![l, r]),
            }
        }
    }
}

pub fn emit_stmt(stmt: &IrStmt) -> Stmt {
    match stmt {
        IrStmt::Local { name, value, .. } => Stmt::Decl(Decl::Var(Box::new(VarDecl {
            span: DUMMY_SP,
            ctxt: SyntaxContext::empty(),
            kind: VarDeclKind::Let,
            declare: false,
            decls: vec![VarDeclarator {
                span: DUMMY_SP,
                name: Pat::Ident(binding(name)),
                init: Some(Box::new(match value {
                    Some(value) => emit_expr(value),
                    None => null_lit(),
                })),
                definite: false,
            }],
        }))),
        IrStmt::Assign { target, value, .. } => {
            let target = match target {
                IrTarget::Var(name) => AssignTarget::Simple(SimpleAssignTarget::Ident(binding(name))),
            };
            Stmt::Expr(ExprStmt {
                span: DUMMY_SP,
                expr: Box::new(Expr::Assign(AssignExpr {
                    span: DUMMY_SP,
                    op: AssignOp::Assign,
                    left: target,
                    right: Box::new(emit_expr(value)),
                })),
            })
        }
        IrStmt::Expr { expr, .. } => Stmt::Expr(ExprStmt {
            span: DUMMY_SP,
            expr: Box::new(emit_expr(expr)),
        }),
        IrStmt::Do { body, .. } => Stmt::Block(BlockStmt {
            span: DUMMY_SP,
            ctxt: SyntaxContext::empty(),
            stmts: body.iter().map(emit_stmt).collect(),
        }),
        IrStmt::Return { values, .. } => Stmt::Return(ReturnStmt {
            span: DUMMY_SP,
            arg: values.first().map(|v| Box::new(emit_expr(v))),
        }),
    }
}

/// Emit a complete JS script (classic script goal, so `return` is allowed at
/// the top level of a chunk) for a validated chunk.
pub fn emit_js(chunk: &IrChunk) -> Result<String, CompileError> {
    let program = Program::Script(Script {
        span: DUMMY_SP,
        body: chunk.body.iter().map(emit_stmt).collect(),
        shebang: None,
    });
    let cm: Lrc<SourceMap> = Default::default();
    cm.new_source_file(Lrc::new(FileName::Anon), String::new());
    let mut buf: Vec<u8> = Vec::new();
    GLOBALS.set(&Default::default(), || -> Result<(), CompileError> {
        let mut emitter = Emitter {
            cfg: Default::default(),
            cm: cm.clone(),
            comments: None,
            wr: JsWriter::new(cm.clone(), "\n", &mut buf, None),
        };
        emitter
            .emit_program(&program)
            .map_err(|e| CompileError::new(format!("js codegen failed: {e}")))?;
        Ok(())
    })?;
    String::from_utf8(buf).map_err(|e| CompileError::new(format!("js codegen utf8: {e}")))
}
