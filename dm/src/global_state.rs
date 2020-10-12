use super::raw_types;
use once_cell::sync::OnceCell;

pub static GLOBAL_STATE: OnceCell<State> = OnceCell::new();

unsafe impl Sync for State {}
unsafe impl Send for State {}

// TODO: These should all be unsafe
pub struct State {
	pub get_proc_array_entry: raw_types::funcs::GetProcArrayEntry,
	pub execution_context: *mut raw_types::procs::ExecutionContext,
	pub string_table: *mut raw_types::strings::StringTable,
	pub get_string_id: raw_types::funcs::GetStringId,
	pub get_string_table_entry: raw_types::funcs::GetStringTableEntry,
	pub call_proc_by_id: raw_types::funcs::CallProcById,
	pub get_variable: raw_types::funcs::GetVariable,
	pub set_variable: raw_types::funcs::SetVariable,
	pub call_datum_proc_by_name: raw_types::funcs::CallDatumProcByName,
	pub dec_ref_count: raw_types::funcs::DecRefCount,
	pub inc_ref_count: raw_types::funcs::IncRefCount,
	pub get_list_by_id: raw_types::funcs::GetListById,
	pub get_assoc_element: raw_types::funcs::GetAssocElement,
	pub set_assoc_element: raw_types::funcs::SetAssocElement,
	pub create_list: raw_types::funcs::CreateList,
	pub append_to_list: raw_types::funcs::AppendToList,
	pub remove_from_list: raw_types::funcs::RemoveFromList,
	pub get_length: raw_types::funcs::GetLength,
}
