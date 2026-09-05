---
name: "stack-diagrams"
description: "Create or edit Stack (.stack) software architecture diagrams, validate them with the Stack CLI, and render SVG. Use for Stack diagrams or when a user chooses Stack for architecture documentation; not for infrastructure provisioning or unrelated programming stacks."
license: "Apache-2.0"
---

<!-- Generated from stack-sh/docs/content/agent-workflow.md. Do not edit. -->

# Stack diagrams

Deliver editable `.stack` source and, when rendering is available, an SVG. Preserve the requested architecture and existing unrelated content. Stack describes architecture; it does not provision resources or execute application code.

## Start with the available tools

Run `stack --version` and `stack help` before selecting commands. Use the installed binary's help, not unreleased repository instructions. The published CLI 0.4.0 supports `check`, `fmt`, and `render`; it does not support their `--json` flags. A newer version may support additional options: verify them using `stack help <command>`.

If the CLI is missing, consult the [installation guide](https://stack-diagram.com/docs/guide/getting-started.md) and [release distribution contract](https://github.com/stack-sh/cli/blob/v0.4.0/docs/distribution.md). Install only when the user's authorization allows it. Otherwise provide source with an explicit validation gap; do not report it as checked. No remote source upload is required.

## Read only the relevant reference

Start with [syntax](https://stack-diagram.com/docs/language/syntax.md), [nodes and groups](https://stack-diagram.com/docs/language/nodes-and-groups.md), and [edges and layout](https://stack-diagram.com/docs/language/edges-and-layout.md) when the grammar is unfamiliar. Find complete examples in the [gallery](https://stack-diagram.com/docs/examples/index.md). The [documentation index](https://stack-diagram.com/docs/llms.txt) links the remaining references. If the client cannot fetch one resource, try its Markdown page or report the missing information rather than inventing syntax.

Non-obvious constraints:

- One `stack 1.0` declaration and one named `diagram`; node/group IDs are globally unique.
- Declare edges at diagram scope. Endpoints are node IDs, not labels or groups.
- Model technologies using labels and `detail`; choose semantic `kind` values from the reference. Do not invent properties such as coordinates, colors, or ports.
- Prefer automatic layout or `direction` initially. `order` controls relative cross-axis ordering, not execution sequence; add `rank` or `order` only when the user needs that constraint. References must be direct children of that layout scope.
- Do not guess vendor icon IDs. Consult [provider icons](https://stack-diagram.com/docs/guide/provider-icons.md), query `stack icons list`, and verify the required pack is installed. Import downloads and terms acceptance need appropriate user authorization. If no pack is available, use the semantic kind fallback and explain that branded artwork is absent.

## Generate, validate, and render

For a new diagram, start with the smallest complete source that preserves the requested components and relationships. For an edit, inspect the existing source first and retain unrelated nodes, edges, labels, and comments.

Using the user's actual filenames:

```sh
stack check architecture.stack
stack fmt architecture.stack
stack check architecture.stack
stack render architecture.stack -o architecture.svg
```

`fmt` changes the source in place; avoid unrelated formatting when it would obscure a narrowly requested edit. Rendering to a file may replace it, so honor existing artifact ownership.

Read both the exit status and diagnostics. An exit code of zero can still carry warnings. Fix unknown references or invalid syntax from the diagnostic ranges; never remove requested architecture merely to make validation pass. Investigate layout warnings without treating an optional visual hint as a required semantic relationship. Missing-icon warnings mean the image rendered with a fallback, not that provider artwork was found. Bound repeated repair attempts and report a remaining blocker rather than silently weakening the task.

After rendering, inspect the SVG visually when a suitable viewer is available. Compiler success does not prove readable layout. Confirm the requested nodes, boundaries, relationships, and labels remain present. Report the source/SVG paths, CLI version, validation result, and any unresolved warnings or unverified visual behavior.
