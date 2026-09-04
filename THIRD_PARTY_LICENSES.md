# Third-party licenses

Audit date: 2026-09-04

## Runtime dependencies

| Component | Version or revision | License | Source | Distribution note |
| --- | --- | --- | --- | --- |
| `stack-engine` / `stack-formatter` | `8b62b0ef77c12b1b88981ea569379d3d9737a824` | Apache-2.0 | <https://github.com/stack-sh/engine> | Linked into the native binary; validates and renders caller-owned provider packs without bundling vendor assets. |
| `stack-compiler` | `4a18fac42afc2256a1bb3a6ff13d12d732a391e7` | Apache-2.0 | <https://github.com/stack-sh/compiler> | Linked transitively through `stack-engine`; preserves namespaced provider icon identifiers in normalized IR. |
| `stack-theme` | `5dbe41326370260cfc6b72d4aab4470318d66dab` | Apache-2.0 | <https://github.com/stack-sh/theme> | Linked directly and through `stack-engine`; its 30 fallback and 12 explicit core SVGs are Stack-authored Apache-2.0 assets. It also provides the asset-free provider-pack contract and types. |
| `roxmltree` | `0.21.1` | MIT OR Apache-2.0 | <https://github.com/RazrFalcon/roxmltree> | Parses untrusted local SVG into a read-only tree before allowlisted serialization. |
| `sha2`, `digest`, `block-buffer`, `crypto-common`, `hybrid-array`, `const-oid`, `typenum` | `0.11.0`, `0.11.3`, `0.12.1`, `0.2.2`, `0.4.14`, `0.10.2`, `1.20.1` | MIT OR Apache-2.0 | <https://github.com/RustCrypto> | Computes complete archive and per-asset SHA-256 identities. |
| `zip` | `6.0.0` | MIT | <https://github.com/zip-rs/zip2> | Reads only audited, allowlisted entries from user-selected local ZIP archives. |
| `flate2` / `zlib-rs` / `crc32fast` | `1.1.10`, `0.6.7`, `1.5.1` | MIT OR Apache-2.0 / Zlib / MIT OR Apache-2.0 | <https://github.com/rust-lang/flate2-rs>, <https://github.com/trifectatechfoundation/zlib-rs>, <https://github.com/srijs/rust-crc32fast> | Pure Rust DEFLATE decoding and integrity checks for ZIP entries. |
| `indexmap` / `hashbrown` / `equivalent` | `2.14.1`, `0.17.1`, `1.0.2` | Apache-2.0 OR MIT | <https://github.com/indexmap-rs/indexmap>, <https://github.com/rust-lang/hashbrown>, <https://github.com/indexmap-rs/equivalent> | ZIP archive entry index. |
| `cfg-if` / `cpufeatures` / `libc` | `1.0.4`, `0.3.1`, `0.2.189` | MIT OR Apache-2.0 | <https://github.com/rust-lang/cfg-if>, <https://github.com/RustCrypto/utils>, <https://github.com/rust-lang/libc> | Target selection and SHA-256 acceleration support. |
| `serde` / `serde_core` | `1.0.229` | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> | Runtime catalog data types through `stack-theme`. |
| `serde_json` | `1.0.151` | MIT OR Apache-2.0 | <https://github.com/serde-rs/json> | Embedded catalog decoding through `stack-theme`. |
| `itoa` | `1.0.18` | MIT OR Apache-2.0 | <https://github.com/dtolnay/itoa> | Transitive runtime dependency of `serde_json`. |
| `memchr` | `2.8.3` | Unlicense OR MIT | <https://github.com/BurntSushi/memchr> | Transitive runtime dependency of `serde_json`. |
| `zmij` | `1.0.23` | MIT | <https://github.com/dtolnay/zmij> | Transitive runtime dependency of `serde_json`. |

## Build-only dependencies

| Component | Version | License | Source | Distribution note |
| --- | --- | --- | --- | --- |
| `serde_derive` | `1.0.229` | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> | Procedural macro; not linked into the release binary. |
| `proc-macro2` | `1.0.107` | MIT OR Apache-2.0 | <https://github.com/dtolnay/proc-macro2> | Procedural-macro build dependency; not linked into the release binary. |
| `quote` | `1.0.47` | MIT OR Apache-2.0 | <https://github.com/dtolnay/quote> | Procedural-macro build dependency; not linked into the release binary. |
| `syn` | `3.0.4` | MIT OR Apache-2.0 | <https://github.com/dtolnay/syn> | Procedural-macro build dependency; not linked into the release binary. |
| `unicode-ident` | `1.0.24` | (MIT OR Apache-2.0) AND Unicode-3.0 | <https://github.com/dtolnay/unicode-ident> | Procedural-macro build dependency; not linked into the release binary. |

No third-party vendor icon is bundled in the repository or binary. Provider-specific assets require a separate rights record covering source revision, copyright, trademark restrictions, modification, software redistribution, commercial diagram output, and required notices.

## Binary distribution requirements

A future binary archive must include:

- this repository's `LICENSE` and `NOTICE`;
- this inventory at the dependency versions resolved in that release's `Cargo.lock`;
- the Apache-2.0 text for Stack dependencies and dependencies distributed under the Apache-2.0 option;
- the complete MIT notices selected for `memchr`, `zmij`, `zip`, and any dependency distributed under the MIT option;
- the Zlib notice for `zlib-rs`;
- any additional license text or attribution introduced by a later runtime dependency or provider pack.

Build-only dependencies do not require inclusion in a binary archive when none of their source or object code is distributed, but they remain listed here so the audited build graph is reproducible.
