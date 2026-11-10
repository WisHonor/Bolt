"""RT12: projects built to be slow, `status` and `save` timed on this build and on main's.
Wide folder, names all outside ASCII, a rules file of one-star lines that all fail, the same
plain line many times with a `/` at its end, many `!` lines.
Run in the sandbox only:  redteam/attacks11/run.sh script slow_projects.py [scale]
"""
import os, shutil, subprocess, sys, tempfile, time

NEW = "/work/target/release/bolt"
OLD = "/work/_main/target/release/bolt"
SCALE = int(sys.argv[1]) if len(sys.argv) > 1 else 1


def timed(bolt, root, args, env):
    t = time.time()
    p = subprocess.run([bolt] + args, cwd=root, env=env, capture_output=True, timeout=1800)
    return round(time.time() - t, 2), p.returncode, p.stderr.decode()[-200:]


def project(build):
    base = tempfile.mkdtemp(prefix="rt12slow")
    root = os.path.join(base, "p"); os.makedirs(root)
    cfg = os.path.join(base, "cfg"); os.makedirs(cfg)
    env = dict(os.environ, BOLT_CONFIG_DIR=cfg, BOLT_USER_NAME="r", BOLT_USER_EMAIL="r@x")
    build(root)
    return base, root, env


def wide(root):
    d = os.path.join(root, "w"); os.makedirs(d)
    for i in range(150_000 * min(SCALE, 1)):
        with open(os.path.join(d, "f%07d" % i), "w") as f:
            f.write("x")


def unicode_names(root):
    for i in range(3000 * SCALE):
        d = os.path.join(root, "é%05d" % i)
        os.makedirs(d)
        with open(os.path.join(d, "ü"), "w") as f:
            f.write("x")


def star_lines(root):
    with open(os.path.join(root, ".gitignore"), "w") as f:
        for i in range(100_000):
            f.write("q%d*z\n" % i)
    for i in range(2000):
        d = os.path.join(root, "d%03d" % (i % 50)); os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "a%05d.txt" % i), "w") as f:
            f.write("x")


def same_plain_dir_lines(root):
    with open(os.path.join(root, ".gitignore"), "w") as f:
        for i in range(100_000):
            f.write("a.txt/\n")
    for i in range(2000):
        d = os.path.join(root, "d%04d" % i); os.makedirs(d)
        with open(os.path.join(d, "a.txt"), "w") as f:
            f.write("x")


def negations(root):
    with open(os.path.join(root, ".gitignore"), "w") as f:
        f.write("*\n")
        for i in range(100_000):
            f.write("!n%d\n" % i)
        f.write("!*/\n!*.txt\n")
    for i in range(2000):
        d = os.path.join(root, "d%04d" % i); os.makedirs(d)
        with open(os.path.join(d, "a.txt"), "w") as f:
            f.write("x")


for name, build in [("wide", wide), ("unicode", unicode_names), ("star lines", star_lines),
                    ("same plain dir lines", same_plain_dir_lines), ("negations", negations)]:
    base, root, env = project(build)
    for bolt in (NEW, OLD):
        r = os.path.join(base, "run"); shutil.rmtree(r, ignore_errors=True)
        shutil.copytree(root, r, symlinks=True)
        s = timed(bolt, r, ["start"], env)
        st = [timed(bolt, r, ["status"], env) for _ in range(2)]
        sv = timed(bolt, r, ["save", "x", "--yes"], env)
        st2 = timed(bolt, r, ["status"], env)
        print(name, "new" if bolt == NEW else "main", "start", s[:2], "status", [x[:2] for x in st],
              "save", sv[:2], "status after", st2[:2], flush=True)
    shutil.rmtree(base, ignore_errors=True)
