// RT14 exploration: Bolt's Git LFS pointer reader against `git lfs pointer --check` on mutated
// pointers. Sandbox only.
use bolt_core::git::lfs_pointer;
use std::process::Command;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn upto(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.upto(xs.len())]
    }
}

const OID: &str = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";
const SPACES: &[&str] = &["", " ", "\t", "\n", "\r\n", "\u{a0}", "\u{85}", "\u{2028}", "\u{b}", "\u{c}", "\u{3000}", "\u{feff}", "\0"];
const VERSIONS: &[&str] = &[
    "https://git-lfs.github.com/spec/v1",
    "https://hawser.github.com/spec/v1",
    "http://git-media.io/v/2",
    "https://git-lfs.github.com/spec/v1 ",
    "https://git-lfs.github.com/spec/v2",
];
const SIZES: &[&str] = &["12", "+12", "-0", "0", "007", "-1", "1e3", " 12", "12 ", "9223372036854775807", "9223372036854775808", "1_2", "0x10", "١٢"];
const EXTS: &[&str] = &["ext-0-foo", "ext-1-bar", "ext-1-foo", "ext-9-_x", "ext-1-!x", "ext-12-a", "ext-a-b", "ext-1-a!b", "ext-1-é", "ext--1-a", "Ext-1-a"];

fn git_lfs_accepts(dir: &std::path::Path, data: &[u8]) -> bool {
    let f = dir.join("p");
    std::fs::write(&f, data).unwrap();
    Command::new("git")
        .args(["lfs", "pointer", "--check", "--file=p"])
        .current_dir(dir)
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn rt14_lfs_vs_git_lfs() {
    let iters: usize = std::env::var("RT14_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(1500);
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let dir = tempfile::tempdir().unwrap();
    assert!(Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().unwrap().success());
    let mut diffs = 0;
    for it in 0..iters {
        let mut lines: Vec<String> = vec![
            format!("version {}", if rng.upto(3) == 0 { rng.pick(VERSIONS) } else { VERSIONS[0] }),
            format!("oid sha256:{OID}"),
            format!("size {}", if rng.upto(3) == 0 { rng.pick(SIZES) } else { "12" }),
        ];
        for _ in 0..rng.upto(3) {
            let at = rng.upto(lines.len() + 1);
            let value = if rng.upto(4) == 0 { "sha256:xyz".to_owned() } else { format!("sha256:{OID}") };
            lines.insert(at, format!("{} {value}", rng.pick(EXTS)));
        }
        match rng.upto(10) {
            0 => lines.swap(rng.upto(3), rng.upto(3)),
            1 => { let i = rng.upto(lines.len()); lines[i] = lines[i].replacen(' ', "\t", 1); }
            2 => { let i = rng.upto(lines.len()); lines[i] = lines[i].replacen(' ', "  ", 1); }
            3 => { let i = rng.upto(lines.len() + 1); lines.insert(i, String::new()); }
            4 => { let i = rng.upto(lines.len() + 1); lines.insert(i, rng.pick(SPACES).to_owned()); }
            5 => { let i = rng.upto(lines.len()); let l = lines[i].clone(); lines.insert(i, l); }
            _ => {}
        }
        let sep = if rng.upto(4) == 0 { "\r\n" } else { "\n" };
        let mut text = String::new();
        text.push_str(rng.pick(SPACES));
        text.push_str(&lines.join(sep));
        if rng.upto(2) == 0 { text.push_str(sep); }
        text.push_str(rng.pick(SPACES));
        let mut data = text.into_bytes();
        match rng.upto(8) {
            0 => data.push(0xff),
            1 => data.insert(0, 0xff),
            2 => { let pad = 1024usize.saturating_sub(data.len()) + rng.upto(3); data.extend(std::iter::repeat(b'\n').take(pad)); }
            _ => {}
        }
        let git = git_lfs_accepts(dir.path(), &data);
        let bolt = lfs_pointer(&data).is_some();
        let text = String::from_utf8_lossy(&data).into_owned();
        let keys: Vec<&str> = text.lines().filter(|l| l.trim_start().starts_with("ext-")).map(|l| l.trim_start().split(' ').next().unwrap_or("")).collect();
        let dup = keys.iter().enumerate().any(|(i, k)| keys[..i].contains(k));
        // Known: over 1,024 bytes (Git LFS's smudge passes it through) and a repeated ext- key.
        if git != bolt && data.len() <= 1024 && !dup {
            diffs += 1;
            if diffs <= 30 {
                println!("DIFF it={it} git={git} bolt={bolt} data={:?}", String::from_utf8_lossy(&data));
            }
        }
    }
    println!("iters={iters} diffs={diffs}");
}
