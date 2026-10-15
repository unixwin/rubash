# Scalar assignment: measured partial follow-up for #609

## Scope and GNU ownership

Baseline: `334afab2`. Candidate code: `6752598ef8231c2c569eed3b51095ae0d3203d2b`.

GNU `variables.c:3054 bind_variable_internal` replaces the value of an existing
scalar cell without changing its attributes. Rubash now reuses the existing
typed/mirror String capacities and mirror key instead of allocating replacement
copies. This does **not** collapse the two stores or bypass attribute checks.

GNU `subst.c:3576 do_assignment_internal` calls `xtrace_print_assignment` only
when `echo_command_at_execute` is enabled. Both permanent and temporary apply
paths now populate the trace-only expansion memo only with xtrace enabled.
The expanded RHS remains available to tracing without executing it again.

The canonical scalar workload runs through the standalone-assignment path in
`command_prepare.rs`, not the memo insertion in `apply_permanent_assignments`.
Memo gating alone therefore did not improve this workload: an initial 9-sample
run measured 1298.94 -> 1292.00 ms (0.5%, within observed noise), with unchanged
allocation counts. Do not attribute the scalar loop cost to that memo insert.

## Method

Measured 2026-10-15 on Linux x86_64, WSL2 kernel
`6.18.33.2-microsoft-standard-WSL2`, glibc 2.39, Python 3.12.3.
All Rubash binaries used the default release profile: opt-level 3, LTO,
one codegen unit. Reference: owner-built GNU Bash 5.3.0.

The native Linux Python parent performs two warmup rounds and nine measured
rounds, rotating baseline/candidate/reference order each round. Every invocation
has a 30-second timeout. Return code, stdout and stderr must match on **every**
run. Other agents paused builds and benchmarks for the measurement window.
Host scheduling still produced visible outliers; no wall-time speedup is claimed.

Reproduce with separately built release binaries from the two commits:

```sh
python3 scripts/bench-interleaved.py \
  --baseline /path/to/baseline/rubash \
  --candidate /path/to/candidate/rubash \
  --reference /usr/local/bin/bash \
  --baseline-ref 334afab2 \
  --candidate-ref 6752598ef8231c2c569eed3b51095ae0d3203d2b \
  --warmup 2 --runs 9 \
  benchmarks/26-scalar-assign-x200000.sh \
  benchmarks/27-prefix-assign-x20000.sh
```

Raw local artifacts, including the diagnostic glibc allocation interposer:
`target/issue-suites/results/issue-609/`.
`linux-buffer-reuse.json` records every sample, executable SHA256 and workload
SHA256. The interposer counts malloc/calloc/realloc calls and requested bytes;
it is a diagnostic, not a wall-time measurement, and is not used to time the runs.

## Results

| Workload | Baseline median/min ms | Candidate median/min ms | GNU median/min ms | Candidate/baseline | Candidate/GNU |
| --- | --- | --- | --- | --- | --- |
| scalar assign x200000 | 1323.35 / 1291.38 | 1322.74 / 1277.99 | 353.01 / 351.06 | 0.9995 | 3.7471 |
| prefix assign x20000 | 453.56 / 450.47 | 448.76 / 445.35 | 109.74 / 109.27 | 0.9894 | 4.0892 |

Scalar samples, milliseconds, in chronological order per binary:

```text
baseline: 1914.778389 1312.283397 1323.348421 1310.506020 1338.304980 1291.381674 1474.617699 1614.969163 1309.716931
candidate: 1862.720820 1305.267800 1312.339216 1361.033034 1322.744278 1277.992472 1495.101755 1308.367232 1575.181059
GNU: 514.791058 361.079400 351.348305 353.008006 365.996750 351.055447 352.042147 351.141835 520.961953
```

Prefix samples:

```text
baseline: 456.607834 459.836834 453.368933 450.465933 456.100534 450.637821 662.739436 453.561119 451.445321
candidate: 447.768032 452.301133 451.194433 453.671633 445.345132 446.554924 448.759522 447.837222 462.137416
GNU: 112.124408 109.289708 109.782908 109.743608 109.380108 109.814556 163.149235 109.702357 109.272456
```

The canonical scalar workload allocation counter was repeated three times:

| Binary | Allocation calls | Requested bytes |
| --- | --- | --- |
| baseline | 6801557 / 6801556 / 6801555 | 106791422 / 106790174 / 106788926 |
| candidate | 6201561 / 6201560 / 6201561 | 104412341 / 104411093 / 104412341 |

This is approximately **600000 fewer allocator calls (8.82%)**, not an observed
wall-time improvement. Both stores remain; #609 is **not resolved**. Its <=1.2x
GNU wall-time target is not met. These Linux figures are not Android validation.

## Semantic verification

- Linux library suite: 583 passed, bounded by a 180-second command timeout.
- Existing scalar/local/subshell/export CLI regression: passed.
- Existing xtrace compound-assignment CLI regression: passed.
- Three new unit tests check disabled/enabled memo population for both apply
  paths and capacity/value consistency of the typed and mirror scalar cells.
- GNU 5.3 script probe covers shrinking scalar values, integer/export attributes,
  indexed-array scalar assignment, nameref and local scope. Baseline and candidate
  stdout/stderr are identical. The probe also exposed an existing associative
  scalar-assignment defect: assigning key 0 loses another key in both Rubash
  binaries but not GNU; this is not repaired or counted as passing GNU parity.
- Windows rustfmt and git diff whitespace checks pass.

## Performance PR acceptance

Do not close a performance issue merely because fmt, compilation or semantic
tests pass. Each performance PR must provide baseline/candidate commits, build
profile, fixed workload, machine/platform, reference binary, warmup, repeated
interleaved samples, median/min, output/exit equality and a clear acceptance
decision. Preserve inconclusive and regressing rows. Partial allocation savings
do not satisfy an outstanding wall-time target; keep the issue open.
