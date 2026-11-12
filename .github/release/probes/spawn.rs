// Starts another program, so the import check must refuse it.
fn main() {
    let _ = std::process::Command::new("true").status();
}
