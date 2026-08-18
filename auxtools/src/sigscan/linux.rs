use std::{
	ffi::{c_void, CStr, CString},
	os::raw::{c_char, c_int}
};

use libc::{dl_iterate_phdr, dl_phdr_info, Elf32_Phdr, PT_LOAD};

#[repr(C)]
struct CallbackData {
	module_name_ptr: *const c_char,
	memory_areas: Vec<(usize, usize)>
}

pub struct Scanner {
	module_name: String
}

extern "C" fn dl_phdr_callback(info: *mut dl_phdr_info, _size: usize, data: *mut c_void) -> c_int {
	let info = unsafe { *info };
	let module_name = unsafe { CStr::from_ptr(info.dlpi_name) }.to_str().unwrap();
	let cb_data = unsafe { &mut *(data as *mut CallbackData) };
	let target_module_name = unsafe { CStr::from_ptr(cb_data.module_name_ptr as *mut c_char) }.to_str().unwrap();
	if !module_name.ends_with(target_module_name) {
		return 0;
	}

	let headers: &'static [Elf32_Phdr] = unsafe { std::slice::from_raw_parts(info.dlpi_phdr, info.dlpi_phnum as usize) };

	// checks whether the segment is loadable and executable (executable flag is 1), adds to memory areas if it is
	for elf_header in headers.iter().filter(|p| p.p_type == PT_LOAD && p.p_flags & 1 == 1) {
		let start = (info.dlpi_addr + elf_header.p_vaddr) as usize;
		cb_data.memory_areas.push((start, elf_header.p_memsz as usize));
	}
	0
}

impl Scanner {
	pub fn for_module(name: &str) -> Option<Scanner> {
		Some(Scanner {
			module_name: name.to_string()
		})
	}

	pub fn find(&self, signature: &[Option<u8>]) -> Option<*mut u8> {
		let module_name = CString::new(self.module_name.clone()).unwrap();
		let module_name_ptr = module_name.as_ptr();
		let mut data = CallbackData {
			module_name_ptr,
			memory_areas: Vec::new()
		};
		unsafe { dl_iterate_phdr(Some(dl_phdr_callback), &mut data as *mut CallbackData as *mut c_void) };

		if data.memory_areas.is_empty() {
			// The module wasn't found.
			return None;
		}

		let mut result: Option<*mut u8> = None;

		for (memory_start, memory_len) in data.memory_areas {
			if memory_len < signature.len() {
				continue;
			}

			let mut data_current = memory_start as *mut u8;
			let data_end = (memory_start + memory_len - signature.len() + 1) as *mut u8;
			let mut signature_offset = 0;

			unsafe {
				while data_current < data_end {
					if signature[signature_offset] == None || signature[signature_offset] == Some(*data_current) {
						if signature.len() <= signature_offset + 1 {
							if result.is_some() {
								// Found two matches.
								return None;
							}
							result = Some(data_current.offset(-(signature_offset as isize)));
							data_current = data_current.offset(-(signature_offset as isize));
							signature_offset = 0;
						} else {
							signature_offset += 1;
						}
					} else {
						data_current = data_current.offset(-(signature_offset as isize));
						signature_offset = 0;
					}

					data_current = data_current.offset(1);
				}
			}
		}

		result
	}
}

#[cfg(test)]
mod tests {}
