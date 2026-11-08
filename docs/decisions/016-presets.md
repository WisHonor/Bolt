# 016 — Presets: written out in full

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 16
- **Refined by:** [decision 056](056-question-before-large-saves.md) (a large save asks first from milestone 3); [decision 082](082-how-big-files-are-cut-into-chunks.md) (the chunking threshold is 1 MiB until the presets make it a `bolt.toml` setting)

## Context

`bolt start` (the owner's signature screen, design.md §1) detects the project type and suggests a preset: a starter set of project rules. Decision 008 put project rules in `bolt.toml` and `.boltignore`. This decision sets how a preset lives in a project afterwards, how detection works, the rule format, and which presets ship first.

## Options

- **Written out in full** (GitHub's `.gitignore` templates; Anchorpoint's per-engine Git settings). All rules are copied into the project's files and the preset name is a label. Nothing hidden; updates arrive only when accepted.
- **A live reference** (like ESLint's `extends`). Tiny files and automatic improvements, but the rules are invisible and can differ between teammates' Bolt versions.
- **A reference with a version number.** Small files and team consistency, but the rules are still invisible, and every old preset version must ship forever.

## Decision

**Presets are written out in full.**
- `bolt start` writes every rule of the chosen preset into `bolt.toml` and `.boltignore`; `preset = "<name>"` stays as a label. From then on, the files are the rules.
- **Detection** reads only file names and sizes, skips cache folders when adding up sizes, and stops early on huge folders. Signals: Unity (`ProjectSettings/ProjectVersion.txt` + `Assets/`), Unreal (`*.uproject`), Godot (`project.godot`), code (language manifests such as `package.json`, `pyproject.toml`, `Cargo.toml`, `go.mod`, `pubspec.yaml`, `*.sln`, `*.csproj`, `pom.xml`, `build.gradle`), AI/data (`.safetensors`, `.onnx`, `.parquet`, `.ipynb`), design/video (`.psd`, `.ai`, `.prproj`, `.aep`, large `.mp4`/`.mov`).
- **Rules** in `bolt.toml` are read top to bottom, later rules winning; file patterns use `.boltignore` syntax. Per-rule settings include `lockable`, `conflict-markers` and engine-specific move hints; project-wide settings include the chunking threshold and the "ask before saving more than" threshold.
- **Presets in v0.1:** Code, Game (Unity, Unreal, Godot) and Custom (Bolt's plain defaults). AI/data and Design/video ship after being tested on real projects.
- **The Code preset** covers apps, websites, libraries and scripts: no lockable files; conflict markers for all text; the ignore lists of every detected language combined (for example `node_modules/`, `dist/`, `target/`, `.venv/`, `__pycache__/`); `.env` files ignored; a low "ask before saving" threshold (around 100 MB); normal chunking for any big file.
- An existing `.gitignore` is offered for import into `.boltignore`.
- **Changing presets:** `bolt setup` shows exactly which lines of the two files would change and applies them as a normal, undoable change; history is never rewritten (decision 005).
- **Which rules apply:** the ones in the working folder. An old save brings back its own rules; combining lines merges the rule files like text; the server checks locks against the rules of the line being sent to.

## Consequences

- Preset definitions ship inside Bolt as data, are tested like code, and carry a version so `bolt setup` can show what changed in newer presets.
- Ignore lists and lockable types for each engine and language come from maintained sources (GitHub's `.gitignore` templates, engine documentation) and are reviewed before shipping.
- Detection and rule parsing handle untrusted projects (topic 18).
- Exact key names in `bolt.toml` are finalised in the format spec (topic 18).
