#!/usr/bin/env python3
"""Differential test: .boltignore vs .gitignore with the same lines (storage-format.md §12 says
they use exactly the .gitignore syntax). Prints every file Bolt stores that Git ignores (the
dangerous direction) and every file Git keeps that Bolt drops.
Run in the sandbox from /work:  python3 -I redteam/attacks2/boltignore-vs-git.py [bolt]"""
import json, os, subprocess, sys, tempfile

BOLT = sys.argv[1] if len(sys.argv) > 1 else "/tmp/t/release/bolt"
ENV = dict(os.environ, BOLT_USER_NAME="T", BOLT_USER_EMAIL="t@example.com",
           BOLT_CONFIG_DIR=tempfile.mkdtemp(), GIT_CONFIG_NOSYSTEM="1", HOME=tempfile.mkdtemp())

# (name, {ignore-file folder: lines}, files)
CASES = [
    ("trailing space", {"": "secret.txt \n"}, ["secret.txt"]),
    ("trailing tab", {"": "secret.txt\t\n"}, ["secret.txt"]),
    ("trailing spaces x3", {"": ".env   \n"}, [".env", "sub/.env"]),
    ("escaped hash", {"": "\\#x\n"}, ["#x"]),
    ("escaped bang", {"": "\\!x\n"}, ["!x"]),
    ("escaped trailing space", {"": "foo\\ \n"}, ["foo ", "foo"]),
    ("double star prefix", {"": "**/build\n"}, ["x/y/build/f", "build/g"]),
    ("dir-only pattern", {"": "build/\n"}, ["build", "sub/build/f"]),
    ("anchored", {"": "/root.txt\n"}, ["root.txt", "sub/root.txt"]),
    ("middle slash", {"": "doc/*.txt\n"}, ["doc/a.txt", "doc/sub/b.txt", "x/doc/c.txt"]),
    ("a/**/b", {"": "a/**/b\n"}, ["a/b", "a/x/b", "a/x/y/b"]),
    ("negated class !", {"": "[!a]bc\n"}, ["abc", "xbc"]),
    ("negated class ^", {"": "[^a]bc\n"}, ["abc", "xbc"]),
    ("range", {"": "[a-c]x\n"}, ["bx", "dx"]),
    ("star then unignore", {"": "*\n!*.txt\n"}, ["a.txt", "b.bin", "sub/c.txt"]),
    ("unignore in subfolder", {"": "*.log\n", "sub": "!keep.log\n"},
     ["sub/keep.log", "sub/x.log", "y.log"]),
    ("triple star", {"": "a***b\n"}, ["ab", "axb", "a/b"]),
    ("trailing backslash", {"": "foo\\\n"}, ["foo", "foo\\"]),
    ("unclosed bracket", {"": "[abc\n"}, ["[abc", "a"]),
    ("leading space", {"": " secret\n"}, ["secret", " secret"]),
    ("spaced comment", {"": " #x\n"}, ["#x", " #x"]),
    ("dir then unignore child", {"": "/sub/*\n!/sub/keep\n"}, ["sub/keep", "sub/drop"]),
    ("double star suffix", {"": "logs/**\n"}, ["logs/a", "logs/b/c", "logs"]),
    ("crlf", {"": "secret.txt\r\nother\r\n"}, ["secret.txt", "other"]),
    ("only backslash escapes", {"": "\\*.txt\n"}, ["*.txt", "a.txt"]),
    ("star slash star", {"": "*/secret\n"}, ["a/secret", "a/b/secret", "secret"]),
    ("question mark", {"": "?.key\n"}, ["a.key", "ab.key", ".key"]),
    ("bracket slash", {"": "a[/]b\n"}, ["a/b", "ab"]),
    ("nested .gitignore name", {"": ".boltignore\n"}, ["x"]),
    ("dotdot pattern", {"sub": "../secret\n"}, ["secret", "sub/x"]),
    ("absolute-looking", {"sub": "/x\n"}, ["sub/x", "x"]),
    ("ignore folder by name w/o slash", {"": "node_modules\n"},
     ["node_modules/a.js", "pkg/node_modules/b.js"]),
    ("brace (not git)", {"": "{a,b}.env\n"}, ["a.env", "b.env", "{a,b}.env"]),
    ("utf8 bom", {"": "\ufeffsecret.txt\n"}, ["secret.txt"]),
]


def build(root, ignores, files, ignore_name):
    for rel in files:
        p = os.path.join(root, rel)
        if rel.endswith("/"):
            os.makedirs(p, exist_ok=True)
            continue
        os.makedirs(os.path.dirname(p), exist_ok=True)
        if os.path.isdir(p):
            continue
        with open(p, "w") as f:
            f.write(rel)
    for folder, text in ignores.items():
        d = os.path.join(root, folder)
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, ignore_name), "w", newline="") as f:
            f.write(text)


def git_keeps(ignores, files):
    root = tempfile.mkdtemp()
    build(root, ignores, files, ".gitignore")
    subprocess.run(["git", "init", "-q"], cwd=root, env=ENV, check=True)
    out = subprocess.run(["git", "ls-files", "-o", "--exclude-standard", "-z"], cwd=root,
                         env=ENV, capture_output=True, check=True).stdout.decode()
    return {p for p in out.split("\0") if p and not p.endswith(".gitignore")}


def bolt_keeps(ignores, files):
    root = tempfile.mkdtemp()
    subprocess.run([BOLT, "start"], cwd=root, env=ENV, capture_output=True, check=True)
    build(root, ignores, files, ".boltignore")
    r = subprocess.run([BOLT, "save", "x"], cwd=root, env=ENV, capture_output=True)
    if r.returncode != 0:
        return "save refused: " + r.stderr.decode().strip().splitlines()[0]
    out = subprocess.run([BOLT, "--json", "show"], cwd=root, env=ENV, capture_output=True,
                         check=True).stdout
    return {c["path"] for c in json.loads(out)["changes"] if not c["path"].endswith(".boltignore")}


for name, ignores, files in CASES:
    g, b = git_keeps(ignores, files), bolt_keeps(ignores, files)
    if isinstance(b, str):
        print(f"{name:32} git keeps {sorted(g)}; bolt: {b}")
        continue
    extra, missing = sorted(b - g), sorted(g - b)
    if extra or missing:
        tag = "BOLT STORES WHAT GIT IGNORES" if extra else "bolt ignores more"
        print(f"{name:32} {tag}: bolt-only {extra}, git-only {missing}")
    else:
        print(f"{name:32} same: {sorted(b)}")
