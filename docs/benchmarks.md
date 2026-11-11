# Bolt Benchmarks

> **Status:** started 2026-11-09 with the two speed gates for v0.1; Bolt against Git and Git LFS, the speeds of a save's parts and the ruling on compressing again were added the same day. A third gate, against Git on a code project, and the profile milestone 6 starts from (§7) were added on 2026-11-10; the numbers in §1 to §6 are from before milestone 6's speed work, and §8, added the same day, gives them after it; §9, added that day too, measures both halves of milestone 6 together, on the code v0.1 is made from. Every speed or size claim Bolt makes comes from this page, the ones Git or Git LFS wins included ([decision 088](decisions/088-benchmarks-and-the-speed-gates.md)).

**In plain words.** Bolt promises three things before v0.1 (decisions [023](decisions/023-v0.1-speed-bar.md) and [099](decisions/099-a-third-speed-gate-within-twice-gits-time.md)): saving a huge file must not eat the computer's memory, asking what changed in a big project must be quick, and on an everyday code project Bolt must stay within twice Git's time. This page says how each was measured, on which machines, and what came out, pass or fail. It then puts Bolt beside Git and Git LFS on the same files, step by step, and says where each one wins.

## 1. The speed gates

- **Memory:** saving a 10 GiB file of incompressible bytes peaks at no more than 500,000,000 bytes of memory.
- **Status:** `bolt status` on an unchanged 100,000-file project takes under 2 s. From v0.1 the nightly job fails at 2 s or more ([decision 129](decisions/129-gotos-limit-until-v0.2-and-the-status-gate-enforced.md)).
- **Against Git** ([decision 099](decisions/099-a-third-speed-gate-within-twice-gits-time.md)): on Git's source at `v2.56.0` (§2's code set), `status` with nothing changed, a small change saved, and going back to the first version each take Bolt at most twice Git's time, on GitHub's Linux runner and on its Windows runner. It is measured as §2 measures, with 10 timed runs of each step, by `bench/run.sh gate`, which compares the medians and fails when a step is over ([decision 108](decisions/108-the-third-gate-compares-medians.md): a flush now and then stalls for 60 ms or more on GitHub's Linux runner, and Git never flushes); each step's mean, best and worst run are printed beside the medians. **For v0.1** ([decision 128](decisions/128-v0.1-ships-without-goto-within-twice-gits-time.md)) `status` and the small save keep the limit of twice Git's time, and going back to the first version is held to three and a half times ([decision 132](decisions/132-gotos-limit-is-three-and-a-half-times-gits-time.md), picked from the eleven runs on record, whose worst was 3.01): twice is its goal for v0.2, and the job says when it is over the goal.

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

**In plain words.** Before making anything faster, Bolt was timed from the inside: a throwaway build printed how long each part of `status`, a small save and `goto` took, on GitHub's runners and on the Windows laptop. On a small project nearly all of the time is the walk of the project folder, which one thread does a folder and a name at a time. Most of that walk is work Git doesn't do, or does once. Bolt's own look at each file, which several threads share, is never what a command waits for. On big files the time is the one flush to the disk at the very end. These are the numbers [decisions 099](decisions/099-a-third-speed-gate-within-twice-gits-time.md) to [103](decisions/103-where-git-keeps-less-zstd-level-3-and-the-chunk-sizes-stay.md) start from.

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

## 8. After milestone 6's speed work

**In plain words.** Milestone 6 set out to stop losing to Git on an everyday code project, and to Git LFS on big files. On Git's own source Bolt now asks "what changed?" in about one and a half times Git's time where it took four times, saves a small change as fast as Git, and goes back to another version in twice Git's time or less where it took four and a half times. On a long history, commands that took from half a second to three seconds take from a few milliseconds to a quarter of a second, and amending the newest save is quicker than Git's own amend. Big files are saved in about 60% of the time wherever the disk keeps up and little faster where it doesn't, and Git LFS still wins their first save on a slow disk, because Bolt waits for the disk and Git LFS doesn't. The new gate (within twice Git's time) is not met yet: `status` and a small save pass in every run but one on Windows, and `goto` on Linux sits at twice Git's time or over, so it passed in only two runs of seven. This section gives every number, the losses included.

Everything here was measured on 2026-11-10. The code is commit `b8fccef` unless a line names another; the commits after it change only how the check before an amend treats a cache that lies (what it does with an honest one is the same code), tests and docs. "Before" is `main` at `73a1a0d`, the code §1 to §7 measured.

### What changed

In the order the work was done ([the plan](plans/2026-11-10-milestone-6-speed-and-space.md)); none of it changes a stored ID, a format, or what a command says:

- **Ignore rules.** Lines that are plain names are found by a hash lookup, and a line with one `*` is matched by the name's two ends. A file the last save holds isn't matched at all until it changes ([decision 107](decisions/107-a-saved-files-ignore-rules-are-read-only-when-it-changed.md)).
- **The file cache** is loaded as one block by a helper thread while the walk runs, asked once for each file, written only when it changed, and never flushed ([decision 100](decisions/100-caches-are-written-without-a-flush-and-only-when-they-changed.md)).
- **The walk** lists a folder before it asks it for anything by name ([decisions 101](decisions/101-the-walk-lists-a-folder-before-it-looks-inside.md) and [106](decisions/106-on-linux-and-macos-a-folder-is-still-asked-for-git-and-bolt-by-name.md)), and does each name's work once.
- **The store** finds and reads a small record in three reads where it took a dozen, and answers small reads that sit side by side from the last block it read of each pack ([decisions 111](decisions/111-rulings-after-the-first-red-team-session-on-the-speed-work.md) and [112](decisions/112-rulings-after-the-second-red-team-session-on-the-speed-work.md) say when a block is read).
- **Listings.** A folder whose entries all match the last save's takes that listing's ID; the scan hands the last save's listings on to the comparisons after it; and those listings are read just ahead of the walk by a helper thread ([decision 110](decisions/110-the-last-saves-listings-are-read-ahead-of-the-walk.md)).
- **Big files.** A pack, or a file a switch writes, is flushed while it is written ([decision 102](decisions/102-big-files-are-flushed-while-they-are-written.md)); a switch reads each chunk whole and keeps the chunks it has checked while it writes one file.
- **Long histories.** `cache/change-ids` keeps each line's saves in order ([decision 105](decisions/105-the-change-id-cache-keeps-each-lines-saves-in-order.md)), and the check before an amend keeps what it read of the other lines in `cache/other-lines` ([decision 112](decisions/112-rulings-after-the-second-red-team-session-on-the-speed-work.md)).
- **The import** keeps 96 MiB of rebuilt objects where it kept 256 MiB ([decision 104](decisions/104-an-import-keeps-96-mib-of-rebuilt-objects.md)).
- **Deep folders.** Past the levels the walk holds open, every 48th level stays open too ([decision 109](decisions/109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md)).

### Before and after on one runner

GitHub's runners differ from one another by more than some of these changes, in their processors and above all in their disks (§6). So the fairest comparison builds `main` and the branch side by side and runs them turn by turn on one machine, with Git on the same files: [run 42910259739](https://github.com/WisHonor/Bolt/actions/runs/42910259739), a throwaway workflow on a branch since deleted. Git's source at `v2.56.0`, set up as §2 sets it up; each number is the median of 40 runs (`status`) or 15 (the others), timed from a shell, so each holds the millisecond or two a shell takes to start a program, which §2's numbers don't.

| Step, on Git's source | Linux: before, after, Git | Times Git's: before, after | Windows: before, after, Git | Times Git's: before, after |
|---|---|---|---|---|
| `status`, nothing changed | 50.0 ms, 17.9 ms, 12.0 ms | 4.2, 1.5 | 127.2 ms, 84.5 ms, 60.8 ms | 2.1, 1.4 |
| A small change, saved | 53.9 ms, 23.8 ms, 24.6 ms | 2.2, 0.97 | 157.4 ms, 108.0 ms, 134.9 ms | 1.2, 0.80 |
| Back to the first version | 55.8 ms, 24.6 ms, 12.3 ms | 4.5, 2.0 | 149.7 ms, 97.3 ms, 68.6 ms | 2.2, 1.4 |
| The first save of all 4,853 files | 263 ms, 246 ms, 1,216 ms | 0.22, 0.20 | 552 ms, 481 ms, 3,599 ms | 0.15, 0.13 |

### The three gates

The first two gates as measured again on the finished code ([run 42910249822](https://github.com/WisHonor/Bolt/actions/runs/42910249822)): saving 10 GiB peaked at 43,667,456 bytes and took 26.2 s on GitHub's Linux runner (the limit is 500,000,000; the time is that runner's disk, and was 28.7 s on the runner of §1). `status` on an unchanged 100,000-file project, whose limit is 2 s, took 0.20 s twice on GitHub's Linux runner, where `main` took 0.61 s and 0.66 s on the same machine, and 1.02 s then 0.92 s on its Windows runner, where `main` took 1.33 s and 1.32 s ([run 42910259739](https://github.com/WisHonor/Bolt/actions/runs/42910259739)).

The third gate, by the `speed-gate` job (`bench/run.sh gate`: medians of 10 runs, timed by `hyperfine` with no shell), in every run of the code from `ecdfb17` on:

| Run, commit | Linux: Bolt, Git, times Git's | Windows: Bolt, Git, times Git's |
|---|---|---|
| [42834201117](https://github.com/WisHonor/Bolt/actions/runs/42834201117), `ecdfb17` | `status` 23.1 ms, 12.8 ms, 1.80; small save 30.2 ms, 29.3 ms, 1.03; `goto` 33.7 ms, 15.9 ms, **2.12 (over)** | `status` 72.9 ms, 43.5 ms, 1.68; small save 82.3 ms, 123.3 ms, 0.67; `goto` 86.9 ms, 53.5 ms, 1.62 |
| [42839335754](https://github.com/WisHonor/Bolt/actions/runs/42839335754), `ecdfb17` | `status` 21.6 ms, 13.4 ms, 1.62; small save 28.4 ms, 30.6 ms, 0.93; `goto` 30.6 ms, 17.1 ms, 1.79 | `status` 84.2 ms, 47.8 ms, 1.76; small save 94.1 ms, 130.7 ms, 0.72; `goto` 100.5 ms, 58.3 ms, 1.72 |
| [42889870154](https://github.com/WisHonor/Bolt/actions/runs/42889870154), `eaf55ee` | `status` 16.8 ms, 9.8 ms, 1.71; small save 23.4 ms, 22.2 ms, 1.05; `goto` 25.3 ms, 12.5 ms, **2.02 (over)** | `status` 52.6 ms, 29.7 ms, 1.77; small save 52.6 ms, 84.4 ms, 0.62; `goto` 59.0 ms, 36.5 ms, 1.62 |
| [42887158889](https://github.com/WisHonor/Bolt/actions/runs/42887158889), in a whole-benchmark run, `eaf55ee` | `status` 17.7 ms, 11.0 ms, 1.60; small save 23.7 ms, 25.5 ms, 0.93; `goto` 26.9 ms, 13.6 ms, 1.99 | `status` 103.1 ms, 46.0 ms, **2.24 (over)**; small save 116.0 ms, 127.8 ms, 0.91; `goto` 121.5 ms, 55.5 ms, **2.19 (over)** |
| [42910249929](https://github.com/WisHonor/Bolt/actions/runs/42910249929), `b8fccef` | `status` 18.3 ms, 10.1 ms, 1.81; small save 24.8 ms, 25.7 ms, 0.97; `goto` 27.3 ms, 11.4 ms, **2.40 (over)** | `status` 82.5 ms, 44.6 ms, 1.85; small save 91.5 ms, 125.9 ms, 0.73; `goto` 95.6 ms, 56.3 ms, 1.70 |
| [42910249822](https://github.com/WisHonor/Bolt/actions/runs/42910249822), in the whole-benchmark run, `b8fccef` | `status` 14.5 ms, 8.4 ms, 1.73; small save 19.6 ms, 20.4 ms, 0.96; `goto` 22.9 ms, 11.0 ms, **2.08 (over)** | `status` 81.8 ms, 45.5 ms, 1.80; small save 90.0 ms, 125.8 ms, 0.72; `goto` 94.9 ms, 55.2 ms, 1.72 |
| [42921981143](https://github.com/WisHonor/Bolt/actions/runs/42921981143), `24bf89d` (the branch's last code, with this page) | `status` 21.2 ms, 12.5 ms, 1.70; small save 28.6 ms, 29.1 ms, 0.98; `goto` 31.8 ms, 12.7 ms, **2.50 (over)** | `status` 79.1 ms, 44.9 ms, 1.76; small save 89.1 ms, 124.1 ms, 0.72; `goto` 94.1 ms, 54.7 ms, 1.72 |

- **`status` and the small save pass on Linux in all seven runs** (1.60 to 1.81 times Git's time, and 0.93 to 1.05).
- **Linux's `goto` passes in two runs of the seven** (1.79 and 1.99) and misses in five (2.02 to 2.50). Turn by turn on one runner it is twice Git's time (the table above).
- **Windows passes all three steps in six runs of the seven,** and misses `status` and `goto` in one (2.24 and 2.19).
- **Why Linux's `goto` sits on the limit.** Bolt's `goto` flushes ten times (the drive clock's file, the diary's pack and its folder, the journal and its folder, the file and its folder, `head` and its folder, and the folder again once the journal is gone); Git's checkout flushes nothing. On the runner where one `goto` was traced, the ten flushes took 2.6 ms of its 25 and its seven renames a millisecond more ("Where the time goes now", below), a seventh of the command between them. On a disk whose flushes take longer, they are more.
- **Why Windows can miss it.** A look at one file costs Bolt five system calls there, and it spreads the looks over several threads; Git makes fewer calls, on one thread. In the run that missed, Bolt's ten `status` runs spread from 96 to 126 ms while Git's stayed within 44 to 49. The likely reading is a runner busy with something else, which several threads feel more than one does; that reading wasn't tested.
- **So the gate isn't met yet as decision 099 states it** (on GitHub's Linux and Windows runners, every run). What would settle each step is in "Where Git or Git LFS still wins" below; none of it was done here, because each touches a flush a crash relies on, the module that decides what is saved, or `unsafe` code the owner reviews.

### Against Git and Git LFS, again

§2's whole table, measured the same way by the same workflow on the finished code ([run 42910249822](https://github.com/WisHonor/Bolt/actions/runs/42910249822)): means of 5 runs, with the lowest and the highest. These runners are not the ones §2's numbers came from, so set each number beside the Git and Git LFS numbers of its own row before setting it beside §2's.

Two things to know when reading them:

- **Going back, on the big-file sets,** reads again whatever the step before wrote less than two seconds earlier: the cache doesn't vouch for a file that young (the 2-second rule), and Bolt won't write over a file it hasn't shown to be saved. So that step is slower the faster the step before it was.
- **The first save of big files that don't compress** (the noise and versions sets) is as fast as the runner's disk takes a gigabyte, because Bolt flushes the pack before it names it. "Big files on one runner" below shows the same code beside Git LFS on a fast disk and on a slow one. Both runners of this run had slow ones.
- **A mean of five runs holds any stall.** On this Linux runner a flush stalled in several of Bolt's steps (a small change on the code set ran from 23.6 to 130 ms) and in some of Git's (Godot's small change ran from 23.9 to 243 ms). The range beside each mean shows it; the gate's medians of ten runs, from the same run, are in the table above.

#### GitHub's Linux runner

**Code**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 350 ms (212–589) | 1.22 s (1.20–1.24) | 1.26 s (1.24–1.27) |
| A small change | 63.5 ms (23.6–130) | 26.0 ms (25.4–27.2) | 57.5 ms (56.4–59.1) |
| `status`, nothing changed | 19.1 ms (17.3–25.5) | 11.6 ms (11.3–11.8) | 12.2 ms (11.1–13.5) |
| `status` after touching the biggest file | 19.8 ms (19.2–20.3) | 15.7 ms (15.1–16.8) | 15.7 ms (15.4–16.1) |
| Back to the first version | 25.0 ms (24.2–25.8) | 14.1 ms (13.9–14.4) | 45.0 ms (44.1–46.6) |

**Godot's demo projects**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 1.10 s (1.07–1.15) | 7.21 s (7.20–7.23) | 3.05 s (3.02–3.06) |
| A small change | 30.6 ms (30.0–31.0) | 69.8 ms (23.9–243) | 54.2 ms (53.4–56.0) |
| `status`, nothing changed | 25.9 ms (25.2–26.7) | 11.7 ms (11.3–12.6) | 11.2 ms (10.5–12.2) |
| `status` after touching the biggest file | 52.8 ms (34.9–121) | 76.0 ms (74.5–78.5) | 88.0 ms (87.5–88.8) |
| Back to the first version | 48.5 ms (39.0–77.5) | 22.3 ms (9.56–71.8) | 56.4 ms (41.2–115) |

**Noise**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 4.76 s (3.84–5.11) | 24.4 s (24.4–24.4) | 1.83 s (1.81–1.85) |
| A small change | 62.2 ms (61.3–62.6) | 1.23 s (1.22–1.24) | 233 ms (152–254) |
| `status`, nothing changed | 6.18 ms (2.25–13.4) | 1.53 ms (1.43–1.77) | 1.56 ms (1.45–1.75) |
| `status` after touching the biggest file | 15.0 ms (14.9–15.1) | 112 ms (112–112) | 117 ms (116–119) |
| Back to the first version | 238 ms (229–255) | 181 ms (159–269) | 238 ms (216–313) |

**Versions**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 5.20 s (5.13–5.34) | 30.9 s (30.8–31.0) | 1.91 s (1.87–1.97) |
| A small change | 1.18 s (1.15–1.24) | 31.0 s (30.9–31.2) | 1.87 s (1.82–1.92) |
| `status`, nothing changed | 2.27 ms (2.20–2.39) | 1.86 ms (1.45–2.65) | 1.51 ms (1.44–1.68) |
| `status` after touching the biggest file | 268 ms (268–269) | 2.32 s (2.31–2.33) | 1.96 s (1.93–2.01) |
| Back to the first version | 4.81 s (4.21–5.62) | 3.25 s (3.22–3.28) | 3.45 s (1.59–6.88) |

**Repeated**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 1.20 s (1.15–1.32) | 30.0 s (29.6–31.0) | 1.89 s (1.86–1.95) |
| A small change | 1.11 s (1.09–1.14) | 30.0 s (29.8–30.2) | 1.90 s (1.88–1.93) |
| `status`, nothing changed | 2.48 ms (2.24–3.05) | 1.49 ms (1.43–1.63) | 1.87 ms (1.45–2.25) |
| `status` after touching the biggest file | 250 ms (249–250) | 2.28 s (2.27–2.30) | 1.99 s (1.97–2.02) |
| Back to the first version | 4.68 s (4.44–5.15) | 3.18 s (3.16–3.22) | 3.52 s (3.47–3.63) |

**Imports in the same job**

- lfs-import: 8.85 s (8.78–8.91)
- history-import: 40.4 s (40.2–40.6)

**Space**

| Set | The files | Bolt | Git | Git with Git LFS |
|---|---|---|---|---|
| Code | 50,141,205 | 16,086,209 | 17,122,254 | 17,123,859 |
| Godot's demo projects | 312,757,968 | 237,701,369 | 229,915,518 | 276,207,113 |
| Noise | 1,048,576,000 | 1,051,271,584 | 1,363,601,944 | 1,363,192,300 |
| Versions | 1,073,741,824 | 1,091,689,096 | 10,740,917,054 | 10,737,640,524 |
| Repeated | 1,073,741,824 | 5,504,717 | 7,518,523,838 | 7,516,228,782 |
- history-bolt: 368,998,025 bytes
- history-git: 313,774,870 bytes
- lfs-bolt: 580,585,708 bytes
- lfs-git: 5,242,922,183 bytes
- lfs-git-lfs-objects: 5,242,880,000 bytes

#### GitHub's Windows runner

**Code**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 424 ms (410–449) | 3.42 s (3.38–3.46) | 3.69 s (3.63–3.76) |
| A small change | 71.3 ms (68.7–75.5) | 105 ms (100–110) | 382 ms (361–454) |
| `status`, nothing changed | 65.6 ms (62.3–77.1) | 38.2 ms (36.0–39.9) | 36.5 ms (33.9–42.6) |
| `status` after touching the biggest file | 65.4 ms (63.0–69.6) | 44.2 ms (42.7–46.1) | 41.1 ms (38.9–42.3) |
| Back to the first version | 74.1 ms (71.5–76.4) | 46.4 ms (45.0–48.5) | 287 ms (276–299) |

**Godot's demo projects**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 1.78 s (1.75–1.82) | 10.5 s (10.4–10.8) | 6.75 s (6.57–7.01) |
| A small change | 81.6 ms (78.4–90.5) | 103 ms (102–104) | 387 ms (375–398) |
| `status`, nothing changed | 72.5 ms (71.4–73.1) | 37.1 ms (35.5–40.7) | 36.6 ms (35.8–37.4) |
| `status` after touching the biggest file | 84.8 ms (84.0–85.3) | 117 ms (115–119) | 293 ms (277–307) |
| Back to the first version | 82.0 ms (77.1–86.6) | 44.9 ms (42.7–47.9) | 308 ms (291–317) |

**Noise**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 5.48 s (5.42–5.53) | 28.0 s (27.6–28.5) | 2.48 s (2.45–2.51) |
| A small change | 103 ms (99.8–108) | 1.52 s (1.46–1.58) | 650 ms (584–881) |
| `status`, nothing changed | 16.1 ms (14.2–17.3) | 26.5 ms (24.1–28.1) | 26.6 ms (24.6–28.6) |
| `status` after touching the biggest file | 31.1 ms (29.4–32.2) | 163 ms (161–166) | 313 ms (306–327) |
| Back to the first version | 275 ms (268–285) | 257 ms (234–337) | 693 ms (673–726) |

**Versions**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 5.62 s (5.55–5.65) | 28.4 s (28.3–28.6) | 2.50 s (2.48–2.52) |
| A small change | 1.38 s (1.35–1.41) | 37.8 s (28.2–52.4) | 2.76 s (2.44–3.92) |
| `status`, nothing changed | 15.4 ms (13.4–17.4) | 25.6 ms (25.3–26.1) | 25.5 ms (25.0–26.8) |
| `status` after touching the biggest file | 303 ms (301–307) | 2.61 s (2.60–2.62) | 2.26 s (2.24–2.29) |
| Back to the first version | 6.09 s (5.42–7.74) | 4.11 s (4.08–4.18) | 4.26 s (2.63–4.72) |

**Repeated**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 1.57 s (1.55–1.59) | 28.2 s (28.1–28.4) | 2.60 s (2.50–2.76) |
| A small change | 1.36 s (1.34–1.37) | 37.8 s (28.3–51.9) | 2.53 s (2.48–2.68) |
| `status`, nothing changed | 15.7 ms (15.5–16.1) | 25.8 ms (25.3–27.0) | 25.6 ms (25.1–27.0) |
| `status` after touching the biggest file | 338 ms (333–345) | 2.55 s (2.55–2.56) | 2.25 s (2.24–2.26) |
| Back to the first version | 5.05 s (5.03–5.09) | 4.07 s (4.04–4.11) | 4.67 s (4.63–4.69) |

**Imports in the same job**

- lfs-import: 11.7 s (11.7–11.8)
- history-import: 38.9 s (38.7–39.1)

**Space**

| Set | The files | Bolt | Git | Git with Git LFS |
|---|---|---|---|---|
| Code | 52,458,897 | 16,085,759 | 17,121,515 | 17,123,138 |
| Godot's demo projects | 312,757,968 | 237,701,360 | 229,915,463 | 276,207,080 |
| Noise | 1,048,576,000 | 1,051,271,583 | 1,363,601,964 | 1,363,192,324 |
| Versions | 1,073,741,824 | 1,091,689,095 | 10,740,917,076 | 10,737,640,549 |
| Repeated | 1,073,741,824 | 5,504,716 | 7,518,523,853 | 7,516,228,810 |
- history-bolt: 368,998,024 bytes
- history-git: 313,774,890 bytes
- lfs-bolt: 580,585,708 bytes
- lfs-git: 5,242,922,196 bytes
- lfs-git-lfs-objects: 5,242,880,000 bytes

### Big files on one runner

`main`, the branch and Git LFS turn by turn on the same files on one machine, three turns each (the lowest and the highest are given). Each row is one runner, because their disks differ and the disk is what decides. The big-file code is the same in every row.

| Runner, run | First save of 1 GiB of noise in 20 files: before, after, Git LFS | First save of one 1 GiB file: before, after, Git LFS | Back to that file's first version: before, after |
|---|---|---|---|
| Linux, [42853821053](https://github.com/WisHonor/Bolt/actions/runs/42853821053) | 4.13–4.16 s, 2.45 s, 2.25–2.28 s | 4.19–4.22 s, 2.49–2.51 s, 2.26–2.29 s | 3.83–4.39 s, 2.67–3.10 s |
| Linux, [42856204349](https://github.com/WisHonor/Bolt/actions/runs/42856204349) | 6.47–6.61 s, 4.90–4.95 s, 1.81–1.86 s | 6.69–6.78 s, 5.17–5.22 s, 1.87–1.93 s | 6.53–6.58 s, 3.98–4.23 s |
| Linux, [42887296255](https://github.com/WisHonor/Bolt/actions/runs/42887296255) | 4.15–4.21 s, 2.44–2.46 s, 2.25–2.36 s | 4.23–4.25 s, 2.50–2.51 s, 2.28–2.37 s | 3.85–4.39 s, 3.09–3.21 s |
| Linux, [42910259739](https://github.com/WisHonor/Bolt/actions/runs/42910259739) | 6.53–7.04 s, 5.01–5.12 s, 1.80–1.86 s | 6.81–7.06 s, 5.26–5.63 s, 1.83–1.91 s | 5.61–6.70 s, 4.26–5.57 s |
| Windows, [42853821053](https://github.com/WisHonor/Bolt/actions/runs/42853821053) | 5.77–6.01 s, 5.67–5.88 s, 2.88–3.40 s | 6.19–6.27 s, 5.87–6.26 s, 2.89–4.49 s | 4.48–5.61 s, 5.13–5.52 s |
| Windows, [42856204349](https://github.com/WisHonor/Bolt/actions/runs/42856204349) | 4.48–6.03 s, 5.98–6.11 s, 2.86–3.48 s | 6.06–6.39 s, 6.12–6.43 s, 2.84–4.08 s | 5.34–5.58 s, 5.25–5.64 s |
| Windows, [42887296255](https://github.com/WisHonor/Bolt/actions/runs/42887296255) | 4.57–5.28 s, 2.76–3.27 s, 3.40–4.17 s | 4.72–5.49 s, 2.86–3.36 s, 3.31–4.02 s | 4.02–4.52 s, 2.51–2.94 s |
| Windows, [42910259739](https://github.com/WisHonor/Bolt/actions/runs/42910259739) | 5.63–6.08 s, 5.61–5.79 s, 2.77–2.89 s | 6.15–6.17 s, 5.77–5.82 s, 2.79–2.83 s | 4.06–5.84 s, 4.47–5.25 s |

- **Where the disk keeps up, the first save takes about 60% of the time it took** (2.45 s where it took 4.15 s), and Bolt is within a tenth of Git LFS, or ahead of it. The pack is flushed to the disk while it is still being written, so the disk works while Bolt reads and hashes ([decision 102](decisions/102-big-files-are-flushed-while-they-are-written.md)).
- **Where the disk is slow, Git LFS wins by a wide margin** (1.8 to 1.9 s against 4.9 to 5.1 s on two Linux runners; about 3 s against 5.6 to 6.1 s on three Windows runners, where flushing while writing changed little or nothing; on those Linux runners the first save takes three quarters of what it took). Git LFS's time doesn't follow the disk, so it isn't waiting for it. Bolt's does: it flushes the pack before it gives it its name, and a save that says it is done is on the disk.
- **Going back to an old version of a big file** is a fifth to a third faster on every Linux runner and on the one Windows runner whose disk kept up, and unchanged on the other three.

### Imports

git/git, imported three times by each `bolt` in turn on one runner ([run 42910249822](https://github.com/WisHonor/Bolt/actions/runs/42910249822), job `import-git`; git/git at `1891de4`, 86,222 commits and 1,029 refs):

| | Import | Peak memory | The first save after it |
|---|---|---|---|
| `main` (`73a1a0d`) | 166.7–166.9 s | 656,176–659,084 KiB (642 MiB) | 0.63–0.64 s |
| The branch | 141.4–141.9 s | 448,440–449,248 KiB (438 MiB) | 0.35–0.36 s |

The import is faster because the store finds and reads records in fewer reads; its memory is lower because it keeps 96 MiB of rebuilt objects where it kept 256 MiB.

And the largest history at hand, once, with `main`'s engine and its 256 MiB of rebuilt objects ([run 42759080455](https://github.com/WisHonor/Bolt/actions/runs/42759080455), GitHub's Linux runner): torvalds/linux at `b3dfbde` (2026-11-09), 1,506,211 commits and 12,024,624 objects. It imports in 2,955 s (49 minutes) with a peak of 3,329,288 KiB (3.4 GB), and the `.bolt` folder takes 64,636,524,309 bytes (64.6 GB) where Git's pack takes 6.6 GB. Bolt stores every version of every file and listing whole or in chunks; Git stores most of them as deltas against another version. That is the biggest loss on this page, and records stored as deltas ([decision 009](decisions/009-storage-engine.md)'s later feature) are what would close it.

### Long histories

git/git at `1891de4`: 86,222 commits, 24,498 of them on `master` along first parents, with its four other branches as lines too (`maint`, `next` and `seen`, each about as long, and `todo`, 2,636 saves). It was imported by `main`'s `bolt` and by the branch's on one runner, with Git on a third clone; three turns each, timed from a shell ([run 42910267213](https://github.com/WisHonor/Bolt/actions/runs/42910267213)). A range is the three turns; a command's very first run, which reads files nothing has read yet, is left out where it stood apart.

| Step | Linux: before, after, Git | Windows: before, after, Git |
|---|---|---|
| The import | 135 s, 116 s | 206 s, 164 s |
| `goto` one save back (35 files) | 1,390–1,539 ms, 48–59 ms, 26–27 ms | 2,403–2,760 ms, 171–197 ms, 124–126 ms |
| `goto` 20 saves back (98 files) | 1,253–1,580 ms, 86–108 ms, 39–44 ms | 2,646–2,724 ms, 248–267 ms, 159–162 ms |
| `goto` 2,000 saves back (3,557 files) | 4.0–5.6 s, 1.8–3.1 s, 0.46–0.75 s | 6.3–6.9 s, 3.9–4.4 s, 1.4–1.7 s |
| `status` | 481–533 ms, 20 ms, 12 ms | 1,336–1,854 ms, 113–118 ms, 71 ms |
| `undo --list` | 583–592 ms, 39–40 ms | 1,259–1,273 ms, 120–126 ms |
| The history listing, its first line | 914–955 ms, 5–6 ms, 2–3 ms | 1,202–1,421 ms, 68–84 ms, 59–69 ms |
| The history listing, all 24,498 lines | 1.37–1.39 s, 0.27 s, 0.18 s | 2.50–2.70 s, 0.67 s, 0.32–0.33 s |
| The first amend of a new save | 1,357 ms, 32 ms, 12 ms | 3,167 ms, 117 ms, 109 ms |
| Each amend after it | 9 ms, 8–9 ms, 12–15 ms | 55–57 ms, 52–57 ms, 109–110 ms |

Git's column is `git checkout`, `git status`, `git log --first-parent --oneline` and `git commit --amend`. One of the branch's nine turns of `goto` 20 saves back took 338 ms on Linux.

- **Before, each of these commands read the whole line from the store,** to shorten a change ID or to find a save: half a second to three seconds, whatever was asked. Now the line's saves are kept in order in a sealed cache, and a command reads only the saves it shows. A list is never the reason a save is shown or gone to unchecked ([decision 109](decisions/109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md)): going to a save by its ID checks the list link by link down to it, which is why `goto` 2,000 saves back reads 2,000 saves.
- **The amend.** Before an amend Bolt checks that no other line shows the save. `main` read each of the four other lines whole the first time, 1.4 s on Linux, and kept a list of each. The branch reads only the saves a line doesn't share with `master`, and keeps them ([decisions 111](decisions/111-rulings-after-the-first-red-team-session-on-the-speed-work.md) and [112](decisions/112-rulings-after-the-second-red-team-session-on-the-speed-work.md)); a later amend reads none, and takes 8 to 9 ms where `git commit --amend` takes 12 to 15. The second red-team session's own shape, 300 lines of 2,000 saves each, took 19 to 21 s for every amend before that cache, and `main` 34 s once ([report](redteam/2026-11-10-milestone-6-speed-session-2.md)).

### Deep folders and tiny chunks

Two shapes only a planted project has, measured on the laptop of §1 in a release build:

- **The deepest chain the limits allow,** 2,048 levels with a file in each: a save took 108 s and takes 2.4 s; a `status` that must read every file took 106 s and takes 1.3 s. Writing that chain out is unchanged and slow: a `goto` that removes the files takes 335 s, and one that writes them back 529 s, because write-out opens every folder again from the top for each file ([decision 048](decisions/048-handle-relative-file-access.md)'s safe design). A tree a tenth as deep costs a hundredth.
- **A list of 8,388,608 one-byte chunks** took 82 s to write out its 8 MiB file and takes 22 s: a chunk the list repeats is read once.

### Where the time goes now

One `status` of Git's source on GitHub's Linux runner, counted by `strace` ([run 42910259739](https://github.com/WisHonor/Bolt/actions/runs/42910259739)):

| | System calls | Looks at files | Reads of packs | Opens | Folder listings | Waits between threads | Threads made |
|---|---|---|---|---|---|---|---|
| `main` | 10,697 | 5,311 | 1,626 | 949 | 512 | 893 | 8 |
| The branch | 9,481 | 5,309 | 655 | 530 | 512 | 1,021 | 10 |
| Git | 6,812 | 5,155 | none (it reads one index file) | 334 | 457 | 6 | 9 |

- **All three look at every file once.** That part is level.
- **What Bolt does and Git doesn't:** it reads the last save's listings from its packs (Git reads one index file); it opens every folder by handle, so that nothing is ever followed through a link; its threads hand work to each other about a thousand times; and it flushes one small file to read the drive's clock, which the 2-second rule rests on.

One `goto` that writes one file, traced the same way ([run 42890907947](https://github.com/WisHonor/Bolt/actions/runs/42890907947)): after the scan it makes 9 more flushes (the diary's pack and its folder, the journal and its folder, the file and its folder, `head` and its folder, the folder again once the journal is gone), 0.15 to 0.4 ms each on that runner's quiet disk and 2.6 ms together with the scan's; and 7 renames, which cost 0.16 to 0.35 ms each wherever one replaces a file. Git's checkout flushes nothing.

### Where Git or Git LFS still wins

Each is a row of the [follow-ups](plans/2026-11-01-milestone-1-follow-ups.md) ("Deferred by milestone 6: speed and space"), with what would change it.

- **`status` on a code project:** 1.5 times Git's time on Linux turn by turn (17.9 ms against 12.0) and 1.4 times on Windows (84.5 ms against 60.8); 1.6 to 1.8 times in the gate's Linux runs and 1.7 to 2.2 in its Windows runs. What is left is in the table above; the walk on several threads, or fewer system calls for each file on Windows, would change it.
- **`goto` on a code project, on Linux:** twice Git's time turn by turn (24.6 ms against 12.3), and 1.8 to 2.5 times across the gate's seven runs, so the third gate misses more often than it passes. What would change it: flushing less in a `goto`, which is the owner's call, or the walk on several threads.
- **Godot's demos:** `status` takes about twice Git's time where it took four to five times (25.9 ms against 11.7 on Linux, 72.5 ms against 37.1 on Windows), and going back to the first version about twice on Windows too (82 ms against 45). A small change is behind Git's on Linux when Git's disk doesn't stall (30.6 ms against 23.9 at Git's best) and ahead of it on Windows (82 ms against 103). Its 4,018 files sit in 569 folders, so what a folder costs shows more (§7).
- **`status` on a project of a few files:** 2.3 to 2.5 ms against Git's 1.5 to 1.9 on Linux (the 6.2 ms mean of the noise set holds a stall); on Windows Bolt is ahead, 15 to 16 ms against 26. That is Bolt starting up: opening the project, its packs, and reading the drive's clock.
- **The first save of big files that don't compress, wherever the disk is slow** (above).
- **Going back to an old version of a big file, where the disk is slow:** 4.7 to 4.8 s for a gigabyte against Git's 3.2 s on the Linux runner of the whole benchmark, and 5.1 to 6.1 s against 4.1 s on its Windows runner. Bolt flushes the file it wrote before it renames it into place; Git doesn't.
- **A switch that rewrites thousands of files:** 1.8 to 3.1 s against Git's 0.46 to 0.75 s on Linux for 3,557 files, and 3.9 to 4.4 s against 1.4 to 1.7 s on Windows. Bolt flushes each file before it renames it into place.
- **The history listing, all of it:** 0.27 s against 0.18 s on Linux for 24,498 saves, 0.67 s against 0.33 s on Windows. Bolt reads and checks each save; `git log --oneline` reads less of each commit.
- **Space, on a long history:** Linux's takes 64.6 GB against Git's 6.6 GB, and git/git's 369 MB against Git's 314 MB. On the five data sets of §2 nothing changed: Bolt keeps 3% more than Git on Godot's demos and less on the others.
- **The import's memory:** 3.4 GB for Linux's history.

## 9. Both halves of milestone 6 together

**In plain words.** Milestone 6 was built in two halves at once: speed and space (§8), and presets and polish (the project kinds, `bolt setup`, `bolt.toml`, `bolt verify`, the new terminal output, Git's words). §8 measured the speed half alone. This section measures the two together, on the code v0.1 is made from, with the same workflow and the same steps. The short of it: nothing the presets-and-polish half added shows in the numbers. Against Git on the same runner, each step's share of Git's time sits where §8's runs put it; the import takes the same time and memory as the speed half alone, turn by turn on one runner; and what Bolt stores is the same size. The same two gates pass, and the third gate misses in the same place: going back to another version on Linux, at twice Git's time or over it.

Everything here was measured on 2026-11-10. The whole benchmark, the import and the first run of the gates are of commit `922a31f`, the commit that merged the speed half into the other ([run 42931440244](https://github.com/WisHonor/Bolt/actions/runs/42931440244)). The commits after it change `bolt verify`, `bolt setup`'s output, tests and documents, none of them a step this page times; the three gates were run once more on `fe5b46c`, the code as its final review left it, to confirm (below).

### The three gates

The first two gates: saving 10 GiB peaked at 44,036,096 bytes and took 26.2 s in the whole-benchmark run, and at 44,179,456 bytes and 26.2 s on `fe5b46c` ([run 42941424523](https://github.com/WisHonor/Bolt/actions/runs/42941424523), the nightly workflow started by hand); the limit is 500,000,000. `status` on an unchanged 100,000-file project, whose limit is 2 s, took 0.20 s, then 0.17 s in that second run on GitHub's Linux runner, after a first save of 1.84 s.

The third gate, by the same `speed-gate` job as §8 (medians of 10 runs), in both runs, with the ratios as the job prints them:

| Run, commit | Linux: Bolt, Git, times Git's | Windows: Bolt, Git, times Git's |
|---|---|---|
| [42931440244](https://github.com/WisHonor/Bolt/actions/runs/42931440244), in the whole-benchmark run, `922a31f` | `status` 17.4 ms, 9.8 ms, 1.768; small save 22.5 ms, 23.5 ms, 0.957; `goto` 24.9 ms, 12.3 ms, **2.019 (over)** | `status` 79.7 ms, 46.1 ms, 1.730; small save 93.3 ms, 128.5 ms, 0.726; `goto` 94.4 ms, 55.4 ms, 1.703 |
| [42941424523](https://github.com/WisHonor/Bolt/actions/runs/42941424523), `fe5b46c` | `status` 21.7 ms, 12.4 ms, 1.754; small save 29.0 ms, 28.1 ms, 1.034; `goto` 32.4 ms, 13.4 ms, **2.425 (over)** | `status` 83.9 ms, 46.7 ms, 1.796; small save 91.1 ms, 133.7 ms, 0.681; `goto` 100.0 ms, 56.7 ms, 1.763 |

- **Memory and `status`:** both pass, with the room §8 found.
- **Against Git, on Windows:** all three steps pass.
- **Against Git, on Linux:** `status` and the small save pass. **Going back to the first version doesn't:** it took 2.019 times Git's time in the first run (24.9 ms against 12.3) and 2.425 times in the second (32.4 ms against 13.4). In §8's seven runs of the speed half alone the same step passed twice and missed five times, between 1.79 and 2.50 times Git's time, and `status` ran between 1.60 and 1.81; these runs are inside both bands. So the gate still isn't met as decision 099 states it, for the reason §8 gives: a `goto` flushes ten times, each flush there for what a crash may leave, and Git's checkout flushes nothing. Nothing was changed for it here. Which of those flushes could go is the owner's call ([decision 109](decisions/109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md)), since the first principle puts never losing data before speed.

### Against Git and Git LFS

§2's whole table once more, as §8 lays it out: means of 5 runs, with the lowest and the highest. The runners are not the ones §8's numbers came from, so compare each tool with the others in the same table, not across pages (§6 says how much GitHub's machines differ). What Bolt stores is the size §8 measured on every set: to the byte on Linux, and within one byte on Windows.

What the tables below say, losses included:

- **On Git's source,** the first save takes a fifth of Git's time on Linux (175 ms against 952 ms) and a tenth on Windows (445 ms against 4.69 s). A small change is level with Git on Linux (19.3 ms against 20.8) and takes two thirds of its time on Windows (82.4 ms against 125).
- **Git is ahead on `status` and on going back:** `status` with nothing changed takes 1.6 times Git's time on both systems (14.5 ms against 9.23; 72.4 ms against 44.5), and going back to the first version 1.8 times on Linux and 1.6 times on Windows in these means of five runs (20.4 ms against 11.2; 86.7 ms against 54.5). The gate's medians of ten runs are in the table above.
- **Big files follow the disk, as §8 found.** On this Linux runner Git LFS made the first save of a gigabyte of noise in 1.41 s against Bolt's 5.00 s, and going back to an old version of a gigabyte took Bolt 4.02 s against Git's 2.40 s. On this Windows runner Bolt was ahead on both (2.74 s against Git LFS's 3.08 s; 3.43 s against Git's 5.11 s).

#### GitHub's Linux runner

**Code**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 175 ms (167–182) | 952 ms (942–969) | 984 ms (970–997) |
| A small change | 19.3 ms (18.7–19.7) | 20.8 ms (19.7–24.6) | 46.8 ms (46.3–47.8) |
| `status`, nothing changed | 14.5 ms (14.1–14.8) | 9.23 ms (8.91–9.61) | 8.79 ms (8.62–9.16) |
| `status` after touching the biggest file | 21.7 ms (15.7–32.3) | 12.0 ms (11.7–12.5) | 12.2 ms (12.0–12.5) |
| Back to the first version | 20.4 ms (20.3–20.7) | 11.2 ms (10.9–11.7) | 37.2 ms (35.6–39.1) |

**Godot's demo projects**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 1.06 s (1.05–1.07) | 5.70 s (5.64–5.76) | 2.40 s (2.37–2.42) |
| A small change | 28.9 ms (23.9–47.5) | 27.0 ms (20.8–44.0) | 46.1 ms (44.0–51.0) |
| `status`, nothing changed | 20.9 ms (20.0–21.8) | 10.3 ms (9.95–10.6) | 9.60 ms (8.87–10.3) |
| `status` after touching the biggest file | 26.4 ms (25.6–27.0) | 56.3 ms (53.6–59.2) | 72.2 ms (70.7–73.8) |
| Back to the first version | 37.2 ms (29.5–55.2) | 26.5 ms (8.73–52.8) | 35.5 ms (33.3–38.3) |

**Noise**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 5.00 s (4.96–5.10) | 18.7 s (18.6–18.7) | 1.41 s (1.40–1.44) |
| A small change | 40.5 ms (39.7–41.1) | 940 ms (926–974) | 209 ms (186–244) |
| `status`, nothing changed | 2.02 ms (1.94–2.15) | 1.37 ms (1.31–1.51) | 1.36 ms (1.29–1.50) |
| `status` after touching the biggest file | 9.40 ms (9.32–9.50) | 77.8 ms (77.6–78.1) | 94.5 ms (91.1–103) |
| Back to the first version | 222 ms (217–240) | 134 ms (116–193) | 168 ms (163–173) |

**Versions**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 4.95 s (3.80–5.27) | 24.7 s (23.4–25.2) | 1.42 s (1.40–1.43) |
| A small change | 730 ms (718–742) | 25.5 s (24.9–25.7) | 1.41 s (1.40–1.43) |
| `status`, nothing changed | 6.67 ms (2.05–24.8) | 1.59 ms (1.24–2.32) | 1.33 ms (1.26–1.48) |
| `status` after touching the biggest file | 150 ms (149–152) | 1.61 s (1.60–1.61) | 1.54 s (1.53–1.57) |
| Back to the first version | 4.02 s (4.01–4.05) | 2.40 s (2.38–2.41) | 2.54 s (1.09–3.71) |

**Repeated**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 742 ms (732–750) | 23.4 s (23.3–23.5) | 1.45 s (1.42–1.49) |
| A small change | 711 ms (703–719) | 23.4 s (23.3–23.5) | 1.42 s (1.40–1.46) |
| `status`, nothing changed | 2.57 ms (1.98–4.60) | 1.32 ms (1.23–1.53) | 1.30 ms (1.25–1.43) |
| `status` after touching the biggest file | 144 ms (143–144) | 1.58 s (1.56–1.59) | 1.57 s (1.51–1.60) |
| Back to the first version | 4.04 s (4.02–4.06) | 2.33 s (2.28–2.37) | 2.36 s (1.09–2.70) |

**Imports in the same job**

- lfs-import: 6.33 s (6.30–6.36)
- history-import: 29.6 s (29.4–30.0)

**Space**

| Set | The files | Bolt | Git | Git with Git LFS |
|---|---|---|---|---|
| Code | 50,141,205 | 16,086,209 | 17,122,254 | 17,123,859 |
| Godot's demo projects | 312,757,968 | 237,701,369 | 229,915,519 | 276,207,142 |
| Noise | 1,048,576,000 | 1,051,271,584 | 1,363,601,946 | 1,363,192,296 |
| Versions | 1,073,741,824 | 1,091,689,096 | 10,740,917,051 | 10,737,640,526 |
| Repeated | 1,073,741,824 | 5,504,717 | 7,518,523,836 | 7,516,228,784 |
- history-bolt: 368,998,025 bytes
- history-git: 313,774,870 bytes
- lfs-bolt: 580,585,708 bytes
- lfs-git: 5,242,922,182 bytes
- lfs-git-lfs-objects: 5,242,880,000 bytes

#### GitHub's Windows runner

**Code**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 445 ms (438–456) | 4.69 s (4.45–5.21) | 4.78 s (4.72–4.84) |
| A small change | 82.4 ms (79.2–87.6) | 125 ms (120–132) | 418 ms (413–420) |
| `status`, nothing changed | 72.4 ms (70.4–77.4) | 44.5 ms (43.4–46.8) | 44.4 ms (43.7–45.9) |
| `status` after touching the biggest file | 73.5 ms (72.4–74.4) | 50.8 ms (49.1–53.2) | 50.1 ms (49.7–51.0) |
| Back to the first version | 86.7 ms (83.8–93.2) | 54.5 ms (53.2–55.7) | 352 ms (349–357) |

**Godot's demo projects**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 1.29 s (1.27–1.32) | 13.1 s (12.9–13.6) | 7.43 s (7.35–7.51) |
| A small change | 89.0 ms (87.7–90.5) | 134 ms (124–143) | 426 ms (411–458) |
| `status`, nothing changed | 82.6 ms (80.1–87.2) | 53.9 ms (48.3–62.1) | 45.0 ms (44.3–46.4) |
| `status` after touching the biggest file | 97.6 ms (94.9–102) | 167 ms (149–206) | 343 ms (339–346) |
| Back to the first version | 95.6 ms (86.8–117) | 58.3 ms (50.8–64.5) | 353 ms (349–356) |

**Noise**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 2.74 s (2.71–2.78) | 34.1 s (34.0–34.4) | 3.08 s (3.04–3.20) |
| A small change | 138 ms (134–143) | 1.78 s (1.78–1.79) | 712 ms (705–721) |
| `status`, nothing changed | 23.2 ms (19.7–35.8) | 32.1 ms (30.3–36.5) | 32.0 ms (30.4–37.3) |
| `status` after touching the biggest file | 43.5 ms (42.8–45.0) | 206 ms (203–212) | 379 ms (375–383) |
| Back to the first version | 167 ms (167–167) | 294 ms (291–299) | 820 ms (816–830) |

**Versions**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 2.79 s (2.75–2.84) | 35.9 s (35.8–36.0) | 3.08 s (3.07–3.09) |
| A small change | 1.89 s (1.85–1.91) | 47.6 s (35.8–65.3) | 3.08 s (3.07–3.11) |
| `status`, nothing changed | 22.9 ms (19.3–35.4) | 31.1 ms (30.3–32.1) | 32.0 ms (30.3–36.9) |
| `status` after touching the biggest file | 472 ms (470–477) | 3.32 s (3.30–3.33) | 2.81 s (2.80–2.81) |
| Back to the first version | 3.43 s (3.41–3.45) | 5.11 s (5.09–5.13) | 5.65 s (5.61–5.68) |

**Repeated**

| Step | Bolt | Git | Git with Git LFS |
|---|---|---|---|
| First save | 2.08 s (2.07–2.09) | 36.0 s (35.9–36.0) | 3.12 s (3.09–3.15) |
| A small change | 1.81 s (1.79–1.83) | 47.5 s (36.0–65.0) | 3.10 s (3.06–3.24) |
| `status`, nothing changed | 19.6 ms (18.7–21.0) | 31.4 ms (31.0–31.6) | 31.0 ms (30.6–31.7) |
| `status` after touching the biggest file | 510 ms (505–519) | 3.26 s (3.24–3.28) | 2.79 s (2.77–2.80) |
| Back to the first version | 3.49 s (3.48–3.50) | 5.09 s (5.04–5.15) | 5.69 s (5.63–5.75) |

**Imports in the same job**

- lfs-import: 13.1 s (13.1–13.2)
- history-import: 49.7 s (49.5–49.9)

**Space**

| Set | The files | Bolt | Git | Git with Git LFS |
|---|---|---|---|---|
| Code | 52,458,897 | 16,085,759 | 17,121,514 | 17,123,139 |
| Godot's demo projects | 312,757,968 | 237,701,360 | 229,915,463 | 276,207,083 |
| Noise | 1,048,576,000 | 1,051,271,583 | 1,363,601,966 | 1,363,192,327 |
| Versions | 1,073,741,824 | 1,091,689,095 | 10,740,917,074 | 10,737,640,551 |
| Repeated | 1,073,741,824 | 5,504,717 | 7,518,523,860 | 7,516,228,806 |
- history-bolt: 368,998,024 bytes
- history-git: 313,774,890 bytes
- lfs-bolt: 580,585,707 bytes
- lfs-git: 5,242,922,198 bytes
- lfs-git-lfs-objects: 5,242,880,000 bytes

### Imports

git/git, imported three times by each `bolt` in turn on one runner (the same run, job `import-git`; git/git at `1891de4`, 86,222 commits and 1,029 refs). "`main`" is the speed half alone:

| | Import | Peak memory | The first save after it |
|---|---|---|---|
| `main` (`dc6c009`) | 114.2–114.7 s | 447,948–450,040 KiB (439 MiB) | 0.31–0.33 s |
| Both halves (`922a31f`) | 114.4–114.7 s | 449,916–450,748 KiB (440 MiB) | 0.32–0.41 s |

The two are the same within what one runner varies. (§8's 141 s for the same import was another runner.)
