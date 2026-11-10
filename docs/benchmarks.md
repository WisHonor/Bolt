# Bolt Benchmarks

> **Status:** started 2026-11-09 with the two speed gates for v0.1; Bolt against Git and Git LFS, the speeds of a save's parts and the ruling on compressing again were added the same day. A third gate, against Git on a code project, and the profile milestone 6 starts from (§7) were added on 2026-11-10; the numbers in §1 to §6 are from before milestone 6's speed work. Every speed or size claim Bolt makes comes from this page, the ones Git or Git LFS wins included ([decision 088](decisions/088-benchmarks-and-the-speed-gates.md)).

**In plain words.** Bolt promises three things before v0.1 (decisions [023](decisions/023-v0.1-speed-bar.md) and [099](decisions/099-a-third-speed-gate-within-twice-gits-time.md)): saving a huge file must not eat the computer's memory, asking what changed in a big project must be quick, and on an everyday code project Bolt must stay within twice Git's time. This page says how each was measured, on which machines, and what came out, pass or fail. It then puts Bolt beside Git and Git LFS on the same files, step by step, and says where each one wins.

## 1. The speed gates

- **Memory:** saving a 10 GiB file of incompressible bytes peaks at no more than 500,000,000 bytes of memory.
- **Status:** `bolt status` on an unchanged 100,000-file project takes under 2 s.
- **Against Git** ([decision 099](decisions/099-a-third-speed-gate-within-twice-gits-time.md)): on Git's source at `v2.56.0` (§2's code set), `status` with nothing changed, a small change saved, and going back to the first version each take Bolt at most twice Git's time, on GitHub's Linux runner and on its Windows runner. It is measured as §2 measures, with 10 timed runs of each step, by `bench/run.sh gate`, which fails when a step is over.

Before milestone 6's speed work the third gate fails five of its six checks (§2's tables, 2026-11-09):

| Step, on Git's source | Linux: Bolt, Git, times Git's | Windows: Bolt, Git, times Git's |
|---|---|---|
| `status`, nothing changed | 62.0 ms, 13.0 ms, 4.8 | 135 ms, 45.4 ms, 3.0 |
| A small change, saved | 68.6 ms, 30.2 ms, 2.3 | 149 ms, 127 ms, 1.2 (passes) |
| Back to the first version | 73.2 ms, 14.8 ms, 4.9 | 152 ms, 55.2 ms, 2.8 |

The first two gates, as measured:

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
- **Memory gate:** [run 42598494181](https://github.com/WisHonor/Bolt/actions/runs/42598494181), job `memory-gate` of `.github/workflows/bench.yml`, which first frees about 22 GiB of the runner's disk. That job is now `.github/workflows/memory-gate.yml`, which the bench and nightly workflows both call. The run is at `0e93681`, on a branch since deleted: this page's commit before the docs were added, with `bca0558`'s Bolt code. The test and its job have since gained one check each that a real measurement was made; neither changes what is measured. The test runs the `bolt` that `cargo test` builds, which has the core's test-support feature on. That feature only adds test overrides, off unless a test sets them, and helpers `bolt` never calls, so it doesn't change the number:

  ```sh
  cargo test --release -p bolt-vcs --test speed -- --ignored --nocapture --exact saving_a_10_gib_file_stays_under_500_mb
  ```

- **Status gate:** [run 42592855953](https://github.com/WisHonor/Bolt/actions/runs/42592855953), job `speed` of `.github/workflows/nightly.yml`, at commit `b36e203`, which has the same Bolt code as `bca0558`:

  ```sh
  cargo test --release -p bolt-vcs --test speed -- --ignored --nocapture --exact status_on_an_unchanged_100_000_file_project
  ```

  The same log gives the test's first save of the 100,000 files, made before it times `status`: 1.93 s.

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

  Its first save of the 100,000 files, made before it times `status`, took 5.10 s.

## 2. Against Git and Git LFS

**In plain words.** The same files go through the same steps three times: with Bolt, with Git, and with Git using Git LFS for every file of 1 MiB or more. Big files are where Bolt pulls ahead. Ten versions of a 1 GiB file take 1 GiB in Bolt and 10 GiB in Git or Git LFS. Saving one more version takes Bolt 1.4 to 2.9 seconds, where Git takes over half a minute. Git LFS is quickest at the first save of big files full of new bytes, since it neither compresses them nor waits for the disk. On the two small projects, Git, with or without Git LFS, is quickest at `status`, small saves and going back to an old version, on both runners. Of those steps, Bolt wins only `status` after touching Godot's biggest file, on Linux. Git is also quickest at going back to an old version of the repeated 1 GiB file on both runners, and of the 10-version one on Linux. On Linux it is quicker at `status` with nothing changed on the big-file sets too, by a millisecond or two. In space, Git keeps Godot's demos in 3% less (219.3 MiB against Bolt's 226.7 MiB), and its deltas keep scattered edits to a big file in less space than Bolt's chunks (§3).

### How it was measured

- **The data sets,** built by `bench/data.sh`. Each tool works on its own copy, every file set an hour old first:
  - **Code:** Git's own source at tag `v2.56.0` (commit `a018953`), from GitHub's tarball. On Linux that is 4,850 files, 47.8 MiB. On Windows, Git Bash's `tar` (MSYS2's) writes the tarball's three links as copies of what they point to, so there it is 4,964 files, 50.0 MiB.
  - **Godot's demo projects** at tag `4.7-6ad6167` (commit `6ad6167`), a real game project: 4,018 files, 298.3 MiB, 47 of them 1 MiB or more (201.4 MiB together).
  - **Noise:** 20 files of 50 MiB.
  - **Versions:** one 1 GiB file of noise, then nine more versions of it, each overwriting 1 MiB and inserting 4 KiB, at offsets fixed for each version.
  - **Repeated:** 1 GiB made of one 1 MiB block of noise, 1,024 times over.

  Noise is the keystream of AES-256-CTR from `openssl`, keyed by a fixed text: nothing in it compresses, and every run builds the same bytes.
- **The steps,** each timed on its own:
  1. **First save:** `bolt save` after `bolt start`; `git add -A && git commit` after `git init`.
  2. **A small change, saved:** a line added to `README.md` (code, Godot); 4 KiB overwritten in one file (noise, repeated); the next version (versions, whose remaining versions are then saved untimed, to make 10).
  3. **`status` with nothing changed,** started 3 seconds after the last change.
  4. **`status` after `touch` on the biggest file,** whose time changes but not its contents, so each tool must read it again.
  5. **Back to the first version,** from the newest: `bolt goto` with the first save's fingerprint (not the empty save `bolt start` makes), `git checkout` with the first commit.

  Then the space each one takes: `.bolt`, or `.git` with Git LFS's objects in it, as `du -sb` gives it (the files' bytes; on Linux each folder's own few KiB too).
- **Timing:** `run.sh` runs each step once to warm up. `hyperfine` then times 5 runs and gives the mean, the fastest and the slowest, with warm caches. A step whose warm-up took over a minute would get 3 runs instead, but none did. `hyperfine` runs each command with bash (Git's own on Windows) and takes the shell's start-up time off. Before every run, untimed, come three things in turn. First a wait for Git's background housekeeping, then the step's setup (a new repository for the first save, or the change for the next one), then a `sync`, so that no run pays for writing what an earlier one left. Every command is in `bench/run.sh`.
- **To keep it fair:**
  - Git's `core.autocrlf` is off, so all three store the same bytes; every other setting is the tool's default.
  - **Waiting for the disk.** Bolt writes each pack through to the disk before it says a save is done, so a crash can't lose it. Git does that for packs, which is where it puts files over 512 MiB (`core.bigFileThreshold`). It doesn't for the loose objects it writes smaller files as: Git's documentation for `core.fsync` says "the default on most platforms is equivalent to `core.fsync=committed,-loose-object`". Git LFS syncs none of the objects `git add` stores: its clean filter writes each to a temporary file and renames it into place, with no sync between (`lfs/gitfilter_clean.go` and `commands/command_clean.go` in Git LFS's source). So their saves of big files can end before the disk has the data, and the next run's untimed `sync` writes it.
  - **Git's housekeeping.** From Git 2.54, a commit can start a repack (the `geometric` maintenance strategy) when its packs no longer grow geometrically. On Linux it runs in the background, out of the commit's time, and each run waits for it to end. Upstream Git's Windows build has no `daemonize` (`setup.c` and `config.mak.uname`), and `builtin/gc.c` says "failure to daemonize is ok, we'll continue in foreground". Git for Windows' own fork wasn't checked, so there the repack is most likely part of `git commit`'s time.
  - **A file just changed.** Bolt's cache doesn't vouch for a file changed in the last 2 seconds, so a change made in that time can't slip by; Git reads again a file changed no earlier than its index was written. That is why step 3 waits.

### GitHub's Linux runner, 2026-11-09

- **Machine:** GitHub's `ubuntu-latest`, image ubuntu-24.04 20261101.360.1: AMD EPYC 7763, 4 virtual CPUs, 16 GB of RAM.
- **Versions:** Bolt at `a2ba223` (on a branch since deleted; its Bolt code is `ba513aa`'s), Git 2.56.0, Git LFS 3.8.1, hyperfine 1.20.0.
- **Run:** [run 42616742873](https://github.com/WisHonor/Bolt/actions/runs/42616742873), job `against-git (ubuntu-latest)` of `.github/workflows/bench.yml`.

Each cell is the mean, then the fastest and slowest of the timed runs.

#### Code

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 236 ms (233–242) | 1.52 s (1.50–1.55) | 1.56 s (1.54–1.59) |
| A small change | 68.6 ms (67.4–69.6) | 30.2 ms (29.4–31.0) | 70.3 ms (69.5–71.2) |
| `status`, nothing changed | 62.0 ms (60.6–63.0) | 13.0 ms (12.7–13.2) | 13.0 ms (12.7–13.3) |
| `status` after touching the biggest file | 62.8 ms (61.9–64.2) | 18.3 ms (17.6–18.9) | 17.9 ms (17.7–18.2) |
| Back to the first version | 73.2 ms (72.4–74.2) | 14.8 ms (13.2–17.1) | 53.6 ms (52.3–56.2) |

#### Godot's demo projects

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 1.10 s (1.07–1.12) | 8.55 s (8.50–8.62) | 3.70 s (3.68–3.75) |
| A small change | 71.9 ms (71.0–72.4) | 28.7 ms (28.1–29.3) | 69.5 ms (68.4–70.9) |
| `status`, nothing changed | 64.8 ms (64.3–65.5) | 13.9 ms (13.1–14.9) | 13.6 ms (13.4–14.0) |
| `status` after touching the biggest file | 86.7 ms (86.2–87.4) | 90.8 ms (90.1–91.6) | 109 ms (107–110) |
| Back to the first version | 92.9 ms (92.1–93.7) | 41.7 ms (11.3–86.7) | 88.0 ms (51.1–144) |

#### Noise

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 4.14 s (4.13–4.16) | 28.2 s (28.1–28.3) | 2.31 s (2.28–2.38) |
| A small change | 81.0 ms (77.3–84.0) | 1.42 s (1.41–1.44) | 316 ms (313–318) |
| `status`, nothing changed | 3.78 ms (3.65–3.97) | 1.78 ms (1.71–1.94) | 1.91 ms (1.82–2.11) |
| `status` after touching the biggest file | 36.8 ms (36.4–37.5) | 136 ms (136–136) | 148 ms (142–155) |
| Back to the first version | 147 ms (142–152) | 219 ms (191–325) | 275 ms (271–281) |

#### Versions

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 4.24 s (4.22–4.25) | 32.0 s (31.9–32.3) | 2.33 s (2.30–2.40) |
| A small change | 1.42 s (1.41–1.43) | 31.9 s (31.8–32.1) | 2.29 s (2.27–2.32) |
| `status`, nothing changed | 3.38 ms (3.20–3.89) | 2.01 ms (1.71–2.30) | 1.79 ms (1.73–1.91) |
| `status` after touching the biggest file | 675 ms (670–679) | 2.82 s (2.81–2.83) | 2.45 s (2.43–2.49) |
| Back to the first version | 4.41 s (4.39–4.45) | 3.84 s (3.83–3.85) | 3.95 s (2.39–4.41) |

#### Repeated

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 1.38 s (1.38–1.39) | 31.8 s (31.8–31.9) | 2.31 s (2.30–2.33) |
| A small change | 1.27 s (1.26–1.28) | 31.9 s (31.8–32.0) | 2.32 s (2.26–2.40) |
| `status`, nothing changed | 3.30 ms (3.18–3.48) | 1.77 ms (1.74–1.86) | 2.47 ms (2.21–2.85) |
| `status` after touching the biggest file | 659 ms (657–663) | 2.78 s (2.78–2.80) | 2.45 s (2.43–2.49) |
| Back to the first version | 5.26 s (5.20–5.31) | 3.79 s (3.78–3.80) | 4.41 s (4.37–4.51) |

The repeated file stores almost nothing new in Bolt (below), yet its first save takes a third to a half of a noise file's: Bolt still reads, fingerprints and cuts every byte, and only finds the chunks it has already stored once it has done so.

### GitHub's Windows runner, 2026-11-09

- **Machine:** GitHub's `windows-latest`, image windows-2025-vs2026 20261103.298.1: Windows Server 2025 Datacenter, AMD EPYC 7763, 4 virtual CPUs, 16 GB of RAM, two 150 GB virtual disks, Windows Defender's real-time protection off. The data is under the runner's temporary folder, on `D:`, the drive with more room.
- **Versions:** Bolt at `a2ba223`, Git 2.56.0.windows.4, Git LFS 3.8.1, hyperfine 1.20.0.
- **Run:** [run 42616742873](https://github.com/WisHonor/Bolt/actions/runs/42616742873), job `against-git (windows-latest)`.

#### Code

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 581 ms (558–621) | 5.07 s (5.04–5.12) | 5.35 s (5.31–5.40) |
| A small change | 149 ms (145–153) | 127 ms (125–129) | 422 ms (418–423) |
| `status`, nothing changed | 135 ms (130–140) | 45.4 ms (44.7–46.4) | 46.0 ms (45.5–46.9) |
| `status` after touching the biggest file | 140 ms (132–149) | 52.5 ms (50.7–55.1) | 51.2 ms (50.3–52.3) |
| Back to the first version | 152 ms (150–157) | 55.2 ms (54.1–56.3) | 353 ms (348–360) |

#### Godot's demo projects

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 1.51 s (1.39–1.62) | 13.1 s (12.8–13.4) | 8.09 s (7.87–8.54) |
| A small change | 192 ms (182–201) | 136 ms (124–153) | 440 ms (431–453) |
| `status`, nothing changed | 177 ms (172–185) | 54.1 ms (44.0–64.6) | 47.8 ms (46.2–50.9) |
| `status` after touching the biggest file | 192 ms (187–198) | 164 ms (160–172) | 349 ms (348–350) |
| Back to the first version | 195 ms (190–200) | 67.7 ms (58.1–90.5) | 363 ms (360–367) |

#### Noise

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 4.48 s (4.31–4.60) | 31.6 s (31.4–32.2) | 3.23 s (3.20–3.27) |
| A small change | 174 ms (172–175) | 1.66 s (1.65–1.67) | 0.80 s (0.73–1.09) |
| `status`, nothing changed | 21.2 ms (20.6–21.7) | 31.3 ms (31.1–31.7) | 30.8 ms (30.2–31.8) |
| `status` after touching the biggest file | 44.6 ms (40.2–49.1) | 198 ms (197–199) | 394 ms (391–402) |
| Back to the first version | 204 ms (176–253) | 281 ms (279–282) | 857 ms (840–905) |

#### Versions

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 4.55 s (4.48–4.59) | 32.7 s (32.5–33.0) | 3.28 s (3.27–3.31) |
| A small change | 2.88 s (2.87–2.89) | 43.6 s (32.6–60.7) | 3.27 s (3.22–3.33) |
| `status`, nothing changed | 25.1 ms (20.9–36.8) | 31.5 ms (31.2–31.8) | 31.9 ms (30.8–33.2) |
| `status` after touching the biggest file | 434 ms (426–441) | 3.18 s (3.17–3.19) | 2.96 s (2.93–2.97) |
| Back to the first version | 4.35 s (4.19–4.48) | 4.84 s (4.82–4.88) | 5.91 s (5.88–5.99) |

#### Repeated

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 1.95 s (1.93–1.95) | 32.7 s (32.6–32.7) | 3.30 s (3.24–3.35) |
| A small change | 1.59 s (1.56–1.62) | 43.8 s (32.6–60.9) | 3.29 s (3.27–3.35) |
| `status`, nothing changed | 20.5 ms (19.5–22.1) | 31.6 ms (30.7–32.8) | 31.6 ms (30.6–33.5) |
| `status` after touching the biggest file | 464 ms (460–471) | 3.16 s (3.15–3.18) | 2.92 s (2.91–2.93) |
| Back to the first version | 5.15 s (5.12–5.20) | 4.82 s (4.79–4.90) | 5.35 s (3.18–5.93) |

On Windows, Git's commits of a 1 GiB file range from 33 to 61 s, the slow ones most likely repacking as they go.

### Space on disk

After the steps: the first save and six small changes, or the 10 versions of the versions set. Both runners gave the same sizes to the tenth of a MiB; only the code set's files differ, 50.0 MiB on Windows, where its three links are copies.

| Set | The files | Bolt | Git | Git with Git LFS |
|---|---|---|---|---|
| Code | 47.8 MiB | 15.3 MiB | 16.3 MiB | 16.3 MiB |
| Godot's demo projects | 298.3 MiB | 226.7 MiB | 219.3 MiB | 263.4 MiB |
| Noise | 1,000.0 MiB | 1,002.6 MiB | 1,300.4 MiB | 1,300.0 MiB |
| Versions | 1,024.0 MiB | 1,041.1 MiB | 10.00 GiB | 10.00 GiB |
| Repeated | 1,024.0 MiB | 5.2 MiB | 7.00 GiB | 7.00 GiB |

- Git and Git LFS keep every version of a changed file whole; Bolt keeps its changed chunks, about 64 KiB each, and a list of the file's chunks.
- Godot's demos take 3% more in Bolt than in Git. Most likely that is because Bolt compresses each chunk of a big file on its own, and keeps a piece raw unless zstd gets it under 95% of its size. Git deflates each file whole and keeps whatever that saves.

## 3. Imports

**In plain words.** `bolt import` turns a Git repository, history and all, into a Bolt project. Three repositories show what it costs in space. A Git LFS repository shrinks to a ninth, since Git LFS keeps every version of a file whole. An honest history of one big file comes out 18% bigger than Git keeps it, since Git stores each version as a small delta. And the red team's RT4-1 repository, built to blow up, comes out 98 times bigger: chunking covers files, not the records a save is made of ([decision 087](decisions/087-what-an-import-writes-once-files-are-chunked.md)).

| Repository | Git (`.git`) | Bolt (`.bolt`) | `bolt import`, Linux | `bolt import`, Windows |
|---|---|---|---|---|
| Git LFS: five files of 100 MiB in 10 versions, each overwriting 1 MiB of each file | 5,000.0 MiB, all but 42 KB of it in `.git/lfs` | 553.7 MiB | 11.4 s (11.4–11.5) | 12.4 s (12.3–12.4) |
| One 200 MiB file in 50 versions, each overwriting 1% (eight runs of 256 KiB) | 299.2 MiB | 351.9 MiB | 48.2 s (48.0–48.5) | 47.2 s (46.7–49.0) |
| RT4-1: 100 commits sharing one 1 MB message | 765,269 bytes | 74,749,439 bytes | 1.0 s | — |

- The first two are built by `bench/data.sh` and timed like the steps above, in the same runs. The Git LFS repository is made with `git lfs track '*.bin'` and a commit per version. The 200 MiB file's history is written by `git fast-import`, which stores each version as a delta on the one before, up to 49 deep.
- **RT4-1** is attack code, so it runs only on a `redteam/` branch: [run 42611137068](https://github.com/WisHonor/Bolt/actions/runs/42611137068), job `rt4-1-sizes`, at `c519636`, image ubuntu-24.04 20261101.360.1. Its test, `rt4_1_a_small_history_without_merges_does_not_import_into_a_huge_bolt` in `crates/bolt-vcs/tests/redteam4.rs`, builds the repository and imports it; the import succeeded in 1.0 s with a peak of 112,283,648 bytes, and the test's bound (40 MiB) failed, as decision 087 expects:

  ```sh
  cargo test --release -p bolt-vcs --test redteam4 -- --ignored --nocapture --exact rt4_1_a_small_history_without_merges_does_not_import_into_a_huge_bolt
  ```

### git/git: main against milestone 5

The whole history of Git's own repository is imported by `bolt` built from `main` and from this branch, in turn, three times each on the same machine. After each import comes its first save: a line added to `README.md`, saved with `--accept-crlf`. That flag is needed because Git wrote three `.bat` files with CRLF, as their `.gitattributes` asks. The repository is a clone of `https://github.com/git/git` made at the run, at `fb25501` (2026-11-06): 86,059 commits and 1,022 refs. Time and peak memory are GNU `time`'s (`%e`, `%M`), and `run.sh import` has the commands. The branch's commits `c519636` and `a2ba223` both have `ba513aa`'s Bolt code.

| Machine and run | `bolt` from | Import: 3 runs | Mean | Peak memory | First save after it |
|---|---|---|---|---|---|
| AMD EPYC 7763, [run 42611120400](https://github.com/WisHonor/Bolt/actions/runs/42611120400) | `main` (`400e09e`) | 166.05, 164.36, 164.66 s | 165.0 s | 572.8–575.5 MB | 0.85, 0.84, 0.85 s |
| | this branch (`c519636`) | 165.05, 165.52, 164.72 s | 165.1 s (+0.04%) | 668.9–674.2 MB | 0.64, 0.64, 0.64 s |
| AMD EPYC 9V74, [run 42616742873](https://github.com/WisHonor/Bolt/actions/runs/42616742873) | `main` (`400e09e`) | 168.63, 168.78, 169.31 s | 168.9 s | 573.2–573.8 MB | 0.95, 0.94, 0.93 s |
| | this branch (`a2ba223`) | 171.31, 171.22, 170.92 s | 171.2 s (+1.3%) | 674.4–680.2 MB | 0.72, 0.73, 0.73 s |

- Milestone 5 imports as fast as before, within 1.3%.
- Its first save after an import is about a quarter faster, though it reads some files twice. The import leaves the file-state cache empty, so the save reads every file. Before that, milestone 5 also reads each file of at most 1,024 bytes that the imported save holds too, to see whether it is a Git LFS pointer standing in for that file ([decision 090](decisions/090-the-first-save-looks-at-the-whole-folder-for-pointers.md)). `main` has no such look. (The line-ending check, which also reads files, is skipped by `--accept-crlf` on both.)
- Its import's peak memory is higher: about 573 MB on `main` and 669–680 MB on milestone 5, about 100 MB more. Most likely it is the Git reader's cache of rebuilt objects. On milestone 5 that cache keeps a base in place when it uses it, and copies in each object a read rebuilds, so it stays near its fixed 256 MiB budget. On `main`, the object a read returned was never kept, and one the cache already held was taken out to be returned, so the cache drained. Nothing measured how full the cache was.
- Both machines are `ubuntu-latest`, image ubuntu-24.04 20261101.360.1, with 4 virtual CPUs. An earlier run of the same job, [run 42609326597](https://github.com/WisHonor/Bolt/actions/runs/42609326597), stopped at the CRLF files, before `--accept-crlf` was added. On its AMD EPYC 9V45, `main`'s first import took 101.3 s, with a peak of 571.6 MB.

## 4. The parts of a save

**In plain words.** A save does three things to each piece of a big file: finds where to cut, fingerprints it, and tries to shrink it. Timed alone, on one thread, they show which part is slow. Shrinking at zstd's level 19 is, by far.

| On one thread | Windows laptop | GitHub's Linux runner | GitHub's Windows runner |
|---|---|---|---|
| The cutter, on noise | 1,696 MB/s | 1,491 MB/s | 1,449 MB/s |
| BLAKE3, on noise | 4,394 MB/s | 3,526 MB/s | 3,552 MB/s |
| zstd level 3, on noise | 815 MB/s | 2,581 MB/s | 474 MB/s |
| zstd level 9, on noise | 1,168 MB/s | 1,388 MB/s | 949 MB/s |
| zstd level 19, on noise | 18 MB/s | 16 MB/s | 13 MB/s |
| zstd level 3, on sound | 975 MB/s | 786 MB/s | 614 MB/s |
| zstd level 9, on sound | 752 MB/s | 582 MB/s | 473 MB/s |
| zstd level 19, on sound | 13 MB/s | 11 MB/s | 10 MB/s |

- The cutter runs over all of the noise as one stream, as a save cuts a big file. BLAKE3 is fed the same noise 64 KiB at a time, and zstd compresses each 64 KiB piece on its own, about the size of a chunk.
- A MB is a million bytes. Noise is 256 MiB of BLAKE3's extendable output, and sound is 64 MiB of the WAV-like data of §5. How fast zstd gives up on noise depends on the level and the machine, so its noise figures say little about real data.
- The tests are `speeds_of_the_parts` and `what_compressing_again_would_save` in `crates/bolt-vcs-core/tests/throughput.rs`, ignored, run in a release build:

  ```sh
  cargo test --release -p bolt-vcs-core --test throughput -- --ignored --nocapture --test-threads=1
  ```

  with `BOLT_BENCH_CODE` and `BOLT_BENCH_GODOT` naming the code set and Godot's demos. On the runners these are the sets of §2, in the same runs. On the laptop, the code set is the same tarball, unpacked by Git's bash (its links as copies), and Godot's demos a sparse checkout of their PNG and OGG files: `git clone --depth 1 --branch 4.7-6ad6167 --filter=blob:none --sparse https://github.com/godotengine/godot-demo-projects`, then `git sparse-checkout set --no-cone '*.png' '*.ogg'`. Both were in a temporary folder, deleted afterwards.
- **The laptop:** the one of §1 (AMD Ryzen 7 7730U, 16 GB, Windows 11 Home 10.0.26200), 2026-11-09, with `-j 2`, the test file as committed, and Bolt's code at `ba513aa`.

## 5. Compressing again: the ruling

**In plain words.** Bolt stores what it saves compressed with zstd at level 3, which is fast. When it merges packs or `tidy` rewrites them, it could compress the same bytes again, harder, to save disk. [Decision 088](decisions/088-benchmarks-and-the-speed-gates.md) set the bar: only if level 9 or 19 saves at least 10% of the stored bytes of code and uncompressed assets, in no more than 5 times level 3's time. Neither does, so nothing changes: merging and `tidy` go on copying stored bytes as they are, and [storage-format.md](storage-format.md) stays as it is.

What each level stores, on the laptop:

| Set | Bytes | Level 3 | Level 9 | Level 19 |
|---|---|---|---|---|
| Code (4,964 files, 4,978 pieces) | 52,458,897 | 15,305,368 in 0.23 s | 14,088,700 in 1.06 s | 13,414,907 in 13.09 s |
| Uncompressed assets (793 pieces) | 67,108,864 | 56,081,421 in 0.11 s | 55,417,364 in 0.45 s | 51,674,617 in 5.54 s |
| The two together | 119,567,761 | 71,386,789 in 0.34 s | 69,506,064 in 1.51 s | 65,089,524 in 18.63 s |
| Compressed assets (521 files, 989 pieces), not counted by the rule | 71,006,988 | 70,642,607 in 0.08 s | 70,564,734 in 0.28 s | 70,423,531 in 4.43 s |

- **Level 9** stores the code and uncompressed sets in 2.6% fewer bytes than level 3 (1,880,725 of 71,386,789), in 4.4 times its time: within the time, far short of 10%.
- **Level 19** saves 8.8% (6,297,265 bytes), in 53.5 times level 3's time: short of 10%, and more than ten times the time allowed.
- Set by set it is no closer. Level 9 saves 7.9% of the code set and 1.2% of the uncompressed one. Level 19 saves 12.4% of the code set, but in about 57 times level 3's time.
- GitHub's runners store exactly the same bytes as the laptop, except the code set on Linux, whose three links aren't followed (4,850 files). On the runners, level 9 saves 2.6% in 4.0 times level 3's time on Linux, and in 3.8 times on Windows. Level 19 saves 8.8%, in 60.4 and 49.5 times.

How it is measured: each set is cut into the pieces a save stores, a file under 1 MiB whole and a bigger one into chunks by Bolt's own cutter. Each piece is compressed on its own at each level with zstd's one-shot compression, as a pack stores it. It is kept compressed only when that is under 95% of its size. The time is the compressing alone, on one thread. The test's own verdict lines add up the times before rounding them. Repeated pieces are counted each time, as they would be compressed again. The sets:

- **Code:** Git's source at `v2.56.0`, as in §2.
- **Uncompressed assets:** two generated files of 32 MiB each, standing in for WAV and BMP files. The first holds 16-bit stereo samples at 44.1 kHz: three tones under a noise of about 6 bits. The second holds 24-bit pixels in rows 4,096 wide: smooth shading with a noise of about 3 bits, a quarter of it flat colour. Their bytes are the same on every machine the test ran on.
- **Compressed assets:** the PNG and OGG files of Godot's demo projects at `4.7-6ad6167`.

## 6. How much GitHub's numbers vary

- **Within one run,** for half the steps the slowest timed run is within 3.1% of the fastest on the Linux runner, and within 3.8% on the Windows one. For nine steps in ten it is within 22% and 44%. The widest are steps of a few milliseconds, checkouts, and Git's commits on Windows that repack.
- **From one machine to another,** much more: from run to run, `ubuntu-latest` was an AMD EPYC 7763, 9V45 or 9V74, and `windows-latest` an AMD EPYC 7763 or 9V74 or an Intel Xeon Platinum 8573C. `main`'s import of git/git took 101.3 s on an EPYC 9V45, 165.0 s on an EPYC 7763 and 168.9 s on an EPYC 9V74; nightly runs on `main` have taken 95 to 168 s. The memory gate's 10 GiB save took 28.6 to 28.7 s in three runs on EPYC 7763s (runs 42598494181, 42609326597 and 42611120400). It took 55.1 s on an EPYC 9V74 (run 42616742873). Its peak memory was 43,372,544 to 43,515,904 bytes in all four.
- **On the same kind of machine** the numbers agree: the code and Godot sets' first saves, in two Linux runs on EPYC 7763s (runs 42609326597 and 42616742873), came within 1% of each other for each tool.
- **A run not published:** [run 42611120400](https://github.com/WisHonor/Bolt/actions/runs/42611120400) timed the same steps, at `c519636`, before two fixes to the method. Git's background repack was still running during the later steps, competing with them, and on Linux the versions set's `.git` measured 17.01 GiB for 10 versions. And `status` started within Bolt's 2-second window, so Bolt read the file just changed again: on Linux, 150 ms for the repeated set, where its steady state is about 3 ms. Its against-git tables were replaced by run 42616742873's. Its git/git import is in §3, and its memory gate in this section.
- So compare tools within one table, which is one machine and one run; between tables, differences of up to about 2 times can be the machine.

## 7. Where the time went, before milestone 6

**In plain words.** Before making anything faster, Bolt was timed from the inside: a throwaway build printed how long each part of `status`, a small save and `goto` took, on GitHub's runners and on the Windows laptop. On a small project nearly all of the time is the walk of the project folder, which one thread does a folder and a name at a time. Most of that walk is work Git doesn't do, or does once. Bolt's own look at each file, which several threads share, is never what a command waits for. On big files the time is the one flush to the disk at the very end. These are the numbers [decisions 099](decisions/099-a-third-speed-gate-within-twice-gits-time.md) to [102](decisions/102-big-files-are-flushed-while-they-are-written.md) start from.

### How it was measured

- **The build:** `main`'s code at `a76f635` with timers added, which print to stderr when a variable is set, on the branch `bench/m6b-profile` (since deleted). The timers cost a few percent: `status` on Git's source took 63.3 ms without them printing and about 65 ms with.
- **Small projects:** [run 42753171139](https://github.com/WisHonor/Bolt/actions/runs/42753171139), 2026-11-10, at `b0e849b`. Git's source and Godot's demos, set up as §2's steps set them up (`bolt start`, the first save, a line added to `README.md`, saved). Each step was then timed with `hyperfine` (30 runs of `status`, 15 of the others, run directly, with no shell) and run three times with the timers printing. The three agreed within about 3%; the tables give one of them. Linux: image ubuntu-24.04 20261101.360.1, AMD EPYC 7763, 4 virtual CPUs, Git 2.56.0. Windows: image windows-2025-vs2026 20261103.298.1, AMD EPYC 9V74, 4 virtual CPUs, Windows Defender's real-time protection off, Git 2.56.0.windows.4. On Linux `strace` also counted the system calls; `perf` wasn't on the runner, so there is no profile by function.
- **Big files:** [run 42755560911](https://github.com/WisHonor/Bolt/actions/runs/42755560911), 2026-11-10, at `79c03de`, both runners on AMD EPYC 7763s, Git LFS 3.8.1. §2's noise, versions and repeated sets, each step run once (twice for going back), so these are single runs, not means.
- **The laptop:** §1's (AMD Ryzen 7 7730U, Windows 11, its virus scanner on), Git 2.50.1, each command timed 10 to 20 times by a script.

### `status` with nothing changed, on Git's source

4,850 files in 226 folders on Linux; 4,964 in 233 on Windows. Times are milliseconds.

| | Linux runner | Windows runner | Windows laptop |
|---|---|---|---|
| The whole command, Bolt | 63.3 | 101.8 | 139 |
| The whole command, Git | 13.0 | 33.4 | 43.5 |
| Before the walk: opening the project, the drive's clock and name test, loading the file cache | 5.6 | 7.6 | 14.4 |
| of which loading the file cache | 2.7 | 2.2 | 3.1 |
| **The walk, on one thread** | **44.5** | **71.8** | **138** |
| matching every name against every ignore line in force (Git's own `.gitignore` has 263) | 17.7 | 16.2 | 25.0 |
| opening and listing each folder | 5.6 | 11.5 | 33.0 |
| reading the last save's listing of each folder from the store | 5.8 | 10.4 | 17.5 |
| opening each folder and asking it for `.git` and `.bolt` by name | 1.2 | 6.7 | 25.0 |
| trying to open `.gitignore` and `.boltignore` in each folder | 1.4 | 4.5 | 15.4 |
| checking each name (the name rules, `.git` and `.bolt` spellings) | 2.3 | 3.7 | 3.9 |
| handing files to the looking threads, and waiting for room | 1.0 | 4.9 | 1.7 |
| the rest: building paths, keeping the books | 9.5 | 13.9 | 16.9 |
| The looks at each file, summed over the looking threads (run beside the walk, not after it) | 10.7 | 104.8 | 292 |
| Filling in the looks, sorting, finding gone folders | 2.3 | 2.1 | 2.7 |
| Comparing each file with the cache and building the listings | 8.6 | 7.9 | 11.3 |
| Writing the file cache again | 2.4 to 4.0 | 2.3 | 5.2 |
| The rest: comparing with the save, printing, freeing memory | 1.5 | 1.9 | 2.9 |

- **A look at one file** takes 2.2 µs on Linux (one call), 21.6 µs on the Windows runner and 60 µs on the laptop (five calls: open, three questions, close; the laptop's virus scanner sees each open). The looking threads finished within a millisecond of the walk on every machine, so the walk is what `status` waits for.
- **System calls on Linux:** Bolt made 5,311 lookups of an entry's details (453 of them the failed lookups of `.git` and `.bolt`), 937 opens (419 failed, for ignore files that aren't there), 1,583 reads from packs (7 for each folder listing), 511 folder reads and 4 flushes. Git made 5,157 lookups, 334 opens, 457 folder reads and no flush.
- **What Git doesn't do.** Git matches ignore lines only against names it doesn't track, and keeps one index where Bolt reads and checks a listing for each folder. Git looks at each file as Bolt does, on several threads.

### Godot's demos

4,018 files in 569 folders, so the cost of each folder shows more.

| | Linux runner | Windows runner |
|---|---|---|
| The whole command, Bolt | 65.8 | 137.9 |
| The whole command, Git | 13.8 | 33.0 |
| The walk, on one thread | 48.9 | 107.3 |
| opening and listing each folder | 10.5 | 27.5 |
| reading the last save's listing of each folder | 12.6 | 26.6 |
| opening each folder and asking it for `.git` and `.bolt` | 3.2 | 17.0 |
| trying to open the two ignore files in each folder | 3.1 | 8.5 |
| matching ignore lines | 3.3 | 3.6 |
| handing files to the looking threads | 2.6 | 7.3 |

### A small save and going back, on Git's source (Linux runner)

- **A small change, saved:** 69.9 ms against Git's 29.7. The same walk (44.3 ms). Before it, 8 ms: the lock is taken at 2.8 ms, then the save it builds on, the drive's clock and the file cache are read. After it, 1.8 ms of checks, 9.4 ms reading the changed file and storing listings, 1.8 ms writing the pack and moving `head`, 2.9 ms writing the two caches, and 1.5 ms counting what changed.
- **Back to the first version:** 71.0 ms at best, against Git's 13.4. The lock is taken at 3.3 ms. The scan of the folder, from the drive's clock to the file cache written, takes 62.3 ms, the same work as `status`. Then 2.3 ms comparing the three saves, 1.5 ms on the saves a switch records, 0.8 ms storing the diary entry, 0.6 ms writing the journal, and 3.3 ms carrying the switch out, of which 2.7 ms is loading the file cache a second time. (The mean was 89.4 ms: the profile had made more than 32 small packs, so some runs merged them. §2's steps make 8.)
- **Flushes:** an unchanged `status` flushes the disk 4 times, a save 12 times and a `goto` 16 times on Linux, at 0.2 to 0.7 ms each on that runner. Git flushes nothing on these steps.
- **The Windows runner** shows the same shape: a small save reaches the walk at 8.6 ms and spends 72.3 ms in it.

### What the third gate needs

Git's `status` takes 13 ms on the Linux runner, so the gate allows Bolt 26. Going back to the first version is the hard one: Git takes 13 to 15 ms, so Bolt has 27 to 30, and a switch must also store a diary entry, write a journal, write the file and move `head`, each flushed. That leaves the scan of the folder about 15 to 19 ms, close to what Git's own `status` takes. On Windows the gate allows about 67 to 91 ms for `status` and 110 ms for going back, and the looks alone, at 21.6 µs a file on four threads, need about 26 ms.

### Big files

One run each, in seconds. "Flush" is the one flush of the pack, or of the file written, at the end.

| Step | Linux runner | Windows runner |
|---|---|---|
| First save, 1,000 MiB of noise in 20 files | 4.19: reading, cutting, hashing and writing 1.89, then the flush 2.29 | 4.52: 2.27, then 2.24 |
| the same, Git and Git LFS | 28.2 and 2.51 | 33.0 and 3.40 |
| First save, one 1 GiB file of noise | 4.28: 1.93, then 2.34 | 4.96: 2.81, then 2.14 |
| the same, Git LFS | 2.27 | 3.30 |
| Back to the first version of the 1 GiB file | 3.86: copying 13,546 chunks out 1.33, the flush 2.35 | 4.12: 1.68, and 2.35 |
| the same, right after another switch wrote the file | 4.39: 0.66 more, reading the file again | 4.39: 0.41 more |
| the same, Git, first and second time | 1.10 and 3.80 | 1.67 and 4.82 |
| Back to the first version of the repeated 1 GiB file | 4.48: copying 2.11, the flush 2.31 | 4.72: 2.28, and 2.33 |
| the same, Git, first and second time | 0.97 and 3.78 | 1.66 and 4.90 |

- **The flush is over half of each step,** and it starts only when everything is written: while Bolt reads and hashes, the disk has nothing to do. Git LFS's `git add` never flushes, and neither does Git's checkout (§2, "Waiting for the disk").
- **Copying a file out** moves each chunk 8 KiB at a time through two fingerprints, one for the chunk and one for the whole file: 151,653 small steps for 13,546 chunks. The repeated file is slower because a chunk already checked once takes a slower path.
- **Right after a switch,** the next one reads the file the first wrote again, since it is too new for the cache to vouch for (the 2-second rule). Git checks such a file again too.
- **Trying to compress noise** costs 0.5 s of processor time on Linux and 1.5 to 1.8 s on Windows for 1 GiB, spread over the worker threads.

### Where the space goes on Godot's demos

Every file was stored each way and added up by kind of file (a throwaway test on the same branch, on the laptop; repeated files are counted each time, so the totals are above §2's):

| | Bytes |
|---|---|
| The files | 312,757,968 |
| Bolt: zstd level 3 on each piece, kept when under 95% | 258,661,764, and 103,440 of chunk lists |
| the same, kept whenever smaller | 257,606,006 |
| zstd level 9 on each piece | 252,246,350 |
| zstd level 3 on each whole file | 250,667,764 |
| zlib level 6 on each whole file, as Git stores it | 249,580,064 |

- Bolt keeps 9.1 MB more than zlib would. 7.0 MB of that is six `.hdr` images of 1 MiB or more: 108.1 MB that Bolt keeps in 96.2 MB and zlib in 89.2 MB. The next kinds are `.exr` images (0.4 MB), `.glb` models (0.3 MB) and `.res` files (0.3 MB).
- On these images zlib's level 6 simply compresses better than zstd's level 3, whole or in pieces. zstd's level 9 on each piece would still leave Bolt 1.1% behind zlib.
- The 95% rule costs 1.1 MB, 0.4%.
