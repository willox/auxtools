use super::raw_types;
use std::fmt;
use std::ffi::CStr;

pub struct StringRef {
	pub internal: *mut raw_types::strings::StringEntry,
}

impl StringRef {
    pub fn new(ptr: *mut raw_types::strings::StringEntry) -> Self {
        // inc ref count

        StringRef {
            internal: ptr,
        }
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
