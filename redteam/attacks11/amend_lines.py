"""RT12: an imported Git repository with many branches over one long history; `save --amend` asks
every other line whether it reaches the save, and the change-ID cache keeps each line's saves.
Peak memory and time of the amend, and of `undo --list`, on this build and on main's, free and
under the 1.5 GB address-space cap cli.rs uses (`bolt_capped`).
Run in the sandbox only:  redteam/attacks11/run.sh script amend_lines.py [commits] [branches]
"""
import os, shutil, subprocess, sys, tempfile, time

NEW = "/work/target/release/bolt"
OLD = "/work/_main/target/release/bolt"
H = int(sys.argv[1]) if len(sys.argv) > 1 else 1000
N = int(sys.argv[2]) if len(sys.argv) > 2 else 15000


def git_repo(path):
    os.makedirs(path)
    env = dict(os.environ, GIT_AUTHOR_NAME="a", GIT_AUTHOR_EMAIL="a@b", GIT_COMMITTER_NAME="a",
               GIT_COMMITTER_EMAIL="a@b")
    subprocess.run(["git", "init", "-q", "-b", "main", path], check=True)
    lines = []
    for i in range(H):
        data = ("v%d\n" % i).encode()
        lines.append(b"commit refs/heads/main\nmark :%d\ncommitter a <a@b> %d +0000\ndata 2\nc\n" % (i + 1, 1700000000 + i))
        if i:
            lines.append(b"from :%d\n" % i)
        lines.append(b"M 100644 inline f\ndata %d\n%s\n" % (len(data), data))
    subprocess.run(["git", "fast-import", "--quiet"], cwd=path, input=b"".join(lines), check=True, env=env)
    parent = subprocess.run(["git", "rev-parse", "main~1"], cwd=path, capture_output=True, check=True).stdout.strip()
    refs = b"".join(b"create refs/heads/b%06d %s\n" % (j, parent) for j in range(N))
    subprocess.run(["git", "update-ref", "--stdin"], cwd=path, input=refs, check=True)
    subprocess.run(["git", "checkout", "-q", "-f", "main"], cwd=path, check=True)


def run(bolt, root, args, env, cap_kb=None):
    pre = None
    if cap_kb:
        import resource
        def pre():
            resource.setrlimit(resource.RLIMIT_AS, (cap_kb * 1024, cap_kb * 1024))
    t = time.time()
    p = subprocess.Popen([bolt] + args, cwd=root, env=env, stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, preexec_fn=pre)
    out, err = p.stdout.read(), p.stderr.read()
    _, status, usage = os.wait4(p.pid, 0)
    code = os.waitstatus_to_exitcode(status)
    return {"secs": round(time.time() - t, 1), "exit": code, "peak_mb": usage.ru_maxrss // 1024,
            "said": (out + err).decode("utf-8", "replace").strip().replace("\n", " | ")[:240]}


base = tempfile.mkdtemp(prefix="rt12amend")
src = os.path.join(base, "git")
git_repo(src)
print("history", H, "branches", N, flush=True)
for name, bolt in (("new", NEW), ("main", OLD)):
    root = os.path.join(base, name)
    shutil.copytree(src, root)
    cfg = os.path.join(base, "cfg-" + name); os.makedirs(cfg)
    env = dict(os.environ, BOLT_CONFIG_DIR=cfg, HOME=cfg, BOLT_USER_NAME="r", BOLT_USER_EMAIL="r@x")
    print(name, "import", run(bolt, root, ["import"], env), flush=True)
    with open(os.path.join(root, "f"), "a") as f:
        f.write("more\n")
    for label, cap in (("capped 1.5 GB", 1_500_000), ("free", None)):
        r = os.path.join(base, name + "-" + label.split()[0])
        shutil.copytree(root, r, symlinks=True)
        print(name, label, "amend", run(bolt, r, ["save", "--amend", "m"], env, cap), flush=True)
        print(name, label, "amend again", run(bolt, r, ["save", "--amend", "m2"], env, cap), flush=True)
        cache = os.path.join(r, ".bolt/cache/change-ids")
        print(name, label, "change-ids bytes", os.path.getsize(cache) if os.path.exists(cache) else None, flush=True)
        shutil.rmtree(r, ignore_errors=True)
shutil.rmtree(base, ignore_errors=True)
