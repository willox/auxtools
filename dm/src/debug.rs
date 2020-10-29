use crate::raw_types::{funcs, procs};
use crate::DMContext;
use crate::Proc;
use crate::StringRef;
use crate::Value;

pub struct StackFrame {
	pub proc: Proc,
	pub usr: Value,
	pub src: Value,
	pub dot: Value,
	pub args: Vec<(Option<StringRef>, Value)>,
	pub locals: Vec<(StringRef, Value)>,
	pub file_name: Option<StringRef>,
	pub line_number: Option<u32>,
	pub time_to_resume: Option<u32>,
	// TODO: current instruction & bytecode offset
}

pub struct CallStacks {
	pub active: Vec<StackFrame>,
	pub suspended: Vec<Vec<StackFrame>>,
}

impl StackFrame {
	unsafe fn from_context(context: *const procs::ExecutionContext) -> StackFrame {
		let instance = (*context).proc_instance;

		let proc = Proc::from_id((*instance).proc).unwrap();
		let param_names = proc.parameter_names();
		let local_names = proc.local_names();

		let names: Vec<String> = param_names.iter().map(|x| String::from(x)).collect();

		let usr = Value::from_raw((*instance).usr);
		let src = Value::from_raw((*instance).src);
		let dot = Value::from_raw((*context).dot);

		// Make sure to handle arguments/locals with no names (when there are more values than names)
		let args = (0..(*instance).args_count)
			.map(|i| {
				let name = match param_names.get(i as usize) {
					Some(name) => Some(name.clone()),
					None => None,
				};
				(name, Value::from_raw(*((*instance).args).add(i as usize)))
			})
			.collect();

		let locals = (0..(*context).locals_count)
			.map(|i| {
				(
					local_names.get(i as usize).unwrap().clone(),
					Value::from_raw(*((*context).locals).add(i as usize)),
				)
			})
			.collect();

		// TODO: When not set this?
		let file_name = Some(StringRef::from_id((*context).filename));
		let line_number = Some((*context).line);

		// TODO: When set this? For all sleepers?
		let time_to_resume = None;

		StackFrame {
			proc,
			usr,
			src,
			dot,
			args,
			locals,
			file_name,
			line_number,
			time_to_resume,
		}
	}
}

impl CallStacks {
	pub fn new(_: &DMContext) -> CallStacks {
		CallStacks {
			active: unsafe { CallStacks::from_context(*funcs::CURRENT_EXECUTION_CONTEXT) },
			suspended: vec![],
		}
	}

	fn from_context(mut context: *const procs::ExecutionContext) -> Vec<StackFrame> {
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

		frames
	}
}
