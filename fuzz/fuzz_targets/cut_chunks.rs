#![no_main]
use opal_core::chunks::{Cutter, MAX_CHUNK, MIN_CHUNK, chunk_len};
use libfuzzer_sys::fuzz_target;
use std::io::{self, Read};

/// Hands out a file a few bytes a read, as many as the next of `steps` says.
struct Uneven<'a> {
    file: &'a [u8],
    steps: std::iter::Cycle<std::slice::Iter<'a, u8>>,
}

impl Read for Uneven<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let step = self.steps.next().map_or(4096, |&s| usize::from(s) * 61 + 1);
        let n = step.min(buf.len()).min(self.file.len());
        let (head, tail) = self.file.split_at(n);
        buf[..n].copy_from_slice(head);
        self.file = tail;
        Ok(n)
    }
}

fuzz_target!(|data: &[u8]| {
    // The first three bytes give a length of up to 1 MiB, and the rest, repeated, the file's
    // bytes, so that short inputs still make files cut more than once.
    let Some((&[a, b, c], pattern)) = data.split_first_chunk::<3>() else {
        return;
    };
    let len = usize::from(a) | usize::from(b) << 8 | usize::from(c & 0x0f) << 16;
    let file: Vec<u8> = if pattern.is_empty() {
        vec![0; len]
    } else {
        pattern.iter().copied().cycle().take(len).collect()
    };

    let mut lens = Vec::new();
    let mut rest = &file[..];
    while !rest.is_empty() {
        let n = chunk_len(rest);
        assert!((1..=MAX_CHUNK).contains(&n));
        lens.push(n);
        rest = &rest[n..];
    }
    // Only the last chunk may be as short as the minimum.
    assert!(lens.iter().rev().skip(1).all(|&n| n > MIN_CHUNK));
    assert_eq!(lens.iter().sum::<usize>(), file.len());

    // A cutter reading the file a few bytes at a time cuts it the same.
    let mut cutter = Cutter::new(Uneven {
        file: &file,
        steps: data.iter().cycle(),
    });
    let mut at = 0;
    for &n in &lens {
        assert_eq!(cutter.next_chunk().unwrap(), Some(&file[at..at + n]));
        at += n;
    }
    assert_eq!(cutter.next_chunk().unwrap(), None);
});
