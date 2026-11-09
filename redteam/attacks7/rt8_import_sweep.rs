//! Exploration: `bolt import` of Git LFS files and a big delta family, stopped at every abort
//! point, then run again: it must finish with every file as Git LFS checks it out.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "fault-injection")]
mod support;

use std::fs;

use bolt_core::git::craft::lfs_pointer_to;
use support::{Env, main_tip, noise, saved_files, text, write};

fn keep(env: &Env, contents: &[u8]) -> String {
    let (pointer, at) = lfs_pointer_to(contents);
    let at = env.root().join(".git").join(at);
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    fs::write(at, contents).unwrap();
    pointer
}

#[test]
fn sweep_import_with_lfs_and_a_family() {
    let t = Env::new();
    t.commit(
        ".gitattributes",
        "*.psd filter=lfs diff=lfs merge=lfs -text\n",
        "Track",
    );
    let lfs_big = noise(21, 3 << 20);
    let lfs_small = b"small picture\n".to_vec();
    write(&t, "art/big.psd", keep(&t, &lfs_big));
    write(&t, "art/small.psd", keep(&t, &lfs_small));
    let mut family = noise(22, 4 << 20);
    write(&t, "data.bin", &family);
    t.commit("readme.txt", "one\n", "One");
    family[100_000..100_008].copy_from_slice(b"EDIT-ONE");
    write(&t, "data.bin", &family);
    let mut lfs_big2 = lfs_big.clone();
    lfs_big2[2_000_000..2_000_008].copy_from_slice(b"EDIT-TWO");
    write(&t, "art/big.psd", keep(&t, &lfs_big2));
    t.commit("readme.txt", "two\n", "Two");
    t.git(&["repack", "-a", "-d", "-f", "-q"]);
    let expected = {
        let probe = Env::empty();
        support::copy_dir(t.root(), probe.root());
        probe.import();
        saved_files(&probe, main_tip(&probe))
    };
    assert!(expected["data.bin"] == family);
    assert!(expected["art/big.psd"] == lfs_big2);
    for n in 1..400 {
        let env = Env::empty();
        support::copy_dir(t.root(), env.root());
        let out = env
            .bolt()
            .arg("import")
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        let said = text(&out.stderr);
        let at = said
            .split_once("injected crash at ")
            .map(|(_, r)| r.lines().next().unwrap().to_owned());
        let st = env.bolt().arg("status").output().unwrap();
        let again = env.bolt().arg("import").output().unwrap();
        let log = format!(
            "n={n} at={at:?} status={:?} {} | import again={:?} {}",
            st.status.code(),
            text(&st.stderr).lines().next().unwrap_or(""),
            again.status.code(),
            text(&again.stderr).lines().next().unwrap_or("")
        );
        eprintln!("{log}");
        if at.is_none() {
            assert_eq!(out.status.code(), Some(0), "{said}");
            break;
        }
        // Run again after a stop, it finishes, or says the project is already there.
        let done = env.root().join(".bolt/format").exists();
        assert!(done, "not finished: {log}\n{}", text(&again.stderr));
        let saved = saved_files(&env, main_tip(&env));
        assert!(saved == expected, "wrong files after {log}");
        let tmp: Vec<_> = fs::read_dir(env.root().join(".bolt/tmp"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(tmp.is_empty(), "litter in .bolt/tmp: {tmp:?} after {log}");
    }
}
