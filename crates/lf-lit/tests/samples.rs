//! Real projects are not distributed with this repository. Point
//! `LF_LIT_SAMPLES` at a folder of `.lit` / `.litsettings` files to check them:
//!
//! ```text
//! LF_LIT_SAMPLES=~/lit-samples cargo test -p lf-lit --test samples
//! ```

use std::path::PathBuf;

use lf_lit::{Container, Document};

fn sample_files() -> Vec<PathBuf> {
    let Some(dir) = std::env::var_os("LF_LIT_SAMPLES") else {
        eprintln!("LF_LIT_SAMPLES is not set; skipping sample files");
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("LF_LIT_SAMPLES is a readable folder")
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e == "lit" || e == "litsettings")
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no .lit or .litsettings files in {dir:?}"
    );
    files
}

#[test]
fn samples_round_trip() {
    for path in sample_files() {
        let bytes = std::fs::read(&path).unwrap();
        let doc = Document::read(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let undecoded = doc.undecoded_objects().count();
        assert_eq!(
            undecoded,
            0,
            "{}: {undecoded} objects with undecoded values",
            path.display()
        );
        let written = doc.to_binary().unwrap();
        match doc.container {
            Container::Binary => {
                assert!(written == bytes, "{}: not byte-identical", path.display())
            }
            Container::Json => {
                let reread = Document::read(&written).unwrap();
                assert!(
                    reread.objects == doc.objects,
                    "{}: objects changed",
                    path.display()
                );
            }
        }
    }
}
