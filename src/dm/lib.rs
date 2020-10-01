mod raw_types;
mod byond_ffi;

use sigscan::Scanner;

byond_ffi_fn! { auxtools_init(input) {
    let scanner = Scanner::for_module("byondcore.dll");

    match scanner {
        Some(byondcore) => {
            let strings = byondcore.find(b"\xA1????\x8B\x04?\x85\xC0\x0F\x84????\x80\x3D????\x00\x8B\x18");
            match strings {
                Some(ptr) => {
                    unsafe {
                        let actual_strings = ptr.offset(1);
                        let strtable = &**(actual_strings as *mut *mut raw_types::strings::StringTable);
                        let string = *strtable.strings.offset(4);
                        return Some("GOOD2");
                    }
                }

                None => {
                    return Some("FAILED2");
                }
            }
        },

        None => {
            return Some("FAILED1");
        }
    }

    
    Some("Good")
} } 

#[cfg(test)]
mod tests {
    #[test]
    fn test() {

    }
}