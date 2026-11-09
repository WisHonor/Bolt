# Bolt Benchmarks

> **Status:** started 2026-11-09 with the two speed gates for v0.1. Every speed or size claim Bolt makes comes from this page, the ones Git or Git LFS wins included ([decision 088](decisions/088-benchmarks-and-the-speed-gates.md)).

**In plain words.** Bolt promises two things before v0.1 ([decision 023](decisions/023-v0.1-speed-bar.md)): saving a huge file must not eat the computer's memory, and asking what changed in a big project must be quick. This page says how both were measured, on which machines, and what came out, pass or fail.

## 1. The speed gates

- **Memory:** saving a 10 GiB file of incompressible bytes peaks at no more than 500,000,000 bytes of memory.
- **Status:** `bolt status` on an unchanged 100,000-file project takes under 2 s.

| Machine | Saving 10 GiB: peak memory | Saving 10 GiB: time | Memory gate | Status, twice | Status gate |
|---|---|---|---|---|---|
| GitHub's Linux runner | 43,495,424 bytes | 28.7 s | passed | 0.70 s, 0.70 s | passed |
| Windows laptop | 48,734,208 bytes | 20.4 s | passed | 1.16 s, 1.15 s | passed |

How they are measured:

- **The file:** 10,737,418,240 bytes of BLAKE3's extendable output for the input `bolt memory gate`, written 1 MiB at a time by `crates/bolt-vcs/tests/speed.rs`. Nothing in it compresses and no two chunks match. It is saved with `bolt start`, then `bolt save "A 10 GiB file" --yes`, in a new temporary folder that is deleted afterwards.
- **Peak memory** is the most memory the `bolt save` process held at once: `VmHWM` on Linux, read from `/proc` every 20 ms, and the peak working set on Windows, read by PowerShell every 200 ms. Both are high-water marks, so only a peak in the last 20 or 200 ms before the process ended could be missed.
- **Time** is wall-clock time from start to exit, with warm caches: the file was just written, and part of it may still be in the system's cache.
- **Status:** the test `status_on_an_unchanged_100_000_file_project` builds 100,000 small files, 100 to a folder, an hour old, saves them, and times `bolt status` twice ([testing.md](testing.md) §5).
- Each number is from one run. GitHub's machines are shared, so their numbers vary from run to run; the nightly jobs run both gates again. There, the memory gate fails its job above the limit, while the status time is printed and read, not enforced, until v0.1.

### GitHub's Linux runner, 2026-11-09

- **Machine:** GitHub's `ubuntu-latest` runner, image ubuntu-24.04 20261101.360.1: AMD EPYC 7763, 4 virtual CPUs, 16 GB of RAM, SSD.
- **Bolt:** the code of commit `bca0558`. The commit that added this page changes no Bolt code, only tests, workflows and docs.
- **Memory gate:** [run 42598494181](https://github.com/WisHonor/Bolt/actions/runs/42598494181), job `memory-gate` of `.github/workflows/bench.yml`, which first frees about 22 GiB of the runner's disk. The run is at `0e93681` (on a branch since deleted): this page's commit before the docs were added, with `bca0558`'s Bolt code. The test has since gained two checks that a real measurement was made, which change nothing measured. It runs the `bolt` that `cargo test` builds, which has the core's test-support feature on; that feature only adds test overrides, off unless a test sets them, and helpers `bolt` never calls, so it doesn't change the number:

  ```sh
  cargo test --release -p bolt-vcs --test speed -- --ignored --nocapture --exact saving_a_10_gib_file_stays_under_500_mb
  ```

- **Status gate:** [run 42592855953](https://github.com/WisHonor/Bolt/actions/runs/42592855953), job `speed` of `.github/workflows/nightly.yml`, at commit `b36e203`, which has the same Bolt code as `bca0558`:

  ```sh
  cargo test --release -p bolt-vcs --test speed -- --ignored --nocapture --exact status_on_an_unchanged_100_000_file_project
  ```

### Windows laptop, 2026-11-09

- **Machine:** AMD Ryzen 7 7730U, 8 cores (16 threads), 16 GB of RAM, a 1 TB NVMe SSD (WD PC SN560); Windows 11 Home 10.0.26200.
- **Bolt:** commit `bca0558`.
- **Memory gate,** in PowerShell from the repository's root, with `BOLT_CONFIG_DIR` set to an empty folder and `BOLT_USER_NAME` and `BOLT_USER_EMAIL` set:

  ```powershell
  New-Item -ItemType Directory "$env:TEMP\bolt-gate"
  $env:BOLT_GATE_FILE = "$env:TEMP\bolt-gate\big.bin"
  cargo test --release -p bolt-vcs --test speed -j 2 -- --ignored --nocapture --exact write_the_memory_gate_file
  cargo build --release -p bolt-vcs -j 2   # after the test, which builds its own bolt.exe
  $bolt = (Resolve-Path target\release\bolt.exe).Path
  cd $env:TEMP\bolt-gate
  & $bolt start
  $p = Start-Process $bolt -ArgumentList 'save', '"A 10 GiB file"', '--yes' -NoNewWindow -PassThru
  $null = $p.Handle   # keeps the exit code and times readable after it ends
  $peak = 0
  while (-not $p.HasExited) {
      try { $p.Refresh(); $peak = [Math]::Max($peak, $p.PeakWorkingSet64) } catch {}
      Start-Sleep -Milliseconds 200
  }
  "$peak bytes, $(($p.ExitTime - $p.StartTime).TotalSeconds) s, exit $($p.ExitCode)"
  cd ..; Remove-Item -Recurse -Force bolt-gate
  ```

  Deleting the folder gave back 20.02 GiB.
- **Status gate:**

  ```sh
  cargo test --release -p bolt-vcs --test speed -j 2 -- --ignored --nocapture --exact status_on_an_unchanged_100_000_file_project --test-threads=2
  ```
