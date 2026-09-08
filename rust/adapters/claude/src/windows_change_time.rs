#![cfg(windows)]

use std::{ffi::c_void, fs::File, mem::MaybeUninit, os::windows::io::AsRawHandle, path::Path};

// Windows supplies a change time distinct from the user-settable last-write
// time. Stable Rust does not expose MetadataExt::change_time yet.
// https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_basic_info
#[repr(C)]
struct FileBasicInfo {
    creation_time: i64,
    last_access_time: i64,
    last_write_time: i64,
    change_time: i64,
    file_attributes: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetFileInformationByHandleEx"]
    fn get_file_information_by_handle_ex(
        file: *mut c_void,
        information_class: i32,
        information: *mut c_void,
        size: u32,
    ) -> i32;
}

pub(crate) fn change_time(path: &Path) -> Option<u64> {
    let file = File::open(path).ok()?;
    let mut info = MaybeUninit::<FileBasicInfo>::uninit();
    // SAFETY: the live File owns this handle. FileBasicInfo is the documented
    // C layout for FileBasicInfo (class 0); the buffer has the supplied size.
    let success = unsafe {
        get_file_information_by_handle_ex(
            file.as_raw_handle(),
            0,
            info.as_mut_ptr().cast(),
            size_of::<FileBasicInfo>() as u32,
        )
    };
    if success == 0 {
        return None;
    }
    // SAFETY: a successful query initialized the FileBasicInfo fields.
    let value = unsafe { info.assume_init() }.change_time;
    u64::try_from(value).ok().filter(|value| *value != 0)
}
