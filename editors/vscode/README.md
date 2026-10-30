# rubash Debugger — zero-JS VS Code extension (WP-16)

A purely declarative extension: `package.json` only. There is no
JavaScript, no activation entry point, and no code to audit — the
manifest contributes a debugger of type `rubash` whose adapter is
the rubash binary itself, in the spelled-out DAP host form the
design specifies (debugger-full-design §2/§13, WP-16):

```
rubash --debug --dap
```

VS Code spawns that command and speaks DAP over its stdio; the
adapter is rubash's own `--dap` frontend (the same session core
the `--debug` terminal frontend drives — one debugger, two
views).

## Requirements

`rubash` on the PATH (1.5.0 or later). The manifest's adapter
resolution is the `program` + `args` pair in the debugger
contribution; to run the adapter without this extension, put the
same command in a launch configuration's
`debugAdapterExecutable` setup (a `rubash` shim that adds
`--debug --dap`), or start `rubash --debug --dap <script>`
directly — with a program operand it serves that program; with
none it runs as a pure DAP server and the program arrives in the
client's `launch` request.

## Launch configuration

The contributed `launch` attributes are exactly the ones the
adapter implements: `program` (required), `args`, `cwd`, `env`,
`stopOnEntry`, `noDebug`. `attach` is accepted only in its
launch-equivalent form (a `program` argument): background jobs
are separate OS processes and cannot be attached.
