//! Shared test harness: execute Lua (in-process via vendored mlua 5.4) and
//! JavaScript (node subprocess, with native TS type-stripping), and compare
//! outputs. Golden snapshots regenerate with `UPDATE_GOLDEN=1 cargo test`.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Absolute path inside the looney crate (tests run with CWD = crate dir, but
/// be explicit so the harness is robust under IDEs).
pub fn manifest_rel(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Absolute path inside the workspace root.
pub fn workspace_rel(rel: &str) -> PathBuf {
    manifest_rel("../../").join(rel)
}

// ---------------------------------------------------------------------------
// Lua execution (mlua, vendored 5.4)
// ---------------------------------------------------------------------------

/// Run a Lua chunk, capturing `print` output (tab-joined lines, as stock Lua).
pub fn run_lua(source: &str) -> Result<String, String> {
    let lua = mlua::Lua::new();
    let buf = Rc::new(RefCell::new(String::new()));
    let sink = Rc::clone(&buf);
    let print = lua
        .create_function(move |_, args: mlua::Variadic<mlua::Value>| {
            let mut parts = Vec::new();
            for arg in args {
                let part = match arg {
                    mlua::Value::Nil => "nil".to_string(),
                    mlua::Value::Boolean(b) => (if b { "true" } else { "false" }).to_string(),
                    mlua::Value::Integer(i) => i.to_string(),
                    mlua::Value::Number(n) => format!("{n}"),
                    mlua::Value::String(s) => s.to_str().map(|s| s.to_string()).unwrap_or_default(),
                    other => format!("{}: <ref>", other.type_name()),
                };
                parts.push(part);
            }
            let mut out = sink.borrow_mut();
            out.push_str(&parts.join("\t"));
            out.push('\n');
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    lua.globals().set("print", print).map_err(|e| e.to_string())?;
    lua.load(source).set_name("case").exec().map_err(|e| e.to_string())?;
    let out = buf.borrow().clone();
    Ok(out)
}

/// Run a js2lua-compiled chunk with the js.lua runtime preloaded.
pub fn run_compiled_lua(emitted: &str) -> Result<String, String> {
    let source = format!("{}\n{}", looney::JS_LUA_RUNTIME, emitted);
    run_lua(&source)
}

// ---------------------------------------------------------------------------
// JS execution (node, native type stripping for the TS runtime)
// ---------------------------------------------------------------------------

/// Prelude for lua2js-compiled code: load the lure runtime and expose its
/// helpers (plus the stdlib subset) as globals.
const LURE_PRELUDE: &str = r#"import * as L from './lure_runtime.ts';
Object.assign(globalThis, L);
globalThis.assert = L.luaAssert;
"#;

/// Prelude for reference runs of JS corpus cases: minimal `assert` shim
/// (node has no global assert), leaving everything else stock.
const JS_REFERENCE_PRELUDE: &str = r#"globalThis.assert = (v, msg) => {
  if (!v) throw new Error(msg ?? 'assertion failed!');
  return v;
};
"#;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn run_node(files: &[(&str, &str)]) -> Result<String, String> {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("looney-test-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, contents) in files {
        fs::write(dir.join(name), contents).map_err(|e| e.to_string())?;
    }
    let output = Command::new("node")
        .arg("--disable-warning=ExperimentalWarning")
        .arg("main.ts")
        .current_dir(&dir)
        .output()
        .map_err(|e| format!("failed to spawn node: {e}"))?;
    let _ = fs::remove_dir_all(&dir);
    if !output.status.success() {
        return Err(format!(
            "node exited with {}:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn lure_runtime_source() -> String {
    let path = workspace_rel("packages/lure/index.ts");
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read lure runtime at {path:?}: {e}"))
}

/// Run lua2js-compiled code against the lure runtime. The chunk is wrapped in
/// an arrow IIFE so that chunk-level `return` (legal in our classic-script
/// output) is also legal inside the ESM harness module.
pub fn run_compiled_js(emitted: &str) -> Result<String, String> {
    let main = format!("{LURE_PRELUDE}\nvoid (() => {{\n{emitted}\n}})();\n");
    run_node(&[
        ("lure_runtime.ts", &lure_runtime_source()),
        ("main.ts", &main),
    ])
}

/// Run a JS corpus case under stock node (reference semantics).
pub fn run_reference_js(source: &str) -> Result<String, String> {
    let main = format!("{JS_REFERENCE_PRELUDE}\n{source}");
    run_node(&[("main.ts", &main)])
}

// ---------------------------------------------------------------------------
// Corpus helpers
// ---------------------------------------------------------------------------

/// List corpus cases: `(name, source_path)` pairs, skipping `_`-prefixed
/// files (scratch). A sibling `<name>.expected` file switches the case to
/// expected-output mode (no reference run — used where canonical formatting
/// intentionally differs from stock reference behavior).
pub fn corpus_cases(dir: &str) -> Vec<(String, PathBuf, Option<String>)> {
    let dir_path = manifest_rel(dir);
    let mut cases = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&dir_path)
        .unwrap_or_else(|e| panic!("cannot read corpus dir {dir_path:?}: {e}"))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        if name.starts_with('_') || name.ends_with(".expected") {
            continue;
        }
        let expected = path.with_extension("expected");
        let expected = if expected.exists() {
            Some(fs::read_to_string(expected).unwrap())
        } else {
            None
        };
        cases.push((name, path, expected));
    }
    assert!(!cases.is_empty(), "no corpus cases found in {dir_path:?}");
    cases
}

pub fn assert_output_eq(context: &str, actual: &str, expected: &str) {
    assert_eq!(
        actual, expected,
        "{context}: output mismatch\n--- expected ---\n{expected}\n--- actual ---\n{actual}"
    );
}

// ---------------------------------------------------------------------------
// Golden snapshots
// ---------------------------------------------------------------------------

pub fn check_golden(name: &str, actual: &str) {
    let path = manifest_rel(&format!("tests/golden/{name}"));
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("missing golden {path:?}; regenerate with UPDATE_GOLDEN=1 cargo test")
    });
    assert_eq!(
        actual, &expected,
        "golden mismatch for {name} (regenerate with UPDATE_GOLDEN=1)"
    );
}
