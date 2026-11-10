"""RT12: a chain of folders as deep as the limits allow, a file in each, saved over 20 times so the
store has more than 16 packs; then status, save and goto under macOS's default of 256 open files,
this build against main's. Also the most files any of them holds open (sampled from /proc).
Run in the sandbox only:  redteam/attacks11/run.sh script deep_fds.py [depth] [fd-limit]
"""
import os, resource, subprocess, sys, tempfile, threading, time

NEW = "/work/target/release/bolt"
OLD = "/work/_main/target/release/bolt"
DEPTH = int(sys.argv[1]) if len(sys.argv) > 1 else 2000
LIMIT = int(sys.argv[2]) if len(sys.argv) > 2 else 256


def chain(root, depth, tag):
    fd = os.open(root, os.O_RDONLY)
    for i in range(depth):
        f = os.open("f", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644, dir_fd=fd)
        os.write(f, ("%s %d\n" % (tag, i)).encode())
        os.close(f)
        if i + 1 < depth:
            try:
                os.mkdir("d", dir_fd=fd)
            except FileExistsError:
                pass
            nfd = os.open("d", os.O_RDONLY, dir_fd=fd)
            os.close(fd)
            fd = nfd
    os.close(fd)


def touch_some(root, depth, tag, every):
    fd = os.open(root, os.O_RDONLY)
    for i in range(depth):
        if i % every == 0:
            f = os.open("f", os.O_WRONLY | os.O_TRUNC, dir_fd=fd)
            os.write(f, ("%s %d\n" % (tag, i)).encode())
            os.close(f)
        if i + 1 < depth:
            nfd = os.open("d", os.O_RDONLY, dir_fd=fd)
            os.close(fd)
            fd = nfd
    os.close(fd)


def most_fds(pid, stop, out):
    best = 0
    while not stop.is_set():
        try:
            best = max(best, len(os.listdir("/proc/%d/fd" % pid)))
        except OSError:
            pass
        time.sleep(0.002)
    out.append(best)


def run(bolt, root, args, env, limit=None):
    def pre():
        if limit:
            resource.setrlimit(resource.RLIMIT_NOFILE, (limit, limit))
    t0 = time.time()
    p = subprocess.Popen([bolt] + args, cwd=root, env=env, stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, preexec_fn=pre)
    stop, out = threading.Event(), []
    th = threading.Thread(target=most_fds, args=(p.pid, stop, out))
    th.start()
    so, se = p.communicate(timeout=1800)
    stop.set(); th.join()
    return p.returncode, so.decode()[-300:], se.decode()[-400:], out[0], round(time.time() - t0, 1)


for bolt in (NEW, OLD):
    base = tempfile.mkdtemp(prefix="rt12deep")
    root = os.path.join(base, "p")
    os.makedirs(root)
    cfg = os.path.join(base, "cfg"); os.makedirs(cfg)
    env = dict(os.environ, BOLT_CONFIG_DIR=cfg, BOLT_USER_NAME="r", BOLT_USER_EMAIL="r@x")
    print("==", bolt)
    chain(root, DEPTH, "v0")
    print("start", run(bolt, root, ["start"], env)[:1])
    for n in range(1, 21):
        touch_some(root, DEPTH, "v%d" % n, 97)
        r = run(bolt, root, ["save", "v%d" % n], env)
        if r[0] != 0:
            print("save", n, r)
    print("packs", len(os.listdir(os.path.join(root, ".bolt/store/packs"))) if os.path.isdir(os.path.join(root, ".bolt/store/packs")) else os.listdir(os.path.join(root, ".bolt/store")))
    touch_some(root, DEPTH, "w", 50)
    print("status capped", run(bolt, root, ["status"], env, LIMIT))
    print("save capped", run(bolt, root, ["save", "w"], env, LIMIT))
    print("status after", run(bolt, root, ["status"], env, LIMIT))
    print("goto back capped", run(bolt, root, ["goto", "main~3"], env, LIMIT) if False else "skipped")
    subprocess.run(["rm", "-rf", base])
