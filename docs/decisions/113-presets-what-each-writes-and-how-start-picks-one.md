# 113 — Presets: what each one writes, and how `bolt start` picks one

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 6 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 016](016-presets.md) (preset contents, detection and the `bolt start` screen); [decision 056](056-question-before-large-saves.md) (presets set the question's numbers); [decision 071](071-a-step-records-its-change-to-the-folder.md) (a new step, `setup`, that writes the rule files)
- **Supersedes in part:** [decision 016](016-presets.md): the sentence "An existing `.gitignore` is offered for import into `.boltignore`"; and, for Custom alone, "Presets are written out in full" (Custom writes no file)

## Context

A preset is a starter set of project rules. Decision 016 settled the shape: `bolt start` looks at the folder, suggests a preset, and writes its rules out in full into `bolt.toml` and `.boltignore`; v0.1 ships Code, Game (Unity, Unreal, Godot) and Custom. It left open what exactly each preset writes, how the question behaves without a terminal, what happens to rule files that are already in the folder, and what "Custom" puts on disk. [Decision 114](114-bolt-toml-keys-and-how-bolt-reads-it.md) settles the keys of `bolt.toml`.

Two things changed since decision 016. Bolt reads `.gitignore` files as they are (decision 038), so there is nothing to import from one. And the diary records what each step did to the folder as a switch between two saves (decision 071), so writing the rule files can be a step that `bolt undo` takes back.

## Options

**What Custom writes**
- **A. Nothing.** Custom is "Bolt's plain defaults" (decision 016), and a project with no `bolt.toml` already gets them: every project made before milestone 6, and every imported one, is such a project. Decision 016 wanted every preset written out so that no rule is hidden or differs between two people's Bolt versions. Bolt's own values for an absent key are part of format 1 and freeze with it ([decision 121](121-format-1-freezes-at-v0.1.md)), so a project with no file reads the same in every Bolt, and `bolt setup` shows the values.
- **B. A `bolt.toml` that spells the defaults out.** Easier to discover, but it freezes today's defaults into every Custom project, and every script and test that starts a project gains a new unsaved file.

**Where an engine's ignore list comes from**
- **A. GitHub's `.gitignore` template for the engine, whole.** Maintained by others, but written for Git users who keep assets elsewhere: Unreal's keeps out `SourceArt/**/*.png`, and all three keep out `*.obj` or `*.exe` by extension, which hides Wavefront models and tools a game team does want saved.
- **B. A reviewed part of each template:** caches, files the engine generates again, build output and crash files; nothing that is named only by an extension an asset can have.
- **C. All three engines' lists in every game project.** No detection needed, but one engine's folder names (`Saved/`, `Build/`) would hide another engine's real folders.

**How the rule files reach the folder**
- **A. `bolt start` writes them itself.** Simple, but the diary's first entry can't be undone, so an existing `.boltignore` it added lines to couldn't be put back.
- **B. A second step, right after the start.** The same code `bolt setup --preset` runs ([decision 115](115-bolt-setup.md)): the files are written through write-out, a crash is finished by the next command, and `bolt undo` takes the step back.

## Decision

**Custom writes nothing (A). Engine lists are a reviewed part of GitHub's templates (B). The rule files are written as a `setup` step (B).**

### The question

- `bolt start --preset code|game|custom` picks the preset. Without the flag, in a terminal (stdin and stderr both), `bolt start` says what the folder looks like and asks, with the suggestion as the default answer:

```text
$ bolt start
Looks like a game project: found Unity files and 2.3 GiB of files.

What kind of project is this?

    1. Code     apps, websites, libraries
    2. Game     Unity, Unreal, Godot, big assets   ← suggested
    3. Custom   Bolt's plain defaults, no rule files

Choice [2]:
```

- The answer is `1`, `2`, `3` or a preset's name; an empty answer takes the suggestion; anything else asks again. The question and its choices go to stderr.
- **A folder that already has a `bolt.toml`** is a project that already has its rules (a teammate's copy without its `.bolt/`, say). This is judged first. There `bolt start` asks nothing, needs neither a terminal nor the flag, writes no rule file and says the rules that are there stay; `--preset` is refused there, before anything is made, with a hint to start without it and switch afterwards with `bolt setup --preset`.
- **Without a terminal, or with `--json`,** in a folder that has no `bolt.toml`, `bolt start` without `--preset` stops before it makes anything, exit code 1: it can't ask, names `--preset`, and says which preset it would have suggested ([decision 017](017-commands-and-output.md): no questions without a terminal). Bolt never picks a preset for a script.

### Detection

- It looks at the names in the project's top folder (and, for Unity, in `ProjectSettings`), and adds up file sizes below it. It reads no file's contents and runs nothing (decision 016).
- **Engines:** Unity when `ProjectSettings/ProjectVersion.txt` and an `Assets` folder are there; Unreal when a top-level name ends in `.uproject`; Godot when `project.godot` is there.
- **Code:** a top-level `package.json` (Node), `pyproject.toml`, `requirements.txt` or `setup.py` (Python), `Cargo.toml` (Rust), `go.mod` (Go), `pubspec.yaml` (Dart), a name ending in `.sln` or `.csproj` (.NET), `pom.xml` (Maven), `build.gradle` or `build.gradle.kts` (Gradle).
- **The suggestion:** Game when an engine is found, else Code when a language is, else Custom.
- **Sizes** are added up through the folder handles of decision 048, never through a link, leaving out `.git`, `.bolt` and the cache folders of the engines found. The walk stops after 20,000 entries and then says "more than".
- What it prints is made of fixed words and numbers only: "found Unity files", never a name read from the folder.

### What each preset writes

`bolt.toml` (the keys are [decision 114](114-bolt-toml-keys-and-how-bolt-reads-it.md)'s):

| | Code | Game | Custom |
|---|---|---|---|
| `preset` | `"code"` | `"game"` | no file |
| `chunk-files-over` | `"1 MiB"` | `"1 MiB"` | (1 MiB) |
| `ask-before-saving-over` | `"100 MB"` | `"1 GiB"` | (100 MB) |
| `ask-before-saving-files-over` | `10000` | `10000` | (10,000) |
| rules | none | below | none |

Game's rules, in this order (later rules win):

1. `lockable = true` for art and audio that can't be merged: `*.psd *.png *.tga *.tif *.tiff *.exr *.jpg *.jpeg *.fbx *.blend *.max *.mb *.wav *.ogg *.mp3 *.mp4 *.mov`.
2. For each engine found:
   - Unity: `*.unity` and `*.prefab` are `lockable = true` and `conflict-markers = false`; `*.meta` is `conflict-markers = false` (decision 007).
   - Unreal: `*.uasset` and `*.umap` are `lockable = true`.
   - Godot: `*.scn` and `*.res` are `lockable = true`.

Nothing enforces `lockable` before syncing arrives (v0.3), and nothing uses `conflict-markers` before combining does (v0.2). The rules are recorded now so a project has them from its first save (decision 014).

`.boltignore` gets one block of lines, between two marker lines that `bolt setup` looks for:

```text
# >>> Bolt preset: game (unity), version 1. `bolt setup` rewrites the lines down to the end mark.
…
# <<< Bolt preset
```

- **Unity:** `.utmp/`, `/[Ll]ibrary/`, `/[Tt]emp/`, `/[Oo]bj/`, `/[Bb]uild/`, `/[Bb]uilds/`, `/[Ll]ogs/`, `/[Uu]ser[Ss]ettings/`, `/[Mm]emory[Cc]aptures/`, `/[Rr]ecordings/`, `.vs/`, `.gradle/`, `ExportedObj/`, `.consulo/`, `/*.csproj`, `/*.unityproj`, `/*.sln`, `/*.slnx`, `/*.booproj`, `*.suo`, `*.user`, `*.userprefs`, `*.pidb`, `*.svd`, `*.pdb`, `*.mdb`, `*.opendb`, `*.VC.db`, `*.pidb.meta`, `*.pdb.meta`, `*.mdb.meta`, `sysinfo.txt`, `mono_crash.*`, `/*_BackUpThisFolder_ButDontShipItWithYourGame/`, `/*_BurstDebugInformation_DoNotShip/`, `crashlytics-build.properties`, `/[Aa]ssets/_Recovery/`, `/[Aa]ssets/_Recovery.meta`, `/[Aa]ssets/Addressables_Temp/`, `/[Aa]ssets/Addressables_Temp.meta`, `/UIElementsSchema/`.
- **Unreal:** `.vs/`, `/Binaries/`, `/Saved/`, `/Intermediate/`, `Plugins/**/Intermediate/`, `/DerivedDataCache/`, `*_BuiltData.uasset`, `/*.sln`, `/*.slnx`, `*.suo`, `*.opensdf`, `*.sdf`, `*.VC.db`, `*.VC.opendb`, `*.xcodeproj`, `*.xcworkspace`.
- **Godot:** `.godot/`, `.import/`, `export.cfg`, `export_credentials.cfg`, `.mono/`, `mono_crash.*.json`.
- **Code,** for each language found: Node `node_modules/`, `.npm/`, `.next/`, `.nuxt/`, `.cache/`, `.parcel-cache/`, `.svelte-kit/`, `.turbo/`, `coverage/`, `npm-debug.log*`, `yarn-debug.log*`, `yarn-error.log*`, `*.tsbuildinfo`; Python `__pycache__/`, `*.py[cod]`, `.venv/`, `venv/`, `dist/`, `*.egg-info/`, `.pytest_cache/`, `.mypy_cache/`, `.ruff_cache/`, `.tox/`, `.ipynb_checkpoints/`; Rust `target/`; Go `*.test`, `*.coverprofile`; Dart `.dart_tool/`, `/build/`, `.flutter-plugins`, `.flutter-plugins-dependencies`; .NET `[Bb]in/`, `[Oo]bj/`, `.vs/`, `*.user`, `*.suo`; Maven `target/`; Gradle `.gradle/`, `**/build/`, `!**/src/**/build/`. Then, always: `.env`, `.env.*`, `!.env.example`.

The rule for the lists: **when in doubt, save it.** They hold caches, files the tool makes again, build output and crash files. They leave out every template line that would keep a file out only for its extension when an asset can have that extension (`*.obj`, `*.exe`, `*.dll`, `*.log`, `*.tmp`, `*.unitypackage`, Unreal's `SourceArt/` lines, Godot's `data_*/`), and every folder that teams do commit things in: Unreal's `Build/`, which holds each platform's icons and packaging files, the `Binaries/` of plugins, which is all a binary-only plugin is, and Node's `dist/`, which some projects publish from. Project files an engine generates at the top (`*.sln`, `*.csproj`) are named at the top only, so a real one deeper down is saved. The lists come from GitHub's templates (github/gitignore, CC0) as read on 2026-11-09.

- **Game with no engine found** (an empty folder, say) writes the common rules and no engine's lines, and says so; `bolt setup --preset game` adds them once the engine has made its project.
- **A `.boltignore` that is already there** keeps every line it has: the block goes in at its top, so the lines below it, the user's own, win.
- **An existing `.gitignore` is not copied.** Bolt reads it as it is (decision 038). The block's lines count after a `.gitignore`'s in the same folder, so they also override a line there that takes a file back; a line below the block in `.boltignore` has the last word.
- **Custom** writes no file. `bolt setup` then shows Bolt's defaults.

### How the files are written

`bolt start` makes the project as before (the diary's first entry changes no file), then, for Code and Game, runs the `setup` step of [decision 115](115-bolt-setup.md): the two files are written through write-out, as one diary step. `bolt undo` right after a start takes the rule files away again, and gives back a `.boltignore` exactly as it was. The files are ordinary unsaved files until the first `bolt save` records them (decision 008).

A start that stops between the two steps leaves a project with no rule files, which is a Custom project; `bolt setup --preset` writes them.

### What `bolt start` says

The outcome first, then the rules in words, then the first save, then the next step:

```text
Started a Bolt project in C:\Users\ana\Skyrunner

Using Game settings (Unity):

    • Files of 1 MiB or more are stored in chunks, so a changed part is stored once
    • Unity's Library/, Temp/, Logs/ and other generated files are left out
    • A save that adds over 1 GiB or 10,000 new files asks first
    • Art, audio, scenes and prefabs are marked lockable; locking arrives with syncing

    The rules are in bolt.toml and .boltignore. See them any time with: bolt setup

Your first save is kxqm on line main.
```

The lines say only what v0.1 does: nothing is "locked while you edit", "downloaded on demand" or "merged line by line" yet.

## Consequences

- Preset definitions are data inside Bolt, each with a version number (1 now) written in the comment at the top of what it writes. [storage-format.md](../storage-format.md) §12 gives the exact bytes, and test vectors pin them.
- Scripts and tests that run `bolt start` without a terminal must pass `--preset`. `--preset custom` behaves as `bolt start` did before milestone 6.
- A project started with Game before its engine exists has no engine lines until `bolt setup --preset game` is run again.
- The lists are deliberately short, so some generated files are saved until the user adds a line for them. The large-save question still catches a cache worth gigabytes.
- Detection can be fooled by planted names, but it only suggests: the person answers the question, a script names the preset, and a preset can only add ignore lines and rules, never run anything.
- An imported project has no preset: `bolt import` starts no question. `bolt setup --preset` gives it one.
- AI/data and Design/video presets stay for later (decision 016).
