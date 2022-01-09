use detour::RawDetour;
use std::ffi::{c_void, CString};
use std::os::raw::c_char;
use std::{cell::UnsafeCell, collections::HashMap, fs::File, io};

use auxtools::{raw_types::procs::ProcId, *};

static mut THREAD_ID: u32 = 0;
static MALLOC_SYMBOL: &[u8] = b"malloc\0";
static REALLOC_SYMBOL: &[u8] = b"realloc\0";
static FREE_SYMBOL: &[u8] = b"free\0";
static NEW_SYMBOL: &[u8] = b"??2@YAPAXI@Z\0";
static DELETE_SYMBOL: &[u8] = b"??3@YAXPAX@Z\0";

fn setup_hooks() {
	static mut DONE: bool = false;

	unsafe {
		if DONE {
			return;
		}

		DONE = true;

		{
			use winapi::um::libloaderapi;

			THREAD_ID = winapi::um::processthreadsapi::GetCurrentThreadId();

			let mut module = std::ptr::null_mut();
			let module_path = CString::new("msvcr120.dll").unwrap();
			if libloaderapi::GetModuleHandleExA(0, module_path.as_ptr(), &mut module) == 0 {
				return;
			}

			let malloc =
				libloaderapi::GetProcAddress(module, MALLOC_SYMBOL.as_ptr() as *const c_char);
			let realloc =
				libloaderapi::GetProcAddress(module, REALLOC_SYMBOL.as_ptr() as *const c_char);
			let free = libloaderapi::GetProcAddress(module, FREE_SYMBOL.as_ptr() as *const c_char);
			let new = libloaderapi::GetProcAddress(module, NEW_SYMBOL.as_ptr() as *const c_char);
			let delete =
				libloaderapi::GetProcAddress(module, DELETE_SYMBOL.as_ptr() as *const c_char);

			// ¯\_(ツ)_/¯
			if malloc.is_null()
				|| realloc.is_null()
				|| free.is_null()
				|| new.is_null() || delete.is_null()
			{
				return;
			}

			{
				let hook = RawDetour::new(malloc as _, malloc_hook as _).unwrap();

				hook.enable().unwrap();
				MALLOC_ORIGINAL = Some(std::mem::transmute(hook.trampoline()));
				std::mem::forget(hook);
			}

			{
				let hook = RawDetour::new(realloc as _, realloc_hook as _).unwrap();

				hook.enable().unwrap();
				REALLOC_ORIGINAL = Some(std::mem::transmute(hook.trampoline()));
				std::mem::forget(hook);
			}

			{
				let hook = RawDetour::new(new as _, new_hook as _).unwrap();

				hook.enable().unwrap();
				NEW_ORIGINAL = Some(std::mem::transmute(hook.trampoline()));
				std::mem::forget(hook);
			}

			{
				let hook = RawDetour::new(free as _, free_hook as _).unwrap();

				hook.enable().unwrap();
				FREE_ORIGINAL = Some(std::mem::transmute(hook.trampoline()));
				std::mem::forget(hook);
			}

			{
				let hook = RawDetour::new(delete as _, delete_hook as _).unwrap();

				hook.enable().unwrap();
				DELETE_ORIGINAL = Some(std::mem::transmute(hook.trampoline()));
				std::mem::forget(hook);
			}
		}
	}
}

static mut MALLOC_ORIGINAL: Option<extern "cdecl" fn(usize) -> *mut c_void> = None;
static mut REALLOC_ORIGINAL: Option<extern "cdecl" fn(*mut c_void, usize) -> *mut c_void> = None;
static mut FREE_ORIGINAL: Option<extern "cdecl" fn(*mut c_void)> = None;

static mut NEW_ORIGINAL: Option<extern "cdecl" fn(usize) -> *mut c_void> = None;
static mut DELETE_ORIGINAL: Option<extern "cdecl" fn(*mut c_void)> = None;

extern "cdecl" fn malloc_hook(size: usize) -> *mut c_void {
	let ptr = unsafe { (MALLOC_ORIGINAL.unwrap())(size) };

	unsafe {
		if THREAD_ID == winapi::um::processthreadsapi::GetCurrentThreadId() {
			if let Some(state) = STATE.get_mut() {
				state.allocate(ptr, size);
			}
		}
	}

	ptr
}

extern "cdecl" fn realloc_hook(ptr: *mut c_void, size: usize) -> *mut c_void {
	let new_ptr = unsafe { (REALLOC_ORIGINAL.unwrap())(ptr, size) };

	unsafe {
		if THREAD_ID == winapi::um::processthreadsapi::GetCurrentThreadId() {
			if let Some(state) = STATE.get_mut() {
				state.free(ptr);
				state.allocate(new_ptr, size);
			}
		}
	}

	new_ptr
}

extern "cdecl" fn new_hook(size: usize) -> *mut c_void {
	let ptr = unsafe { (NEW_ORIGINAL.unwrap())(size) };

	unsafe {
		if THREAD_ID == winapi::um::processthreadsapi::GetCurrentThreadId() {
			if let Some(state) = STATE.get_mut() {
				state.allocate(ptr, size);
			}
		}
	}

	ptr
}

extern "cdecl" fn free_hook(ptr: *mut c_void) {
	unsafe {
		(FREE_ORIGINAL.unwrap())(ptr);
	}

	unsafe {
		if THREAD_ID == winapi::um::processthreadsapi::GetCurrentThreadId() {
			if let Some(state) = STATE.get_mut() {
				state.free(ptr);
			}
		}
	}
}

extern "cdecl" fn delete_hook(ptr: *mut c_void) {
	unsafe {
		(DELETE_ORIGINAL.unwrap())(ptr);
	}

	unsafe {
		if THREAD_ID == winapi::um::processthreadsapi::GetCurrentThreadId() {
			if let Some(state) = STATE.get_mut() {
				state.free(ptr);
			}
		}
	}
}

static mut STATE: UnsafeCell<Option<State>> = UnsafeCell::new(None);

struct State {
	file: File,
	live_allocs: HashMap<*const c_void, Allocation>,
}

impl State {
	fn new(dump_path: &str) -> io::Result<State> {
		Ok(State {
			file: File::create(dump_path)?,
			live_allocs: HashMap::new(),
		})
	}

	fn allocate(&mut self, ptr: *const c_void, size: usize) {
		if let Some(proc) = Self::current_proc_id() {
			self.live_allocs.insert(ptr, Allocation { proc, size });
		}
	}

	fn free(&mut self, ptr: *const c_void) {
		self.live_allocs.remove(&ptr);
	}

	fn dump(mut self) {
		use std::io::prelude::*;

		let mut totals: Vec<(ProcId, u64)> = vec![];

		for (_ptr, Allocation { proc, size }) in self.live_allocs {
			let proc_idx = proc.0 as usize;

			if totals.len() <= proc_idx {
				totals.resize(proc_idx + 1, (ProcId(0), 0));
			}

			totals[proc_idx].0 = proc;
			totals[proc_idx].1 += size as u64;
		}

		totals.sort_by(|x, y| x.1.cmp(&y.1));

		for (proc, total) in totals {
			if let Some(proc) = Proc::from_id(proc) {
				writeln!(self.file, "{} = {}", proc.path, total).unwrap();
			}
		}
	}

	fn current_proc_id() -> Option<ProcId> {
		unsafe {
			let ctx = *raw_types::funcs::CURRENT_EXECUTION_CONTEXT;
			if ctx.is_null() {
				return None;
			}

			let instance = (*ctx).proc_instance;
			if instance.is_null() {
				return None;
			}

			Some((*instance).proc)
		}
	}
}

struct Allocation {
	proc: ProcId,
	size: usize,
}

pub fn begin(path: &str) -> io::Result<()> {
	setup_hooks();

	unsafe {
		*STATE.get_mut() = Some(State::new(path)?);
	}

	Ok(())
}

pub fn end() {
	let state = unsafe { STATE.get_mut().take() };

	if let Some(state) = state {
		State::dump(state);
	}
}

#[shutdown]
fn shutdown() {
	unsafe {
		// Force recording to stop if the DM state is being destroyed
		STATE.get_mut().take();
	}
}
