use super::raw_types;
use super::GLOBAL_STATE;
use std::ffi::{CStr, CString};
use std::fmt;

pub struct StringRef {
    pub internal: *const raw_types::strings::StringEntry,
}

impl StringRef {
    pub fn new(ptr: *const raw_types::strings::StringEntry) -> Self {
        // inc ref count

        StringRef { internal: ptr }
    }

    pub fn from_id<I: Into<u32>>(id: I) -> Self {
        Self::new(unsafe { (GLOBAL_STATE.get().unwrap().get_string_table_entry)(id.into()) })
    }
}

impl Drop for StringRef {
    fn drop(&mut self) {
        // dec string ref
    }
}

impl fmt::Debug for StringRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        unsafe {
            // TODO: Show ref count? Escape special chars?
            let string = CStr::from_ptr((*self.internal).data);
            write!(f, "{}", string.to_string_lossy())
        }
    }
}

fn string_to_stringref(string: &str) -> Option<StringRef> {
    if let Ok(string) = CString::new(string) {
        unsafe {
            let index =
                (GLOBAL_STATE.get().unwrap().get_string_id)(string.as_ptr(), true, false, true);
            let strings = (*GLOBAL_STATE.get().unwrap().string_table).strings;

            return Some(StringRef::new(*strings.add(index as usize)));
        }
    }
    None
}

impl From<&str> for StringRef {
    fn from(s: &str) -> Self {
        string_to_stringref(s).unwrap()
    }
}

impl From<String> for StringRef {
    fn from(s: String) -> Self {
        string_to_stringref(s.as_str()).unwrap()
    }
}

impl Into<String> for StringRef {
    fn into(self) -> String {
        unsafe {
            CStr::from_ptr((*self.internal).data)
                .to_string_lossy()
                .into()
        }
    }
}
