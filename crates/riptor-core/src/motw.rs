//! Mark-of-the-Web on finished downloads (Windows only), like browsers do.

use std::path::Path;

#[cfg(windows)]
pub fn mark(path: &Path) -> std::io::Result<()> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(stream, b"[ZoneTransfer]\r\nZoneId=3\r\n")
}

#[cfg(not(windows))]
pub fn mark(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn writes_zone_identifier_stream() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("setup.exe");
        std::fs::write(&f, b"MZ").unwrap();
        super::mark(&f).unwrap();
        let mut s = f.as_os_str().to_owned();
        s.push(":Zone.Identifier");
        let z = std::fs::read_to_string(s).unwrap();
        assert!(z.contains("ZoneId=3"));
        assert_eq!(std::fs::read(&f).unwrap(), b"MZ");
    }
}
