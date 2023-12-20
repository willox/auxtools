use crate::{server_types::BreakpointReason, DEBUG_SERVER};
use auxtools::*;

use windows::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter;
use windows::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS;

extern "system" fn exception_filter(_: *const EXCEPTION_POINTERS) -> i32 {
	unsafe {
		if let Some(dbg) = &mut *DEBUG_SERVER.get() {
			let ctx = *raw_types::funcs::CURRENT_EXECUTION_CONTEXT;

			dbg.handle_breakpoint(
				ctx,
				BreakpointReason::Runtime("native exception".to_owned()),
			);
		}
	}

	return 1;
}

#[init(full)]
fn crash_handler_init() -> Result<(), String> {
	unsafe {
		SetUnhandledExceptionFilter(Some(exception_filter));
	}
	Ok(())
}
