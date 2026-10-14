//! IR shape contract: one sample per IR node variant, validated and emitted
//! in both directions. NO wildcard match arms over IR enums here — adding a
//! variant breaks this test's compilation until validator and both emitters
//! handle it (that enforcement is the point).

mod harness;

use looney::ir::{IrBinOp, IrChunk, IrExpr, IrStmt, IrTarget, IrUnOp, Span, validate};
use looney::{js2lua, lua2js};

fn sample_expr(expr: &IrExpr) -> IrExpr {
    expr.clone()
}

fn expr_samples() -> Vec<IrExpr> {
    let span = Span::DUMMY;
    let atoms = vec![
        IrExpr::Nil,
        IrExpr::Bool(true),
        IrExpr::Num(42.5),
        IrExpr::Str("hello".into()),
        IrExpr::Var("x".into()),
    ];
    let mut out = atoms.clone();
    for op in [IrUnOp::Neg, IrUnOp::Not, IrUnOp::Len] {
        for a in &atoms {
            out.push(IrExpr::Un {
                op,
                expr: Box::new(sample_expr(a)),
                span,
            });
        }
    }
    for op in [
        IrBinOp::Add,
        IrBinOp::Sub,
        IrBinOp::Mul,
        IrBinOp::Div,
        IrBinOp::Mod,
        IrBinOp::Pow,
        IrBinOp::Concat,
        IrBinOp::Eq,
        IrBinOp::Ne,
        IrBinOp::Lt,
        IrBinOp::Le,
        IrBinOp::Gt,
        IrBinOp::Ge,
        IrBinOp::And,
        IrBinOp::Or,
    ] {
        out.push(IrExpr::Bin {
            op,
            lhs: Box::new(IrExpr::Var("a".into())),
            rhs: Box::new(IrExpr::Var("b".into())),
            span,
        });
    }
    out.push(IrExpr::Call {
        func: Box::new(IrExpr::Var("f".into())),
        args: atoms,
        span,
    });
    out.push(IrExpr::Cond {
        cond: Box::new(IrExpr::Var("c".into())),
        then: Box::new(IrExpr::Num(1.0)),
        else_: Box::new(IrExpr::Num(2.0)),
        span,
    });
    out.push(IrExpr::Function {
        params: vec!["p".into()],
        body: vec![IrStmt::Return {
            values: vec![IrExpr::Var("p".into())],
            span,
        }],
        span,
    });
    out
}

fn stmt_samples() -> Vec<IrStmt> {
    let span = Span::DUMMY;
    vec![
        IrStmt::Local {
            name: "x".into(),
            value: None,
            span,
        },
        IrStmt::Local {
            name: "y".into(),
            value: Some(IrExpr::Num(1.0)),
            span,
        },
        IrStmt::Assign {
            target: IrTarget::Var("x".into()),
            value: IrExpr::Bool(false),
            span,
        },
        IrStmt::Expr {
            expr: IrExpr::Call {
                func: Box::new(IrExpr::Var("f".into())),
                args: vec![],
                span,
            },
            span,
        },
        IrStmt::Do {
            body: vec![IrStmt::Assign {
                target: IrTarget::Var("x".into()),
                value: IrExpr::Nil,
                span,
            }],
            span,
        },
        IrStmt::Return {
            values: vec![],
            span,
        },
        IrStmt::Return {
            values: vec![IrExpr::Var("x".into())],
            span,
        },
        IrStmt::If {
            arms: vec![
                (
                    IrExpr::Bool(true),
                    vec![IrStmt::Assign {
                        target: IrTarget::Var("x".into()),
                        value: IrExpr::Num(1.0),
                        span,
                    }],
                ),
                (IrExpr::Bool(false), vec![]),
            ],
            else_: Some(vec![IrStmt::Break { span }]),
            span,
        },
        IrStmt::While {
            cond: IrExpr::Var("x".into()),
            body: vec![IrStmt::Break { span }],
            span,
        },
        IrStmt::NumericFor {
            var: "i".into(),
            start: IrExpr::Num(1.0),
            stop: IrExpr::Num(3.0),
            step: Some(IrExpr::Num(1.0)),
            body: vec![IrStmt::Expr {
                expr: IrExpr::Call {
                    func: Box::new(IrExpr::Var("f".into())),
                    args: vec![IrExpr::Var("i".into())],
                    span,
                },
                span,
            }],
            span,
        },
        IrStmt::NumericFor {
            var: "j".into(),
            start: IrExpr::Num(1.0),
            stop: IrExpr::Num(3.0),
            step: None,
            body: vec![],
            span,
        },
        IrStmt::Break { span },
        IrStmt::Local {
            name: "rec".into(),
            value: Some(IrExpr::Function {
                params: vec!["n".into()],
                body: vec![IrStmt::Return {
                    values: vec![IrExpr::Call {
                        func: Box::new(IrExpr::Var("rec".into())),
                        args: vec![IrExpr::Var("n".into())],
                        span,
                    }],
                    span,
                }],
                span,
            }),
            span,
        },
    ]
}

#[test]
fn every_node_validates_and_emits_both_directions() {
    for expr in expr_samples() {
        let chunk = IrChunk {
            body: vec![IrStmt::Return {
                values: vec![expr.clone()],
                span: Span::DUMMY,
            }],
        };
        validate(&chunk).unwrap_or_else(|e| panic!("validate failed for {expr:?}: {e}"));
        let js = lua2js::emit::emit_js(&chunk).expect("js emit failed");
        let lua = js2lua::emit::emit_lua(&chunk).expect("lua emit failed");
        assert!(!js.trim().is_empty(), "empty js for {expr:?}");
        assert!(!lua.trim().is_empty(), "empty lua for {expr:?}");
    }
    for stmt in stmt_samples() {
        let chunk = IrChunk {
            body: vec![stmt.clone()],
        };
        validate(&chunk).unwrap_or_else(|e| panic!("validate failed for {stmt:?}: {e}"));
        let js = lua2js::emit::emit_js(&chunk).expect("js emit failed");
        let lua = js2lua::emit::emit_lua(&chunk).expect("lua emit failed");
        assert!(!js.trim().is_empty(), "empty js for {stmt:?}");
        assert!(!lua.trim().is_empty(), "empty lua for {stmt:?}");
    }
}

#[test]
fn validator_rejects_non_call_expr_stmt() {
    let chunk = IrChunk {
        body: vec![IrStmt::Expr {
            expr: IrExpr::Num(1.0),
            span: Span::DUMMY,
        }],
    };
    assert!(validate(&chunk).is_err());
}

#[test]
fn validator_rejects_multi_return_in_phase_1() {
    let chunk = IrChunk {
        body: vec![IrStmt::Return {
            values: vec![IrExpr::Num(1.0), IrExpr::Num(2.0)],
            span: Span::DUMMY,
        }],
    };
    assert!(validate(&chunk).is_err());
}

#[test]
fn validator_rejects_bad_identifiers() {
    let chunk = IrChunk {
        body: vec![IrStmt::Local {
            name: "not valid".into(),
            value: None,
            span: Span::DUMMY,
        }],
    };
    assert!(validate(&chunk).is_err());
}
