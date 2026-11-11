//! RT12 exploration: a folder swapped for another folder (not a link) once the walk has listed it
//! and starts looking at its files, when the file cache vouches for some of its files and not
//! others. `status` and `save` must refuse, or answer wholly for one folder: never a mix.
//! Run in the sandbox only: redteam/attacks11/run.sh explore rt12_mid_walk
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::time::{Duration, SystemTime};

use bolt_core::fsio::fault;
use bolt_core::keyed::{self, KEY_LEN};
use bolt_core::repo::Repository;
use bolt_core::snapshot::{SaveRequest, save_folder};
use bolt_core::status::status;
use support::Env;

fn old(path: &std::path::Path) {
    let f = fs::File::options().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
}

fn project() -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    fs::create_dir(env.root().join("sub")).unwrap();
    for (name, bytes) in [("a.txt", "aaaa"), ("b.txt", "bbbb"), ("c.txt", "cccc")] {
        let p = env.root().join("sub").join(name);
        fs::write(&p, bytes).unwrap();
        old(&p);
    }
    env.bolt().args(["save", "Files"]).assert().success();
    env.bolt().arg("status").assert().success();
    // Read again next time: changed now, the same size.
    fs::write(env.root().join("sub/b.txt"), "BBBB").unwrap();
    env
}

fn swap(root: std::path::PathBuf) -> impl FnOnce() + Send + 'static {
    move || {
        fs::rename(root.join("sub"), root.join("sub-old")).unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        for (name, bytes) in [("a.txt", "AAAA"), ("b.txt", "XXXX"), ("c.txt", "CCCC")] {
            let p = root.join("sub").join(name);
            fs::write(&p, bytes).unwrap();
        }
    }
}

fn keyed(env: &Env) -> Repository {
    let key: [u8; KEY_LEN] = fs::read(env.config().join(keyed::KEY_FILE)).unwrap().try_into().unwrap();
    let mut repo = Repository::open(env.root()).unwrap();
    repo.set_cache_key(key);
    repo
}

#[test]
fn a_folder_swapped_for_a_folder_mid_walk() {
    let env = project();
    fault::act_at_on_any_thread("walk:looking", swap(env.root().to_owned()));
    let mut repo = keyed(&env);
    match status(&mut repo, false, None) {
        Ok(s) => println!(
            "status answered: {:?}",
            s.changes.iter().map(|c| c.path.clone()).collect::<Vec<_>>()
        ),
        Err(e) => println!("status refused: {e}"),
    }
    drop(repo);

    let env = project();
    fault::act_at_on_any_thread("walk:looking", swap(env.root().to_owned()));
    let mut repo = keyed(&env);
    let who = support::ada();
    let req = SaveRequest {
        message: Some("Swapped"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
        amend: false,
    };
    match save_folder(&mut repo, &req, &mut |_| {}, &mut |_| true) {
        Ok(_) => {
            drop(repo);
            let saved = support::saved_files(&env, support::main_tip(&env));
            println!("save made: {saved:?}");
        }
        Err(e) => println!("save refused: {e}"),
    }
}
