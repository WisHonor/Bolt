//! What `bolt goto` shows people: where it switched to, and what it wrote.

use std::collections::BTreeMap;

use bolt_core::diary::Position;
use bolt_core::id::Id;
use bolt_core::save::Save;
use bolt_core::switch::GotoOutcome;
use bolt_core::text::{escape, thousands};

use crate::output;
use crate::style::{self, Role};

/// `bolt goto`'s outcome line and details. `short` names the save it is at, `save` is that save
/// if it could be read, and `lines` are the project's lines now. `stepped` is whether the goto
/// recorded a step: when it didn't, the project already was there, and only that is said.
pub fn switched(
    done: &GotoOutcome,
    short: &str,
    save: Option<&Save>,
    lines: &BTreeMap<String, Id>,
    stepped: bool,
) -> String {
    let s = style::out();
    let at = match save {
        Some(save) => format!(
            "{} {}",
            s.paint(Role::ChangeId, short),
            output::quoted_title(save)
        ),
        None => s.paint(Role::Fingerprint, short),
    };
    let switched = s.paint(Role::Ok, "Switched");
    if !stepped {
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
    if let Position::AtSave(id) = &done.position {
        details.push("Saving here asks for a new line's name.".to_owned());
        if let Some((line, _)) = lines.iter().find(|(_, tip)| *tip == id) {
            let line = escape(line, false);
            let goto = s.paint(Role::Command, &format!("bolt goto {line}"));
            details.push(format!("To keep saving on {line}: {goto}"));
        }
    }
    text.push('\n');
    for detail in details {
        text += &format!("    {detail}\n");
    }
    text
}
