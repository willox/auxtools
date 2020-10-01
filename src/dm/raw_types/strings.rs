use std::os::raw::c_char;

#[repr(C)]
pub struct StringRef {
    index: u32,
}

#[repr(C)]
pub struct StringEntry {
    data: *mut c_char,
    this: StringRef,
    left: *mut StringEntry,
    right: *mut StringEntry,
    unk_0: u32,
    unk_1: u32,
    unk_2: u32,
}

#[repr(C)]
pub struct StringTable {
    pub strings: *mut *mut StringEntry,
    pub size: u32,
}