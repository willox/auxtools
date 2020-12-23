use auxtools::raw_types::procs::ProcId;
use auxtools::*;

use std::{collections::HashMap, fs::File, fs::OpenOptions, io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write}, time::{Duration, Instant}};
use serde::{Deserialize, Serialize};


#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
struct Code {
	proc: ProcId,
	offset: u16,
}

pub struct Profiler {
	file: BufWriter<File>,
	delay: Duration,
	last_hit: Instant,
}

impl Profiler {
	pub fn new() -> Self {
		let file = OpenOptions::new()
			.read(true)
			.write(true)
			.create(true)
			.truncate(true)
			.open("E:/profiler.txt")
			.unwrap();

		Self {
			file: BufWriter::with_capacity(128 * 1024, file),
			delay: Duration::from_nanos(250000),
			last_hit: Instant::now(),
		}
	}

	pub fn poll(&mut self, mut ctx: *mut raw_types::procs::ExecutionContext) {
		if self.last_hit.elapsed() < self.delay {
			return;
		}

		let mut frames = vec![];

		unsafe {
			while !ctx.is_null() {
				let instance = (*ctx).proc_instance;
				frames.push(((*instance).proc, (*ctx).bytecode_offset));
				ctx = (*ctx).parent_context;
			}
		}

		let length = frames.len() as u32;

		self.file.write_all(&length.to_le_bytes()).unwrap();

		for (proc, offset) in frames {
			self.file.write_all(&proc.0.to_le_bytes()).unwrap();
			self.file.write_all(&offset.to_le_bytes()).unwrap();
		}

		self.last_hit = Instant::now();
	}

	pub fn collect_symbols(file: &File) -> HashMap<(u32, u16), (String, Option<u32>)> {
		let mut symbols = HashMap::new();
		let mut file = BufReader::new(file);
		file.seek(SeekFrom::Start(0)).unwrap();

		loop {
			let mut count_bytes = [0, 0, 0, 0];
			file.read_exact(&mut count_bytes).unwrap();
			let count = u32::from_le_bytes(count_bytes);

			if count == u32::MAX {
				break;
			}

			for _ in 0..count {
				let mut proc_bytes = [0, 0, 0, 0];
				let mut offset_bytes = [0, 0];

				file.read_exact(&mut proc_bytes).unwrap();
				let proc = u32::from_le_bytes(proc_bytes);

				file.read_exact(&mut offset_bytes).unwrap();
				let offset = u16::from_le_bytes(offset_bytes);

				symbols.entry((proc, offset)).or_insert_with(|| {
					let proc = Proc::from_id(raw_types::procs::ProcId(proc)).unwrap();
					let line = proc.line_number(offset as u32);
					(proc.path, line)
				});
			}
		}

		symbols
	}

	pub fn finish(mut self) {
		// Finish our treace stream
		self.file.write_all(&u32::MAX.to_le_bytes()).unwrap();

		let mut file = self.file.into_inner().unwrap();
		let symbols = Self::collect_symbols(&file);
		let symbols = bincode::serialize(&symbols).unwrap();
		file.seek(SeekFrom::End(0)).unwrap();
		file.write_all(&symbols).unwrap();
	}

	pub fn results(&mut self) {
		/*
		let mut deduplicated: HashMap<Code, u32> = HashMap::new();

		for item in &self.hits {
			*deduplicated.entry(*item).or_insert(0) += 1;
		}

		let mut vec: Vec<(Code, u32)> = deduplicated.into_iter().collect();
		vec.sort_by_key(|(_, count)| *count);

		for (code, _) in vec.iter().rev() {
			let proc = Proc::from_id(code.proc).unwrap();
			writeln!(&mut self.buffer, "{} @ {:?}", proc.path, proc.line_number(code.offset as u32)).unwrap();
		}
		*/
		//self.buffer.flush();
	}
}
