//! js2lua API surface: golden snapshots and rejection of out-of-subset input.

mod harness;

#[test]
fn golden_arith() {
    let source = std::fs::read_to_string(harness::manifest_rel("tests/cases_j2l/arith.js")).unwrap();
    let emitted = looney::compile_js_to_lua(&source).unwrap();
    harness::check_golden("j2l_arith.lua.snap", &emitted);
}

#[test]
fn golden_keeps_runtime_separate() {
    // The compiler output is just the chunk; the runtime ships separately.
    let emitted = looney::compile_js_to_lua("console.log(1);").unwrap();
    assert!(!emitted.contains("jsTruthy"));
    assert!(looney::JS_LUA_RUNTIME.contains("function jsTruthy"));
}

#[test]
fn rejects_out_of_subset_constructs_with_positioned_errors() {
    let cases: &[&str] = &[
        "let a = 1 == 1;",      // loose equality
        "let a = 1 != 2;",      // loose inequality
        "x.y;",                 // property access: Phase 3
        "let o = {};",          // object literals: Phase 3
        "let a = [];",          // array literals: Phase 3
        "function f() {}",      // functions: Phase 2
        "let f = () => 1;",     // arrows: Phase 2
        "if (true) {}",         // if: Phase 2
        "while (true) {}",      // while: Phase 2
        "for (;;) {}",          // for: Phase 2
        "let a = 1 ? 2 : 3;",   // ternary: Phase 2
        "return 1;",            // top-level return: Phase 2 (functions only)
        "let a = x ?? y;",      // nullish coalescing
        "let a = `t`;",         // template literals
        "let a$b = 1;",         // `$` in identifiers
        "import x from 'y';",   // modules
        "a += 1;",              // compound assignment
        "a++;",                 // update expressions
        "let {a} = b;",         // destructuring
    ];
    for source in cases {
        match looney::compile_js_to_lua(source) {
            Ok(emitted) => panic!("expected `{source}` to be rejected, got:\n{emitted}"),
            Err(e) => {
                assert!(
                    e.line.is_some(),
                    "error for `{source}` lacks source position: {e}"
                );
            }
        }
    }
}

#[test]
fn rejects_parse_errors() {
    assert!(looney::compile_js_to_lua("let = ;").is_err());
}
