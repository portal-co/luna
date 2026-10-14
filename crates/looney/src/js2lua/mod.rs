//! js2lua lowering: SWC AST -> LunaIR.
//!
//! HYGIENE (workspace rule): scope membership is binding identity, so scopes
//! are keyed by `swc_ecma_ast::Id` (`(Atom, SyntaxContext)`), never by a
//! flattened `String`. `.sym` text is only read to *compute a mangled display
//! name* for an already-identified binding, and to recognize the fixed global
//! sentinel names (`undefined`, `NaN`, `Infinity`) — textual domains, which is
//! the rule's stated exception.
//!
//! SCOPE TRACKING (mirrors the lua2js frontend): a reference to a declared
//! local is mangled if the name collides with a Lua keyword or runtime helper;
//! a reference to an undeclared name is a runtime global (`console`, `assert`,
//! ...) and passes through unmangled (except Lua keywords, which must mangle
//! for syntactic validity).

pub mod emit;

use crate::error::CompileError;
use crate::ir::{IrBinOp, IrChunk, IrExpr, IrStmt, IrTarget, IrUnOp, Span};
use std::collections::HashSet;
use swc_common::{sync::Lrc, FileName, SourceMap, Span as SwcSpan, Spanned, GLOBALS};
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
    "console", "jsAdd", "jsAnd", "jsArith", "jsConcat", "jsDiv", "jsGet", "jsInfinity", "jsLe",
    "jsLen", "jsLog", "jsLt", "jsMul", "jsNaN", "jsNeg", "jsNot", "jsOr", "jsPow", "jsRem",
    "jsSet", "jsStrictEq", "jsSub", "jsToNumber", "jsToString", "jsTruthy", "jsTypeof",
    "jsUndefined", "jsGe", "jsGt",
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
    if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
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
}

impl Lower {
    fn new(cm: Lrc<SourceMap>) -> Self {
        Lower {
            cm,
            scopes: vec![HashSet::new()],
        }
    }

    fn is_local(&self, id: &Id) -> bool {
        self.scopes.iter().rev().any(|scope| scope.contains(id))
    }

    /// Resolve a *reference* to a (possibly global) name.
    fn resolve_name(&self, ident: &Ident) -> Result<String, CompileError> {
        let sym = ident.sym.as_str();
        check_portable(sym).map_err(|e| self.err_at(ident.span, e.message))?;
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

    /// Declare a local in the current scope; returns its emitted name.
    fn declare_local(&mut self, ident: &Ident) -> Result<String, CompileError> {
        let sym = ident.sym.as_str();
        check_portable(sym).map_err(|e| self.err_at(ident.span, e.message))?;
        self.scopes
            .last_mut()
            .expect("scope stack is never empty")
            .insert(ident.to_id());
        Ok(if needs_mangle(sym) {
            format!("{sym}_")
        } else {
            sym.to_string()
        })
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

    fn lower_stmt(&mut self, stmt: &Stmt, out: &mut Vec<IrStmt>) -> Result<(), CompileError> {
        match stmt {
            Stmt::Decl(Decl::Var(decl)) => {
                if decl.kind == VarDeclKind::Var {
                    return Err(self.unsupported(decl.span, "`var` (use `let`/`const`)"));
                }
                for declarator in &decl.decls {
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
                    // JS scoping: like Lua, the binding is not visible in its
                    // own initializer (let/const TDZ — close enough for v0).
                    let name = self.declare_local(&binding.id)?;
                    out.push(IrStmt::Local { name, value, span });
                }
                Ok(())
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
                    let span = self.span_of(assign.span);
                    if assign.op != AssignOp::Assign {
                        return Err(self.unsupported(assign.span, "compound assignment (`+=` etc.)"));
                    }
                    let target = match &assign.left {
                        AssignTarget::Simple(SimpleAssignTarget::Ident(binding)) => {
                            IrTarget::Var(self.resolve_name(&binding.id)?)
                        }
                        AssignTarget::Simple(_) => {
                            return Err(self.unsupported(assign.span, "member assignment (Phase 3)"));
                        }
                        AssignTarget::Pat(_) => {
                            return Err(self.unsupported(assign.span, "destructuring assignment"));
                        }
                    };
                    out.push(IrStmt::Assign {
                        target,
                        value: self.lower_expr(&assign.right)?,
                        span,
                    });
                    Ok(())
                }
                _ => Err(self.unsupported(
                    expr_stmt.span,
                    "expression statements other than calls and assignments",
                )),
            },
            Stmt::Block(block) => {
                let span = self.span_of(block.span);
                self.scopes.push(HashSet::new());
                let result = (|| {
                    let mut body = Vec::new();
                    for inner in &block.stmts {
                        self.lower_stmt(inner, &mut body)?;
                    }
                    Ok(body)
                })();
                self.scopes.pop();
                out.push(IrStmt::Do {
                    body: result?,
                    span,
                });
                Ok(())
            }
            Stmt::If(_) => Err(self.unsupported(stmt.span(), "`if` (Phase 2)")),
            Stmt::While(_) | Stmt::DoWhile(_) => {
                Err(self.unsupported(stmt.span(), "loops (Phase 2)"))
            }
            Stmt::For(_) | Stmt::ForOf(_) | Stmt::ForIn(_) => {
                Err(self.unsupported(stmt.span(), "loops (Phase 2)"))
            }
            Stmt::Decl(Decl::Fn(_)) => Err(self.unsupported(stmt.span(), "functions (Phase 2)")),
            Stmt::Return(_) => Err(self.unsupported(
                stmt.span(),
                "top-level `return` (only valid inside functions, Phase 2)",
            )),
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
            Expr::Member(_) => Err(self.unsupported(expr.span(), "property access (Phase 3)")),
            Expr::Fn(_) | Expr::Arrow(_) => {
                Err(self.unsupported(expr.span(), "functions (Phase 2)"))
            }
            Expr::Object(_) | Expr::Array(_) => {
                Err(self.unsupported(expr.span(), "object/array literals (Phase 3)"))
            }
            Expr::Cond(_) => Err(self.unsupported(expr.span(), "ternary expressions (Phase 2)")),
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
                let mut body = Vec::new();
                for stmt in &script.body {
                    lower.lower_stmt(stmt, &mut body)?;
                }
                Ok(IrChunk { body })
            }
            Program::Module(module) => {
                let mut body = Vec::new();
                for item in &module.body {
                    let ModuleItem::Stmt(stmt) = item else {
                        return Err(lower.err_at(
                            item.span(),
                            "import/export declarations are outside the subset",
                        ));
                    };
                    lower.lower_stmt(stmt, &mut body)?;
                }
                Ok(IrChunk { body })
            }
        }
    })
}
