#!/usr/bin/env python3
"""RT15 probes of the rename (decision 150), run in the sandbox only:
    python3 redteam/attacks14/probe.py <name>...   (no name: all)
Uses the release build at /home/attacker/target/release/opalro."""
import json, os, shutil, subprocess, sys, tempfile

OPALRO = os.environ.get("OPALRO", "/home/attacker/target/release/opalro")
CLEAN = {k: v for k, v in os.environ.items()
         if not k.startswith(("OPALRO_", "OPAL_", "BOLT_", "GIT_"))}


def run(cwd, *args, env=None, user=True, config=None, timeout=120):
    e = dict(CLEAN)
    home = config or tempfile.mkdtemp(prefix="cfg-")
    e.update({"HOME": home, "XDG_CONFIG_HOME": os.path.join(home, ".config")})
    if user:
        e.update({"OPALRO_USER_NAME": "Test", "OPALRO_USER_EMAIL": "test@example.com",
                  "OPALRO_CONFIG_DIR": home})
    e.update(env or {})
    p = subprocess.run([OPALRO, *args], cwd=cwd, env=e, capture_output=True, timeout=timeout)
    return p.returncode, p.stdout.decode("utf-8", "replace"), p.stderr.decode("utf-8", "replace")


def project(files=None):
    root = tempfile.mkdtemp(prefix="p-")
    for path, data in (files or {}).items():
        full = os.path.join(root, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "wb") as f:
            f.write(data)
    return root


def saved_paths(root, cfg, line="main"):
    code, out, err = run(root, "show", "--json", line, config=cfg)
    if code != 0:
        return f"show failed {code}: {err.strip()[:300]}"
    doc = json.loads(out)
    return doc


def show(title, result):
    print(f"--- {title}")
    code, out, err = result
    print(f"exit {code}")
    if out.strip():
        print("stdout:", out.strip()[:1500])
    if err.strip():
        print("stderr:", err.strip()[:1500])


# P1: settings and identity under the old names.
def p_settings():
    home = tempfile.mkdtemp(prefix="home-")
    mallory = "[user]\nname = \"Mallory\"\nemail = \"mallory@evil.example\"\n"
    for old in ("bolt", "opal", "Bolt", "Opal"):
        d = os.path.join(home, ".config", old)
        os.makedirs(d)
        open(os.path.join(d, "config.toml"), "w").write(mallory)
    elsewhere = tempfile.mkdtemp(prefix="old-dir-")
    open(os.path.join(elsewhere, "config.toml"), "w").write(mallory)
    old_vars = {"BOLT_USER_NAME": "Mallory", "BOLT_USER_EMAIL": "m@evil.example",
                "OPAL_USER_NAME": "Mallory", "OPAL_USER_EMAIL": "m@evil.example",
                "BOLT_CONFIG_DIR": elsewhere, "OPAL_CONFIG_DIR": elsewhere,
                "OPALRO_CONFIG": elsewhere, "OPALCONFIG_DIR": elsewhere}
    root = project({"a.txt": b"a\n"})
    show("start with only old names set", run(root, "start", "--preset", "custom", user=False,
                                              config=home, env=old_vars))
    show("save with only old names set", run(root, "save", "x", user=False, config=home,
                                             env=old_vars))
    show("history --json", run(root, "history", "--json", user=False, config=home, env=old_vars))


# P2: look-alikes of .opalro in a working folder on Linux.
LOOKALIKES = [
    ".OPALRO", ".Opalro", ".opalro.", ".opalro ", ".opalro. .", "opalro~1", "OPALRO~4",
    "opalro~1.", ".o‌palro", "﻿.opalro", ".opalr⁯o", ".opalro‍",
    ".opalro:x", ".opalro::$INDEX_ALLOCATION", "‪.opalro",
]


def p_lookalikes_saved():
    root = project({"keep.txt": b"k\n"})
    cfg = tempfile.mkdtemp(prefix="cfg-")
    show("start", run(root, "start", "--preset", "custom", config=cfg))
    for name in LOOKALIKES:
        os.makedirs(os.path.join(root, name), exist_ok=True)
        open(os.path.join(root, name, "planted"), "wb").write(b"p\n")
        os.makedirs(os.path.join(root, "sub", name), exist_ok=True)
        open(os.path.join(root, "sub", name, "planted"), "wb").write(b"p\n")
    show("save", run(root, "save", "--json", "x", config=cfg))
    show("status", run(root, "status", config=cfg))


# P3: the old magic bytes and format line planted in .opalro.
def started(files=None, saves=1):
    root = project(files or {"a.txt": b"a\n", "big.bin": os.urandom(3 << 20)})
    cfg = tempfile.mkdtemp(prefix="cfg-")
    assert run(root, "start", "--preset", "custom", config=cfg)[0] == 0
    for i in range(saves):
        open(os.path.join(root, "a.txt"), "ab").write(b"more %d\n" % i)
        assert run(root, "save", f"s{i}", config=cfg)[0] == 0
    return root, cfg


def swap_magic(path, old, new):
    data = open(path, "rb").read()
    assert old in data, (path, old)
    open(path, "wb").write(data.replace(old, new))


def p_old_magic():
    root, cfg = started()
    fmt = os.path.join(root, ".opalro", "format")
    good = open(fmt, "rb").read()
    open(fmt, "wb").write(good.replace(b"opal-repository", b"bolt-repository"))
    show("format bolt-repository: status", run(root, "status", config=cfg))
    show("format bolt-repository: start", run(root, "start", "--preset", "custom", config=cfg))
    open(fmt, "wb").write(good.replace(b"opal-repository", b"opalro-repository"))
    show("format opalro-repository: status", run(root, "status", config=cfg))
    open(fmt, "wb").write(good)
    packs = os.path.join(root, ".opalro", "store", "packs")
    names = sorted(os.listdir(packs))
    print("packs:", names)
    for name in names:
        swap_magic(os.path.join(packs, name), b"OPALPACK", b"BOLTPACK")
        swap_magic(os.path.join(packs, name), b"OPALPEND", b"BOLTPEND")
    show("packs BOLTPACK: status", run(root, "status", config=cfg))
    show("packs BOLTPACK: history", run(root, "history", config=cfg))
    show("packs BOLTPACK: verify", run(root, "verify", config=cfg))
    show("packs BOLTPACK: save", run(root, "save", "after", config=cfg))
    show("packs BOLTPACK: tidy", run(root, "tidy", config=cfg))


def p_old_caches():
    root, cfg = started(saves=2)
    cache = os.path.join(root, ".opalro", "cache")
    run(root, "status", config=cfg)
    run(root, "history", config=cfg)
    print("cache:", sorted(os.listdir(cache)))
    for name, old, new in (("files", b"OPALFILE", b"BOLTFILE"),
                           ("change-ids", b"OPALCHID", b"BOLTCHID"),
                           ("other-lines", b"OPALLINE", b"BOLTLINE")):
        path = os.path.join(cache, name)
        if os.path.exists(path):
            swap_magic(path, old, new)
    show("old caches: status", run(root, "status", config=cfg))
    show("old caches: history", run(root, "history", config=cfg))
    show("old caches: save", run(root, "save", "x", config=cfg))
    for name, magic in (("files", b"OPALFILE"), ("change-ids", b"OPALCHID")):
        path = os.path.join(cache, name)
        print(name, "now starts", open(path, "rb").read(8) if os.path.exists(path) else None)
    journal = os.path.join(root, ".opalro", "state", "journal")
    open(journal, "wb").write(b"BOLTJRNL" + b"\x01" + b"\x00" * 200)
    show("BOLTJRNL journal: status", run(root, "status", config=cfg))
    show("BOLTJRNL journal: goto main", run(root, "goto", "main", config=cfg))
    show("BOLTJRNL journal: save", run(root, "save", "y", config=cfg))


# P4: start and import into a .opalro that another tool made, or an old build's folders.
def p_foreign_opalro():
    for inside in (["format"], ["store", "notes.txt"], ["FORMAT"], ["config.toml"], ["state"],
                   ["tmp", "cache", "store", "state", "format.bak"]):
        root = project({"a.txt": b"a\n"})
        cfg = tempfile.mkdtemp(prefix="cfg-")
        os.makedirs(os.path.join(root, ".opalro"))
        for name in inside:
            p = os.path.join(root, ".opalro", name)
            if "." in name or name.isupper():
                open(p, "wb").write(b"x")
            else:
                os.makedirs(p)
        show(f".opalro holding {inside}: start", run(root, "start", "--preset", "custom",
                                                     config=cfg))
        print("now holds", sorted(os.listdir(os.path.join(root, ".opalro"))))


def p_old_project_folders():
    root = project({"a.txt": b"a\n", ".bolt/format": b"bolt-repository 1\nhash blake3\n",
                    ".bolt/state/head": b"0" * 64 + b"\n", ".opal/format": b"x",
                    "bolt.toml": b"chunk-files-over = \"1 KiB\"\n", "opal.toml": b"x = 1\n",
                    ".boltignore": b"a.txt\n", ".opalignore": b"a.txt\n",
                    "sub/.bolt/format": b"bolt-repository 1\n", "sub/f": b"f\n"})
    cfg = tempfile.mkdtemp(prefix="cfg-")
    show("start beside .bolt/.opal", run(root, "start", "--preset", "custom", "--json",
                                         config=cfg))
    show("show main", run(root, "show", "main", config=cfg))
    os.makedirs(os.path.join(root, "deeper"))
    show("save from inside .bolt", run(os.path.join(root, ".bolt"), "save", "x", config=cfg))
    show("status from inside .bolt/state", run(os.path.join(root, ".bolt", "state"), "status",
                                               config=cfg))


# P5: paths and patterns typed on the command line that could reach .opalro.
def tree(root):
    out = []
    for d, dirs, files in os.walk(os.path.join(root, ".opalro")):
        for f in files:
            out.append(os.path.relpath(os.path.join(d, f), root))
    return sorted(out)


def p_typed_patterns():
    for pattern in (".opalr?", ".opalr*", "[.]opalro", "*", ".*", "?opalro", ".opalro/*",
                    "*/head", "**/head", ".OPALR?", "[.]OPALRO/x"):
        root, cfg = started(files={"a.txt": b"a\n"}, saves=1)
        os.makedirs(os.path.join(root, ".OPALRO"))
        open(os.path.join(root, ".OPALRO", "x"), "wb").write(b"x")
        before = tree(root)
        for args in (["discard", pattern], ["save", "p", pattern], ["shelf", "s", pattern],
                     ["changes", pattern], ["status", pattern]):
            code, out, err = run(root, *args, config=cfg)
            print(f"{pattern!r} {args[0]}: exit {code} {(out + err).strip()[:160]!r}")
        after = tree(root)
        if before != after:
            print("  .opalro changed:", sorted(set(before) ^ set(after)))
        print("  .OPALRO/x still there:", os.path.exists(os.path.join(root, ".OPALRO", "x")))


# P6: a project's own .opalro/config.toml naming someone (decision 025), under every old name too.
def p_project_identity():
    root, cfg = started(files={"a.txt": b"a\n"}, saves=0)
    mallory = "[user]\nname = \"Mallory\"\nemail = \"mallory@evil.example\"\n"
    open(os.path.join(root, ".opalro", "config.toml"), "w").write(mallory)
    for old in ("bolt.toml", "opal.toml", ".bolt/config.toml", ".opal/config.toml",
                "opalro.toml"):
        p = os.path.join(root, old)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        open(p, "w").write(mallory)
    open(os.path.join(root, "a.txt"), "ab").write(b"b\n")
    # Only a name in the environment, no email: nobody is named, so nothing may be borrowed.
    show("save, half an identity in the environment",
         run(root, "save", "x", user=False, config=cfg,
             env={"OPALRO_USER_NAME": "Half", "OPALRO_CONFIG_DIR": cfg}))
    show("save, the user's identity", run(root, "save", "--json", "x", config=cfg))


# P7: import into another tool's .opalro, and of a Git commit tracking .opalro spellings.
def git(root, *args):
    e = dict(CLEAN, GIT_CONFIG_NOSYSTEM="1", HOME=root, GIT_AUTHOR_NAME="G",
             GIT_AUTHOR_EMAIL="g@x", GIT_COMMITTER_NAME="G", GIT_COMMITTER_EMAIL="g@x")
    subprocess.run(["git", "-c", "init.defaultBranch=main", *args], cwd=root, env=e,
                   check=True, capture_output=True)


def p_import_names():
    root = project({"a.txt": b"a\n"})
    git(root, "init", "-q")
    git(root, "add", "-A")
    git(root, "commit", "-qm", "one")
    os.makedirs(os.path.join(root, ".opalro"))
    open(os.path.join(root, ".opalro", "notes.txt"), "wb").write(b"theirs")
    show("import into another tool's .opalro", run(root, "import"))
    print("now holds", sorted(os.listdir(os.path.join(root, ".opalro"))))
    for tracked in (".OPALRO/x", "opalro~2/x", ".o‌palro/x", ".opalro./x", "sub/.opalro/format",
                    ".bolt/format", ".opalroignore"):
        root = project({"a.txt": b"a\n", tracked: b"planted\n"})
        git(root, "init", "-q")
        git(root, "add", "-A")
        git(root, "commit", "-qm", "one")
        code, out, err = run(root, "import")
        print(f"import, Git tracking {tracked!r}: exit {code} {(out + err).strip()[:300]!r}")


if __name__ == "__main__":
    names = sys.argv[1:] or [n[2:] for n in globals() if n.startswith("p_")]
    for n in names:
        print(f"===== {n}")
        globals()["p_" + n]()
