//! js2lua lowering: SWC AST -> LunaIR.
//!
//! HYGIENE (workspace rule): scope membership is binding identity, so scopes
//! are keyed by `swc_ecma_ast::Id` (`(Atom, SyntaxContext)`), never by a
//! flattened `String`. `.sym` text is only read to *compute a mangled display
//! name* for an already-identified binding, and to recognize the fixed global
//! sentinel names (`undefined`, `NaN`, `Infinity`) — textual domains, which is
//! the rule's stated exception.
//!
//! SCOPE MODEL: JS `let`/`const` are block-scoped with the binding covering
//! the *whole* block (TDZ before the declaration line — a runtime error in
//! JS; we resolve to the local anyway, a documented divergence), so each block
//! pre-declares all of its locals in a first pass. Function declarations are
//! additionally *hoisted*: they are lowered to `local f = function...` at the
//! top of their block, so calls before the declaration line work.
//!
//! CLASSIC FOR: lowered to IR NumericFor only when it matches the shape
//! `for (let i = e1; i <= e2; i++/i += e3)` (or the `>=`/`-=` mirror). The
//! loop variable is compared by `Id`, never by name text.

pub mod emit;

use crate::error::CompileError;
use crate::ir::{IrBinOp, IrChunk, IrExpr, IrStmt, IrTarget, IrUnOp, Span};
use std::collections::HashSet;
use swc_common::{FileName, GLOBALS, SourceMap, Span as SwcSpan, Spanned, sync::Lrc};
use swc_ecma_ast::*;
use swc_ecma_parser::{EsSyntax, Parser, StringInput, Syntax};

/// Lua keywords plus names the emitted Lua expects in scope (js.lua runtime
/// helpers/globals). Colliding JS identifiers get a `_` suffix.
const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// Names injected by the js.lua runtime (helpers, sentinels, stdlib shims).
const LUA_RUNTIME_NAMES: &[&str] = &[
    "console", "jsAdd", "jsAnd", "jsArith", "jsConcat", "jsCond", "jsDiv", "jsGe", "jsGet",
    "jsGt", "jsInfinity", "jsLe", "jsLen", "jsLog", "jsLt", "jsMul", "jsNaN", "jsNeg", "jsNot",
    "jsOr", "jsPow", "jsRem", "jsSet", "jsStrictEq", "jsSub", "jsToNumber", "jsToString",
    "jsTruthy", "jsTypeof", "jsUndefined",
];

fn runtime_like(name: &str) -> bool {
    LUA_RUNTIME_NAMES.contains(&name)
        || (name.starts_with("js")
            && name[2..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase()))
}

/// The fixed JS globals that map to js.lua sentinel values.
fn sentinel_target(name: &str) -> Option<&'static str> {
    match name {
        "undefined" => Some("jsUndefined"),
        "NaN" => Some("jsNaN"),
        "Infinity" => Some("jsInfinity"),
        _ => None,
    }
}

/// Whether a JS *local* with this source name must be mangled when embedded
/// in Lua output.
fn needs_mangle(name: &str) -> bool {
    LUA_KEYWORDS.contains(&name) || runtime_like(name) || sentinel_target(name).is_some()
}

/// The portable identifier grammar (SUBSET.md §2): no `$`, no non-ASCII.
fn check_portable(name: &str) -> Result<(), CompileError> {
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(CompileError::new(format!(
            "identifier `{name}` is outside the subset (no `$` or non-ASCII characters)"
        )))
    }
}

/// The JS-side lowering state: a stack of local scopes holding swc `Id`s
/// (never flattened strings — see the hygiene note at the top of this file).
struct Lower {
    cm: Lrc<SourceMap>,
    scopes: Vec<HashSet<Id>>,
    in_function: bool,
}

impl Lower {
    fn new(cm: Lrc<SourceMap>) -> Self {
        Lower {
            cm,
            scopes: vec![HashSet::new()],
            in_function: false,
        }
    }

    fn is_local(&self, id: &Id) -> bool {
        self.scopes.iter().rev().any(|scope| scope.contains(id))
    }

    /// Resolve a *reference* to a (possibly global) name.
    fn resolve_name(&self, ident: &Ident) -> Result<String, CompileError> {
        let sym = ident.sym.as_str();
        check_portable(sym).map_err(|e| self.err_at(ident.span, e.message))?;
        if sym == "arguments" {
            return Err(self.unsupported(ident.span, "`arguments`"));
        }
        if self.is_local(&ident.to_id()) {
            return Ok(if needs_mangle(sym) {
                format!("{sym}_")
            } else {
                sym.to_string()
            });
        }
        if let Some(target) = sentinel_target(sym) {
            return Ok(target.to_string());
        }
        if LUA_KEYWORDS.contains(&sym) {
            // Syntactic necessity: a global named e.g. `end` cannot be spelled
            // bare in Lua output.
            return Ok(format!("{sym}_"));
        }
        Ok(sym.to_string())
    }

    /// Declare a local in the current scope. Callers that need the mangled
    /// name afterwards use `resolve_name` (which will now see the local).
    fn declare_local(&mut self, ident: &Ident) -> Result<(), CompileError> {
        let sym = ident.sym.as_str();
        check_portable(sym).map_err(|e| self.err_at(ident.span, e.message))?;
        self.scopes
            .last_mut()
            .expect("scope stack is never empty")
            .insert(ident.to_id());
        Ok(())
    }

    fn err_at(&self, span: SwcSpan, message: impl Into<String>) -> CompileError {
        let loc = self.cm.lookup_char_pos(span.lo);
        CompileError::at(loc.line as u32, loc.col.0 as u32 + 1, message)
    }

    fn unsupported(&self, span: SwcSpan, what: &str) -> CompileError {
        self.err_at(span, format!("{what} is outside the subset"))
    }

    fn span_of(&self, span: SwcSpan) -> Span {
        let file_start = self.cm.files()[0].start_pos.0;
        Span {
            lo: span.lo.0.saturating_sub(file_start),
            hi: span.hi.0.saturating_sub(file_start),
        }
    }

    /// Lower a statement list with JS block semantics: pre-declare all locals
    /// (block-wide binding), hoist function declarations to the top.
    fn lower_stmt_list(&mut self, stmts: &[Stmt]) -> Result<Vec<IrStmt>, CompileError> {
        // Pass 1: declare every local name in the block.
        for stmt in stmts {
            match stmt {
                Stmt::Decl(Decl::Var(decl)) => {
                    if decl.kind == VarDeclKind::Var {
                        return Err(self.unsupported(decl.span, "`var` (use `let`/`const`)"));
                    }
                    for declarator in &decl.decls {
                        let Pat::Ident(binding) = &declarator.name else {
                            return Err(self.unsupported(declarator.span, "destructuring"));
                        };
                        self.declare_local(&binding.id)?;
                    }
                }
                Stmt::Decl(Decl::Fn(f)) => self.declare_local(&f.ident)?,
                _ => {}
            }
        }
        // Pass 2: hoist function declarations to the top (definition order).
        // First emit bare `local name` forward declarations so every hoisted
        // body sees every hoisted name as an upvalue (JS: both the binding and
        // the *value* are available before the declaration line — mutual
        // recursion calls work).
        let mut out = Vec::new();
        for stmt in stmts {
            if let Stmt::Decl(Decl::Fn(f)) = stmt {
                out.push(IrStmt::Local {
                    name: self.resolve_name(&f.ident)?,
                    value: None,
                    span: self.span_of(f.function.span),
                });
            }
        }
        for stmt in stmts {
            if let Stmt::Decl(Decl::Fn(f)) = stmt {
                out.push(self.lower_fn_decl(f)?);
            }
        }
        // Pass 3: everything else, in order.
        for stmt in stmts {
            if matches!(stmt, Stmt::Decl(Decl::Fn(_))) {
                continue;
            }
            self.lower_stmt(stmt, &mut out)?;
        }
        Ok(out)
    }

    /// Lower a block as a nested scope.
    fn lower_block_stmt(&mut self, block: &BlockStmt) -> Result<Vec<IrStmt>, CompileError> {
        self.scopes.push(HashSet::new());
        let result = self.lower_stmt_list(&block.stmts);
        self.scopes.pop();
        result
    }

    /// Lower a single statement into block form (for if/loop bodies).
    fn lower_body_stmt(&mut self, stmt: &Stmt) -> Result<Vec<IrStmt>, CompileError> {
        match stmt {
            Stmt::Block(block) => self.lower_block_stmt(block),
            other => {
                self.scopes.push(HashSet::new());
                let result = (|| {
                    let mut body = Vec::new();
                    self.lower_stmt(other, &mut body)?;
                    Ok(body)
                })();
                self.scopes.pop();
                result
            }
        }
    }

    /// Lower params + body in a fresh scope; `fn_name`, if present, is the
    /// function-expression self name visible inside the body.
    fn lower_function(
        &mut self,
        fn_name: Option<&Ident>,
        params: &[Param],
        body: &BlockStmt,
    ) -> Result<(Vec<String>, Vec<IrStmt>), CompileError> {
        self.scopes.push(HashSet::new());
        let prev_in_function = self.in_function;
        self.in_function = true;
        let result = (|| {
            if let Some(name) = fn_name {
                self.declare_local(name)?;
            }
            let mut lowered_params = Vec::new();
            for param in params {
                let Pat::Ident(binding) = &param.pat else {
                    return Err(self.unsupported(param.span, "parameter patterns/defaults"));
                };
                self.declare_local(&binding.id)?;
                lowered_params.push(self.resolve_name(&binding.id)?);
            }
            let body = self.lower_stmt_list(&body.stmts)?;
            Ok((lowered_params, body))
        })();
        self.in_function = prev_in_function;
        self.scopes.pop();
        result
    }

    fn lower_fn_decl(&mut self, decl: &FnDecl) -> Result<IrStmt, CompileError> {
        let span = self.span_of(decl.function.span);
        // Hoisted: the forward `local name` was emitted in pass 2 already, so
        // this is the value *assignment*, not a fresh declaration (a second
        // `local name` would shadow the binding the body captured).
        let name = self.resolve_name(&decl.ident)?;
        if decl.function.is_async || decl.function.is_generator {
            return Err(self.unsupported(decl.function.span, "async/generator functions"));
        }
        let body = decl.function.body.as_ref().ok_or_else(|| {
            self.unsupported(decl.function.span, "bodyless function declarations")
        })?;
        let (params, body) = self.lower_function(None, &decl.function.params, body)?;
        Ok(IrStmt::Assign {
            target: IrTarget::Var(name),
            value: IrExpr::Function {
                params,
                body,
                span,
            },
            span,
        })
    }

    /// Lower a classic `for` loop, requiring the numeric-for shape
    /// `for (let i = e1; i <= e2; i++/i += e3)` (or `>=`/`-=` mirror).
    fn lower_for(&mut self, for_stmt: &ForStmt) -> Result<IrStmt, CompileError> {
        let span = self.span_of(for_stmt.span);
        let shape_err = |lower: &Lower| {
            lower.unsupported(
                for_stmt.span,
                "this `for` shape (subset: `for (let i = e1; i <= e2; i++/i += e3)` and mirrors)",
            )
        };
        let init = for_stmt
            .init
            .as_ref()
            .ok_or_else(|| shape_err(self))?;
        let VarDeclOrExpr::VarDecl(decl) = init else {
            return Err(shape_err(self));
        };
        if decl.kind == VarDeclKind::Var {
            return Err(self.unsupported(decl.span, "`var` (use `let`/`const`)"));
        }
        if decl.decls.len() != 1 {
            return Err(shape_err(self));
        }
        let declarator = &decl.decls[0];
        let Pat::Ident(binding) = &declarator.name else {
            return Err(shape_err(self));
        };
        let Some(start_expr) = &declarator.init else {
            return Err(shape_err(self));
        };
        // The init expression sees the *outer* binding of the name (JS).
        let start = self.lower_expr(start_expr)?;
        self.scopes.push(HashSet::new());
        let result = (|| {
            self.declare_local(&binding.id)?;
            let loop_id = binding.id.to_id();
            // Test must be `i <= e2` or `i >= e2`.
            let test = for_stmt.test.as_ref().ok_or_else(|| shape_err(self))?;
            let Expr::Bin(test_bin) = &**test else {
                return Err(shape_err(self));
            };
            if !matches!(test_bin.op, BinaryOp::LtEq | BinaryOp::GtEq) {
                return Err(shape_err(self));
            }
            let Expr::Ident(test_var) = &*test_bin.left else {
                return Err(shape_err(self));
            };
            if test_var.to_id() != loop_id {
                return Err(shape_err(self));
            }
            let stop = self.lower_expr(&test_bin.right)?;
            // Update must be `i++`, `i--`, `i += e3`, or `i -= e3`.
            let update = for_stmt.update.as_ref().ok_or_else(|| shape_err(self))?;
            let step = match &**update {
                Expr::Update(update) => {
                    let Expr::Ident(arg) = &*update.arg else {
                        return Err(shape_err(self));
                    };
                    if arg.to_id() != loop_id {
                        return Err(shape_err(self));
                    }
                    match update.op {
                        UpdateOp::PlusPlus => IrExpr::Num(1.0),
                        UpdateOp::MinusMinus => IrExpr::Num(-1.0),
                    }
                }
                Expr::Assign(assign) => {
                    let AssignTarget::Simple(SimpleAssignTarget::Ident(target)) = &assign.left
                    else {
                        return Err(shape_err(self));
                    };
                    if target.id.to_id() != loop_id {
                        return Err(shape_err(self));
                    }
                    match assign.op {
                        AssignOp::AddAssign => self.lower_expr(&assign.right)?,
                        AssignOp::SubAssign => IrExpr::Un {
                            op: IrUnOp::Neg,
                            expr: Box::new(self.lower_expr(&assign.right)?),
                            span,
                        },
                        _ => return Err(shape_err(self)),
                    }
                }
                _ => return Err(shape_err(self)),
            };
            let body = self.lower_body_stmt(&for_stmt.body)?;
            let var = self.resolve_name(&binding.id)?;
            Ok(IrStmt::NumericFor {
                var,
                start,
                stop,
                step: Some(step),
                body,
                span,
            })
        })();
        self.scopes.pop();
        result
    }

    /// `x += e` / `x -= e` as a statement, desugared to `x = x + e`.
    fn lower_compound_assign(
        &mut self,
        assign: &AssignExpr,
        op: IrBinOp,
    ) -> Result<IrStmt, CompileError> {
        let span = self.span_of(assign.span);
        let AssignTarget::Simple(SimpleAssignTarget::Ident(binding)) = &assign.left else {
            return Err(self.unsupported(assign.span, "compound assignment to members (Phase 3)"));
        };
        let name = self.resolve_name(&binding.id)?;
        let rhs = self.lower_expr(&assign.right)?;
        Ok(IrStmt::Assign {
            target: IrTarget::Var(name.clone()),
            value: IrExpr::Bin {
                op,
                lhs: Box::new(IrExpr::Var(name)),
                rhs: Box::new(rhs),
                span,
            },
            span,
        })
    }

    /// `x++` / `x--` as a statement, desugared to `x = x + 1` / `x = x - 1`.
    fn lower_update_stmt(&mut self, update: &UpdateExpr) -> Result<IrStmt, CompileError> {
        let span = self.span_of(update.span);
        let Expr::Ident(arg) = &*update.arg else {
            return Err(self.unsupported(update.span, "update expressions on members (Phase 3)"));
        };
        let name = self.resolve_name(arg)?;
        let op = match update.op {
            UpdateOp::PlusPlus => IrBinOp::Add,
            UpdateOp::MinusMinus => IrBinOp::Sub,
        };
        Ok(IrStmt::Assign {
            target: IrTarget::Var(name.clone()),
            value: IrExpr::Bin {
                op,
                lhs: Box::new(IrExpr::Var(name)),
                rhs: Box::new(IrExpr::Num(1.0)),
                span,
            },
            span,
        })
    }

    fn lower_if(&mut self, if_stmt: &IfStmt) -> Result<IrStmt, CompileError> {
        let span = self.span_of(if_stmt.span);
        let mut arms = Vec::new();
        let mut current: &IfStmt = if_stmt;
        loop {
            let cond = self.lower_expr(&current.test)?;
            let body = self.lower_body_stmt(&current.cons)?;
            arms.push((cond, body));
            match &current.alt {
                Some(alt) => match &**alt {
                    Stmt::If(next) => current = next,
                    other => {
                        let else_ = self.lower_body_stmt(other)?;
                        return Ok(IrStmt::If {
                            arms,
                            else_: Some(else_),
                            span,
                        });
                    }
                },
                None => {
                    return Ok(IrStmt::If {
                        arms,
                        else_: None,
                        span,
                    });
                }
            }
        }
    }

    fn lower_stmt(&mut self, stmt: &Stmt, out: &mut Vec<IrStmt>) -> Result<(), CompileError> {
        match stmt {
            Stmt::Decl(Decl::Var(decl)) => {
                for declarator in &decl.decls {
                    // Name patterns were pre-declared in pass 1.
                    let Pat::Ident(binding) = &declarator.name else {
                        return Err(self.unsupported(declarator.span, "destructuring"));
                    };
                    let span = self.span_of(declarator.span);
                    let value = match &declarator.init {
                        Some(init) => Some(self.lower_expr(init)?),
                        // `let x;` initializes x to `undefined` — which is NOT
                        // nil/null in the mapping — so make the sentinel
                        // explicit rather than emitting a bare `local x`.
                        None => Some(IrExpr::Var("jsUndefined".into())),
                    };
                    let name = self.resolve_name(&binding.id)?;
                    out.push(IrStmt::Local { name, value, span });
                }
                Ok(())
            }
            Stmt::Decl(Decl::Fn(_)) => {
                unreachable!("function declarations are hoisted in pass 2")
            }
            Stmt::Expr(expr_stmt) => match &*expr_stmt.expr {
                Expr::Call(_) => {
                    let span = self.span_of(expr_stmt.span);
                    out.push(IrStmt::Expr {
                        expr: self.lower_expr(&expr_stmt.expr)?,
                        span,
                    });
                    Ok(())
                }
                Expr::Assign(assign) => {
                    match assign.op {
                        AssignOp::Assign => {
                            let span = self.span_of(assign.span);
                            let target = match &assign.left {
                                AssignTarget::Simple(SimpleAssignTarget::Ident(binding)) => {
                                    IrTarget::Var(self.resolve_name(&binding.id)?)
                                }
                                AssignTarget::Simple(_) => {
                                    return Err(self
                                        .unsupported(assign.span, "member assignment (Phase 3)"));
                                }
                                AssignTarget::Pat(_) => {
                                    return Err(
                                        self.unsupported(assign.span, "destructuring assignment")
                                    );
                                }
                            };
                            out.push(IrStmt::Assign {
                                target,
                                value: self.lower_expr(&assign.right)?,
                                span,
                            });
                            Ok(())
                        }
                        AssignOp::AddAssign => {
                            out.push(self.lower_compound_assign(assign, IrBinOp::Add)?);
                            Ok(())
                        }
                        AssignOp::SubAssign => {
                            out.push(self.lower_compound_assign(assign, IrBinOp::Sub)?);
                            Ok(())
                        }
                        _ => Err(self.unsupported(assign.span, "this compound assignment")),
                    }
                }
                Expr::Update(update) => {
                    out.push(self.lower_update_stmt(update)?);
                    Ok(())
                }
                _ => Err(self.unsupported(
                    expr_stmt.span,
                    "expression statements other than calls and assignments",
                )),
            },
            Stmt::Block(block) => {
                let span = self.span_of(block.span);
                out.push(IrStmt::Do {
                    body: self.lower_block_stmt(block)?,
                    span,
                });
                Ok(())
            }
            Stmt::If(if_stmt) => {
                out.push(self.lower_if(if_stmt)?);
                Ok(())
            }
            Stmt::While(while_stmt) => {
                let span = self.span_of(while_stmt.span);
                out.push(IrStmt::While {
                    cond: self.lower_expr(&while_stmt.test)?,
                    body: self.lower_body_stmt(&while_stmt.body)?,
                    span,
                });
                Ok(())
            }
            Stmt::For(for_stmt) => {
                out.push(self.lower_for(for_stmt)?);
                Ok(())
            }
            Stmt::Break(break_stmt) => {
                if break_stmt.label.is_some() {
                    return Err(self.unsupported(break_stmt.span, "labeled `break`"));
                }
                out.push(IrStmt::Break {
                    span: self.span_of(break_stmt.span),
                });
                Ok(())
            }
            Stmt::Return(ret) => {
                if !self.in_function {
                    return Err(self.unsupported(ret.span, "top-level `return` (JS has none)"));
                }
                let span = self.span_of(ret.span);
                let values = match &ret.arg {
                    Some(arg) => vec![self.lower_expr(arg)?],
                    None => vec![],
                };
                out.push(IrStmt::Return { values, span });
                Ok(())
            }
            Stmt::DoWhile(_) => Err(self.unsupported(stmt.span(), "`do...while`")),
            Stmt::ForOf(_) | Stmt::ForIn(_) => {
                Err(self.unsupported(stmt.span(), "`for..of`/`for..in` (Phase 5)"))
            }
            Stmt::Decl(_) => Err(self.unsupported(stmt.span(), "this declaration form")),
            other => Err(self.unsupported(other.span(), "this statement form")),
        }
    }

    fn lower_call(&mut self, call: &CallExpr) -> Result<IrExpr, CompileError> {
        let span = self.span_of(call.span);
        let Callee::Expr(callee) = &call.callee else {
            return Err(self.unsupported(call.span, "super/import calls"));
        };
        let mut args = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            if arg.spread.is_some() {
                return Err(self.unsupported(call.span, "spread arguments (Phase 4)"));
            }
            args.push(self.lower_expr(&arg.expr)?);
        }
        // Stdlib shim: `console.log(...)` lowers to the js.lua `jsLog` entry.
        if let Expr::Member(member) = &**callee {
            if let (Expr::Ident(obj), MemberProp::Ident(prop)) = (&*member.obj, &member.prop)
                && obj.sym.as_str() == "console"
                && prop.sym.as_str() == "log"
                && !self.is_local(&obj.to_id())
            {
                return Ok(IrExpr::Call {
                    func: Box::new(IrExpr::Var("jsLog".into())),
                    args,
                    span,
                });
            }
            return Err(self.unsupported(member.span, "method calls (Phase 3)"));
        }
        let func = self.lower_expr(callee)?;
        Ok(IrExpr::Call {
            func: Box::new(func),
            args,
            span,
        })
    }

    fn lower_expr(&mut self, expr: &Expr) -> Result<IrExpr, CompileError> {
        match expr {
            Expr::Lit(lit) => match lit {
                Lit::Null(_) => Ok(IrExpr::Nil),
                Lit::Bool(b) => Ok(IrExpr::Bool(b.value)),
                Lit::Num(n) => Ok(IrExpr::Num(n.value)),
                Lit::Str(s) => Ok(IrExpr::Str(s.value.to_string_lossy().into_owned())),
                Lit::Regex(r) => Err(self.unsupported(r.span, "regex literals")),
                Lit::BigInt(b) => Err(self.unsupported(b.span, "bigint literals")),
                _ => Err(self.unsupported(expr.span(), "this literal form")),
            },
            Expr::Ident(id) => Ok(IrExpr::Var(self.resolve_name(id)?)),
            Expr::Paren(paren) => self.lower_expr(&paren.expr),
            Expr::Bin(bin) => {
                let span = self.span_of(bin.span);
                let op = match bin.op {
                    BinaryOp::Add => IrBinOp::Add,
                    BinaryOp::Sub => IrBinOp::Sub,
                    BinaryOp::Mul => IrBinOp::Mul,
                    BinaryOp::Div => IrBinOp::Div,
                    BinaryOp::Mod => IrBinOp::Mod,
                    BinaryOp::Exp => IrBinOp::Pow,
                    BinaryOp::EqEqEq => IrBinOp::Eq,
                    BinaryOp::NotEqEq => IrBinOp::Ne,
                    BinaryOp::Lt => IrBinOp::Lt,
                    BinaryOp::LtEq => IrBinOp::Le,
                    BinaryOp::Gt => IrBinOp::Gt,
                    BinaryOp::GtEq => IrBinOp::Ge,
                    BinaryOp::LogicalAnd => IrBinOp::And,
                    BinaryOp::LogicalOr => IrBinOp::Or,
                    BinaryOp::EqEq | BinaryOp::NotEq => {
                        return Err(self.unsupported(bin.span, "loose equality (use `===`/`!==`)"));
                    }
                    _ => {
                        return Err(self.unsupported(
                            bin.span,
                            "this binary operator (bitwise, `??`, `in`, `instanceof`)",
                        ));
                    }
                };
                Ok(IrExpr::Bin {
                    op,
                    lhs: Box::new(self.lower_expr(&bin.left)?),
                    rhs: Box::new(self.lower_expr(&bin.right)?),
                    span,
                })
            }
            Expr::Unary(un) => {
                let span = self.span_of(un.span);
                match un.op {
                    UnaryOp::Minus => Ok(IrExpr::Un {
                        op: IrUnOp::Neg,
                        expr: Box::new(self.lower_expr(&un.arg)?),
                        span,
                    }),
                    UnaryOp::Bang => Ok(IrExpr::Un {
                        op: IrUnOp::Not,
                        expr: Box::new(self.lower_expr(&un.arg)?),
                        span,
                    }),
                    // `typeof` lowers to a runtime call (lowering-internal idiom).
                    UnaryOp::TypeOf => Ok(IrExpr::Call {
                        func: Box::new(IrExpr::Var("jsTypeof".into())),
                        args: vec![self.lower_expr(&un.arg)?],
                        span,
                    }),
                    _ => Err(self.unsupported(un.span, "this unary operator")),
                }
            }
            Expr::Call(call) => self.lower_call(call),
            Expr::Cond(cond) => {
                let span = self.span_of(cond.span);
                Ok(IrExpr::Cond {
                    cond: Box::new(self.lower_expr(&cond.test)?),
                    then: Box::new(self.lower_expr(&cond.cons)?),
                    else_: Box::new(self.lower_expr(&cond.alt)?),
                    span,
                })
            }
            Expr::Fn(fn_expr) => {
                let span = self.span_of(fn_expr.function.span);
                if fn_expr.function.is_async || fn_expr.function.is_generator {
                    return Err(self.unsupported(fn_expr.function.span, "async/generator functions"));
                }
                let body = fn_expr.function.body.as_ref().ok_or_else(|| {
                    self.unsupported(fn_expr.function.span, "bodyless function expression")
                })?;
                let (params, body) =
                    self.lower_function(fn_expr.ident.as_ref(), &fn_expr.function.params, body)?;
                Ok(IrExpr::Function {
                    params,
                    body,
                    span,
                })
            }
            Expr::Arrow(arrow) => {
                let span = self.span_of(arrow.span);
                if arrow.is_async || arrow.is_generator {
                    return Err(self.unsupported(arrow.span, "async/generator functions"));
                }
                self.scopes.push(HashSet::new());
                let prev_in_function = self.in_function;
                self.in_function = true;
                let result = (|| {
                    let mut lowered_params = Vec::new();
                    for pat in &arrow.params {
                        let Pat::Ident(binding) = pat else {
                            return Err(
                                self.unsupported(arrow.span, "parameter patterns/defaults/rest")
                            );
                        };
                        self.declare_local(&binding.id)?;
                        lowered_params.push(self.resolve_name(&binding.id)?);
                    }
                    let body = match &*arrow.body {
                        BlockStmtOrExpr::BlockStmt(block) => self.lower_stmt_list(&block.stmts)?,
                        BlockStmtOrExpr::Expr(expr) => {
                            let value = self.lower_expr(expr)?;
                            vec![IrStmt::Return {
                                values: vec![value],
                                span,
                            }]
                        }
                    };
                    Ok((lowered_params, body))
                })();
                self.in_function = prev_in_function;
                self.scopes.pop();
                let (params, body) = result?;
                Ok(IrExpr::Function {
                    params,
                    body,
                    span,
                })
            }
            Expr::Member(_) => Err(self.unsupported(expr.span(), "property access (Phase 3)")),
            Expr::Object(_) | Expr::Array(_) => {
                Err(self.unsupported(expr.span(), "object/array literals (Phase 3)"))
            }
            Expr::This(_) => Err(self.unsupported(expr.span(), "`this` (methods are Phase 3)")),
            Expr::Update(_) => Err(self.unsupported(
                expr.span(),
                "update expressions in expression position (statements only)",
            )),
            Expr::Assign(_) => Err(self.unsupported(
                expr.span(),
                "assignment as an expression (assignments are statements in the subset)",
            )),
            other => Err(self.unsupported(other.span(), "this expression form")),
        }
    }
}

/// Parse JS source and lower it to LunaIR (the shared subset).
pub fn lower_js(source: &str) -> Result<IrChunk, CompileError> {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(Lrc::new(FileName::Anon), source.to_string());
    GLOBALS.set(&Default::default(), || {
        let mut parser = Parser::new(
            Syntax::Es(EsSyntax::default()),
            StringInput::from(&*fm),
            None,
        );
        let program = parser.parse_program().map_err(|e| {
            let loc = cm.lookup_char_pos(e.span().lo);
            CompileError::at(
                loc.line as u32,
                loc.col.0 as u32 + 1,
                format!("js parse error: {:?}", e.kind()),
            )
        })?;
        let mut lower = Lower::new(cm.clone());
        if let Some(err) = parser.take_errors().into_iter().next() {
            let loc = cm.lookup_char_pos(err.span().lo);
            return Err(CompileError::at(
                loc.line as u32,
                loc.col.0 as u32 + 1,
                format!("js parse error: {:?}", err.kind()),
            ));
        }
        match &program {
            Program::Script(script) => {
                let body = lower.lower_stmt_list(&script.body)?;
                Ok(IrChunk { body })
            }
            Program::Module(module) => {
                for item in &module.body {
                    if !matches!(item, ModuleItem::Stmt(_)) {
                        return Err(lower.err_at(
                            item.span(),
                            "import/export declarations are outside the subset",
                        ));
                    }
                }
                // Safe: every item is a statement (checked above).
                let stmts: Vec<Stmt> = module
                    .body
                    .iter()
                    .filter_map(|item| match item {
                        ModuleItem::Stmt(stmt) => Some(stmt.clone()),
                        _ => None,
                    })
                    .collect();
                let body = lower.lower_stmt_list(&stmts)?;
                Ok(IrChunk { body })
            }
        }
    })
}
