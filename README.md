# ikigai-text

Unix-like **text endpoints** for [ikigai](https://github.com/ikigai-rs) — a
module crate of `urn:text:*` resources that turn the REPL into a coreutils
analog.

The engine already speaks Unix: `|` pipes one resource's representation into the
next, and `..` maps an endpoint over a newline-separated list. These endpoints
are the coreutils to match:

```text
source urn:file:log.txt | urn:text:grep pattern=ERROR | urn:text:wc
```

Like `ikigai-fn`, this is a standalone **module crate**: a host links it in and
mounts [`space`], rather than the engine shipping the behaviour itself. It
depends only on the published `ikigai-core` kernel, uses no OS or platform APIs,
and compiles to `wasm32-unknown-unknown` — so the same module links into a
native CLI and an in-browser WebAssembly host alike.

## Endpoints

Every endpoint is a **single-verb `Source`**, reads its piped input from the
`in` argument, produces `text/plain; charset=utf-8`, and — being a pure function
of its inputs — is **cacheable**. Line tools split the input into lines the Unix
way (`str::lines`: a trailing newline and `\r\n` are handled), and join their
output with `\n` and no trailing newline, so a result flows straight into the
next stage or a `..` map.

| IRI | args | behaviour |
|-----|------|-----------|
| `urn:text:wc` | `count=` `lines`\|`words`\|`bytes` (default `lines`) | the single count, as a number |
| `urn:text:head` | `n=` integer (default `10`) | the first `n` lines |
| `urn:text:tail` | `n=` integer (default `10`) | the last `n` lines |
| `urn:text:grep` | `pattern=` (required); `i=` bool; `v=` bool | lines containing `pattern` (literal substring, **not** regex); `i` = case-insensitive, `v` = invert |
| `urn:text:sort` | `r=` bool; `n=` bool; `u=` bool | sorted lines; `r` reverse, `n` numeric (non-numeric → 0), `u` unique (global) |
| `urn:text:uniq` | `c=` bool | collapse **adjacent** duplicate lines; `c` prefixes run counts |
| `urn:text:nl` | — | number every line |
| `urn:text:rev` | — | reverse each line's characters |

Booleans take `true`/`false`; an absent flag is `false`, and any other value is
an error (a mistyped flag never silently reads as off). `uniq` collapses only
*adjacent* duplicates (Unix semantics) — pipe through `sort` first for a global
dedup.

Each endpoint authors a full self-description — an XSD `class` on every input,
`one_of` for the enums, `default` where applicable — so that each projects to a
well-typed, MCP-legible tool once the action manifold is projected.

## Conformance

The module **passes
[`ikigai-conformance`](https://github.com/ikigai-rs/ikigai-conformance)** with
no opt-outs: `tests/conformance.rs` walks every `urn:text:*` endpoint and runs
every check (ArgSpec completeness, declared = enforced, cacheability, pipeline
citizenship, naming). Every endpoint is declared `pure` there — it reads nothing
but its inline arguments, so its cacheable result rightly has no golden thread —
and `cacheable`, so a future dependency that silently downgraded the effective
expiry would fail the test rather than slow every read.

## Usage

Mount the whole library at its conventional `urn:text:*` IRIs and chain your own
bindings on top (`EndpointSpace::bind` is a builder):

```rust
use ikigai_core::{Exact, Kernel};
use std::sync::Arc;

let space = ikigai_text::space()
    .bind(Exact::new("urn:file:log.txt"), my_file_endpoint());
let kernel = Kernel::new(Arc::new(space));
```

Or pull the individual constructors (`ikigai_text::grep()`, …) and bind them at
IRIs of your choosing — binding authority is a host concern.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at
your option.
