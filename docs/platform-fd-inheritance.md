# Platform Gap: Foreign Children Cannot Receive Exec-Established Numbered Fds

> Status: documented platform limitation, closed as `platform-gap`
> (rubash#378, verdict 2026-10). This page is the durable record of what the
> limitation is, why no sound fix path exists on Windows, and which
> descriptor-passing shapes rubash *does* support. Rubash's platform identity
> (native Windows, no MSYS/Cygwin runtime layer — see
> [`platform-identity.md`](platform-identity.md)) is load-bearing for the
> verdict: everything below is evaluated for a self-contained binary speaking
> Win32 directly, not for a bash port riding a POSIX emulation layer.

## The gap in one paragraph

GNU bash on Unix implements `exec 3>file` as `dup2` into the process
descriptor table, and that table is inherited in full by the forked child
(`third_party/bash/redir.c:1170 do_redirection_internal` establishes the
binding; `redir.c:1188-1192` keeps it open across `exec`; the child
inherits the whole table, so a foreign command that itself does `1>&3`
works with no cooperation from the parent spawn). Windows has no
`fork()`-style descriptor-table inheritance: `std::process::Command` (and
`CreateProcessW` underneath it) passes exactly three std handles —
`STARTUPINFO.hStdInput` / `hStdOutput` / `hStdError` — and nothing else.
A numbered descriptor that rubash established in its own virtual fd table
therefore cannot materialize as fd 3 (or any fd ≥ 3) inside an arbitrary
*foreign* child (`sh -c '...'`, `cmd /c ...`) that was not written to look
for it. The child either fails (`3: Bad file descriptor`) or silently
writes nowhere.

## Why the OS layer cannot express it

Three facts, each anchored to Microsoft's documentation:

1. **`PROC_THREAD_ATTRIBUTE_HANDLE_LIST` is a filter, never a value
   setter.** It only limits *which already-inheritable* handles are copied,
   and an inherited handle "refers to the same object in the child process
   ... It also has the same value"; the child must already "know" the object
   and retrieve the handle value.
   <https://learn.microsoft.com/en-us/windows/win32/procthread/inheritance>
2. **The only value-remapping slots Windows has are the three std
   handles** — `STARTF_USESTDHANDLES`'s `hStdInput`/`hStdOutput`/
   `hStdError`, exactly fds 0/1/2.
   <https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/ns-processthreadsapi-startupinfoa>
3. **`DuplicateHandle`'s `lpTargetHandle` is `[out]`** — the duplicate's
   value is system-assigned, and a cross-process duplicate is useless until
   "the source process must use interprocess communication to pass the
   value of the duplicate handle to the target process", i.e. the child
   must cooperate.
   <https://learn.microsoft.com/en-us/windows/win32/api/handleapi/nf-handleapi-duplicatehandle>

There is no supported way to make a *chosen* fd number exist in a child
that was not written to look for it. That is the gap.

## Where "fd 3" actually lives: a CRT fiction with one private channel

On Windows, fd numbers are CRT bookkeeping over `HANDLE`s. The UCRT builds
fds only from the three std handles plus its reserved inherit block —
verified in the UCRT sources shipped with the Windows SDK
(`ucrt/lowio/ioinit.cpp`):

- `initialize_stdio_handles_nolock` (ioinit.cpp:123-149) creates fds 0/1/2
  from `GetStdHandle` and nothing else;
- `initialize_inherited_file_handles_nolock` (ioinit.cpp:59-118) is the
  **only** path that materializes higher fds: it parses
  `GetStartupInfoW` → `lpReserved2` as an `int count; BYTE flags[count];
  intptr_t handles[count]` block and fills `__pioinfo[fh]->osfhnd/osfile`
  from it.

That layout is the ancient MSVCRT "io inherit block", still understood by
`msvcrt.dll` (which is what `System32\cmd.exe` imports on Win10); Wine
documents it in source comments — `msvcrt_create_io_inherit_block` at
`dlls/msvcrt/file.c:653-697`, child side `msvcrt_init_io` at `file.c:703+`
(<https://github.com/wine-mirror/wine/blob/master/dlls/msvcrt/file.c>).

But the field it rides is explicitly out of bounds for applications —
`STARTUPINFO.cbReserved2` is "**Reserved for use by the C Run-time; must be
zero**" and `lpReserved2` is "**Reserved for use by the C Run-time; must be
NULL**"
(<https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/ns-processthreadsapi-startupinfoa>).

A live probe on Windows 10 19045 (issue #378 discussion) confirmed the
boundary: a hand-built io-inherit block passed via `CreateProcessW` with
`bInheritHandles=TRUE` and `STARTF_USESTDHANDLES` rescues MSVCRT/UCRT-linked
children (`cmd /c 'echo hi 1>&3'` lands the bytes) but **never** MSYS2/Cygwin
children (`sh -c 'echo hi 1>&3'` → `3: Bad file descriptor`, exit 1),
because the msys runtime claims the *same reserved field* for its own
magic-checked `child_info` protocol (`winsup/cygwin/fork.cc:341-342` sets
`si.lpReserved2`/`si.cbReserved2`;
<https://github.com/cygwin/cygwin/blob/main/winsup/cygwin/fork.cc>). A
foreign block in that slot is noise to `msys-2.0.dll`, and implementing
Cygwin's private per-DLL-version protocol inside rubash is not a fix, it is
a hostage situation. Cygwin/MSYS2 can give fd ≥ 3 to a child *only when both
sides are Cygwin* — their `fork()` uses `CREATE_SUSPENDED`
(`fork.cc:286`), `WriteProcessMemory` into the child (`child_copy`,
`fork.cc:791`), and the child reconstructs its fd table from Cygwin's own
blob — cooperative machinery that never applies to a foreign binary.

Every other runtime lands in the same place:

- **Python**: `subprocess` `pass_fds` is marked "(POSIX only)"; on Windows
  you get `close_fds` + the `handle_list` filter + the three std handles.
  <https://docs.python.org/3/library/subprocess.html>
- **Node/libuv**: fds > 2 ride the *same* `STARTUPINFO.lpReserved2` slot
  with a libuv-private layout (`src/win/process-stdio.c:29-33`, count
  capped at 255 at `:178`; wired at `src/win/process.c:1041-1042`) —
  parsed only by a child that also contains libuv.
  <https://github.com/libuv/libuv/blob/v1.x/src/win/process-stdio.c>
- **Rust std**: `std::process::Command` on Windows has no API to map
  arbitrary handles to child fd numbers; even the nightly
  `spawn_with_attributes` exposes only `ProcThreadAttributeList` (fact 1
  above), not `STARTUPINFO` reserved fields.
  <https://doc.rust-lang.org/std/os/windows/process/trait.CommandExt.html>

## What rubash supports today (the supported-forms ledger)

The GNU contract stays intact wherever the child can be served. Forms that
**work**, with their mechanism:

| Form | Mechanism | Anchor |
| --- | --- | --- |
| `cmd >file 2>&1 <file` — stdio redirections, pipelines, here-docs | std::process three-channel wiring (fds 0/1/2) | `src/executor/external_inner.rs` spawn path (`Stdio::from` wiring) |
| Redirections rubash itself plans for the child (`exec 3>&2; cmd >&3`) | child-side redirect targets resolve against the fd table's live bindings at spawn time (#370 fix) | fd-table resolution in `src/executor/external_inner.rs` |
| Process-substitution operands (`cmd <(a) >(b)`, `/dev/fd/N` words) | on Windows the `/dev/fd/N` argv word is rewritten to a real path the child opens itself (temp file / fd backing file / flush-after-exit temp); on Unix the read end is pinned at fd N between fork and exec | `src/executor/dev_fd_operands.rs:285 materialize_dev_fd_operands` (Windows rewrite `:340-395`, table resolution `dev_fd_table_operand`) |
| rubash → rubash children (`exec 3>f; rubash -c 'echo hi >&3'`) | `__RUBASH_FD_*` env-key channel: each fd ≥ 3 is duplicated as a fresh INHERITABLE handle, named in the child's environment, parent-side dup closed after spawn — GNU's open→fork→close-unwind shape; the child's startup consumes the keys | `src/executor/external_inner.rs:755 spawn_external_process`, env keys `:791-795`; consumer in `src/executor/init.rs` |

The `__RUBASH_FD_*` channel is *cooperative*: rubash children honor it and
foreign children ignore it by construction — a foreign child simply sees
one extra env var (`spawn_external_process` comment,
`src/executor/external_inner.rs:767-777`). That is the correct steady
state, not a bug.

What does **not** work and is excluded from the rubash semantic ledger by
this ruling: a *foreign* child that itself opens or dups an fd number that
an enclosing `exec N>&M` established inside rubash
(`exec 3>f; sh -c 'echo hi >&3'`). Byte-level GNU parity for this shape is
unreachable on Windows for the reasons above.

## Verdict and ruling

Per the 2026-10 full pass through the Windows spawn surface (issue #378
discussion): **no sound fix path exists for foreign children**; the issue is
closed as `platform-gap` rather than fixed. Two consequences:

1. The spawn layer should not spend effort on a
   `CREATE_SUSPENDED` + `NtDuplicateObject`/`WriteProcessMemory` route: it
   cannot dictate child handle values (fact 3 above), it creates no CRT fds
   in a foreign child, and it is the shape of process-injection tooling
   rather than process plumbing. Cygwin only gets away with it because both
   sides are its own DLL — a dependency rubash's native-Windows identity
   rules out (rubash does not ship or require a POSIX runtime layer; the
   msys-2.0.dll fork emulation that Git Bash/MSYS2 rely on is precisely the
   layer rubash exists to not have).
2. The fd/unix-vs-windows split stays: on Unix the descriptor table is
   real and the fork semantics carry numbered fds natively; the gap is
   Windows-shaped and tracked as such, not as a rubash semantics bug.

## If demand ever materializes

The only known channel that would serve a *partial* bridge (UCRT/MSVCRT-shaped
children such as `cmd /c` only) is the CRT io-inherit block over
`lpReserved2`; a handle-reuse naming protocol (extend the cooperative
`__RUBASH_FD_*` env-key channel toward a documented convention foreign
launchers could opt into) is the other candidate. Any such work must be
labeled as depending on a field documented as "Reserved for use by the C
Run-time" or on child-side cooperation, with the MSYS2 `sh -c` shape
explicitly out of scope. Nothing here is a commitment — this page records
the map, not a plan.
