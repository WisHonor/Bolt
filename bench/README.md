# Benchmarks

These scripts time Bolt, Git, and Git with Git LFS through the same steps on the same data. The results, with the machines and versions, are in [docs/benchmarks.md](../docs/benchmarks.md).

- `data.sh` builds each data set: a code project and Godot's demo projects at fixed tags, downloaded without history, and sets generated from fixed seeds (noise from `openssl`'s AES-256-CTR, keyed by a text).
- `run.sh sets <data> <results>` runs every step for each tool on its own copy of each set, timed with `hyperfine`: `run.sh` runs each step once to warm up, then `hyperfine` times 5 runs (3 when the warm-up took over a minute), each after an untimed wait for Git's background housekeeping and a `sync`. It writes hyperfine's JSON and Markdown for each step and the sizes in `sizes.tsv`; `run.sh summary <results>` prints them.
- `run.sh gate <data> <results>` runs the same steps on the code set alone, for Bolt and Git, with 10 timed runs each. It then compares the means of `status`, the small change saved and the way back to the first version, and fails when Bolt's is over twice Git's; it runs in the `Speed gate` workflow (`.github/workflows/speed-gate.yml`), alone on a push to a `gate/` branch, and from the `Bench` and `Nightly` workflows.
- `run.sh import <repository> <bolt> <bolt>` imports a Git repository with two builds of Bolt in turn, three times each, printing each one's time and peak memory, then times the first save after each import.

They run in the `Bench` workflow (`.github/workflows/bench.yml`), on GitHub's Linux and Windows machines, on a push to a `bench/` branch. A push to a `redteam/<name>-sizes` branch, such as `redteam/m5-sizes`, runs only the import of the red team's RT4-1 repository: it is attack code, so it runs nowhere else. The scripts need `bolt`, `git`, `git lfs`, `hyperfine`, `jq`, `curl`, `openssl` and GNU coreutils; on Windows, Git's own bash. The sets take up to about 25 GiB at once, while Git repacks the 10 versions of a 1 GiB file. The data folder is the caller's: `run.sh` deletes each set once it is done with it, but leaves whatever a failure stops it at. `BENCH_MIB=1024` makes every generated set 1,024 times smaller, to try the scripts quickly.

The speeds of the parts and what a stronger zstd level would save come from `crates/bolt-vcs-core/tests/throughput.rs`, run in a release build with `BOLT_BENCH_CODE` and `BOLT_BENCH_GODOT` naming the two downloaded sets:

```sh
cargo test --release -p bolt-vcs-core --test throughput -- --ignored --nocapture --test-threads=1
```
