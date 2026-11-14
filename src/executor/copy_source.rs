use std::io;

use super::external_file_builtins::CpNode;

/// GNU spells the operands of a `src/.` copy as the source plus
/// its contents: the source itself must resolve to a directory even
/// before enumeration. A `CpNode` carries that check through the
/// root it was resolved under (rubash#3268 (d)): in a dead cwd the
/// directory test must consult the anchor, not the joined spelling.
pub(in crate::executor) fn require_directory_contents_source(source: &CpNode) -> io::Result<()> {
    if source.stat()?.is_dir {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, "Not a directory"))
    }
}
