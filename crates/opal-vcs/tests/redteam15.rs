//! Red-team session RT15 (decision 024): the rename to Opalro, attacked in the sandbox. Each test
//! asserts the safe behaviour, so it fails until its finding is fixed. See
//! docs/redteam/2026-11-12-the-name.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use support::Env;

/// What `opalro import` printed, both streams.
fn import(env: &Env) -> (bool, String) {
    let out = env.opalro().arg("import").output().unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), said)
}

/// A Git commit tracking a spelling of `.opalro` with a joiner inside: at the top the import stops,
/// below it the import warns. Both name the entry, and the name must show the joiner, as the
/// import's own list of left-out entries and the save's warning do (`escape_all`), or the person
/// reads `.opalro` where the entry isn't that.
#[test]
fn the_import_shows_the_joiner_in_a_tracked_opalro_spelling() {
    let mut hidden = Vec::new();
    for tracked in [".o\u{200c}palro/x", "sub/.o\u{200d}palro/x"] {
        let env = Env::new();
        env.commit(tracked, "planted\n", "First");
        let (ok, said) = import(&env);
        println!("Git tracks {tracked:?}: import succeeded: {ok}\n{said}");
        let lines: Vec<&str> = said
            .lines()
            .filter(|l| l.contains('\u{200c}') || l.contains('\u{200d}'))
            .collect();
        if !lines.is_empty() {
            hidden.push(format!("{tracked:?}: {lines:?}"));
        }
    }
    assert!(
        hidden.is_empty(),
        "a joiner was printed as it is, so the name reads as .opalro:\n{}",
        hidden.join("\n")
    );
}
