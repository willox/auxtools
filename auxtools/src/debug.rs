use crate::{
	raw_types::{funcs, procs},
	Proc, StringRef, Value
};

pub struct StackFrame {
	pub context: *mut procs::ExecutionContext,
	pub instance: *mut procs::ProcInstance,
	pub proc: Proc,
	pub offset: u16,
	pub usr: Value,
	pub src: Value,
	pub dot: Value,
	pub args: Vec<(Option<StringRef>, Value)>,
	pub locals: Vec<(StringRef, Value)>,
	pub file_name: Option<StringRef>,
	pub line_number: Option<u32> /* pub time_to_resume: Option<u32>,
	                              * TODO: current instruction & bytecode offset */
}

pub struct CallStacks {
	pub active: Vec<StackFrame>,
	pub suspended: Vec<Vec<StackFrame>>
}

impl StackFrame {
	unsafe fn from_context(context: *mut procs::ExecutionContext) -> StackFrame {
		let instance = (*context).proc_instance;

		let proc = Proc::from_id((*instance).proc).unwrap();
		let offset = (*context).bytecode_offset;
		let param_names = proc.parameter_names();
		let local_names = proc.local_names();

		let usr = Value::from_raw((*instance).usr);
		let src = Value::from_raw((*instance).src);
		let dot = Value::from_raw((*context).dot);

		// Make sure to handle arguments/locals with no names (when there are more
		// values than names)
		let args = (0..(*instance).args_count())
			.map(|i| {
				let name = param_names.get(i as usize).cloned();
				(name, Value::from_raw(*((*instance).args()).add(i as usize)))
			})
			.collect();

		let locals = (0..(*context).locals_count)
			.map(|i| {
				(
					local_names.get(i as usize).unwrap().clone(),
					Value::from_raw(*((*context).locals).add(i as usize))
				)
			})
			.collect();

		// Only populate the line number if we've got a file-name
		let mut file_name = None;
		let mut line_number = None;
		if (*context).filename.valid() {
			file_name = Some(StringRef::from_id((*context).filename));
			line_number = Some((*context).line);
		}

		// TODO: When set this? For all sleepers?
		// let time_to_resume = None;

		StackFrame {
			context,
			instance,
			proc,
			offset,
			usr,
			src,
			dot,
			args,
			locals,
			file_name,
			line_number // time_to_resume,
		}
	}
}

enum CallStackKind {
	Active,
	Suspended
}

impl Default for CallStacks {
	fn default() -> Self {
		Self::new()
	}
}

impl CallStacks {
	pub fn new() -> CallStacks {
		let mut suspended = vec![];

		unsafe {
			let buffer = (*funcs::SUSPENDED_PROCS_BUFFER).buffer;
			let procs = funcs::SUSPENDED_PROCS;
			let front = (*procs).front;
			let back = (*procs).back;

			for x in front..back {
				let instance = *buffer.add(x);
				let context = (*instance).context;
				suspended.push(CallStacks::from_context(context, CallStackKind::Suspended));
			}
		}

		CallStacks {
			active: unsafe { CallStacks::from_context(*funcs::CURRENT_EXECUTION_CONTEXT, CallStackKind::Active) },
			suspended
		}
	}

	fn from_context(mut context: *mut procs::ExecutionContext, kind: CallStackKind) -> Vec<StackFrame> {
		let mut frames = vec![];

		loop {
			if context.is_null() {
				break;
			}

			unsafe {
				frames.push(StackFrame::from_context(context));
				context = (*context).parent_context;
			}
		}

		// BYOND stores sleeping stacks' frames in reverse-order
		match kind {
			CallStackKind::Active => frames,
			CallStackKind::Suspended => frames.into_iter().rev().collect()
		}
	}
}
