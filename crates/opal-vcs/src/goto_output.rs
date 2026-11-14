//! What `opalro goto` shows people: where it switched to, and what it wrote.

use opal_core::diary::Position;
use opal_core::save::Save;
use opal_core::switch::GotoOutcome;
use opal_core::text::{escape, thousands};

use crate::output;
use crate::style::{self, Role};

/// A save as outcome lines name it: `short`, its change ID, and its message's first line in
/// quotes; or `short` alone, 12 hex digits of its fingerprint, when it couldn't be read.
pub fn at_save(short: &str, save: Option<&Save>) -> String {
    let s = style::out();
    match save {
        Some(save) => format!(
            "{} {}",
            s.paint(Role::ChangeId, short),
            output::quoted_title(save)
        ),
        None => s.paint(Role::Fingerprint, short),
    }
}

/// How to keep saving on a line whose newest save `goto` went to.
pub enum KeepSaving {
    /// `opalro goto <line>` goes onto it.
    Goto(String),
    /// The name is refused, because it is also `why`: the line must be renamed first.
    Rename { line: String, why: &'static str },
}

/// `opalro goto`'s outcome line and details. `short` names the save it is at, `save` is that save
/// if it could be read, and `keep` says how to keep saving on a line at it. A goto that recorded
/// no step found the project already there, and only that is said.
pub fn switched(
    done: &GotoOutcome,
    short: &str,
    save: Option<&Save>,
    keep: Option<&KeepSaving>,
) -> String {
    let s = style::out();
    let at = at_save(short, save);
    let switched = s.paint(Role::Ok, "Switched");
    if !done.recorded {
        return match &done.position {
            Position::OnLine(line) => {
                let line = s.paint(Role::Line, &escape(line, false));
                format!("Already on line {line}, at save {at}.\n")
            }
            Position::AtSave(_) => format!("Already at save {at}.\n"),
        };
    }
    let mut text = match &done.position {
        Position::OnLine(line) => {
            let line = s.paint(Role::Line, &escape(line, false));
            format!("{switched} to line {line} at save {at}.\n")
        }
        Position::AtSave(_) => format!("{switched} to save {at}. You are not on a line.\n"),
    };
    let (written, removed) = (done.applied.written, done.applied.removed);
    let mut details = vec![match (written, removed) {
        (0, 0) => "No file needed changing.".to_owned(),
        (0, removed) => format!("{} removed.", output::files(removed)),
        (written, 0) => format!("{} written.", output::files(written)),
        (written, removed) => format!(
            "{} written, {} removed.",
            output::files(written),
            thousands(removed as u64)
        ),
    }];
    if done.carried > 0 {
        let carried = output::files(done.carried);
        details.push(format!("Your unsaved changes in {carried} came along."));
    }
    if let Position::AtSave(_) = &done.position {
        details.push("Saving here asks for a new line's name.".to_owned());
        match keep {
            Some(KeepSaving::Goto(line)) => {
                let line = escape(line, false);
                let goto = style::whole(&s.paint(Role::Command, &format!("opalro goto {line}")));
                let line = style::whole(&line);
                details.push(format!("To keep saving on {line}: {goto}"));
            }
            Some(KeepSaving::Rename { line, why }) => {
                let line = style::whole(&escape(line, false));
                let rename = "opalro line rename <old> <new>";
                let rename = style::whole(&s.paint(Role::Command, rename));
                details.push(format!(
                    "Line {line} is here too, but {line} is also {why}. To keep saving on the \
                     line, rename it first: {rename}"
                ));
            }
            None => {}
        }
    }
    text.push('\n');
    for detail in details {
        text += &s.detail(None, &detail);
        text.push('\n');
    }
    text
}
