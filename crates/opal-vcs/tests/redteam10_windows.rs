//! Red-team checks for milestone 6 that need Windows (decision 024): junctions at the rule files'
//! names. Each asserts the safe behaviour; CI runs them on a throwaway Windows machine.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;

use support::{Env, junction, text};

/// A junction at `opal.toml` or `.opalignore`, to a folder outside the project: `setup` refuses,
/// `opal setup` and `save` read nothing through it, and nothing outside is written.
#[test]
fn rt10_w1_a_junction_at_a_rule_file_is_never_followed() {
    for name in ["opal.toml", ".opalignore"] {
        let env = Env::empty();
        env.opal()
            .args(["start", "--preset", "custom"])
            .assert()
            .success();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("kept.txt"), "outside\n").unwrap();
        junction(&env.root().join(name), outside.path());
        let out = env
            .opal()
            .args(["setup", "--preset", "code", "--yes"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{name}: {}", text(&out.stdout));
        // An `opal.toml` Opalro can't use is passed over with a warning; a `.opalignore` that isn't
        // a regular file stops every command that reads it, as it stops a save.
        let shown = env.opal().arg("setup").output().unwrap();
        let said = text(&shown.stderr);
        if name == "opal.toml" {
            assert_eq!(shown.status.code(), Some(0), "{said}");
            assert!(said.contains("opal.toml"), "{said}");
        } else {
            assert_eq!(shown.status.code(), Some(1), "{said}");
            assert!(said.contains(".opalignore isn't a regular file"), "{said}");
        }
        env.opal().args(["save", "--yes", "x"]).output().unwrap();
        let left: Vec<_> = fs::read_dir(outside.path()).unwrap().collect();
        assert_eq!(left.len(), 1, "{name}: something was written outside");
        assert_eq!(
            fs::read_to_string(outside.path().join("kept.txt")).unwrap(),
            "outside\n"
        );
    }
}
