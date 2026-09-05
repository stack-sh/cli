import { spawnSync } from "node:child_process"

const supportedCommands = new Set(["check", "fmt", "render"])
const [command, ...arguments_] = process.argv.slice(2)
if (!supportedCommands.has(command)) {
  process.stderr.write("Usage: consume-cli-json.mjs <check|fmt|render> [ARGUMENTS...]\n")
  process.exitCode = 2
} else {
  const binary = process.env.STACK_BINARY ?? "stack"
  const completed = spawnSync(binary, [command, ...arguments_, "--json"], {
    encoding: "utf8",
  })
  if (completed.error) throw completed.error
  if (completed.signal !== null || completed.status === null) {
    throw new Error("Stack CLI did not return a process exit status")
  }
  if (completed.stderr !== "") {
    throw new Error("Stack CLI emitted unexpected standard error in JSON mode")
  }

  const envelope = JSON.parse(completed.stdout)
  if (envelope.schemaVersion !== 1) {
    throw new Error(`Unsupported Stack CLI schema version: ${envelope.schemaVersion}`)
  }
  if (envelope.command !== command) {
    throw new Error(`Expected ${command} output, received ${envelope.command}`)
  }
  if (envelope.exitStatus !== completed.status) {
    throw new Error("Envelope exitStatus does not match the process exit status")
  }

  const summary = {
    command: envelope.command,
    outcome: envelope.outcome,
    exitStatus: envelope.exitStatus,
    diagnosticCodes: envelope.diagnostics.map(({ code }) => code),
    artifacts: envelope.artifacts.map(({ kind, path }) => ({ kind, path })),
    errorCode: envelope.error?.code ?? null,
  }
  process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`)
  process.exitCode = completed.status
}
