use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::TryInto;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use auxtools::*;
use dmasm::Instruction;

use rustc_hash::FxHashMap;

use crate::{COVERAGE_TRACKER, cobertura::{output_cobertura, CovResult, ResultTuple}};

pub struct Tracker {
	proc_id_map: Vec<Option<Rc<RefCell<Vec<u64>>>>>, // could make this faster by figuring out a value proc IDs never use and removing the Option<>
	filename_map: HashMap::<String, Rc<RefCell<Vec<u64>>>>
}

#[hook("/proc/enable_code_coverage")]
fn enable_code_coverage() {
	let tracker = Tracker::new();

	unsafe {
		*COVERAGE_TRACKER.get() = Some(tracker);
	}

	Ok(Value::null())
}

#[shutdown]
fn code_coverage_writeout() {
	unsafe {
		if let Some(coverage) = &mut *COVERAGE_TRACKER.get() {
			coverage.finalize();
			*COVERAGE_TRACKER.get() = None;
		}
	}
}

impl Tracker {
	fn new() -> Tracker {
		let mut line_data = HashMap::<String, HashSet<u32>>::new();
		let mut i: u32 = 0;
		loop {
			let proc_option = Proc::from_id(raw_types::procs::ProcId(i));
			if proc_option.is_none() {
				break;
			}

			let proc = proc_option.unwrap();
			i = i + 1;

			let mut current_file_option;
			let bytecode;
			unsafe {
				current_file_option = proc.file_name();
				bytecode = proc.bytecode().to_vec();
			}

			let mut env = crate::disassemble_env::DisassembleEnv;
			let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode[..], &mut env);
			for node in nodes {
				match node {
					dmasm::Node::Instruction(instruction, _) => {
						match instruction {
							Instruction::DbgFile(file) => {
								let string_ref_result = StringRef::from_raw(&file.0);
								match string_ref_result {
									Ok(string_ref) => current_file_option = Some(string_ref),
									Err(_) => current_file_option = None,
								}
							},
							Instruction::DbgLine(line) => {
								if let Some(current_file) = &current_file_option {
									let mut file_name = current_file.to_string();

									// strip quotes
									file_name = file_name[1..file_name.len() - 1].to_string();
									if !file_name.ends_with(".dm") {
										continue;
									}

									match line_data.get_mut(&file_name) {
										Some(existing_set) =>{
											 existing_set.insert(line);
										},
										None => {
											let mut new_set = HashSet::<u32>::new();
											new_set.insert(line);
											line_data.insert(file_name, new_set);
										},
									}
								}
							}
							_ => { }
						}
					},
					_ => { }
				}
			}
		}

		let mut tracker = Tracker {
			proc_id_map: Vec::new(),
			filename_map: HashMap::new()
		};

		for (file_name, executable_lines) in line_data {
			let file_name = file_name.to_string();
			if !file_name.ends_with(".dm") {
				continue;
			}

			let mut hit_map = Vec::<u64>::new();
			for line in executable_lines {
				let i: usize = line.try_into().unwrap();
				if i > hit_map.len() {
					hit_map.resize(i, 0);
				}

				hit_map[i - 1] = 1;
			}

			let hit_map_rc = Rc::new(RefCell::new(hit_map));
			tracker.filename_map.insert(file_name, hit_map_rc);
		}

		tracker
	}

	// returns true if we need to pause
	pub fn process_instruction(&mut self, ctx: &raw_types::procs::ExecutionContext, proc_instance: &raw_types::procs::ProcInstance) {
		if ctx.line == 0 || !ctx.filename.valid() {
			return
		}

		// Fast: Seen this proc before, array access based on ID
		let proc_map_index: usize = proc_instance.proc.0.try_into().unwrap();
		let line: usize = ctx.line.try_into().unwrap();

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
				return;
			}
		}

		// Slow: Need to lookup based on filename and create proc entry
		let mut file_name;
		unsafe {
			// TODO reverse this when not debugging
			file_name = StringRef::from_id(ctx.filename).to_string();
		}

		// strip quotes
		file_name = file_name[1..file_name.len() - 1].to_string();

		// WHY BYOND? WHY
		// Procs, datums, random-ass strings... Just why?
		if !file_name.ends_with(".dm") {
			return;
		}

		if needs_extending {
			self.proc_id_map.resize(proc_map_index + 1, None);
		}

		match self.filename_map.get(&file_name) {
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
				return;
			},
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
				self.filename_map.insert(file_name, hit_map_rc.clone());
				self.proc_id_map[proc_map_index] = Some(hit_map_rc);
			}
		}
	}

	pub fn finalize(&mut self) {
		let result_tuples: Vec<ResultTuple> = self.filename_map.iter().map(|(file_name, hit_map)|{
			let mut new_map = BTreeMap::<u32, u64>::new();
			for (line_minus_one, hits) in hit_map.borrow().iter().enumerate() {
				if *hits == 0 {
					continue;
				}

				new_map.insert((line_minus_one + 1).try_into().unwrap(), *hits - 1);
			}

			let path = PathBuf::from(file_name);
			(
				path.clone(),
				path,
				CovResult {
					lines: new_map,
					branches: BTreeMap::default(),
					functions: FxHashMap::default(),
				}
			)
		})
		.collect();

		output_cobertura(None, &result_tuples, Some(Path::new("auxtools_coverage.xml")), false);
	}
}
