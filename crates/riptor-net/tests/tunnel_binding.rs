//! Every tunnel connection goes through `TunnelEnd`; binding by address alone leaks on Linux.

use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn nothing_binds_to_the_tunnel_by_address_alone() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["crates", "src-tauri/src"] {
        sources(&root.join(dir), &mut files);
    }
    assert!(files.len() > 50, "found only {} sources", files.len());
    let allowed = Path::new("riptor-net").join("src").join("tunnel.rs");
    // Split so this file doesn't match itself.
    let needle = [".local_", "address("].concat();
    let mut offenders = Vec::new();
    for f in &files {
        if f.ends_with(&allowed) || f.components().any(|c| c.as_os_str() == "target") {
            continue;
        }
        let text = std::fs::read_to_string(f).unwrap();
        for (n, line) in text.lines().enumerate() {
            if line.contains(&needle) {
                offenders.push(format!("{}:{}", f.display(), n + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "use riptor_net::TunnelEnd::http instead of local_address: {offenders:?}"
    );
}
