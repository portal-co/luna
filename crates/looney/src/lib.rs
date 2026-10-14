//! looney: the luna compilers. Two directions, one shared subset (LunaIR):
//!
//! - [`compile_lua_to_js`]: Lua source -> JS text, run against the `lure`
//!   runtime (`packages/lure`).
//! - [`compile_js_to_lua`]: JS source -> Lua text, run against the js.lua
//!   runtime ([`JS_LUA_RUNTIME`]).
//!
//! See docs/SUBSET.md for the language subset and PLAN.md for the roadmap.

pub mod error;
pub mod ir;
pub mod js2lua;
pub mod lua2js;

pub use error::CompileError;

/// The Lua-side JS runtime, embedded for inclusion in compiled output.
pub const JS_LUA_RUNTIME: &str = include_str!("runtime/js.lua");

/// Compile Lua source (subset) to JavaScript text.
///
/// The output is a classic script (top-level `return` allowed, matching Lua
/// chunk semantics) and expects the `lure` runtime's helpers (`luaAdd`, ...)
/// in scope as globals.
pub fn compile_lua_to_js(source: &str) -> Result<String, CompileError> {
    let chunk = lua2js::lower_lua(source)?;
    ir::validate(&chunk)?;
    lua2js::emit::emit_js(&chunk)
}

/// Compile JavaScript source (subset) to Lua text.
///
/// The output is a Lua chunk expecting the [`JS_LUA_RUNTIME`] globals
/// (`jsAdd`, `jsTruthy`, `console`, ...); prepend it or preload it.
pub fn compile_js_to_lua(source: &str) -> Result<String, CompileError> {
    let chunk = js2lua::lower_js(source)?;
    ir::validate(&chunk)?;
    js2lua::emit::emit_lua(&chunk)
}
