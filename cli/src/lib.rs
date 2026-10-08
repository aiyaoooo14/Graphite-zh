//! Library half of `graphite-cli`: the headless editor session used by the binary and by
//! integration tests.
//!
//! v2 形态（CLI 接口方案 §〇）：Rust 只做 API 引擎 —— 四原语 `inspect` / `exec` / `save` /
//! `export`；原子命令组合在 Python 层（tools/graphite_draw/）。
pub mod exec;
pub mod export;
pub mod inspect;
pub mod node_list;
pub mod session;
