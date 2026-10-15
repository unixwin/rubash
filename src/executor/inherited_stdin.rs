use std::fs::File;
use std::io;

pub(in crate::executor) fn duplicate_process_stdin() -> io::Result<File> {
    crate::fd::duplicate_handle(crate::fd::process_std_handle(0)).map(crate::fd::handle_to_file)
}
