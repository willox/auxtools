use std::collections::HashSet;
use std::ffi::{c_void, CStr, CString};
use std::os::raw::{c_char, c_int};

use libc::{dl_iterate_phdr, dl_phdr_info, Elf32_Phdr, PT_LOAD};
use crate::signature;

#[repr(C)]
struct CallbackData {
	module_name_ptr: *const c_char,
	memory_start: usize,
	memory_len: usize,
	memory_area: Option<&'static [u8]>,
}

pub struct Scanner {
	module_name: String,
}

extern "C" fn dl_phdr_callback(info: *mut dl_phdr_info, _size: usize, data: *mut c_void) -> c_int {
	let info = unsafe { *info };
	let module_name = unsafe { CStr::from_ptr(info.dlpi_name) }.to_str().unwrap();
	let cb_data: &mut CallbackData = unsafe { std::mem::transmute(data) };
	let target_module_name = unsafe { CStr::from_ptr(cb_data.module_name_ptr as *mut c_char) }
		.to_str()
		.unwrap();
	if !module_name.ends_with(target_module_name) {
		return 0;
	}

	let headers: &'static [Elf32_Phdr] =
		unsafe { std::slice::from_raw_parts(info.dlpi_phdr, info.dlpi_phnum as usize) };
	let elf_header = headers
		.iter()
		.filter(|p| p.p_type == PT_LOAD)
		.next()
		.unwrap();

	let start = (info.dlpi_addr + elf_header.p_vaddr) as usize;
	let end = start + elf_header.p_memsz as usize;
	let len = end - start;

	cb_data.memory_start = start;
	cb_data.memory_len = len;
	cb_data.memory_area = Some(unsafe { std::slice::from_raw_parts(start as *const u8, len) });
	0
}

impl Scanner {
	pub fn for_module(name: &str) -> Option<Scanner> {
		Some(Scanner {
			module_name: name.to_string(),
		})
	}

	pub fn find(&self, signature: &[Option<u8>]) -> Option<*mut u8> {
		let all = self.find_all(signature);
		if all.len() == 1 {
			return all.into_iter().next();
		}
		return None;
	}

	pub fn find_all(&self, signature: &[Option<u8>]) -> Vec<*mut u8> {
		let module_name = CString::new(self.module_name.clone()).unwrap();
		let module_name_ptr = module_name.as_ptr();
		let data = CallbackData {
			module_name_ptr,
			memory_start: 0,
			memory_len: 0,
			memory_area: None,
		};
		unsafe { dl_iterate_phdr(Some(dl_phdr_callback), std::mem::transmute(&data)) };

		let mut data_current = data.memory_start as *mut u8;
		let data_end = (data.memory_start + data.memory_len) as *mut u8;
		let mut signature_offset = 0;
		let mut result = Vec::new();

		unsafe {
			while data_current <= data_end {
				if signature[signature_offset] == None
					|| signature[signature_offset] == Some(*data_current)
				{
					if signature.len() <= signature_offset + 1 {
						result.push(data_current.offset(-(signature_offset as isize)));
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

		result
	}
}

#[cfg(test)]
mod tests {}
