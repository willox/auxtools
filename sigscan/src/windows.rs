use std::mem;
use std::ptr;

use winapi::shared::minwindef;
use winapi::um::libloaderapi;
use winapi::um::processthreadsapi;
use winapi::um::psapi;

pub struct Scanner {
	_module: minwindef::HMODULE,
	data_begin: *mut u8,
	data_end: *mut u8,
}

impl Scanner {
	pub fn for_module(name: &str) -> Option<Scanner> {
		let mut module: minwindef::HMODULE = ptr::null_mut();
		let data_begin: *mut u8;
		let data_end: *mut u8;

		// Construct a null-terminated UTF-16 string to pass to the Windows API
		let name_winapi: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

		unsafe {
			if libloaderapi::GetModuleHandleExW(0, name_winapi.as_ptr(), &mut module) == 0 {
				return None;
			}

			let mut module_info_wrapper = mem::MaybeUninit::<psapi::MODULEINFO>::zeroed();
			if psapi::GetModuleInformation(
				processthreadsapi::GetCurrentProcess(),
				module,
				module_info_wrapper.as_mut_ptr(),
				mem::size_of::<psapi::MODULEINFO>() as u32,
			) == 0
			{
				libloaderapi::FreeLibrary(module);
				return None;
			}

			let module_info = module_info_wrapper.assume_init();
			data_begin = module_info.lpBaseOfDll as *mut u8;
			data_end = data_begin
				.offset(module_info.SizeOfImage as isize)
				.offset(-1);
		}

		Some(Scanner {
			_module: module,
			data_begin,
			data_end,
		})
	}

	pub fn find(&self, signature: Vec<Option<u8>>) -> Option<*mut u8> {
		let mut data_current = self.data_begin;
		let data_end = self.data_end;
		let mut signature_offset = 0;
		let mut result: Option<*mut u8> = None;

		unsafe {
			while data_current <= data_end {
				if signature[signature_offset] == None
					|| signature[signature_offset] == Some(*data_current)
				{
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

		result
	}
}

impl Drop for Scanner {
	fn drop(&mut self) {
		// TODO: WTf this started throwing?!
		/*
		unsafe {
			libloaderapi::FreeLibrary(self.module);
		}
		*/
	}
}

#[cfg(test)]
mod tests {}
