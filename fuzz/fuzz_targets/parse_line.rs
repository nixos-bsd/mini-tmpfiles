#![no_main]

extern crate mini_tmpfiles;

use std::path::Path;

use mini_tmpfiles::parser::{FileSpan, parse_line};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = parse_line(FileSpan::from_slice(data.split(|&b|b==b'\n').next().unwrap(), Path::new("")));
});
