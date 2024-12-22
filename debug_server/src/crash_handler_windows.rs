use auxtools::*;
use winapi::{
	shared::ntdef::LONG,
	um::{errhandlingapi::SetUnhandledExceptionFilter, winnt::EXCEPTION_POINTERS},
	vc::excpt::EXCEPTION_EXECUTE_HANDLER
};

use crate::{server_types::BreakpointReason, DEBUG_SERVER};

extern "system" fn exception_filter(_: *mut EXCEPTION_POINTERS) -> LONG {
	unsafe {
		if let Some(dbg) = &mut *DEBUG_SERVER.get() {
			let ctx = *raw_types::funcs::CURRENT_EXECUTION_CONTEXT;

			dbg.handle_breakpoint(ctx, BreakpointReason::Runtime("native exception".to_owned()));
		}
	}

	EXCEPTION_EXECUTE_HANDLER
}

#[init(full)]
fn crash_handler_init() -> Result<(), String> {
	unsafe {
		SetUnhandledExceptionFilter(Some(exception_filter));
	}

	Ok(())
}
