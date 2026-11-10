"""RT13: differential fuzz of lines, amends and gotos by every kind of ID, this build against
main's, with cache/change-ids kept, removed, or planted from an older state between commands.
Aims at RT12-1's fix (`lines_reaching`, the amend's "shown elsewhere" answer) and RT12-3's
(`by_fingerprint_prefix` with `links_hold`). Every command's exit code and normalised output, and
the folder's files, must agree.
Run in the sandbox only:  redteam/attacks12/run.sh script lines_fuzz.py [rounds] [seed]
"""
import json, os, random, re, shutil, subprocess, sys, tempfile

NEW = "/work/target/release/bolt"
OLD = "/work/_main/target/release/bolt"
ID = re.compile(r"\b[k-z]{4,32}\b|\b[0-9a-f]{8,64}\b|/tmp/[^\s\"']+|\d+ (seconds?|minutes?) ago|just now")


def norm(s):
    return ID.sub("<id>", s)


def run(bolt, root, args, env):
    p = subprocess.run([bolt] + args, cwd=root, env=env, capture_output=True, timeout=300)
    return p.returncode, norm(p.stdout.decode("utf-8", "replace")), norm(p.stderr.decode("utf-8", "replace"))


def known(bolt, root, env, into):
    p = subprocess.run([bolt, "history", "--json"], cwd=root, env=env, capture_output=True, timeout=300)
    try:
        for s in json.loads(p.stdout)["saves"]:
            into[s["message"]] = (s["id"], s["change_id"], s["change_id_short"])
    except (ValueError, KeyError):
        pass


def files(root):
    out = []
    for d, dirs, fs in os.walk(root):
        if d == root and ".bolt" in dirs:
            dirs.remove(".bolt")
        dirs.sort()
        for f in sorted(fs):
            with open(os.path.join(d, f), "rb") as h:
                out.append((os.path.relpath(d, root), f, h.read()))
    return out


def plan(rng, steps):
    script = []
    n = 0
    for _ in range(steps):
        n += 1
        k = rng.random()
        if k < 0.6:
            script.append(("write", rng.choice(["a", "b", "d/c"]), "v%d" % n))
        if rng.random() < 0.15:
            script.append(("cache", rng.choice(["rm", "keep", "old"])))
        k = rng.random()
        if k < 0.22:
            script.append(("bolt", ["save", "m%d" % n]))
        elif k < 0.37:
            script.append(("bolt", ["save", "--amend", "a%d" % n]))
        elif k < 0.55:
            script.append(("goto", rng.choice(["full", "hex8", "change", "short"])))
        elif k < 0.65:
            script.append(("bolt", ["save", "--line", "l%d" % n, "m%d" % n]))
        elif k < 0.75:
            script.append(("gotoline", None))
        elif k < 0.82:
            script.append(("bolt", ["undo"]))
        elif k < 0.86:
            script.append(("bolt", ["redo"]))
        elif k < 0.90:
            script.append(("bolt", ["undo", "--list"]))
        elif k < 0.95:
            script.append(("bolt", ["history"]))
        else:
            script.append(("bolt", ["status"]))
    return script


def one(seed, steps):
    rng = random.Random(seed)
    script = plan(rng, steps)
    base = tempfile.mkdtemp(prefix="rt13l")
    seen = {}
    for b, name in ((NEW, "new"), (OLD, "old")):
        root = os.path.join(base, name)
        os.makedirs(root)
        cfg = os.path.join(base, "cfg-" + name)
        os.makedirs(cfg)
        env = dict(os.environ, BOLT_CONFIG_DIR=cfg, HOME=cfg, BOLT_USER_NAME="r", BOLT_USER_EMAIL="r@x")
        pick = random.Random(seed * 7 + 1)
        saves, lines = {}, ["main"]
        old_cache = None
        outs = [run(b, root, ["start"], env)]
        cache = os.path.join(root, ".bolt", "cache", "change-ids")
        for step in script:
            kind = step[0]
            if kind == "write":
                path = os.path.join(root, step[1])
                os.makedirs(os.path.dirname(path), exist_ok=True)
                with open(path, "w") as f:
                    f.write(step[2])
                continue
            if kind == "cache":
                if step[1] == "rm" and os.path.exists(cache):
                    os.remove(cache)
                elif step[1] == "keep" and os.path.exists(cache):
                    with open(cache, "rb") as f:
                        old_cache = f.read()
                elif step[1] == "old" and old_cache is not None:
                    with open(cache, "wb") as f:
                        f.write(old_cache)
                continue
            if kind == "goto":
                msgs = sorted(saves)
                if not msgs:
                    continue
                full, change, short = saves[pick.choice(msgs)]
                arg = {"full": full, "hex8": full[:8], "change": change, "short": short}[step[1]]
                args = ["goto", arg]
            elif kind == "gotoline":
                args = ["goto", pick.choice(lines)]
            else:
                args = step[1]
                if args[:2] == ["save", "--line"]:
                    lines.append(args[2])
            outs.append((kind, args[0], run(b, root, args, env)))
            known(b, root, env, saves)
            if args[0] in ("goto", "undo", "redo", "save"):
                outs.append(("files", files(root)))
        seen[b] = outs
    bad = None
    for a, o in zip(seen[NEW], seen[OLD]):
        if a != o:
            bad = (a, o)
            break
    if bad:
        print("SEED", seed, "differs")
        print(" new:", str(bad[0])[:600])
        print(" old:", str(bad[1])[:600])
        print(" script:", script[:80])
    shutil.rmtree(base, ignore_errors=True)
    return bad is None


if __name__ == "__main__":
    rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 50
    seed0 = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    steps = int(sys.argv[3]) if len(sys.argv) > 3 else 40
    ok = sum(one(seed0 + i, steps) for i in range(rounds))
    print("rounds", rounds, "agreed", ok)
