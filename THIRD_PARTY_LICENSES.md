# Third-party licenses

Audit date: 2026-09-05

## Runtime dependencies

| Component | Version or revision | License | Source | Distribution note |
| --- | --- | --- | --- | --- |
| `stack-engine` / `stack-formatter` | `9af727aea79233b8389e0ed6fdbae7d3f388dc29` | Apache-2.0 | <https://github.com/stack-sh/engine> | Linked into the native binary; validates and renders caller-owned provider packs without bundling vendor assets. |
| `stack-compiler` | `84ab5663a7f7c5b7dc0b5e9e2f04c8894ed02820` | Apache-2.0 | <https://github.com/stack-sh/compiler> | Linked directly for protocol-neutral language intelligence and transitively through `stack-engine`; performs no runtime I/O. |
| `stack-theme` | `7e208d6a3c90d255799f390a4e8b86248c73caee` | Apache-2.0 | <https://github.com/stack-sh/theme> | Linked directly and through `stack-engine`; its 30 fallback and 12 explicit core SVGs are Stack-authored Apache-2.0 assets. It also provides the asset-free provider-pack contract and types. |
| `roxmltree` | `0.21.1` | MIT OR Apache-2.0 | <https://github.com/RazrFalcon/roxmltree> | Parses untrusted local SVG into a read-only tree before allowlisted serialization. |
| `sha2`, `digest`, `block-buffer`, `crypto-common`, `hybrid-array`, `const-oid`, `typenum` | `0.11.0`, `0.11.3`, `0.12.1`, `0.2.2`, `0.4.14`, `0.10.2`, `1.20.1` | MIT OR Apache-2.0 | <https://github.com/RustCrypto> | Computes complete archive and per-asset SHA-256 identities. |
| `zip` | `6.0.0` | MIT | <https://github.com/zip-rs/zip2> | Reads audited, allowlisted entries from verified official ZIP archives. |
| `flate2` / `zlib-rs` / `crc32fast` | `1.1.10`, `0.6.7`, `1.5.1` | MIT OR Apache-2.0 / Zlib / MIT OR Apache-2.0 | <https://github.com/rust-lang/flate2-rs>, <https://github.com/trifectatechfoundation/zlib-rs>, <https://github.com/srijs/rust-crc32fast> | Pure Rust DEFLATE decoding and integrity checks for provider ZIPs and release tarballs. |
| `tar` / `filetime` | `0.4.46`, `0.2.29` | MIT OR Apache-2.0 | <https://github.com/composefs/tar-rs>, <https://github.com/alexcrichton/filetime> | Reads the authenticated release archive and validates its exact entry metadata before replacement. |
| `semver` | `1.0.28` | MIT OR Apache-2.0 | <https://github.com/dtolnay/semver> | Parses and orders exact stable and release-candidate update versions. |
| `indexmap` / `hashbrown` / `equivalent` | `2.14.1`, `0.17.1`, `1.0.2` | Apache-2.0 OR MIT | <https://github.com/indexmap-rs/indexmap>, <https://github.com/rust-lang/hashbrown>, <https://github.com/indexmap-rs/equivalent> | ZIP archive entry index. |
| `cfg-if` / `cpufeatures` / `libc` | `1.0.4`, `0.3.1`, `0.2.189` | MIT OR Apache-2.0 | <https://github.com/rust-lang/cfg-if>, <https://github.com/RustCrypto/utils>, <https://github.com/rust-lang/libc> | Target selection and SHA-256 acceleration support. |
| `serde` / `serde_core` | `1.0.229` | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> | Runtime catalog data types through `stack-theme`. |
| `serde_json` | `1.0.151` | MIT OR Apache-2.0 | <https://github.com/serde-rs/json> | Embedded catalog decoding through `stack-theme`. |
| `itoa` | `1.0.18` | MIT OR Apache-2.0 | <https://github.com/dtolnay/itoa> | Transitive runtime dependency of `serde_json`. |
| `memchr` | `2.8.3` | Unlicense OR MIT | <https://github.com/BurntSushi/memchr> | Transitive runtime dependency of `serde_json`. |
| `zmij` | `1.0.23` | MIT | <https://github.com/dtolnay/zmij> | Transitive runtime dependency of `serde_json`. |
| `serde_yaml_ng` / `unsafe-libyaml` / `ryu` | `0.10.0`, `0.2.11`, `1.0.23` | MIT / MIT / Apache-2.0 OR BSL-1.0 | <https://github.com/acatton/serde-yaml-ng>, <https://github.com/dtolnay/unsafe-libyaml>, <https://github.com/dtolnay/ryu> | Parses the bounded user configuration file. |
| `ureq` / `ureq-proto` / `utf8-zero` | `3.4.0`, `0.6.1`, `0.8.1` | MIT OR Apache-2.0 | <https://github.com/algesten/ureq>, <https://github.com/algesten/ureq-proto>, <https://github.com/algesten/utf8-zero> | Downloads audited provider archives over HTTPS with bounded response bodies. |
| `rustls` / `rustls-pki-types` / `rustls-webpki` | `0.23.43`, `1.15.1`, `0.103.15` | Apache-2.0 OR ISC OR MIT / MIT OR Apache-2.0 / ISC | <https://github.com/rustls/rustls>, <https://github.com/rustls/pki-types>, <https://github.com/rustls/webpki> | TLS implementation and certificate validation for provider archive downloads. |
| `ring` / `untrusted` | `0.17.14`, `0.9.0` | Apache-2.0 AND ISC / ISC | <https://github.com/briansmith/ring>, <https://github.com/briansmith/untrusted> | Cryptography and bounded certificate parsing through `rustls`. |
| `webpki-roots` | `1.0.9` | CDLA-Permissive-2.0 | <https://github.com/rustls/webpki-roots> | Mozilla-derived trust anchors used by the Rustls connector. |
| `base64` / `percent-encoding` / `http` / `bytes` / `httparse` | `0.23.1`, `2.3.2`, `1.5.0`, `1.12.1`, `1.10.1` | MIT OR Apache-2.0 / MIT OR Apache-2.0 / MIT OR Apache-2.0 / MIT / MIT OR Apache-2.0 | <https://github.com/marshallpierce/rust-base64>, <https://github.com/servo/rust-url>, <https://github.com/hyperium/http>, <https://github.com/tokio-rs/bytes>, <https://github.com/seanmonstar/httparse> | HTTP request, response, and URL representation through `ureq`. |
| `log` / `once_cell` | `0.4.34`, `1.21.4` | MIT OR Apache-2.0 | <https://github.com/rust-lang/log>, <https://github.com/matklad/once_cell> | Runtime support through the HTTP and TLS stack. |
| `getrandom` / `subtle` / `zeroize` | `0.2.17`, `2.6.1`, `1.9.0` | MIT OR Apache-2.0 / BSD-3-Clause / Apache-2.0 OR MIT | <https://github.com/rust-random/getrandom>, <https://github.com/dalek-cryptography/subtle>, <https://github.com/RustCrypto/utils> | Randomness and cryptographic value handling through `ring` and `rustls`. |
| `wasi` | `0.11.1+wasi-snapshot-preview1` | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | <https://github.com/rust-lang/wasi> | Target-specific randomness bindings. |
| `windows-sys`, `windows-targets`, and target architecture crates | `0.52.0`, `0.52.6` | MIT OR Apache-2.0 | <https://github.com/microsoft/windows-rs> | Target-specific Windows runtime bindings. |

## Build-only dependencies

| Component | Version | License | Source | Distribution note |
| --- | --- | --- | --- | --- |
| `serde_derive` | `1.0.229` | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> | Procedural macro; not linked into the release binary. |
| `proc-macro2` | `1.0.107` | MIT OR Apache-2.0 | <https://github.com/dtolnay/proc-macro2> | Procedural-macro build dependency; not linked into the release binary. |
| `quote` | `1.0.47` | MIT OR Apache-2.0 | <https://github.com/dtolnay/quote> | Procedural-macro build dependency; not linked into the release binary. |
| `syn` | `3.0.4` | MIT OR Apache-2.0 | <https://github.com/dtolnay/syn> | Procedural-macro build dependency; not linked into the release binary. |
| `unicode-ident` | `1.0.24` | (MIT OR Apache-2.0) AND Unicode-3.0 | <https://github.com/dtolnay/unicode-ident> | Procedural-macro build dependency; not linked into the release binary. |
| `cc` / `find-msvc-tools` / `shlex` | `1.4.5`, `0.1.12`, `2.0.1` | MIT OR Apache-2.0 | <https://github.com/rust-lang/cc-rs> | Build dependencies of `ring`; not linked into the release binary. |

No third-party vendor icon is bundled in the repository or binary. Provider-specific assets require a separate rights record covering source revision, copyright, trademark restrictions, modification, software redistribution, commercial diagram output, and required notices.

## Binary distribution requirements

A future binary archive must include:

- this repository's `LICENSE` and `NOTICE`;
- this inventory at the dependency versions resolved in that release's `Cargo.lock`;
- the Apache-2.0 text for Stack dependencies and dependencies distributed under the Apache-2.0 option;
- the complete MIT notices selected for `memchr`, `zmij`, `zip`, and any dependency distributed under the MIT option;
- the Zlib notice for `zlib-rs`;
- the ISC, BSD-3-Clause, CDLA-Permissive-2.0, BSL-1.0, and other applicable notices selected for the resolved networking and TLS dependencies;
- any additional license text or attribution introduced by a later runtime dependency or provider pack.

Build-only dependencies do not require inclusion in a binary archive when none of their source or object code is distributed, but they remain listed here so the audited build graph is reproducible.
