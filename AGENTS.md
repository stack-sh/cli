# Repository Guide

## Language

Write repository content, code comments, commit messages, issues, and pull requests in English even though this repository is private.

## Architecture

- Keep this repository focused on the native `stack` command, host I/O, exit codes, configuration discovery, and future authenticated theme delivery.
- Link `stack-engine` as a native Rust dependency; do not duplicate compiler, formatter, layout, or SVG-rendering logic.
- Keep user authentication, token issuance, billing, and entitlement authority in the future web service. The CLI may only act as a scoped client.
- Do not add auth or entitlement behavior before its threat model, token lifecycle, storage mechanism, and public service contract are reviewed.

## Licensing and security

- Do not describe this private source repository as open source or add an open-source `LICENSE` without an explicit product decision.
- Complete `LICENSING.md` release requirements before distributing a CLI binary externally.
- Never commit credentials, tokens, private keys, signing material, paid-theme contents, customer data, or local auth state.

## Delivery

- Use a topic branch and pull request; squash merge after approval.
- Work in small increments and add repository-specific formatting, linting, tests, and release checks with the code that needs them.
