use windows_sys::Win32::Foundation::HANDLE as SysHandle;

#[inline]
pub(crate) fn sys_handle(handle: isize) -> SysHandle {
    handle as SysHandle
}

#[inline]
pub(crate) fn local_handle(handle: SysHandle) -> isize {
    handle as isize
}
