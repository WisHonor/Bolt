// RT14 exploration: Bolt's `.git` look-alike test against Git's own (verify_path with
// core.protectNTFS and core.protectHFS, through `git update-index --index-info`). Sandbox only.
use bolt_core::names::is_git_alias;
use std::io::Write;
use std::process::{Command, Stdio};

const PARTS: &[&str] = &[
    ".", "g", "G", "i", "I", "t", "T", "~", "1", "2", " ", "..", ":", ":x", "\\", "\u{200c}",
    "\u{200f}", "\u{202a}", "\u{206f}", "\u{feff}", "\u{fffe}", "\u{ffff}", "\u{2060}", "\u{00ad}",
    "\u{0130}", "\u{0131}", "\u{212a}", "x", "git", ".git", "git~1", "GIT~1", "gi7eba~1",
];

fn names() -> Vec<String> {
    let mut out = Vec::new();
    // Every spelling up to four parts long from the start of `.git` or `git~1`, then random.
    let mut seed: u64 = 0xdead_beef_1234_5678;
    for len in 1..=3 {
        let mut idx = vec![0usize; len];
        loop {
            out.push(idx.iter().map(|&i| PARTS[i]).collect::<String>());
            let mut k = 0;
            while k < len {
                idx[k] += 1;
                if idx[k] < PARTS.len() {
                    break;
                }
                idx[k] = 0;
                k += 1;
            }
            if k == len {
                break;
            }
        }
    }
    for _ in 0..200_000 {
        let n = 2 + (seed % 5) as usize;
        let mut s = String::new();
        for _ in 0..n {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            s.push_str(PARTS[(seed % PARTS.len() as u64) as usize]);
        }
        out.push(s);
    }
    out.retain(|n| !n.is_empty() && n != "." && n != ".." && !n.contains('/'));
    out.sort();
    out.dedup();
    out
}

#[test]
fn rt14_alias_vs_git() {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| Command::new("git").args(args).current_dir(dir.path()).output().unwrap();
    git(&["init", "-q"]);
    let blob = String::from_utf8(git(&["hash-object", "-w", "/dev/null"]).stdout).unwrap();
    let names = names();
    let mut input = Vec::new();
    for (i, n) in names.iter().enumerate() {
        // A folder named `n` holding a file; numbered so each name is its own entry.
        input.extend_from_slice(format!("100644 {} 0\t{n}/{i}", blob.trim()).as_bytes());
        input.push(0);
    }
    let mut child = Command::new("git")
        .args(["-c", "core.protectNTFS=true", "-c", "core.protectHFS=true", "update-index", "-z", "--index-info"])
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    child.wait().unwrap();
    let listed = git(&["ls-files", "-z"]).stdout;
    let mut accepted = std::collections::HashSet::new();
    for p in listed.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        let p = String::from_utf8(p.to_vec()).unwrap();
        let (_, i) = p.rsplit_once('/').unwrap();
        accepted.insert(i.parse::<usize>().unwrap());
    }
    let (mut bolt_misses, mut bolt_extra) = (0, 0);
    for (i, n) in names.iter().enumerate() {
        let git_refuses = !accepted.contains(&i);
        let bolt_refuses = is_git_alias(n);
        if git_refuses && !bolt_refuses {
            bolt_misses += 1;
            if bolt_misses <= 30 {
                println!("MISS (Git refuses, Bolt takes) {n:?}");
            }
        }
        if bolt_refuses && !git_refuses {
            bolt_extra += 1;
            if bolt_extra <= 15 {
                println!("EXTRA (Bolt refuses, Git takes) {n:?}");
            }
        }
    }
    println!("names={} misses={bolt_misses} extra={bolt_extra}", names.len());
}
