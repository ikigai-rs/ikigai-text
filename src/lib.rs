//! `ikigai-text` — Unix-like text endpoints as pure pipeline citizens.
//!
//! A standalone **ikigai module crate** (like `ikigai-fn`): a host links it in
//! and mounts [`space`], rather than the engine shipping the behaviour itself.
//! It depends only on the published `ikigai-core` kernel, uses no OS or platform
//! APIs, and compiles to `wasm32-unknown-unknown` — so the same module links
//! into a native CLI and an in-browser WebAssembly host alike.
//!
//! Scaffold only, for now: [`space`] mounts nothing yet. The `urn:text:*`
//! endpoints (`wc`, `head`, `tail`, `grep`, `sort`, `uniq`, `nl`, `rev`) land in
//! the first feature PR.

use ikigai_core::EndpointSpace;

/// The text-endpoint library as a mountable [`EndpointSpace`]. Empty for now;
/// the `urn:text:*` bindings land in the first feature PR.
pub fn space() -> EndpointSpace {
    EndpointSpace::new()
}
