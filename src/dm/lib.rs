#![feature(type_ascription)]

use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};

use context::DMContext;
pub use dm_impl;
use global_state::GLOBAL_STATE;
use value::EitherValue;
use value::Value;

mod byond_ffi;
mod context;
mod global_state;
mod hooks;
mod proc;
mod raw_types;
mod string;
mod value;

macro_rules! signature {
	($sig:tt) => {
		$crate::dm_impl::convert_signature!($sig)
	};
}

fn random_string(n: usize) -> String {
	thread_rng().sample_iter(&Alphanumeric).take(n).collect()
}

#[cfg(windows)]
static SIGNATURES: phf::Map<&'static str, &'static [Option<u8>]> = phf::phf_map! {
	"string_table" => signature!("A1 ?? ?? ?? ?? 8B 04 ?? 85 C0 0F 84 ?? ?? ?? ?? 80 3D ?? ?? ?? ?? 00 8B 18 "),
	"get_proc_array_entry" => signature!("E8 ?? ?? ?? ?? 8B C8 8D 45 ?? 6A 01 50 FF 76 ?? 8A 46 ?? FF 76 ?? FE C0"),
	"get_string_id" => signature!("55 8B EC 8B 45 ?? 83 EC ?? 53 56 8B 35"),
	"call_proc_by_id" => signature!("55 8B EC 81 EC ?? ?? ?? ?? A1 ?? ?? ?? ?? 33 C5 89 45 ?? 8B 55 ?? 8B 45"),
	"get_variable" => signature!("55 8B EC 8B 4D ?? 0F B6 C1 48 83 F8 ?? 0F 87 ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 ?? FF 75 ?? E8"),
	"set_variable" => signature!("55 8B EC 8B 4D 08 0F B6 C1 48 57 8B 7D 10 83 F8 53 0F ?? ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 18 FF 75 14 57 FF 75 0C E8 ?? ?? ?? ?? 83 C4 10 5F 5D C3"),
	"get_string_table_entry" => signature!("55 8B EC 8B 4D 08 3B 0D ?? ?? ?? ?? 73 10 A1"),
	"call_datum_proc_by_name" => signature!("55 8B EC 83 EC 0C 53 8B 5D 10 8D 45 FF 56 8B 75 14 57 6A 01 50 FF 75 1C C6 45 FF 00 FF 75 18 6A 00 56 53 "),
	"dec_ref_count_call" => signature!("E8 ?? ?? ?? ?? FF 77 ?? FF 77 ?? E8 ?? ?? ?? ?? 8D 77 ?? 56 E8 ?? ?? ?? ??"),
	"inc_ref_count_call" => signature!("E8 ?? ?? ?? ?? 83 C4 0C 81 FF FF FF 00 00 74 ?? 85 FF 74 ?? 57 FF 75 ??"),
};

#[cfg(unix)]
static SIGNATURES: phf::Map<&'static str, &'static [Option<u8>]> = phf::phf_map! {
	"string_table" => signature!("A1 ?? ?? ?? ?? 8B 04 ?? 85 C0 74 ?? 8B 18 89 75 ?? 89 34 24 E8 ?? ?? ?? ??"),
	"get_proc_array_entry" => signature!("E8 ?? ?? ?? ?? 8B 00 89 04 24 E8 ?? ?? ?? ?? 8B 00 89 44 24 ?? 8D 45 ??"),
	"get_string_id" => signature!("55 89 E5 57 56 89 CE 53 89 D3 83 EC 5C 8B 55 ?? 85 C0 88 55 ?? 0F 84 ?? ?? ?? ??"),
	"call_proc_by_id" => signature!("55 89 E5 81 EC D8 00 00 00 89 5D ?? 89 C3 0F B6 45 ?? 81 7D ?? FF FF 00 00"),
	"get_variable" => signature!("55 89 E5 81 EC C8 00 00 00 8B 55 ?? 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ??"),
	"set_variable" => signature!("55 89 E5 81 EC A8 00 00 00 8B 55 ?? 8B 45 ?? 89 5D ?? 8B 5D ?? 89 7D ??"),
	"get_string_table_entry" => signature!("55 89 E5 83 EC 18 8B 45 ?? 39 05 ?? ?? ?? ?? 76 ?? 8B 15 ?? ?? ?? ?? 8B 04 ??"),
	"call_datum_proc_by_name" => signature!("00"),
	"dec_ref_count_call" => signature!("E8 ?? ?? ?? ?? 8B 4D ?? C7 44 24 ?? 00 00 00 00 C7 44 24 ?? 00 00 00 00 89 0C 24"),
	"inc_ref_count_call" => signature!("E8 ?? ?? ?? ?? 8B 43 ?? 80 48 ?? 04 8B 5D ?? 8B 75 ?? 8B 7D ?? 89 EC 5D"),
};

#[cfg(unix)]
const BYONDCORE: &'static str = "libbyond.so";

#[cfg(windows)]
const BYONDCORE: &'static str = "byondcore.dll";

byond_ffi_fn! { auxtools_init(_input) {
	// Already initialized. Just succeed?
	if GLOBAL_STATE.get().is_some() {
		return Some("SUCCESS".to_owned());
	}

	let byondcore = match sigscan::Scanner::for_module(BYONDCORE) {
		Some(v) => v,
		None => return Some("FAILED (Couldn't create scanner for byondcore.dll)".to_owned())
	};

	let mut string_table: *mut raw_types::strings::StringTable = unsafe { std::mem::transmute(0) };
	if let Some(ptr) = byondcore.find(SIGNATURES.get("string_table").unwrap().to_vec()) {
		unsafe {
			// TODO: Could be nulls
			string_table = *(ptr.offset(1) as *mut *mut raw_types::strings::StringTable);
		}
	} else {
		return Some("FAILED (Couldn't find stringtable)".to_owned())
	}

	let mut get_proc_array_entry: raw_types::funcs::GetProcArrayEntry  = unsafe { std::mem::transmute(0) };
	if let Some(ptr) = byondcore.find(SIGNATURES.get("get_proc_array_entry").unwrap().to_vec()) {
		unsafe {
			// TODO: Could be nulls
			let offset = *(ptr.offset(1) as *const isize);
			get_proc_array_entry = std::mem::transmute(ptr.offset(5).offset(offset) as *const ());
		}
	} else {
		//return Some("FAILED (Couldn't find GetProcArrayEntry)".to_owned())
	}

	let mut get_string_id: raw_types::funcs::GetStringId  = unsafe { std::mem::transmute(0) };;
	if let Some(ptr) = byondcore.find(SIGNATURES.get("get_string_id").unwrap().to_vec()) {
		unsafe {
			// TODO: Could be nulls
			get_string_id = std::mem::transmute(ptr as *const ());
		}
	} else {
		//return Some("FAILED (Couldn't find GetStringId)".to_owned())
	}


	let mut call_proc_by_id: raw_types::funcs::CallProcById = unsafe { std::mem::transmute(0) };
	if let Some(ptr) = byondcore.find(SIGNATURES.get("call_proc_by_id").unwrap().to_vec()) {
		unsafe {
			// TODO: Could be nulls
			call_proc_by_id = std::mem::transmute(ptr as *const ());
		}
	} else {
		//return Some("FAILED (Couldn't find CallGlobalProc)".to_owned())
	}

	let get_variable: raw_types::funcs::GetVariable;
	if let Some(ptr) = byondcore.find(SIGNATURES.get("get_variable").unwrap().to_vec()) {
		unsafe {
			// TODO: Could be nulls
			get_variable = std::mem::transmute(ptr as *const ());
		}
	} else {
		return Some("FAILED (Couldn't find GetVariable)".to_owned())
	}

	let set_variable: raw_types::funcs::SetVariable;
	if let Some(ptr) = byondcore.find(SIGNATURES.get("set_variable").unwrap().to_vec()) {
		unsafe {
			// TODO: Could be nulls
			set_variable = std::mem::transmute(ptr as *const ());
		}
	} else {
		return Some("FAILED (Couldn't find SetVariable)".to_owned())
	}

	let get_string_table_entry: raw_types::funcs::GetStringTableEntry;
	if let Some(ptr) = byondcore.find(SIGNATURES.get("get_string_table_entry").unwrap().to_vec()) {
		unsafe {
			// TODO: Could be nulls
			get_string_table_entry = std::mem::transmute(ptr as *const ());
		}
	} else {
		return Some("FAILED (Couldn't find GetStringTableEntry)".to_owned())
	}

	let mut call_datum_proc_by_name: raw_types::funcs::CallDatumProcByName = unsafe { std::mem::transmute(0) };
	if let Some(ptr) = byondcore.find(SIGNATURES.get("call_datum_proc_by_name").unwrap().to_vec()) {
		unsafe {
			// TODO: Could be nulls
			call_datum_proc_by_name = std::mem::transmute(ptr as *const ());
		}
	} else {
		//return Some("FAILED (Couldn't find CallDatumProcByName)".to_owned())
	}

	/*
	char* x_ref_count_call = (char*)Pocket::Sigscan::FindPattern(BYONDCORE, "3D ?? ?? ?? ?? 74 14 50 E8 ?? ?? ?? ?? FF 75 0C FF 75 08 E8", 20);
	DecRefCount = (DecRefCountPtr)(x_ref_count_call + *(int*)x_ref_count_call + 4); //x_ref_count_call points to the relative offset to DecRefCount from the call site
	x_ref_count_call = (char*)Pocket::Sigscan::FindPattern(BYONDCORE, "FF 75 10 E8 ?? ?? ?? ?? FF 75 0C 8B F8 FF 75 08 E8 ?? ?? ?? ?? 57", 17);
	IncRefCount = (IncRefCountPtr)(x_ref_count_call + *(int*)x_ref_count_call + 4);
	*/

	let mut dec_ref_count: raw_types::funcs::DecRefCount = unsafe { std::mem::transmute(0) };;
	if let Some(ptr) = byondcore.find(SIGNATURES.get("dec_ref_count_call").unwrap().to_vec()) {
		unsafe {
			// TODO: Could be nulls
			let offset = *(ptr.offset(1) as *const isize);
			dec_ref_count = std::mem::transmute(ptr.offset(5).offset(offset) as *const ());
		}
	} else {
		//return Some("FAILED (Couldn't find dec_ref_count)".to_owned())
	}

	let mut inc_ref_count: raw_types::funcs::DecRefCount = unsafe { std::mem::transmute(0) };;
	if let Some(ptr) = byondcore.find(SIGNATURES.get("inc_ref_count_call").unwrap().to_vec()) {
		unsafe {
			// TODO: Could be nulls
			let offset = *(ptr.offset(1) as *const isize);
			inc_ref_count = std::mem::transmute(ptr.offset(5).offset(offset) as *const ());
		}
	} else {
		//return Some("FAILED (Couldn't find dec_ref_count)".to_owned())
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
		inc_ref_count: inc_ref_count

	}).is_err() {
		panic!();
	}

	if let Err(error) = hooks::init() {
		return Some(error);
	}

	proc::populate_procs();

	hooks::hook("/proc/react", hello_proc_hook).unwrap_or_else(|e| {
			//msgbox::create("Failed to hook!", e.to_string().as_str(), msgbox::IconType::Error)
			eprintln!("Failed to hook /proc/react: {}", e.to_string());
		}
	);

	Some("SUCCESS".to_owned())
} }

macro_rules! args {
    () => {
        None
    };
    ($($x:expr),+ $(,)?) => {
        Some(vec![$(value::EitherValue::from($x),)+])
    };
}

fn hello_proc_hook<'a>(
	ctx: &'a DMContext,
	src: Value<'a>,
	usr: Value<'a>,
	args: &Vec<Value<'a>>,
) -> EitherValue<'a> {
	let dat = args[0];

	let string: string::StringRef = "penis".into();
	let string2: string::StringRef = "penisaaa".into();

	string.into()
}

#[cfg(test)]
mod tests {
	#[test]
	fn test() {}
}
