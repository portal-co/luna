//! Behavioral matrix (PLAN.md §7): every corpus case runs compiled AND against
//! the stock reference implementation of its source language; stdout must
//! match. Cases with a `.expected` sibling skip the reference (documented
//! divergences, e.g. canonical number formatting) and check both compiled
//! outputs against the checked-in expectation instead.

mod harness;

use std::fs;

#[test]
fn lua_corpus_compiled_matches_reference() {
    for (name, path, expected) in harness::corpus_cases("tests/cases_l2j") {
        let source = fs::read_to_string(&path).unwrap();
        let emitted = looney::compile_lua_to_js(&source)
            .unwrap_or_else(|e| panic!("{name}: compile failed: {e}"));
        let actual = harness::run_compiled_js(&emitted).unwrap_or_else(|e| {
            panic!("{name}: compiled run failed: {e}\n--- emitted ---\n{emitted}")
        });
        match expected {
            Some(expected) => {
                harness::assert_output_eq(&format!("{name} (expected)"), &actual, &expected)
            }
            None => {
                let reference = harness::run_lua(&source)
                    .unwrap_or_else(|e| panic!("{name}: reference lua run failed: {e}"));
                harness::assert_output_eq(&format!("{name} (vs reference)"), &actual, &reference);
            }
        }
    }
}

#[test]
fn js_corpus_compiled_matches_reference() {
    for (name, path, expected) in harness::corpus_cases("tests/cases_j2l") {
        let source = fs::read_to_string(&path).unwrap();
        let emitted = looney::compile_js_to_lua(&source)
            .unwrap_or_else(|e| panic!("{name}: compile failed: {e}"));
        let actual = harness::run_compiled_lua(&emitted).unwrap_or_else(|e| {
            panic!("{name}: compiled run failed: {e}\n--- emitted ---\n{emitted}")
        });
        match expected {
            Some(expected) => {
                harness::assert_output_eq(&format!("{name} (expected)"), &actual, &expected)
            }
            None => {
                let reference = harness::run_reference_js(&source)
                    .unwrap_or_else(|e| panic!("{name}: reference js run failed: {e}"));
                harness::assert_output_eq(&format!("{name} (vs reference)"), &actual, &reference);
            }
        }
    }
}
