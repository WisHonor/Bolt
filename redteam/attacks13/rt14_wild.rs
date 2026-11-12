// RT14 exploration: random ignore files from a wide alphabet, Bolt's Git verdict against
// `git check-ignore`, with letter case counted and folded. Sandbox only.
use bolt_core::drive::Drive;
use bolt_core::ignore::{Decision, Matcher, Rules};
use std::io::Write;
use std::process::{Command, Stdio};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[(self.next() % xs.len() as u64) as usize]
    }
    fn upto(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const PIECES: &[&str] = &[
    "a", "b", "A", "B", "z", "/", "*", "**", "?", "[", "]", "!", "^", "-", "\\", ":", "[:alpha:]",
    "[:upper:]", "[:lower:]", "[:digit:]", " ", ".", "#", "[a-z]", "[A-Z]", "[Z-a]", "[]]", "[!]",
    "[^a]", "\\*", "\\[", "\\]", "\\/", "**/", "/**", "[\\]]", "[a-]", "[-a]", "\\ ", "[[]",
    "[:", ":]", "Z", "_", "`",
];
const NAMES: &[&str] = &[
    "a", "b", "A", "B", "z", "Z", "ab", "aB", "Ab", "]", "[", "-", "!", "^", ":", ".", " ", "a b",
    "a]", "[a]", "\\", "_", "`", "*", "?", "#a", "a\\", "zZ", "a-b", "AB",
];

fn git_verdicts(dir: &std::path::Path, fold: bool, paths: &[String]) -> Vec<Decision> {
    let mut child = Command::new("git")
        .env("GIT_LITERAL_PATHSPECS", "1").args(["-c", &format!("core.ignorecase={fold}"), "check-ignore", "--no-index", "-v", "-n", "-z", "--stdin"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = Vec::new();
    for p in paths {
        input.extend_from_slice(p.as_bytes());
        input.push(0);
    }
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let out = child.wait_with_output().unwrap().stdout;
    let fields: Vec<&[u8]> = out.split(|&b| b == 0).collect();
    let mut v = Vec::new();
    for rec in fields.chunks(4) {
        if rec.len() < 4 {
            break;
        }
        v.push(if rec[0].is_empty() {
            Decision::NoRule
        } else if rec[2].starts_with(b"!") {
            Decision::Kept
        } else {
            Decision::Ignored
        });
    }
    v
}

fn version_changed(line: &str) -> bool {
    let text = line.strip_prefix('!').unwrap_or(line);
    let text = text.strip_suffix('/').unwrap_or(text);
    if !text.contains('/') {
        return false;
    }
    let text = text.strip_prefix('/').unwrap_or(text);
    let literal = text.find(['*', '?', '[', '\\']).unwrap_or(text.len());
    literal > 0 && text[literal..].starts_with("**") && !text[..literal].ends_with('/')
}

#[test]
fn rt14_wild_vs_git() {
    let iters: u64 = std::env::var("RT14_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(500);
    let seed: u64 = std::env::var("RT14_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0x9e3779b97f4a7c15);
    let mut rng = Rng(seed);
    let dir = tempfile::tempdir().unwrap();
    assert!(Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().unwrap().success());
    let mut bad = 0;
    for it in 0..iters {
        let nlines = 1 + rng.upto(3);
        let mut lines = Vec::new();
        while lines.len() < nlines as usize {
            let n = 1 + rng.upto(5);
            let mut l = String::new();
            for _ in 0..n {
                l.push_str(rng.pick(PIECES));
            }
            if version_changed(&l) {
                continue;
            }
            lines.push(l);
        }
        let text = lines.join("\n") + "\n";
        std::fs::write(dir.path().join(".gitignore"), &text).unwrap();
        let mut paths = Vec::new();
        for _ in 0..40 {
            let depth = 1 + rng.upto(3);
            let p: Vec<&str> = (0..depth).map(|_| rng.pick(NAMES)).collect();
            let p = p.join("/");
            if p.split('/').any(|c| c == "." || c == ".git") {
                continue;
            }
            paths.push(p);
        }
        for fold in [false, true] {
            let git = git_verdicts(dir.path(), fold, &paths);
            let mut m = Matcher::new(Drive { ignores_case: fold, composes_unicode: false });
            m.enter("", vec![Rules::parse(text.as_bytes(), "", true)]);
            for (p, g) in paths.iter().zip(&git) {
                // Git reports a path below a folder its rules keep out by that folder's line.
                let parts: Vec<&str> = p.split('/').collect();
                let parent = (1..parts.len())
                    .any(|k| m.verdict(parts[..k].join("/"), true).git == Decision::Ignored);
                let b = if parent { Decision::Ignored } else { m.verdict(p, false).git };
                if b != *g {
                    bad += 1;
                    if bad <= 40 {
                        println!("DIFF it={it} fold={fold} rules={text:?} path={p:?} bolt={b:?} git={g:?}");
                    }
                }
            }
        }
    }
    println!("iters={iters} diffs={bad}");
}
