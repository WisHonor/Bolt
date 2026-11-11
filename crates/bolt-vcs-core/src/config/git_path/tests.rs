#![allow(clippy::unwrap_used)]
use super::*;

/// What Windows adds: `\`, drives and network paths.
#[test]
fn git_config_system_is_tidied_as_git_tidies_it() {
    let failed = None;
    let to = |text: &str| Some(text.to_owned());
    let windows = [
        ("\\dev\\null", to("/dev/null")),
        ("C:\\dir\\..\\x", to("C:/x")),
        ("C:dir/../..", failed.clone()),
        ("C:\\..", failed.clone()),
        ("\\\\server\\share\\dir\\..\\x", to("//server/share/x")),
        ("//server/share/../x", failed.clone()),
        // A network path with no share keeps no root, as in Git.
        ("//server", to("server")),
        ("\u{e4}:\\x\\.\\y", to("\u{e4}:/x/y")),
    ];
    let tidy = |text: &str, windows| {
        let tidied = tidy_path(text.as_bytes(), windows)?;
        Some(String::from_utf8(tidied).unwrap())
    };
    // `\` is no separator elsewhere.
    assert_eq!(tidy("\\dev\\null", false), to("\\dev\\null"));
    for (text, want) in windows {
        assert_eq!(tidy(text, true), want, "{text:?} on Windows");
    }
}
