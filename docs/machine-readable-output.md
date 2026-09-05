# Machine-readable CLI output

`stack check`, `stack fmt`, and `stack render` accept `--json` for CI, editor, and agent integrations. JSON mode writes exactly one newline-terminated JSON envelope to standard output and preserves the command's normal process exit status.

```sh
stack check architecture.stack --json
stack fmt --check architecture.stack --json
stack render architecture.stack -o architecture.svg --json
```

Human mode is unchanged. In JSON mode, Stack diagnostics and operational failures are represented in the envelope and standard error remains empty. If the process cannot serialize or write the envelope itself, it exits `2` and reports that last-resort failure on standard error because valid JSON can no longer be guaranteed. Help remains human-readable even when `--json` is also present.

## Version 1 envelope

Every envelope contains the same required fields:

```json
{
  "$schema": "https://raw.githubusercontent.com/stack-sh/cli/main/schemas/cli-output-v1.schema.json",
  "schemaVersion": 1,
  "command": "check",
  "outcome": "success",
  "exitStatus": 0,
  "diagnostics": [],
  "artifacts": [],
  "error": null
}
```

- `command` is `check`, `fmt`, or `render`.
- `outcome` is `success`, `changes-required`, `stack-error`, or `operational-error`.
- `exitStatus` is the process status: `0` for success including warnings, `1` for Stack errors or a `fmt --check` difference, and `2` for argument, host, configuration, provider-pack, engine, or internal failures.
- `diagnostics` contains portable Stack diagnostics in deterministic source order.
- `artifacts` identifies formatted source, rendered SVG, and provider notice results.
- `error` is non-null only for `operational-error`.

Consumers should branch on `outcome`, stable diagnostic codes, artifact kinds, and operational error codes. Human-readable `message` and `help` text can improve logs but are not parsing contracts.

## Diagnostics and ranges

Each diagnostic includes `code`, `severity`, `message`, `path`, `range`, `expected`, `help`, and `related`. Ranges are end-exclusive. `byteOffset` is a zero-based UTF-8 byte offset; `line` and Unicode scalar `column` are one-based. Related locations use the same path and range representation.

Paths are reported as the CLI received or derived them; they are not canonicalized. Standard-input diagnostics use `<stdin>`.

## Artifacts

Each artifact includes `kind`, `path`, `mediaType`, and `content`:

| Kind | File output | Standard-output result in JSON mode |
| --- | --- | --- |
| `formatted-source` | `path` identifies the formatted file and `content` is `null` | `path` is `null` and `content` contains canonical Stack source |
| `rendered-svg` | `path` identifies the written SVG and `content` is `null` | `path` is `null` and `content` contains standalone SVG |
| `provider-notice` | `path` identifies the written notice and `content` is `null` | Not applicable |

`fmt --check` produces no artifact because it never writes formatted source. A Stack semantic error can coexist with a `formatted-source` artifact, matching human mode's existing formatter behavior. Only successfully produced artifacts are listed.

## Operational errors

The operational `error.code` categories are stable within schema version 1:

| Code | Category |
| --- | --- |
| `CLI1001` | Invalid or conflicting command arguments |
| `CLI1002` | Standard-stream or filesystem I/O failure |
| `CLI1003` | Configuration or provider icon-store failure |
| `CLI1004` | Engine operational failure |
| `CLI1005` | Internal output-contract invariant failure |

Stack source problems remain in `diagnostics` with `STK` codes and use outcome `stack-error`; they are not operational errors.

## Compatibility and validation

`schemaVersion: 1` and [`schemas/cli-output-v1.schema.json`](../schemas/cli-output-v1.schema.json) define an immutable envelope contract. A structural change, field removal or addition, enum expansion, type change, or semantic reinterpretation requires a new schema version and a separately named schema file. Diagnostic messages can change without a schema version change; diagnostic codes and source-range semantics follow their owning public Stack contracts.

Consumers may vendor the schema or pin its raw GitHub URL to a reviewed commit. Repository CI validates the checked-in golden fixtures with the exact, lockfile-pinned JSON Schema validator and checks process-level output against those fixtures.

[`examples/consume-cli-json.mjs`](../examples/consume-cli-json.mjs) is a dependency-free consumer prototype. It verifies the schema version and exit-status parity, then extracts stable summary fields without parsing messages:

```sh
STACK_BINARY=target/debug/stack \
  node examples/consume-cli-json.mjs check tests/fixtures/render.stack
```
