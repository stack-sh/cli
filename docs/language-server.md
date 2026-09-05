# Native Stack language server

`stack lsp` exposes Stack language intelligence through the Language Server Protocol (LSP) 3.18. It is a long-running standard-input/standard-output process intended to be launched by an editor or another LSP client:

```sh
stack lsp
```

Standard output is reserved for `Content-Length` framed JSON-RPC messages. Operational framing failures are written to standard error and terminate the process with exit status `2`. Stack source diagnostics are sent through `textDocument/publishDiagnostics` and do not use the process exit status.

## Capabilities

The server advertises these static capabilities during `initialize`:

| LSP capability | Method or notification | Behavior |
| --- | --- | --- |
| Incremental synchronization | `textDocument/didOpen`, `didChange`, `didClose` | Maintains one current, monotonically versioned UTF-8 snapshot per open URI and applies ranged changes in order. |
| Diagnostics | `textDocument/publishDiagnostics` | Publishes compiler errors and warnings after open and every accepted change, with the matching document version, and clears them on close. |
| Completion | `textDocument/completion` | Returns syntax- and scope-aware keywords, properties, enum values, document identifiers, and bundled core icon IDs. |
| Hover | `textDocument/hover` | Returns plain-text information for declarations, references, edges, and properties. |
| Document symbols | `textDocument/documentSymbol` | Returns a hierarchical diagram, group, node, and edge outline. |
| Formatting | `textDocument/formatting` | Returns one whole-document edit when the engine formatter changes valid source, or no edits for canonical or syntactically invalid source. |

Completion uses the bundled Stack core icon catalog. User-imported provider packs remain local rendering inputs and are not read by the language-server process.

## Client setup

Configure the client to associate the `stack` language ID and `.stack` extension with the command `stack lsp`. The executable must be available in the environment inherited by the editor. A minimal Neovim setup is:

```lua
vim.filetype.add({ extension = { stack = "stack" } })

vim.api.nvim_create_autocmd("FileType", {
  pattern = "stack",
  callback = function(args)
    vim.lsp.start({
      name = "stack",
      cmd = { "stack", "lsp" },
      root_dir = vim.fs.root(args.buf, { ".git" }) or vim.fn.getcwd(),
    })
  end,
})
```

Restart the editor after installing or replacing the binary so the new process uses the expected version. Run `stack lsp --help` in the same environment when diagnosing executable discovery.

## Protocol lifecycle and positions

The server accepts `initialize` exactly once, then normal requests and notifications, then `shutdown` followed by `exit`. Requests before initialization return `ServerNotInitialized`; requests after shutdown return `InvalidRequest`. An `exit` before `shutdown` terminates with status `1`, as required by the LSP lifecycle.

The client may list `general.positionEncodings` in preference order. The server selects the first supported value among `utf-8`, `utf-16`, and `utf-32`, and defaults to the LSP-required UTF-16 encoding when the client omits the list. Ranges are end-exclusive. Incremental edits that split a Unicode scalar, address a missing line, move backwards, or do not increase the document version are rejected without changing the stored snapshot.

The adapter processes messages serially. It accepts `$/cancelRequest`, bounds remembered request IDs, returns `RequestCancelled` when an ID is cancelled before its request begins, and never replaces an already committed result with cancellation. Work already executing in this synchronous MVP is not preempted. Versioned diagnostics and serialized snapshot access prevent results for an older accepted change from being published as current.

## Resource and failure boundaries

| Input | Limit |
| --- | ---: |
| One JSON-RPC payload | 8 MiB |
| One header block | 32 KiB |
| One open document | 4 MiB |
| Open documents | 64 |
| Remembered cancelled or completed request IDs | 1,024 per set |
| Document URI | 4,096 Unicode scalars |

Malformed JSON produces a JSON-RPC parse error and the next correctly framed message can still be processed. Invalid methods, parameters, document versions, and source positions produce standard JSON-RPC or LSP errors. Invalid notification parameters are ignored and reported with `window/logMessage`. Invalid framing, truncated bodies, unsupported declared charsets, and I/O failures terminate the transport without attempting to resynchronize an untrusted byte stream.

## Ownership boundary

The CLI owns LSP framing, lifecycle, negotiated coordinate conversion, bounded open-document state, incremental changes, cancellation bookkeeping, and conversion to protocol values. `stack-compiler` owns diagnostics, completion, hover, symbol semantics, stable codes, and authored source spans for one immutable snapshot. `stack-engine` owns canonical formatting, and `stack-theme` owns bundled core icon metadata. None of those pure libraries perform editor transport, filesystem, network, clock, or process-environment access for an LSP request.

The protocol reference is the [Language Server Protocol 3.18 specification](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.18/specification.md).
