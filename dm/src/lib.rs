use context::DMContext;
pub use dm_impl;
use dm_impl::hook;
use global_state::GLOBAL_STATE;
use value::Value;

pub mod byond_ffi;
pub mod context;
pub mod global_state;
pub mod hooks;
pub mod list;
pub mod proc;
pub mod raw_types;
pub mod string;
pub mod value;

extern crate inventory;

macro_rules! signature {
	($sig:tt) => {
		$crate::dm_impl::convert_signature!($sig)
	};
}

macro_rules! signatures {
	( $( $name:ident => $sig:tt ),* ) => {
		struct Signatures {
			$( $name: &'static [Option<u8>], )*
		}

		static SIGNATURES: Signatures = Signatures {
			$( $name: signature!($sig), )*
		};
	}
}

#[cfg(windows)]
const BYONDCORE: &'static str = "byondcore.dll";
#[cfg(windows)]
signatures! {
	string_table => "A1 ?? ?? ?? ?? 8B 04 ?? 85 C0 0F 84 ?? ?? ?? ?? 80 3D ?? ?? ?? ?? 00 8B 18",
	get_proc_array_entry => "E8 ?? ?? ?? ?? 8B C8 8D 45 ?? 6A 01 50 FF 76 ?? 8A 46 ?? FF 76 ?? FE C0",
	get_string_id => "55 8B EC 8B 45 ?? 83 EC ?? 53 56 8B 35",
	call_proc_by_id => "55 8B EC 81 EC ?? ?? ?? ?? A1 ?? ?? ?? ?? 33 C5 89 45 ?? 8B 55 ?? 8B 45",
	get_variable => "55 8B EC 8B 4D ?? 0F B6 C1 48 83 F8 ?? 0F 87 ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 ?? FF 75 ?? E8",
	set_variable => "55 8B EC 8B 4D 08 0F B6 C1 48 57 8B 7D 10 83 F8 53 0F ?? ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 18 FF 75 14 57 FF 75 0C E8 ?? ?? ?? ?? 83 C4 10 5F 5D C3",
	get_string_table_entry => "55 8B EC 8B 4D 08 3B 0D ?? ?? ?? ?? 73 10 A1",
	call_datum_proc_by_name => "55 8B EC 83 EC 0C 53 8B 5D 10 8D 45 FF 56 8B 75 14 57 6A 01 50 FF 75 1C C6 45 FF 00 FF 75 18 6A 00 56 53 ",
	dec_ref_count => "E8 ?? ?? ?? ?? 83 C4 0C 81 FF FF FF 00 00 74 ?? 85 FF 74 ?? 57 FF 75 ??",
	inc_ref_count => "E8 ?? ?? ?? ?? FF 77 ?? FF 77 ?? E8 ?? ?? ?? ?? 8D 77 ?? 56 E8 ?? ?? ?? ??",
	get_list_by_id => "E8 ?? ?? ?? ?? 83 C4 04 85 C0 75 13 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 04 5D E9 ?? ?? ?? ?? 5D C3",
	get_assoc_element => "55 8B EC 51 8B 4D 08 C6 45 FF 00 80 F9 05 76 11 80 F9 21 74 10 80 F9 0D 74 0B 80 F9 0E 75 65 EB 04 84 C9 74 5F 6A 00 8D 45 FF 50 FF 75 0C 51 6A 00 6A 7B",
	set_assoc_element => "55 8B EC 83 EC 14 8B 4D 08 C6 45 FF 00 80 F9 05 76 15 80 F9 21 74 14 80 F9 0D 74 0F 80 F9 0E 0F 85 ?? ?? ?? ?? EB 04 84 C9 74 7A 6A 00",
	create_list => "55 8B EC 8B ?? ?? ?? ?? ?? 56 85 C9 74 1B A1 ?? ?? ?? ?? 49 89 ?? ?? ?? ?? ?? 8B 34 88 81 FE ?? ?? ?? ?? 0F 85 ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? 8B F1 81 F9 ?? ?? ?? ?? 75 1B 51 68 ?? ?? ?? ?? 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 0C B8 ?? ?? ?? ?? 5E 5D C3",
	append_to_list => "55 8B EC 8B 4D 08 0F B6 C1 48 56 83 F8 53 0F 87 ?? ?? ?? ?? 0F B6 ?? ?? ?? ?? ?? FF 24 ?? ?? ?? ?? ?? FF 75 0C E8 ?? ?? ?? ?? 8B F0 83 C4 04 85 F6 0F 84 ?? ?? ?? ?? 8B 46 0C 40 50 56 E8 ?? ?? ?? ?? 8B 56 0C 83 C4 08 85 D2",
	remove_from_list => "55 8B EC 8B 4D 08 83 EC 0C 0F B6 C1 48 53 83 F8 53 0F 87 ?? ?? ?? ?? 0F B6 ?? ?? ?? ?? ?? 8B 55 10 FF 24 ?? ?? ?? ?? ?? 6A 0F FF 75 0C 51 E8 ?? ?? ?? ?? 50 E8 ?? ?? ?? ?? 83 C4 10 85 C0 0F 84 ?? ?? ?? ?? 8B 48 0C 8B 10 85 C9 0F 84 ?? ?? ?? ?? 8B 45 14 8B 5D 10",
	get_length => "55 8B EC 8B 4D 08 83 EC 18 0F B6 C1 48 53 56 57 83 F8 53 0F 87 ?? ?? ?? ?? 0F B6 ?? ?? ?? ?? ?? FF 24 ?? ?? ?? ?? ?? FF 75 0C"
}

#[cfg(unix)]
const BYONDCORE: &'static str = "libbyond.so";
#[cfg(unix)]
signatures! {
	string_table => "A1 ?? ?? ?? ?? 8B 04 ?? 85 C0 74 ?? 8B 18 89 75 ?? 89 34 24 E8 ?? ?? ?? ??",
	get_proc_array_entry => "E8 ?? ?? ?? ?? 8B 00 89 04 24 E8 ?? ?? ?? ?? 8B 00 89 44 24 ?? 8D 45 ??",
	get_string_id => "55 89 E5 57 56 89 CE 53 89 D3 83 EC 5C 8B 55 ?? 85 C0 88 55 ?? 0F 84 ?? ?? ?? ??",
	call_proc_by_id => "55 89 E5 81 EC D8 00 00 00 89 5D ?? 89 C3 0F B6 45 ?? 81 7D ?? FF FF 00 00",
	get_variable => "55 89 E5 81 EC C8 00 00 00 8B 55 ?? 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ??",
	set_variable => "55 89 E5 81 EC A8 00 00 00 8B 55 ?? 8B 45 ?? 89 5D ?? 8B 5D ?? 89 7D ??",
	get_string_table_entry => "55 89 E5 83 EC 18 8B 45 ?? 39 05 ?? ?? ?? ?? 76 ?? 8B 15 ?? ?? ?? ?? 8B 04 ??",
	call_datum_proc_by_name => "00",
	dec_ref_count => "E8 ?? ?? ?? ?? 8B 4D ?? C7 44 24 ?? 00 00 00 00 C7 44 24 ?? 00 00 00 00 89 0C 24",
	inc_ref_count => "E8 ?? ?? ?? ?? 8B 43 ?? 80 48 ?? 04 8B 5D ?? 8B 75 ?? 8B 7D ?? 89 EC 5D"
}

macro_rules! find_function {
	($scanner:ident, $name:ident, $typ:ident) => {
		let $name: $crate::raw_types::funcs::$typ;
		if let Some(ptr) = $scanner.find(SIGNATURES.$name.to_vec()) {
			unsafe {
				$name = std::mem::transmute(ptr as *const ());
				}
		} else {
			return Some(format!("FAILED (Couldn't find {})", stringify!($name)));
			}
	};
}

macro_rules! find_function_by_call {
	($scanner:ident, $name:ident, $typ:ident) => {
		let $name: $crate::raw_types::funcs::$typ;
		if let Some(ptr) = $scanner.find(SIGNATURES.$name.to_vec()) {
			unsafe {
				let offset = *(ptr.offset(1) as *const isize);
				$name = std::mem::transmute(ptr.offset(5).offset(offset) as *const ());
				}
		} else {
			return Some(format!("FAILED (Couldn't find {})", stringify!($name)));
			}
	};
}

macro_rules! with_scanner {
	($scanner:ident, $( $name:ident: $typ:ident),* ) => {
		$( find_function!($scanner, $name, $typ); )*
	};
}

macro_rules! with_scanner_by_call {
	($scanner:ident, $( $name:ident: $typ:ident),* ) => {
		$( find_function_by_call!($scanner, $name, $typ); )*
	};
}

byond_ffi_fn! { auxtools_init(_input) {
	// Already initialized. Just succeed?
	if GLOBAL_STATE.get().is_some() {
		return Some("SUCCESS".to_owned());
	}

	let byondcore = match sigscan::Scanner::for_module(BYONDCORE) {
		Some(v) => v,
		None => return Some("FAILED (Couldn't create scanner for byondcore.dll)".to_owned())
	};

	with_scanner! { byondcore,
		get_string_id: GetStringId,
		call_proc_by_id: CallProcById,
		get_variable: GetVariable,
		set_variable: SetVariable,
		get_string_table_entry: GetStringTableEntry,
		call_datum_proc_by_name: CallDatumProcByName,
		get_assoc_element: GetAssocElement,
		set_assoc_element: SetAssocElement,
		append_to_list: AppendToList,
		remove_from_list: RemoveFromList,
		get_length: GetLength,
		create_list: CreateList
	}

	with_scanner_by_call! {byondcore,
		get_proc_array_entry: GetProcArrayEntry,
		dec_ref_count: DecRefCount,
		inc_ref_count: IncRefCount,
		get_list_by_id: GetListById
	}

	let string_table: *mut raw_types::strings::StringTable;
	if let Some(ptr) = byondcore.find(SIGNATURES.string_table.to_vec()) {
		unsafe {
			// TODO: Could be nulls
			string_table = *(ptr.offset(1) as *mut *mut raw_types::strings::StringTable);
		}
	} else {
		return Some("FAILED (Couldn't find stringtable)".to_owned())
	}

	if GLOBAL_STATE.set(global_state::State {
		get_proc_array_entry: get_proc_array_entry,
		get_string_id: get_string_id,
		execution_context: std::ptr::null_mut(),
		string_table: string_table,
		call_proc_by_id: call_proc_by_id,
		get_variable: get_variable,
		set_variable: set_variable,
		get_string_table_entry: get_string_table_entry,
		call_datum_proc_by_name: call_datum_proc_by_name,
		dec_ref_count: dec_ref_count,
		inc_ref_count: inc_ref_count,
		get_list_by_id: get_list_by_id,
		get_assoc_element: get_assoc_element,
		set_assoc_element: set_assoc_element,
		create_list: create_list,
		append_to_list: append_to_list,
		remove_from_list: remove_from_list,
		get_length: get_length,
	}).is_err() {
		return Some("FAILED (Could not initialize global state)".to_owned());
	}

	if let Err(error) = hooks::init() {
		return Some(error);
	}

	for cthook in inventory::iter::<hooks::CompileTimeHook> {
		if let Err(e) = hooks::hook(cthook.proc_path, cthook.hook) {
			return Some(format!("FAILED (Could not hook proc {}: {:?})", cthook.proc_path, e));
		}
	}

	Some("SUCCESS".to_owned())
} }

#[hook("/proc/react")]
fn hello_proc_hook(some_datum: Value) {
	if let Some(num) = some_datum.get_number("hello") {
		(num * 2.0).into()
	} else {
		Value::null()
	}
}

#[hook("/datum/getvartest/proc/hookme")]
fn datum_proc_hook_test() {
	if let Some(mut l) = src.get_list("listvar") {
		l.set("bonk", &Value::from(7.0));

		src.call(
			"march_of_progress",
			&[&Value::from(1.0), &Value::from(2.0), &src, &Value::from(l)],
		);
	}

	let mut list = list::List::new();
	list.append(&Value::from(1.0));
	list.append(&src.get("hello"));
	list.remove(&Value::from(1.0));
	Value::from(list.len() as f32)
}

#[cfg(test)]
mod tests {
	#[test]
	fn test() {}
}
