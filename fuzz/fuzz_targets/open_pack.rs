#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    use std::io::Write;
    let Ok(dir) = tempfile::tempdir() else { return };
    let path = dir.path().join("pack");
    let Ok(mut f) = std::fs::File::create(&path) else { return };
    if f.write_all(data).is_err() {
        return;
    }
    drop(f);
    if let Ok(reader) = bolt_core::pack::PackReader::open(&path) {
        let _ = reader.check_index();
        // Every line in order, then each found again by its ID, as a lookup finds it.
        for entry in reader.entries() {
            let Ok(entry) = entry else { break };
            let _ = reader.find(&entry.id);
            let _ = reader.read_record(&entry);
            let _ = reader.copy_content(&entry, &mut std::io::sink());
        }
        let _ = reader.find(&bolt_core::id::Id([0xff; 32]));
    }
});
