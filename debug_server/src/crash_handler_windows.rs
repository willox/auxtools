use crate::{server_types::BreakpointReason, DEBUG_SERVER};
use auxtools::*;
use winapi::{
	shared::ntdef::LONG,
	um::{errhandlingapi::SetUnhandledExceptionFilter, winnt::EXCEPTION_POINTERS},
	vc::excpt::EXCEPTION_EXECUTE_HANDLER,
};

extern "system" fn exception_filter(_: *mut EXCEPTION_POINTERS) -> LONG {
	DEBUG_SERVER.with(|cell| {
		let mut serber = cell.borrow_mut();
		if let Some(server) = serber.as_mut() {
			let ctx = unsafe { *CURRENT_EXECUTION_CONTEXT.with(|cell| cell.get()) };
			server.handle_breakpoint(
				ctx,
				BreakpointReason::Runtime("native exception".to_owned()),
			);
		}
	});

	return EXCEPTION_EXECUTE_HANDLER;
}

#[init(full)]
fn crash_handler_init() -> Result<(), String> {
	unsafe {
		SetUnhandledExceptionFilter(Some(exception_filter));
	}

	Ok(())
}
