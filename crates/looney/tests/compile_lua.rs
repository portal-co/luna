//! lua2js API surface: golden snapshots and rejection of out-of-subset input.

mod harness;

#[test]
fn golden_arith() {
    let source =
        std::fs::read_to_string(harness::manifest_rel("tests/cases_l2j/arith.lua")).unwrap();
    let emitted = looney::compile_lua_to_js(&source).unwrap();
    harness::check_golden("l2j_arith.js.snap", &emitted);
}

#[test]
fn golden_control() {
    let source =
        std::fs::read_to_string(harness::manifest_rel("tests/cases_l2j/control.lua")).unwrap();
    let emitted = looney::compile_lua_to_js(&source).unwrap();
    harness::check_golden("l2j_control.js.snap", &emitted);
}

#[test]
fn golden_functions() {
    let source =
        std::fs::read_to_string(harness::manifest_rel("tests/cases_l2j/functions.lua")).unwrap();
    let emitted = looney::compile_lua_to_js(&source).unwrap();
    harness::check_golden("l2j_functions.js.snap", &emitted);
}

#[test]
fn rejects_out_of_subset_constructs_with_positioned_errors() {
    let cases: &[&str] = &[
        "x.y = 1",                     // index assignment: Phase 3
        "local a, b = 1, 2",           // multiple assignment: Phase 4
        "function a.b.f() end",        // dotted function names: Phase 3
        "function a:m() end",          // method definitions: Phase 3
        "for k in pairs({}) do end",   // generic for: Phase 5
        "repeat until true",           // repeat: out of subset v0
        "local t = {}",                // tables: Phase 3
        "print(...)",                  // varargs: Phase 4
        "local x = a % 2 goto done",   // goto: out of subset v0
        "local f = function(...) end", // vararg params: Phase 4
    ];
    for source in cases {
        match looney::compile_lua_to_js(source) {
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
    assert!(looney::compile_lua_to_js("local = = 1").is_err());
}
