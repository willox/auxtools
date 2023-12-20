use std::mem;

use windows::core::HSTRING;
use windows::Win32::Foundation;
use windows::Win32::System::LibraryLoader;
use windows::Win32::System::ProcessStatus;
use windows::Win32::System::Threading;

pub struct Scanner {
	_module: Foundation::HINSTANCE,
	data_begin: *mut u8,
	data_end: *mut u8,
}

impl Scanner {
	pub fn for_module(name: &str) -> Option<Scanner> {
		let mut module: Foundation::HINSTANCE = Default::default();
		let data_begin: *mut u8;
		let data_end: *mut u8;

		// Construct a null-terminated UTF-16 string to pass to the Windows API
		let name_winapi: HSTRING = name.into();

		unsafe {
			if !LibraryLoader::GetModuleHandleExW(0, &name_winapi, &mut module).as_bool() {
				return None;
			}

			let mut module_info_wrapper = mem::MaybeUninit::<ProcessStatus::MODULEINFO>::zeroed();
			if !ProcessStatus::K32GetModuleInformation(
				Threading::GetCurrentProcess(),
				module,
				module_info_wrapper.as_mut_ptr(),
				mem::size_of::<ProcessStatus::MODULEINFO>() as u32,
			)
			.as_bool()
			{
				LibraryLoader::FreeLibrary(module);
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

	pub fn find(&self, signature: &[Option<u8>]) -> Option<*mut u8> {
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
			LibraryLoader::FreeLibrary(self.module);
		}
		*/
	}
}

#[cfg(test)]
mod tests {}
