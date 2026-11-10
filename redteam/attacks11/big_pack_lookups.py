"""RT12: lookups in a big pack. A project saved with many small files (one pack of that many
objects), then many new small files saved on top: every new file is looked up in the big pack
and isn't there. Timed on this build and on main's, three times each, alternating.
Run in the sandbox only:  redteam/attacks11/run.sh script big_pack_lookups.py [first] [new]
"""
import os, shutil, subprocess, sys, tempfile, time

NEW = "/work/target/release/bolt"
OLD = "/work/_main/target/release/bolt"
FIRST = int(sys.argv[1]) if len(sys.argv) > 1 else 200_000
MORE = int(sys.argv[2]) if len(sys.argv) > 2 else 100_000


def files(root, prefix, count):
    for i in range(count):
        d = os.path.join(root, "%s%03d" % (prefix, i % 500))
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "f%07d" % i), "w") as f:
            f.write("%s %d\n" % (prefix, i))


def timed(bolt, root, args, env):
    t = time.time()
    p = subprocess.run([bolt] + args, cwd=root, env=env, capture_output=True, timeout=3000)
    return round(time.time() - t, 2), p.returncode, p.stdout.decode()[-80:].strip()


base = tempfile.mkdtemp(prefix="rt12pack")
seed = os.path.join(base, "seed"); os.makedirs(seed)
cfg = os.path.join(base, "cfg"); os.makedirs(cfg)
env = dict(os.environ, BOLT_CONFIG_DIR=cfg, BOLT_USER_NAME="r", BOLT_USER_EMAIL="r@x")
files(seed, "a", FIRST)
print("start (main's bolt)", timed(OLD, seed, ["start"], env), flush=True)
print("first save (main's bolt)", timed(OLD, seed, ["save", "first", "--yes"], env), flush=True)
packs = os.path.join(seed, ".bolt/store/packs")
print("packs", [os.path.getsize(os.path.join(packs, p)) for p in os.listdir(packs)], flush=True)
files(seed, "b", MORE)
for round_ in range(3):
    for name, bolt in (("new", NEW), ("main", OLD)):
        r = os.path.join(base, "r"); shutil.rmtree(r, ignore_errors=True)
        shutil.copytree(seed, r)
        print(round_, name, "status", timed(bolt, r, ["status"], env)[:2],
              "save", timed(bolt, r, ["save", "more", "--yes"], env), flush=True)
shutil.rmtree(base, ignore_errors=True)
