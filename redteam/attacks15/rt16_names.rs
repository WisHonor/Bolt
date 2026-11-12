//! RT16 exploration (decision 024), sandbox only: `is_opal_alias` against the spec's words and
//! against Git's `is_ntfs_dot_generic`, ported. Not a proof: it reports what it finds.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::names::{NotWritable, is_opal_alias, validate_name, writable_here};

fn ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{200C}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{206A}'..='\u{206F}' | '\u{FEFF}'
    )
}

/// storage-format.md section 3, word for word.
fn spec(name: &str) -> bool {
    let kept: String = name.chars().filter(|&c| !ignorable(c)).collect();
    let part = kept.split([':', '\\', '/']).next().unwrap_or("");
    let s = part.trim_end_matches(['.', ' ']).to_ascii_lowercase();
    let b = s.as_bytes();
    if s == ".opalro" {
        return true;
    }
    if b.len() == 8 && &b[..7] == b"opalro~" && (b'1'..=b'9').contains(&b[7]) {
        return true;
    }
    if b.len() == 8
        && let Some(k) = b.iter().position(|&c| c == b'~')
    {
        return k <= 6
            && b[..k] == b"op9975"[..k]
            && (b'1'..=b'9').contains(&b[k + 1])
            && b[k + 2..].iter().all(u8::is_ascii_digit);
    }
    false
}

/// Git's `is_ntfs_dot_generic("opalro", "op9975")`, ported, with Opalro's ~1 to ~9 for the plain
/// short name; run on the name without ignorables, cut at the first `\`.
fn git_form(name: &str) -> bool {
    let kept: String = name.chars().filter(|&c| !ignorable(c)).collect();
    let part = kept.split(['\\', '/']).next().unwrap_or("");
    let n = part.as_bytes();
    let at = |i: usize| n.get(i).copied().unwrap_or(0);
    let spaces_and_periods = |mut i: usize| loop {
        let c = at(i);
        i += 1;
        if c == 0 || c == b':' {
            return true;
        }
        if c != b' ' && c != b'.' {
            return false;
        }
    };
    if at(0) == b'.' && n.len() >= 7 && n[1..7].eq_ignore_ascii_case(b"opalro") {
        return spaces_and_periods(7);
    }
    if n.len() >= 8
        && n[..6].eq_ignore_ascii_case(b"opalro")
        && n[6] == b'~'
        && (b'1'..=b'9').contains(&n[7])
    {
        return spaces_and_periods(8);
    }
    let prefix = b"op9975";
    let mut saw_tilde = false;
    let mut i = 0;
    while i < 8 {
        let c = at(i);
        if c == 0 {
            return false;
        } else if saw_tilde {
            if !c.is_ascii_digit() {
                return false;
            }
        } else if c == b'~' {
            i += 1;
            if !(b'1'..=b'9').contains(&at(i)) {
                return false;
            }
            saw_tilde = true;
        } else if i >= 6 || c & 0x80 != 0 || c.to_ascii_lowercase() != prefix[i] {
            return false;
        }
        i += 1;
    }
    spaces_and_periods(8)
}

fn check(name: &str, counts: &mut [usize; 4], shown: &mut Vec<String>) {
    let code = is_opal_alias(name);
    let by_spec = spec(name);
    let by_git = git_form(name);
    counts[0] += 1;
    counts[1] += usize::from(code);
    let valid = validate_name(name);
    let writable = writable_here(name, &[]);
    let paths_agree = if code {
        valid.is_err() && writable == Err(NotWritable::OpalName)
    } else {
        // Another rule may refuse it, never as an .opalro spelling.
        writable != Err(NotWritable::OpalName)
    };
    if code != by_spec || !paths_agree {
        counts[2] += 1;
        if shown.len() < 40 {
            shown.push(format!(
                "code={code} spec={by_spec} git={by_git} validate={valid:?} writable={writable:?}: {name:?}"
            ));
        }
    }
    if code != by_git {
        counts[3] += 1;
    }
}

#[test]
fn rt16_names() {
    let pieces = [
        "",
        ".",
        "o",
        "O",
        "p",
        "P",
        "9",
        "7",
        "5",
        "a",
        "l",
        "r",
        "R",
        "~",
        "1",
        "2",
        "0",
        " ",
        ":",
        "\\",
        "x",
        "\u{200C}",
        "\u{FEFF}",
        "\u{206F}",
        "op9975",
        "OP997",
        "Op99",
        "opalro",
        ".opalro",
        "OPALRO~",
        "$INDEX_ALLOCATION",
        "\u{200B}",
    ];
    let mut counts = [0usize; 4];
    let mut shown = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for a in pieces {
        for b in pieces {
            for c in pieces {
                for d in pieces {
                    let name = format!("{a}{b}{c}{d}");
                    if !name.is_empty() && seen.insert(name.clone()) {
                        check(&name, &mut counts, &mut shown);
                    }
                }
            }
        }
    }
    // Every 8-character form with `~`: each start of `op9975` in each case, every counter.
    let starts = [
        "", "o", "op", "op9", "op99", "op997", "op9975", "O", "OP", "Op", "oP", "OP9975", "op9976",
        "pp9975", "o~", "q",
    ];
    let endings = [
        "", ".", " ", ". .", ":", ":x", "::$DATA", "x", ".txt", "\u{200C}", "~",
    ];
    for s in starts {
        for n in 0..=9_999_999u32 {
            if n % 997 != 0 && n > 2000 {
                continue;
            }
            for zeros in [false, true] {
                let counter = if zeros {
                    format!("{n:07}")
                } else {
                    n.to_string()
                };
                for e in endings {
                    let name = format!("{s}~{counter}{e}");
                    if seen.insert(name.clone()) {
                        check(&name, &mut counts, &mut shown);
                    }
                }
            }
        }
    }
    // Ignorables before and after the `~`, in each place of a hashed name.
    for base in ["op9975~1", "op997~12", "o~123456", "~1234567", "opalro~5"] {
        for at in 0..=base.len() {
            for c in [
                '\u{200C}', '\u{200F}', '\u{202A}', '\u{202E}', '\u{206A}', '\u{206F}', '\u{FEFF}',
                '\u{200B}', '\u{00AD}',
            ] {
                let name = format!("{}{c}{}", &base[..at], &base[at..]);
                if seen.insert(name.clone()) {
                    check(&name, &mut counts, &mut shown);
                }
            }
        }
    }
    println!(
        "{} names, {} refused as .opalro; {} differ from the spec's words or between the three \
         checks; {} differ from Git's form",
        counts[0], counts[1], counts[2], counts[3]
    );
    for s in &shown {
        println!("  {s}");
    }
    // Ordinary names: which are refused?
    for name in [
        "~$document.docx",
        "~$Budget.xlsx",
        "~lock.file#",
        ".~lock.report.odt#",
        "~WRL0001.tmp",
        "~1",
        "~10",
        "~123",
        "~123456",
        "~1234567",
        "~12345678",
        "~1234567.txt",
        "~0123456",
        "~9999999",
        "~1000000",
        "o~123456",
        "O~654321",
        "op~12345",
        "op9~1234",
        "op99~123",
        "op997~12",
        "op9975~1",
        "OP9975~9",
        "op9975~10",
        "op9975~0",
        "op9975",
        "op9975~",
        "op9975.txt",
        "opalro~0",
        "opalro~5",
        "OPALRO~9",
        "opalro~10",
        "opalr~10",
        "opal~100",
        "op~10000",
        "notes~1",
        "backup~1234567",
        "~",
        "~~",
        "~.txt",
        "~1.txt",
        "~1 ",
        "~1234567 .",
    ] {
        println!(
            "  {:<22} refused={}",
            format!("{name:?}"),
            is_opal_alias(name)
        );
    }
}
