//! LunaIR -> JavaScript emission via the SWC AST + codegen.
//!
//! Direction-paired with the Lua frontend: assumes Lua-origin IR and lowers
//! all Lua semantics into `lure` runtime helper calls (`luaAdd`, `luaTruthy`,
//! ...). The emitted script is a classic (non-module) script so that a
//! top-level `return` stays legal, matching Lua chunk semantics.
//!
//! Notes:
//! - Lua locals become JS `let`/`const`: block scoping matches, and `let` in a
//!   `for` init gives the per-iteration binding freshness Lua numeric-for has
//!   for closures.
//! - `local function f()` becomes `const f = function f() {...}`: the named
//!   function expression makes recursion work without any hoisting analysis
//!   (mutual recursion resolves at call time, as in Lua).
//! - Functions normalize `undefined` params to `null` (Lua pads missing args
//!   with nil) and get a trailing `return null;` (Lua fallthrough returns
//!   nothing = nil = null, not undefined).
//! - Numeric `for` desugars to a block computing limit/step once (Lua
//!   semantics) with a sign-dependent continuation condition.

use crate::error::CompileError;
use crate::ir::{IrBinOp, IrChunk, IrExpr, IrStmt, IrTable, IrTarget, IrUnOp};
use swc_atoms::Atom;
use swc_common::sync::Lrc;
use swc_common::{DUMMY_SP, FileName, GLOBALS, SourceMap, SyntaxContext};
use swc_ecma_ast::*;
use swc_ecma_codegen::{Emitter, text_writer::JsWriter};

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

fn num_lit(value: f64) -> Expr {
    Expr::Lit(Lit::Num(Number {
        span: DUMMY_SP,
        value,
        raw: None,
    }))
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

/// `() => body` — used to preserve short-circuit semantics for and/or/cond.
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

fn block(stmts: Vec<Stmt>) -> BlockStmt {
    BlockStmt {
        span: DUMMY_SP,
        ctxt: SyntaxContext::empty(),
        stmts,
    }
}

fn var_decl(kind: VarDeclKind, name: &str, init: Option<Expr>) -> Stmt {
    Stmt::Decl(Decl::Var(Box::new(VarDecl {
        span: DUMMY_SP,
        ctxt: SyntaxContext::empty(),
        kind,
        declare: false,
        decls: vec![VarDeclarator {
            span: DUMMY_SP,
            name: Pat::Ident(binding(name)),
            init: init.map(Box::new),
            definite: false,
        }],
    })))
}

fn assign_stmt(name: &str, value: Expr) -> Stmt {
    Stmt::Expr(ExprStmt {
        span: DUMMY_SP,
        expr: Box::new(Expr::Assign(AssignExpr {
            span: DUMMY_SP,
            op: AssignOp::Assign,
            left: AssignTarget::Simple(SimpleAssignTarget::Ident(binding(name))),
            right: Box::new(value),
        })),
    })
}

/// `p = p === void 0 ? null : p;` — Lua pads missing call arguments with nil,
/// and JS `undefined` must not leak into the Lua value domain.
fn param_prologue(name: &str) -> Stmt {
    assign_stmt(
        name,
        Expr::Cond(CondExpr {
            span: DUMMY_SP,
            test: Box::new(Expr::Bin(BinExpr {
                span: DUMMY_SP,
                op: BinaryOp::EqEqEq,
                left: Box::new(Expr::Ident(ident(name))),
                right: Box::new(Expr::Unary(UnaryExpr {
                    span: DUMMY_SP,
                    op: UnaryOp::Void,
                    arg: Box::new(num_lit(0.0)),
                })),
            })),
            cons: Box::new(null_lit()),
            alt: Box::new(Expr::Ident(ident(name))),
        }),
    )
}

/// `function [name](params) { ...prologue... body; return null; }`
fn function_expr(name: Option<&str>, params: &[String], body: &[IrStmt]) -> Expr {
    let mut stmts: Vec<Stmt> = params.iter().map(|p| param_prologue(p)).collect();
    stmts.extend(body.iter().map(emit_stmt));
    if !matches!(body.last(), Some(IrStmt::Return { .. })) {
        stmts.push(Stmt::Return(ReturnStmt {
            span: DUMMY_SP,
            arg: Some(Box::new(null_lit())),
        }));
    }
    Expr::Fn(FnExpr {
        ident: name.map(ident),
        function: Box::new(Function {
            params: params
                .iter()
                .map(|p| Param {
                    span: DUMMY_SP,
                    decorators: vec![],
                    pat: Pat::Ident(binding(p)),
                })
                .collect(),
            decorators: vec![],
            span: DUMMY_SP,
            ctxt: SyntaxContext::empty(),
            body: Some(block(stmts)),
            is_generator: false,
            is_async: false,
            type_params: None,
            return_type: None,
        }),
    })
}

pub fn emit_expr(expr: &IrExpr) -> Expr {
    match expr {
        IrExpr::Nil => null_lit(),
        IrExpr::Bool(value) => Expr::Lit(Lit::Bool(Bool {
            span: DUMMY_SP,
            value: *value,
        })),
        IrExpr::Num(value) => num_lit(*value),
        IrExpr::Str(value) => Expr::Lit(Lit::Str(Str {
            span: DUMMY_SP,
            value: Atom::from(value.as_str()).into(),
            raw: None,
        })),
        IrExpr::Var(name) => Expr::Ident(ident(name)),
        IrExpr::Call { func, args, .. } => helper_call(
            "luaCall",
            vec![
                emit_expr(func),
                Expr::Array(ArrayLit {
                    span: DUMMY_SP,
                    elems: args
                        .iter()
                        .map(|arg| {
                            Some(ExprOrSpread {
                                spread: None,
                                expr: Box::new(emit_expr(arg)),
                            })
                        })
                        .collect(),
                }),
            ],
        ),
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
        IrExpr::Cond {
            cond, then, else_, ..
        } => helper_call(
            "luaCond",
            vec![
                emit_expr(cond),
                thunk(emit_expr(then)),
                thunk(emit_expr(else_)),
            ],
        ),
        IrExpr::Function { params, body, .. } => function_expr(None, params, body),
        IrExpr::Index { obj, key, .. } => {
            helper_call("luaGet", vec![emit_expr(obj), emit_expr(key)])
        }
        IrExpr::Table { fields, .. } => match fields {
            // Array part: `luaTable([v1, v2, ...])` fills keys 1..=n (Lua base).
            IrTable::Array(items) => helper_call(
                "luaTable",
                vec![Expr::Array(ArrayLit {
                    span: DUMMY_SP,
                    elems: items
                        .iter()
                        .map(|item| {
                            Some(ExprOrSpread {
                                spread: None,
                                expr: Box::new(emit_expr(item)),
                            })
                        })
                        .collect(),
                })],
            ),
            // Record part: `luaTableRecord([[k1, v1], [k2, v2], ...])` — keys
            // evaluated left to right, so key order is observable.
            IrTable::Record(pairs) => helper_call(
                "luaTableRecord",
                vec![Expr::Array(ArrayLit {
                    span: DUMMY_SP,
                    elems: pairs
                        .iter()
                        .map(|(key, value)| {
                            Some(ExprOrSpread {
                                spread: None,
                                expr: Box::new(Expr::Array(ArrayLit {
                                    span: DUMMY_SP,
                                    elems: vec![
                                        Some(ExprOrSpread {
                                            spread: None,
                                            expr: Box::new(emit_expr(key)),
                                        }),
                                        Some(ExprOrSpread {
                                            spread: None,
                                            expr: Box::new(emit_expr(value)),
                                        }),
                                    ],
                                })),
                            })
                        })
                        .collect(),
                })],
            ),
        },
    }
}

fn emit_if(arms: &[(IrExpr, Vec<IrStmt>)], else_: Option<&Vec<IrStmt>>) -> Stmt {
    let (cond, body) = &arms[0];
    let alt = if arms.len() > 1 {
        Some(Box::new(emit_if(&arms[1..], else_)))
    } else {
        else_.map(|stmts| Box::new(Stmt::Block(block(emit_stmts(stmts)))) as Box<Stmt>)
    };
    Stmt::If(IfStmt {
        span: DUMMY_SP,
        test: Box::new(helper_call("luaTruthy", vec![emit_expr(cond)])),
        cons: Box::new(Stmt::Block(block(emit_stmts(body)))),
        alt,
    })
}

fn emit_numeric_for(
    var: &str,
    start: &IrExpr,
    stop: &IrExpr,
    step: Option<&IrExpr>,
    body: &[IrStmt],
) -> Stmt {
    // `$` names are unforgeable from Lua source, so these temps never collide.
    const LIMIT: &str = "lua$limit";
    const STEP: &str = "lua$step";
    let step_expr = step.map(emit_expr).unwrap_or_else(|| num_lit(1.0));
    let v = || Expr::Ident(ident(var));
    let limit = || Expr::Ident(ident(LIMIT));
    let step_ref = || Expr::Ident(ident(STEP));
    let cond = Expr::Cond(CondExpr {
        span: DUMMY_SP,
        test: Box::new(Expr::Bin(BinExpr {
            span: DUMMY_SP,
            op: BinaryOp::Gt,
            left: Box::new(step_ref()),
            right: Box::new(num_lit(0.0)),
        })),
        cons: Box::new(Expr::Bin(BinExpr {
            span: DUMMY_SP,
            op: BinaryOp::LtEq,
            left: Box::new(v()),
            right: Box::new(limit()),
        })),
        alt: Box::new(Expr::Bin(BinExpr {
            span: DUMMY_SP,
            op: BinaryOp::GtEq,
            left: Box::new(v()),
            right: Box::new(limit()),
        })),
    });
    let update = Expr::Assign(AssignExpr {
        span: DUMMY_SP,
        op: AssignOp::Assign,
        left: AssignTarget::Simple(SimpleAssignTarget::Ident(binding(var))),
        right: Box::new(helper_call("luaAdd", vec![v(), step_ref()])),
    });
    Stmt::Block(block(vec![
        var_decl(VarDeclKind::Let, LIMIT, Some(emit_expr(stop))),
        var_decl(VarDeclKind::Let, STEP, Some(step_expr)),
        Stmt::For(ForStmt {
            span: DUMMY_SP,
            init: Some(VarDeclOrExpr::VarDecl(Box::new(VarDecl {
                span: DUMMY_SP,
                ctxt: SyntaxContext::empty(),
                kind: VarDeclKind::Let,
                declare: false,
                decls: vec![VarDeclarator {
                    span: DUMMY_SP,
                    name: Pat::Ident(binding(var)),
                    init: Some(Box::new(emit_expr(start))),
                    definite: false,
                }],
            }))),
            test: Some(Box::new(cond)),
            update: Some(Box::new(update)),
            body: Box::new(Stmt::Block(block(emit_stmts(body)))),
        }),
    ]))
}

pub fn emit_stmt(stmt: &IrStmt) -> Stmt {
    match stmt {
        IrStmt::Local { name, value, .. } => {
            let init = match value {
                // Named function expression: recursion without hoisting.
                Some(IrExpr::Function { params, body, .. }) => {
                    Some(function_expr(Some(name), params, body))
                }
                Some(value) => Some(emit_expr(value)),
                None => Some(null_lit()),
            };
            var_decl(VarDeclKind::Let, name, init)
        }
        IrStmt::Assign { target, value, .. } => match target {
            IrTarget::Var(name) => assign_stmt(name, emit_expr(value)),
            IrTarget::Index { obj, key } => Stmt::Expr(ExprStmt {
                span: DUMMY_SP,
                expr: Box::new(helper_call(
                    "luaSet",
                    vec![emit_expr(obj), emit_expr(key), emit_expr(value)],
                )),
            }),
        },
        IrStmt::Expr { expr, .. } => Stmt::Expr(ExprStmt {
            span: DUMMY_SP,
            expr: Box::new(emit_expr(expr)),
        }),
        IrStmt::Do { body, .. } => Stmt::Block(block(emit_stmts(body))),
        IrStmt::Return { values, .. } => Stmt::Return(ReturnStmt {
            span: DUMMY_SP,
            arg: Some(Box::new(match values.first() {
                Some(value) => emit_expr(value),
                // `return` with no values is Lua-nil, not JS-undefined.
                None => null_lit(),
            })),
        }),
        IrStmt::If { arms, else_, .. } => emit_if(arms, else_.as_ref()),
        IrStmt::While { cond, body, .. } => Stmt::While(WhileStmt {
            span: DUMMY_SP,
            test: Box::new(helper_call("luaTruthy", vec![emit_expr(cond)])),
            body: Box::new(Stmt::Block(block(emit_stmts(body)))),
        }),
        IrStmt::NumericFor {
            var,
            start,
            stop,
            step,
            body,
            ..
        } => emit_numeric_for(var, start, stop, step.as_ref(), body),
        IrStmt::Break { .. } => Stmt::Break(BreakStmt {
            span: DUMMY_SP,
            label: None,
        }),
    }
}

pub fn emit_stmts(stmts: &[IrStmt]) -> Vec<Stmt> {
    stmts.iter().map(emit_stmt).collect()
}

/// Emit a complete JS script (classic script goal, so `return` is allowed at
/// the top level of a chunk) for a validated chunk.
pub fn emit_js(chunk: &IrChunk) -> Result<String, CompileError> {
    let program = Program::Script(Script {
        span: DUMMY_SP,
        body: emit_stmts(&chunk.body),
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
