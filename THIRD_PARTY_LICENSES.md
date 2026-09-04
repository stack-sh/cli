# Third-party licenses

Audit date: 2026-09-04

## Runtime dependencies

| Component | Version or revision | License | Source | Distribution note |
| --- | --- | --- | --- | --- |
| `stack-engine` / `stack-formatter` | `e1240661b6d8cebd95ef24207618a62fadb15b48` | Apache-2.0 | <https://github.com/stack-sh/engine> | Linked into the native binary; includes the repository-authored core icon catalog through `stack-theme`. |
| `stack-compiler` | `3d2379483da1edaeb24a26d43743587a4f5bd645` | Apache-2.0 | <https://github.com/stack-sh/compiler> | Linked transitively through `stack-engine`. |
| `stack-theme` | `d25b883884420adcc124e4c9c786ad92925eae60` | Apache-2.0 | <https://github.com/stack-sh/theme> | Linked transitively through `stack-engine`; its 30 fallback and 12 explicit core SVGs are Stack-authored Apache-2.0 assets. |
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
- the complete MIT notices selected for `memchr`, `zmij`, and any dependency distributed under the MIT option;
- any additional license text or attribution introduced by a later runtime dependency or provider pack.

Build-only dependencies do not require inclusion in a binary archive when none of their source or object code is distributed, but they remain listed here so the audited build graph is reproducible.
