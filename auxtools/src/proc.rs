use std::{
	cell::RefCell,
	collections::{hash_map::Entry, HashMap},
	fmt,
	marker::PhantomData
};

use ahash::RandomState;
use fxhash::FxHashMap;

use crate::*;

// ### A note on Override IDs
//
// Procs in DM can be defined multiple times.
//
// ```
// /proc/hello() // Override #0 or base proc
// 		world << "Hello"
//
// 	/hello() // Override #1
// 		..() // Calls override #0
// 		world << "World"
//
// 	/hello() // Override #2
// 		..() // Calls override #1
// 		world << "!!!"
// 	```
//
// 	To get the nth override, use [Proc::find_override]: `let hello = Proc::find_override("/proc/hello", n).unwrap()`
// [Proc::find] retrieves the base proc.

/// Used to hook and call procs.
///
/// A `Proc` stays on the thread BYOND runs on. Calling one from anywhere else
/// would run DM code while the game is in the middle of something else.
///
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<auxtools::Proc>();
/// ```
#[derive(Clone)]
pub struct Proc {
	pub id: raw_types::procs::ProcId,
	pub path: String,
	// Keeps a Proc from being sent to another thread
	phantom: PhantomData<*mut ()>
}

fn entry_by_id(id: raw_types::procs::ProcId) -> *mut raw_types::procs::ProcEntry {
	let mut entry: *mut raw_types::procs::ProcEntry = std::ptr::null_mut();
	unsafe {
		assert_eq!(raw_types::funcs::get_proc_array_entry(&mut entry, id), 1);
	}
	entry
}

impl Proc {
	/// The override id of a proc set apart (see [`Proc::set_apart`])
	pub const APART: u32 = u32::MAX;

	/// The definitions of a path, by override id
	pub fn overrides<S: Into<String>>(path: S) -> Vec<Self> {
		let path = strip_path(path.into());
		PROCS_BY_NAME.with(|h| h.borrow().get(&path).cloned().unwrap_or_default())
	}

	/// Renumber a path's definitions after reload, using the supplied order.
	/// Omitted definitions are no longer available through
	/// [`Proc::find_override`].
	pub fn set_overrides<S: Into<String>>(path: S, procs: Vec<Self>) {
		let path = strip_path(path.into());
		PROC_OVERRIDE_IDS.with(|override_ids| {
			let mut override_ids = override_ids.borrow_mut();
			for (i, proc) in procs.iter().enumerate() {
				override_ids.insert(proc.id, i as u32);
			}
		});
		PROCS_BY_NAME.with(|h| h.borrow_mut().insert(path, procs));
	}

	/// BYOND's own record for this proc.
	///
	/// Don't hold on to the pointer. BYOND moves its whole proc table whenever
	/// the table grows (`new` on a verb path does it), so it is only good until
	/// the next time DM code runs.
	pub fn entry(&self) -> *mut raw_types::procs::ProcEntry {
		entry_by_id(self.id)
	}

	/// Finds the first proc with the given path
	pub fn find<S: Into<String>>(path: S) -> Option<Self> {
		get_proc(path)
	}

	/// Finds the n'th re-defined proc with the given path. [`Proc::APART`]
	/// finds the one set apart.
	pub fn find_override<S: Into<String>>(path: S, override_id: u32) -> Option<Self> {
		get_proc_override(path, override_id)
	}

	pub fn from_id(id: raw_types::procs::ProcId) -> Option<Self> {
		let proc_entry = entry_by_id(id);
		if proc_entry.is_null() {
			return None;
		}
		let path = strip_path(unsafe { StringRef::from_id((*proc_entry).path).into() });
		Some(Proc {
			id,
			path,
			phantom: PhantomData
		})
	}

	pub unsafe fn file_name(&self) -> Option<StringRef> {
		let bytecode = self.bytecode();
		if bytecode.len() < 2 || bytecode[0] != 0x84 {
			return None;
		}

		let file_id = raw_types::strings::StringId(bytecode[0x01]);
		if !file_id.valid() {
			return None;
		}

		Some(StringRef::from_id(file_id))
	}

	pub fn parameter_names(&self) -> Vec<StringRef> {
		unsafe {
			let (data, count) = raw_types::misc::get_parameters(*(*self.entry()).metadata.parameters());
			(0..count).map(|i| StringRef::from_variable_id((*data.add(i)).name)).collect()
		}
	}

	pub fn local_names(&self) -> Vec<StringRef> {
		unsafe {
			let (names, count) = raw_types::misc::get_locals(*(*self.entry()).metadata.locals());
			(0..count).map(|i| StringRef::from_variable_id(*names.add(i))).collect()
		}
	}

	pub fn set_bytecode(&self, bytecode: Vec<u32>) {
		crate::bytecode_manager::set_bytecode(self, bytecode);
	}

	/// Register proc table entries appended since `first` for name lookup.
	pub fn add_to_table(first: raw_types::procs::ProcId) {
		let mut id = first.0;
		while let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) {
			register_proc(proc);
			id += 1;
		}
	}

	/// How many locals a new frame of this proc gets.
	pub fn locals_count(&self) -> usize {
		unsafe { raw_types::misc::get_locals(*(*self.entry()).metadata.locals()).1 }
	}

	pub unsafe fn bytecode_mut_ptr(&self) -> (*mut u32, u16) {
		raw_types::misc::get_bytecode(*(*self.entry()).metadata.bytecode())
	}

	pub unsafe fn bytecode(&self) -> &[u32] {
		let (ptr, count) = self.bytecode_mut_ptr();
		std::slice::from_raw_parts(ptr, count as usize)
	}

	/// Calls a global proc with the given arguments.
	///
	/// # Examples
	///
	/// This function is equivalent to `return do_explode(3)` in DM.
	/// ```ignore
	/// #[hook("/proc/my_proc")]
	/// fn my_proc_hook() -> DMResult {
	///     let proc = Proc::find("/proc/do_explode").unwrap();
	///     proc.call(&[&Value::from(3.0)])
	/// }
	/// ```
	pub fn call(&self, args: &[&Value]) -> runtime::DMResult {
		let mut ret = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 }
		};

		unsafe {
			// Increment ref-count of args permenently before passing them on
			for v in args {
				raw_types::funcs::inc_ref_count(v.raw);
			}

			let args: Vec<_> = args.iter().map(|e| e.raw).collect();

			// 2 is the proc_type BYOND's own direct calls pass. It never passes
			// 0, and a `..()` with no parent hands back garbage in
			// a frame that has it.
			if raw_types::funcs::call_proc_by_id(&mut ret, Value::NULL.raw, 2, self.id, 0, Value::NULL.raw, args.as_ptr(), args.len(), 0, 0) == 1 {
				return Ok(Value::from_raw_owned(ret));
			}
		}

		Err(runtime!("External proc call failed"))
	}

	/// Exclude a debugger helper from source override numbering.
	/// The helper uses [`Proc::APART`]; existing and future source definitions
	/// keep their source-order IDs. Only one helper per path can be set apart.
	pub fn set_apart(&self) {
		PROC_OVERRIDE_IDS.with(|override_ids| {
			let mut override_ids = override_ids.borrow_mut();
			PROCS_BY_NAME.with(|h| {
				if let Some(procs) = h.borrow_mut().get_mut(&self.path) {
					procs.retain(|proc| proc.id != self.id);
					for (i, proc) in procs.iter().enumerate() {
						override_ids.insert(proc.id, i as u32);
					}
				}
			});
			override_ids.insert(self.id, Self::APART);
		});
		PROCS_APART.with(|apart| apart.borrow_mut().insert(self.path.clone(), self.clone()));
	}

	pub fn override_id(&self) -> u32 {
		PROC_OVERRIDE_IDS.with(|override_ids| match override_ids.borrow().get(&self.id) {
			Some(id) => *id,
			None => 0
		})
	}
}

impl fmt::Debug for Proc {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let path = unsafe { (*self.entry()).path };
		write!(f, "Proc({:?})", unsafe { StringRef::from_id(path) })
	}
}

thread_local!(static PROCS_BY_NAME: RefCell<HashMap<String, Vec<Proc>, RandomState>> = RefCell::new(HashMap::with_hasher(RandomState::default())));
thread_local!(static PROC_OVERRIDE_IDS: RefCell<FxHashMap<raw_types::procs::ProcId, u32>> = RefCell::new(FxHashMap::default()));
thread_local!(static PROCS_APART: RefCell<HashMap<String, Proc, RandomState>> = RefCell::new(HashMap::with_hasher(RandomState::default())));

fn strip_path(p: String) -> String {
	p.replace("/proc/", "/").replace("/verb/", "/")
}

pub fn populate_procs() {
	let mut i: u32 = 0;
	loop {
		let proc = Proc::from_id(raw_types::procs::ProcId(i));
		if proc.is_none() {
			break;
		}
		register_proc(proc.unwrap());

		i += 1;
	}
}

fn register_proc(proc: Proc) {
	PROC_OVERRIDE_IDS.with(|override_ids| {
		let mut override_ids = override_ids.borrow_mut();

		PROCS_BY_NAME.with(|h| {
			match h.borrow_mut().entry(proc.path.clone()) {
				Entry::Occupied(mut o) => {
					let vec = o.get_mut();
					override_ids.insert(proc.id, vec.len() as u32);
					vec.push(proc);
				}
				Entry::Vacant(v) => {
					override_ids.insert(proc.id, 0);
					v.insert(vec![proc]);
				}
			};
		});
	});
}

pub fn clear_procs() {
	PROCS_BY_NAME.with(|h| h.borrow_mut().clear());
	PROC_OVERRIDE_IDS.with(|override_ids| override_ids.borrow_mut().clear());
	PROCS_APART.with(|apart| apart.borrow_mut().clear());
}

pub fn get_proc_override<S: Into<String>>(path: S, override_id: u32) -> Option<Proc> {
	let s = strip_path(path.into());
	if override_id == Proc::APART {
		return PROCS_APART.with(|apart| apart.borrow().get(&s).cloned());
	}
	PROCS_BY_NAME.with(|h| h.borrow().get(&s)?.get(override_id as usize).cloned())
}

/// Retrieves the 0th override of a proc.
pub fn get_proc<S: Into<String>>(path: S) -> Option<Proc> {
	get_proc_override(path, 0)
}
