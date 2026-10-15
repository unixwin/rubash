use std::fs;
use std::io;
use std::path::Path;

pub(in crate::executor) fn require_directory_contents_source(source: &Path) -> io::Result<()> {
    if fs::metadata(source)?.is_dir() {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, "Not a directory"))
    }
}
