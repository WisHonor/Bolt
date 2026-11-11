"""RT13, after RT12-1's fix: `save --amend` no longer keeps a list for each other line; it walks
each other line until it meets the amended line's list. Lines whose histories never meet it (each
branch its own chain of commits off the first commit) are walked to their roots on every amend.
Main kept their lists in cache/change-ids, so its second amend read nothing. Time and peak memory
of three amends in a row, this build against main's.
Run in the sandbox only:  redteam/attacks12/run.sh script amend_side_histories.py [branches] [depth]
"""
import os, shutil, subprocess, sys, tempfile, time

NEW = "/work/target/release/bolt"
OLD = "/work/_main/target/release/bolt"
N = int(sys.argv[1]) if len(sys.argv) > 1 else 200
D = int(sys.argv[2]) if len(sys.argv) > 2 else 1000


def git_repo(path):
    os.makedirs(path)
    env = dict(os.environ, GIT_AUTHOR_NAME="a", GIT_AUTHOR_EMAIL="a@b", GIT_COMMITTER_NAME="a",
               GIT_COMMITTER_EMAIL="a@b")
    subprocess.run(["git", "init", "-q", "-b", "main", path], check=True)
    out = []
    mark = 1
    out.append(b"commit refs/heads/main\nmark :1\ncommitter a <a@b> 1700000000 +0000\ndata 1\nr\nM 100644 inline f\ndata 2\nr\n\n")
    out.append(b"commit refs/heads/main\nmark :2\ncommitter a <a@b> 1700000001 +0000\ndata 1\nm\nfrom :1\nM 100644 inline f\ndata 2\nm\n\n")
    mark = 2
    for b in range(N):
        prev = 1
        for i in range(D):
            mark += 1
            data = b"b%d.%d\n" % (b, i)
            out.append(b"commit refs/heads/b%05d\nmark :%d\ncommitter a <a@b> %d +0000\ndata 1\nc\nfrom :%d\n"
                       b"M 100644 inline g\ndata %d\n%s\n" % (b, mark, 1700000002 + mark, prev, len(data), data))
            prev = mark
    subprocess.run(["git", "fast-import", "--quiet"], cwd=path, input=b"".join(out), check=True, env=env)
    subprocess.run(["git", "checkout", "-q", "-f", "main"], cwd=path, check=True)


def run(bolt, root, args, env):
    t = time.time()
    p = subprocess.Popen([bolt] + args, cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    out, err = p.stdout.read(), p.stderr.read()
    _, status, usage = os.wait4(p.pid, 0)
    return {"secs": round(time.time() - t, 2), "exit": os.waitstatus_to_exitcode(status),
            "peak_mb": usage.ru_maxrss // 1024,
            "said": (out + err).decode("utf-8", "replace").strip().replace("\n", " | ")[:160]}


base = tempfile.mkdtemp(prefix="rt13side")
src = os.path.join(base, "git")
git_repo(src)
print("branches", N, "each", D, "commits off the first", flush=True)
for name, bolt in (("new", NEW), ("main", OLD)):
    root = os.path.join(base, name)
    shutil.copytree(src, root)
    cfg = os.path.join(base, "cfg-" + name)
    os.makedirs(cfg)
    env = dict(os.environ, BOLT_CONFIG_DIR=cfg, HOME=cfg, BOLT_USER_NAME="r", BOLT_USER_EMAIL="r@x")
    print(name, "import", run(bolt, root, ["import"], env)["secs"], "s", flush=True)
    for n in range(3):
        with open(os.path.join(root, "f"), "a") as f:
            f.write("more %d\n" % n)
        print(name, "amend", n + 1, run(bolt, root, ["save", "--amend", "m%d" % n], env), flush=True)
    cache = os.path.join(root, ".bolt/cache/change-ids")
    print(name, "change-ids bytes", os.path.getsize(cache) if os.path.exists(cache) else None, flush=True)
    shutil.rmtree(root, ignore_errors=True)
shutil.rmtree(base, ignore_errors=True)
