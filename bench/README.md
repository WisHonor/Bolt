# Benchmarks

These scripts time Opalro, Git, and Git with Git LFS through the same steps on the same data. The results, with the machines and versions, are in [docs/benchmarks.md](../docs/benchmarks.md).

- `data.sh` builds each data set: a code project and Godot's demo projects at fixed tags, downloaded without history, and sets generated from fixed seeds (noise from `openssl`'s AES-256-CTR, keyed by a text).
- `run.sh sets <data> <results>` runs every step for each tool on its own copy of each set, timed with `hyperfine`: `run.sh` runs each step once to warm up, then `hyperfine` times 5 runs (3 when the warm-up took over a minute), each after an untimed wait for Git's background housekeeping and a `sync`. It writes hyperfine's JSON and Markdown for each step and the sizes in `sizes.tsv`; `run.sh summary <results>` prints them.
- `run.sh gate <data> <results>` runs the same steps on the code set alone, for Opalro and Git, with 10 timed runs each. It then compares the medians of `status`, the small change saved and the way back to the first version, and fails when Opalro's is over twice Git's for the first two, or over three and a half times Git's for the way back; twice is the way back's goal for v0.2, and its line says when it is over that. It runs in the `Speed gate` workflow (`.github/workflows/speed-gate.yml`), alone on a push to a `gate/` branch, and from the `Bench` and `Nightly` workflows.
- `run.sh import <repository> <opal> <opal>` imports a Git repository with two builds of Opalro in turn, three times each, printing each one's time and peak memory, then times the first save after each import.

They run in the `Bench` workflow (`.github/workflows/bench.yml`), on GitHub's Linux and Windows machines, on a push to a `bench/` branch. A push to a `redteam/<name>-sizes` branch, such as `redteam/m5-sizes`, runs only the import of the red team's RT4-1 repository: it is attack code, so it runs nowhere else. The scripts need `opal`, `git`, `git lfs`, `hyperfine`, `jq`, `curl`, `openssl` and GNU coreutils; on Windows, Git's own bash. The sets take up to about 25 GiB at once, while Git repacks the 10 versions of a 1 GiB file. The data folder is the caller's: `run.sh` deletes each set once it is done with it, but leaves whatever a failure stops it at. `BENCH_MIB=1024` makes every generated set 1,024 times smaller, to try the scripts quickly.

The speeds of the parts and what a stronger zstd level would save come from `crates/opal-vcs-core/tests/throughput.rs`, run in a release build with `OPAL_BENCH_CODE` and `OPAL_BENCH_GODOT` naming the two downloaded sets:

```sh
cargo test --release -p opal-vcs-core --test throughput -- --ignored --nocapture --test-threads=1
```
