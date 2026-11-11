"""RT12: random projects, random ignore files, random edits, run through this build's bolt and
main's side by side; every command's exit code, output and warnings must agree.
Run in the sandbox only:  redteam/attacks11/run.sh script diff_fuzz.py [rounds] [seed]
"""
import os, random, re, shutil, subprocess, sys, tempfile

NEW = "/work/target/release/bolt"
OLD = "/work/_main/target/release/bolt"

NAMES = ["a", "A", "b.log", "B.LOG", "c.txt", "x y", "#h", "!n", "[ab]", "ab", "ba", "a.b",
         "keep", "Keep", "lib", "out", "z*", "q?", "aXb", "aab", "abb", "é", "é"]
DIRS = ["s", "S", "lib", "out", "d", "deep"]
LINES = ["a", "A", "*.log", "*.LOG", "a*", "*b", "a*b", "?b*", "[ab]*", "\\*x", "!a", "/a",
         "s/a", "s/", "**/a", "s/**", "out/", "!out/keep", "keep", "a ", "a\\ ", "#a", "\\#h",
         "\\!n", "[A-Z]*", "*", "!*.txt", "!c.txt", "lib", "/lib/", "d/*", "!d/a", "a?b",
         "[!a]b", "*.[lL][oO][gG]", "x y", "x\\ y", "aa*", "*bb", "a*a", "\\[ab]", "ab*ab",
         "*/a", "S/A", "deep/**/b.log", "é", "é"]

ID = re.compile(r"\b[k-z]{4,32}\b|\b[0-9a-f]{8,64}\b|/tmp/[^\s\"']+")


def norm(s):
    return ID.sub("<id>", s)


def run(bolt, root, args, env):
    p = subprocess.run([bolt] + args, cwd=root, env=env, capture_output=True, timeout=120)
    return p.returncode, norm(p.stdout.decode("utf-8", "replace")), norm(p.stderr.decode("utf-8", "replace"))


def write(root, rel, data):
    path = os.path.join(root, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    if os.path.isdir(path):
        return
    with open(path, "wb") as f:
        f.write(data)


def plan_tree(rng):
    files = []
    for _ in range(rng.randint(3, 14)):
        depth = rng.randint(0, 3)
        parts = [rng.choice(DIRS) for _ in range(depth)] + [rng.choice(NAMES)]
        files.append("/".join(parts))
    return files


def rules(rng):
    return "\n".join(rng.choice(LINES) for _ in range(rng.randint(1, 6))) + "\n"


def mutate(rng, files, dirs_with_rules):
    ops = []
    for _ in range(rng.randint(1, 4)):
        k = rng.random()
        if k < 0.35:
            ops.append(("write", rng.choice(files), os.urandom(rng.randint(0, 8))))
        elif k < 0.55:
            ops.append(("write", "/".join([rng.choice(DIRS)] * rng.randint(0, 2) + [rng.choice(NAMES)]).lstrip("/"), b"new"))
        elif k < 0.75:
            d = rng.choice(dirs_with_rules + [""])
            name = rng.choice([".gitignore", ".boltignore"])
            ops.append(("write", (d + "/" if d else "") + name, rules(rng).encode()))
        elif k < 0.9:
            ops.append(("rm", rng.choice(files), None))
        else:
            ops.append(("chmod", rng.choice(files), None))
    return ops


def apply(root, ops):
    for op, rel, data in ops:
        path = os.path.join(root, rel)
        try:
            if op == "write":
                write(root, rel, data)
            elif op == "rm":
                if os.path.isfile(path):
                    os.remove(path)
            elif op == "chmod":
                if os.path.isfile(path):
                    os.chmod(path, os.stat(path).st_mode ^ 0o100)
        except (NotADirectoryError, IsADirectoryError, FileExistsError):
            pass


def one(seed):
    rng = random.Random(seed)
    base = tempfile.mkdtemp(prefix="rt12f")
    roots = {b: os.path.join(base, n) for b, n in ((NEW, "new"), (OLD, "old"))}
    envs = {}
    for b, r in roots.items():
        os.makedirs(r)
        cfg = os.path.join(base, "cfg-" + os.path.basename(r))
        os.makedirs(cfg)
        envs[b] = dict(os.environ, BOLT_CONFIG_DIR=cfg, BOLT_USER_NAME="r", BOLT_USER_EMAIL="r@x")
    files = plan_tree(rng)
    dirs_with_rules = sorted({f.rsplit("/", 1)[0] for f in files if "/" in f})
    init = []
    for f in files:
        init.append(("write", f, os.urandom(rng.randint(0, 6))))
    for _ in range(rng.randint(0, 3)):
        d = rng.choice(dirs_with_rules + [""])
        init.append(("write", (d + "/" if d else "") + rng.choice([".gitignore", ".boltignore"]), rules(rng).encode()))
    script = [("ops", init), ("bolt", ["start"]), ("bolt", ["status"])]
    for step in range(rng.randint(2, 6)):
        script.append(("ops", mutate(rng, files, dirs_with_rules)))
        script.append(("bolt", rng.choice([["status"], ["status"], ["save", "s%d" % step],
                                           ["save", "s%d" % step], ["changes"], ["discard"],
                                           ["goto", "main"], ["undo"]])))
        script.append(("bolt", ["status"]))
    seen = {}
    for b, root in roots.items():
        outs = []
        for kind, what in script:
            if kind == "ops":
                apply(root, what)
            else:
                outs.append((what, run(b, root, what, envs[b])))
        seen[b] = outs
    bad = None
    for (cmd, a), (_, o) in zip(seen[NEW], seen[OLD]):
        if a != o:
            bad = (cmd, a, o)
            break
    if bad:
        print("SEED", seed, "differs at", bad[0])
        print(" new:", bad[1])
        print(" old:", bad[2])
        print(" script:", [s for s in script if s[0] == "bolt" or True][:40])
    shutil.rmtree(base, ignore_errors=True)
    return bad is None


if __name__ == "__main__":
    rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 50
    seed0 = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    ok = sum(one(seed0 + i) for i in range(rounds))
    print("rounds", rounds, "agreed", ok)
