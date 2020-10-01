use winapi::shared::minwindef;
use winapi::um::processthreadsapi;
use winapi::um::libloaderapi;
use winapi::um::psapi;
use std::ptr;
use std::mem;

pub struct SigScan {
    module: minwindef::HMODULE,
    data_begin: *mut u8,
    data_length: usize,
}

impl SigScan {
    pub fn for_executable() -> Option<SigScan> {
        let mut module: minwindef::HMODULE = ptr::null_mut();
        let data_begin: *mut u8;
        let data_length: usize;

        unsafe {
            if libloaderapi::GetModuleHandleExW(0, ptr::null_mut(), &mut module) == 0 {
                return None
            }

            let mut module_info_wrapper = mem::MaybeUninit::<psapi::MODULEINFO>::zeroed();
            if psapi::GetModuleInformation(processthreadsapi::GetCurrentProcess(), module, module_info_wrapper.as_mut_ptr(), mem::size_of::<psapi::MODULEINFO>() as u32) == 0 {
                libloaderapi::FreeLibrary(module);
                return None
            }

            let module_info = module_info_wrapper.assume_init();
            data_begin = module_info.lpBaseOfDll as *mut u8;
            data_length = module_info.SizeOfImage as usize;
        }

        Some(SigScan {
            module: module,
            data_begin: data_begin,
            data_length: data_length,
        })
    }

    pub fn find(&self, signature: &[u8]) -> Option<*mut u8> {
        unsafe {
            let data = std::slice::from_raw_parts(self.data_begin, self.data_length);
            let mut signature_offset = 0;
            let mut data_offset = 0;
            while data_offset < self.data_length {
                if signature[signature_offset] == b'?' || signature[signature_offset] == data[data_offset] {
                    if signature.len() <= signature_offset + 1 {
                        return Some(self.data_begin.offset(-(signature_offset as isize)).offset(data_offset as isize))
                    }
                    signature_offset += 1;
                } else {
                    data_offset -= signature_offset;
                    signature_offset = 0;
                }
                data_offset += 1;
            }
        }

        None
    }
}

impl Drop for SigScan {
    fn drop(&mut self) {
        unsafe {
            libloaderapi::FreeLibrary(self.module);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::SigScan;

    #[test]
    fn scan_self() {
        let scanner = SigScan::for_executable().unwrap();
        println!("Fuck!");
        let res = scanner.find(b"Fuck!");
        println!("ok");
    }
}