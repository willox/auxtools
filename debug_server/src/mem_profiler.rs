use detour::RawDetour;
use std::ffi::c_void;
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
			use windows::core::PCSTR;
			use windows::Win32::System::LibraryLoader;

			THREAD_ID = windows::Win32::System::Threading::GetCurrentThreadId();

			let mut module = windows::Win32::Foundation::HINSTANCE::default();
			if !LibraryLoader::GetModuleHandleExA(0, windows::s!("msvcr120.dll"), &mut module)
				.as_bool()
			{
				return;
			}

			let malloc =
				LibraryLoader::GetProcAddress(module, PCSTR::from_raw(MALLOC_SYMBOL.as_ptr()));
			let realloc =
				LibraryLoader::GetProcAddress(module, PCSTR::from_raw(REALLOC_SYMBOL.as_ptr()));
			let free = LibraryLoader::GetProcAddress(module, PCSTR::from_raw(FREE_SYMBOL.as_ptr()));
			let new = LibraryLoader::GetProcAddress(module, PCSTR::from_raw(NEW_SYMBOL.as_ptr()));
			let delete =
				LibraryLoader::GetProcAddress(module, PCSTR::from_raw(DELETE_SYMBOL.as_ptr()));

			// ¯\_(ツ)_/¯
			if malloc.is_none()
				|| realloc.is_none()
				|| free.is_none()
				|| new.is_none() || delete.is_none()
			{
				return;
			}

			{
				let hook = RawDetour::new(malloc.unwrap() as _, malloc_hook as _).unwrap();

				hook.enable().unwrap();
				MALLOC_ORIGINAL = Some(std::mem::transmute(hook.trampoline()));
				std::mem::forget(hook);
			}

			{
				let hook = RawDetour::new(realloc.unwrap() as _, realloc_hook as _).unwrap();

				hook.enable().unwrap();
				REALLOC_ORIGINAL = Some(std::mem::transmute(hook.trampoline()));
				std::mem::forget(hook);
			}

			{
				let hook = RawDetour::new(new.unwrap() as _, new_hook as _).unwrap();

				hook.enable().unwrap();
				NEW_ORIGINAL = Some(std::mem::transmute(hook.trampoline()));
				std::mem::forget(hook);
			}

			{
				let hook = RawDetour::new(free.unwrap() as _, free_hook as _).unwrap();

				hook.enable().unwrap();
				FREE_ORIGINAL = Some(std::mem::transmute(hook.trampoline()));
				std::mem::forget(hook);
			}

			{
				let hook = RawDetour::new(delete.unwrap() as _, delete_hook as _).unwrap();

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
		if THREAD_ID == windows::Win32::System::Threading::GetCurrentThreadId() {
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
		if THREAD_ID == windows::Win32::System::Threading::GetCurrentThreadId() {
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
		if THREAD_ID == windows::Win32::System::Threading::GetCurrentThreadId() {
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
		if THREAD_ID == windows::Win32::System::Threading::GetCurrentThreadId() {
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
		if THREAD_ID == windows::Win32::System::Threading::GetCurrentThreadId() {
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
