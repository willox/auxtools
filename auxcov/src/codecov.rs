use std::{
	cell::RefCell,
	collections::{BTreeMap, HashMap, HashSet},
	convert::TryInto,
	fs::create_dir_all,
	io::Error,
	panic::catch_unwind,
	path::{Path, PathBuf},
	rc::Rc
};

use auxtools::{raw_types::strings::StringId, *};
use dmasm::Instruction;
use grcov::{output_cobertura, CovResult, FunctionMap, ResultTuple};
use instruction_hooking::{disassemble_env::DisassembleEnv, InstructionHook};

struct TrackerContext {
	output_file_name: String,
	proc_id_map: Vec<Option<Rc<RefCell<Vec<u64>>>>>,
	filename_map: HashMap<String, Rc<RefCell<Vec<u64>>>>
}

pub struct Tracker {
	hittable_lines: HashMap<String, HashSet<u32>>,
	contexts: Vec<TrackerContext>,
	total_procs: u32
}

impl Tracker {
	pub fn new() -> Tracker {
		let mut hittable_lines = HashMap::<String, HashSet<u32>>::new();
		let mut i = 0_u32;
		while let Some(proc) = Proc::from_id(raw_types::procs::ProcId(i)) {
			i += 1;

			let mut current_file_option;
			let bytecode;
			unsafe {
				current_file_option = proc.file_name();
				bytecode = proc.bytecode().to_vec();
			}

			let mut env = DisassembleEnv;
			let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode[..], &mut env);

			for node in nodes {
				if let dmasm::Node::Instruction(instruction, _) = node {
					match instruction {
						Instruction::DbgFile(file) => {
							let string_ref_result = StringRef::from_raw(&file.0);
							match string_ref_result {
								Ok(string_ref) => current_file_option = Some(string_ref),
								Err(_) => current_file_option = None
							}
						}
						Instruction::DbgLine(line) => {
							if let Some(current_file) = &current_file_option {
								let mut file_name = current_file.to_string();

								// strip quotes
								file_name = file_name[1..file_name.len() - 1].to_string();
								if !file_name.ends_with(".dm") {
									continue;
								}

								hittable_lines.entry(file_name).or_default().insert(line);
							}
						}
						_ => {}
					}
				}
			}
		}

		Tracker {
			hittable_lines,
			contexts: Vec::new(),
			total_procs: i
		}
	}

	pub fn init_context(&mut self, output_file_name: String) -> bool {
		if self.contexts.iter().any(|context| context.output_file_name == *output_file_name) {
			return false;
		}

		let mut context: TrackerContext = TrackerContext {
			output_file_name,
			proc_id_map: Vec::new(),
			filename_map: HashMap::new()
		};

		context.proc_id_map.reserve(self.total_procs as usize);
		context.filename_map.reserve(self.hittable_lines.len());

		for (file_name, executable_lines) in &self.hittable_lines {
			let file_name = file_name.to_string();
			if !file_name.ends_with(".dm") {
				continue;
			}

			let mut hit_map = Vec::<u64>::new();
			for line in executable_lines {
				let i: usize = *line as usize;
				if i > hit_map.len() {
					hit_map.resize(i, 0);
				}

				hit_map[i - 1] = 1;
			}

			let hit_map_rc = Rc::new(RefCell::new(hit_map));
			context.filename_map.insert(file_name, hit_map_rc);
		}

		self.contexts.push(context);
		true
	}

	// returns true if we need to pause
	pub fn process_dbg_line(&mut self, ctx: &raw_types::procs::ExecutionContext, proc_instance: &raw_types::procs::ProcInstance) {
		if ctx.line == 0 || !ctx.filename.valid() {
			return;
		}

		let filename_id = ctx.filename;
		let proc_map_index = proc_instance.proc.0 as usize;
		let line = ctx.line as usize;

		let mut known_file_name: Option<String> = None;
		for context in &mut self.contexts {
			match &known_file_name {
				Some(file_name) => {
					context.process_dbg_line(filename_id, proc_map_index, line, Some(file_name));
				}
				None => {
					let processed_file_name = context.process_dbg_line(filename_id, proc_map_index, line, None);
					if let Some((file_name, valid)) = processed_file_name {
						if !valid {
							break;
						}

						known_file_name = Some(file_name)
					}
				}
			}
		}
	}

	pub fn finalize_context(&mut self, output_file_name: &String) -> Result<bool, Error> {
		let mut remove_index_option = None;
		let mut result = Ok(());
		for (i, context) in self.contexts.iter().enumerate() {
			if context.output_file_name == *output_file_name {
				remove_index_option = Some(i);
				result = context.finalize();
				break;
			}
		}

		match remove_index_option {
			Some(remove_index) => {
				self.contexts.remove(remove_index);
				result.map(|_| true)
			}
			None => Ok(false)
		}
	}

	fn finalize(&mut self) -> Result<(), Vec<Error>> {
		let mut errors_option = None;
		for context in &self.contexts {
			let result = context.finalize(); // dropping the results because what can ya do?
			if let Err(error) = result {
				match &mut errors_option {
					None => {
						errors_option = Some(vec![error]);
					}
					Some(existing_vec) => {
						existing_vec.push(error);
					}
				}
			}
		}

		self.contexts.clear();

		if let Some(errors) = errors_option {
			return Err(errors);
		}

		Ok(())
	}
}

impl Drop for Tracker {
	fn drop(&mut self) {
		let _result = self.finalize(); // dropping the result here because what can ya
		                         // do?
	}
}

impl InstructionHook for Tracker {
	fn handle_instruction(&mut self, ctx: *mut raw_types::procs::ExecutionContext) {
		let ctx_ref;
		let proc_instance_ref;
		unsafe {
			ctx_ref = &*ctx;
			proc_instance_ref = &*ctx_ref.proc_instance;
		}

		self.process_dbg_line(ctx_ref, proc_instance_ref);
	}
}

impl TrackerContext {
	fn process_dbg_line(
		&mut self,
		filename_id: StringId,
		proc_map_index: usize,
		line: usize,
		known_file_name: Option<&String>
	) -> Option<(String, bool)> {
		let needs_extending = self.proc_id_map.len() < proc_map_index + 1;

		if !needs_extending {
			if let Some(hit_map_cell) = &self.proc_id_map[proc_map_index] {
				let mut hit_map = hit_map_cell.borrow_mut();
				if hit_map.len() < line {
					hit_map.resize(line, 0);
				}

				let i = line - 1;
				let mut current_hits = hit_map[i];
				let existing_line = current_hits > 0;
				if !existing_line {
					current_hits = 1;
				}

				hit_map[i] = current_hits + 1;
				return None;
			}
		}

		// Slow: Need to lookup based on filename and create proc entry
		let file_name;
		let using_local_file_name = known_file_name.is_none();
		let return_value;
		if using_local_file_name {
			let quoted_file_name;
			unsafe {
				quoted_file_name = StringRef::from_id(filename_id).to_string();
			}

			// strip quotes
			let local_file_name = quoted_file_name[1..quoted_file_name.len() - 1].to_string();

			// WHY BYOND? WHY
			// Procs, datums, random-ass strings... Just why?
			if !local_file_name.ends_with(".dm") {
				return Some((local_file_name, false));
			}

			return_value = Some((local_file_name, true));
			file_name = &return_value.as_ref().unwrap().0;
		} else {
			file_name = known_file_name.unwrap();
			return_value = None;
		}

		if needs_extending {
			self.proc_id_map.resize(proc_map_index + 1, None);
		}

		match self.filename_map.get(file_name) {
			Some(hit_map_cell) => {
				let mut hit_map = hit_map_cell.borrow_mut();
				if hit_map.len() < line {
					hit_map.resize(line, 0);
				}

				let i = line - 1;
				let mut current_hits = hit_map[i];
				let existing_line = current_hits > 0;
				if !existing_line {
					current_hits = 1;
				}

				hit_map[i] = current_hits + 1;

				self.proc_id_map[proc_map_index] = Some(hit_map_cell.clone());
			}
			None => {
				// Slower: Need to insert both file and proc
				let mut hit_map = Vec::<u64>::new();
				if hit_map.len() < line {
					hit_map.resize(line, 0);
				}

				let i = line - 1;
				let mut current_hits = hit_map[i];
				let existing_line = current_hits > 0;
				if !existing_line {
					current_hits = 1;
				}

				hit_map[i] = current_hits + 1;

				let hit_map_rc = Rc::new(RefCell::new(hit_map));
				self.filename_map.insert(file_name.clone(), hit_map_rc.clone());
				self.proc_id_map[proc_map_index] = Some(hit_map_rc);
			}
		}

		return_value
	}

	fn finalize(&self) -> Result<(), Error> {
		let result_tuples: Vec<ResultTuple> = self
			.filename_map
			.iter()
			.map(|(file_name, hit_map)| {
				let mut new_map = BTreeMap::<u32, u64>::new();
				for (line_minus_one, hits) in hit_map.borrow().iter().enumerate() {
					if *hits == 0 {
						continue;
					}

					new_map.insert((line_minus_one + 1).try_into().unwrap(), *hits - 1);
				}

				let path = PathBuf::from(file_name);
				(path.clone(), path, CovResult {
					lines: new_map,
					branches: BTreeMap::default(),
					functions: FunctionMap::default()
				})
			})
			.collect();

		let output_path = Path::new(&self.output_file_name);
		let mut path_buf = output_path.to_path_buf();
		if path_buf.pop() {
			create_dir_all(path_buf)?;
		}

		// reee wtf mozilla, what is this shitty rust?
		let result = catch_unwind(|| {
			output_cobertura(None, &result_tuples, Some(output_path), false, true);
		});

		if result.is_err() {
			// bruh what do we even do
			return Err(Error::last_os_error());
		}

		Ok(())
	}
}
