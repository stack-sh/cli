# Repository Guide

## Language

Write repository content, code comments, commit messages, issues, and pull requests in English even though this repository is private.

## Architecture

- Keep this repository focused on the native `stack` command, host I/O, exit codes, configuration discovery, provider-pack import, and notice output.
- Link `stack-engine` as a native Rust dependency; do not duplicate compiler, formatter, layout, or SVG-rendering logic.
- Keep provider-pack import, local cache behavior, provenance display, and notice output at the CLI boundary. Do not make the engine read the filesystem or network.
- Do not add authentication, billing, entitlement, or proprietary-theme delivery behavior; they are outside the product roadmap.

## Licensing and security

- Repository-authored work is Apache-2.0. Record every distributed dependency and asset in `THIRD_PARTY_LICENSES.md` before release.
- Do not bundle vendor icons unless their exact terms permit the relevant source, binary, and generated-output distribution channels. Preserve upstream artwork, provenance, and notices.
- Never commit credentials, tokens, private keys, signing material, customer data, or local auth state.
- Report security vulnerabilities privately through the process in `SECURITY.md`.

## Delivery

- Use a topic branch and pull request; squash merge after approval.
- Work in small increments and add repository-specific formatting, linting, tests, and release checks with the code that needs them.
