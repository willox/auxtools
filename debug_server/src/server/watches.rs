//! Detect data breakpoint changes between instructions.
//!
//! Polling covers assignments, list operations, pointers, engine updates and
//! byondapi writes without separate hooks. Stop after the instruction that
//! changed the value; writing the same value does not count.
//!
//! Object fields, list entries and globals use BYOND pointers. Frame locals
//! and arguments use slots in a specific proc instance and are checked when
//! that frame runs. Writes through exported local pointers are detected when
//! the owning frame next executes.

use auxtools::raw_types::values::Value as RawValue;

use super::*;

/// Active data breakpoint.
pub(super) struct Watched {
	id: u32,
	client: ClientId,
	/// What the client knows it as
	name: String,
	place: Place,
	/// Last observed value.
	last: Value,
	condition: Option<String>,
	hit: Option<HitCondition>,
	hits: u32,
	/// Whether a condition error has already been reported.
	told: bool
}

enum Place {
	/// An object's var, a list's entry, a global: BYOND's pointer to it
	Pointer(Value),
	/// A proc's own var, in one frame
	Slot {
		/// Stable BYOND proc instance ID. Addresses change on first sleep;
		/// spawned copies must not share the watch.
		instance: u32,
		/// Proc ID used to detect instance ID reuse after return.
		proc: raw_types::procs::ProcId,
		slot: Slot
	}
}

/// A proc instance's id
unsafe fn instance_id(instance: *mut raw_types::procs::ProcInstance) -> u32 {
	unsafe { (*instance).argslist_idx.id }
}

#[derive(Clone, Copy)]
enum Slot {
	Local(usize),
	Argument(usize),
	Dot
}

/// Condition bindings for the current and previous values.
const CONDITION_NEW: &str = "value";
const CONDITION_OLD: &str = "old";

fn same(a: RawValue, b: RawValue) -> bool {
	a.tag == b.tag && unsafe { a.data.id == b.data.id }
}

/// What's in a frame's slot, if it has one. `ctx` is the frame's own.
unsafe fn read_slot(ctx: *mut raw_types::procs::ExecutionContext, slot: Slot) -> Option<RawValue> {
	unsafe {
		match slot {
			Slot::Local(index) => (index < *(*ctx).locals_count() as usize).then(|| *(*(*ctx).locals()).add(index)),
			Slot::Argument(index) => {
				let instance = *(*ctx).proc_instance();
				(index < (*instance).args_count() as usize).then(|| *(*instance).args().add(index))
			}
			Slot::Dot => Some(*(*ctx).dot())
		}
	}
}

impl Server {
	pub fn has_watches(&self) -> bool {
		!self.watches.is_empty()
	}

	/// Check watches before the instruction at `ctx`. Return changes passing
	/// conditions and hit filters; expire watches whose targets are gone.
	pub fn watched_changes(&mut self, ctx: *mut raw_types::procs::ExecutionContext) -> Option<Vec<DataChange>> {
		let instance = unsafe { *(*ctx).proc_instance() };
		let mut changed: Vec<(usize, RawValue)> = vec![];
		let mut over: Vec<(u32, &'static str)> = vec![];

		// Pointer-read errors from deleted targets belong to watch handling,
		// not the world's runtime error breakpoints.
		self.in_eval = true;
		for (index, watch) in self.watches.iter().enumerate() {
			match &watch.place {
				Place::Pointer(pointer) => match auxtools::pointer::read_raw(pointer) {
					Some(now) => {
						if !same(now, watch.last.raw) {
							changed.push((index, now));
						}
					}
					None => over.push((watch.id, "watch target no longer exists"))
				},

				Place::Slot {
					instance: watched,
					proc,
					slot
				} => {
					if unsafe { instance_id(instance) } != *watched {
						continue;
					}
					unsafe {
						// The original frame ended between checks; this ID now
						// belongs to another call.
						if (*instance).proc != *proc || *(*ctx).bytecode_offset() == 0 {
							over.push((watch.id, "watched proc ended"));
							continue;
						}
						match read_slot(ctx, *slot) {
							Some(now) => {
								if !same(now, watch.last.raw) {
									changed.push((index, now));
								}
							}
							None => {
								over.push((watch.id, "watched proc ended"));
								continue;
							}
						}
						// Its last instruction: nothing changes it after this
						let opcode = *(*(*ctx).bytecode()).add(*(*ctx).bytecode_offset() as usize);
						if opcode == dmasm::Opcode::Ret.word() || opcode == dmasm::Opcode::End.word() {
							over.push((watch.id, "watched proc returned"));
						}
					}
				}
			}
		}
		self.in_eval = false;
		self.eval_error = None;

		if changed.is_empty() && over.is_empty() {
			return None;
		}

		// What was there, and what is now
		let mut changes = vec![];
		for (index, now) in changed {
			let watch = &mut self.watches[index];
			let now = unsafe { Value::from_raw(now) };
			let before = std::mem::replace(&mut watch.last, now.clone());
			changes.push((watch.id, before, now));
		}
		for (id, reason) in over {
			self.end_watch(id, reason);
		}

		// Conditions evaluate in the running frame and need its stack snapshot.
		let paused = self.state.is_some();
		if !paused {
			self.state = Some(State::new());
		}
		let mut wanted = vec![];
		for (id, before, now) in changes {
			let Some(watch) = self.watches.iter().find(|watch| watch.id == id) else {
				continue;
			};
			let (name, condition) = (watch.name.clone(), watch.condition.clone());
			if let Some(condition) = condition {
				self.eval_given = vec![(CONDITION_NEW.to_owned(), now.clone()), (CONDITION_OLD.to_owned(), before.clone())];
				let passed = self.eval_expr(Some(0), &condition).map(|result| result.is_truthy());
				if let Some(state) = self.state.as_mut() {
					state.invalidate_stacks();
				}
				match passed {
					Ok(true) => {}
					Ok(false) => continue,
					// Skip failed conditions and report the first error.
					Err(error) => {
						let Some(watch) = self.watches.iter_mut().find(|watch| watch.id == id) else {
							continue;
						};
						if !std::mem::replace(&mut watch.told, true) {
							let client = watch.client;
							let message = format!(
								"The condition ({}) of the watch on {} failed; skipping affected changes (reported once): {}",
								condition, name, error
							);
							self.tell(client, Response::ConditionFailed { message });
						}
						continue;
					}
				}
			}
			// The condition's code could have taken the watch away
			let Some(watch) = self.watches.iter_mut().find(|watch| watch.id == id) else {
				continue;
			};
			watch.hits += 1;
			if watch.hit.is_some_and(|hit| !hit.takes(watch.hits)) {
				continue;
			}
			wanted.push(DataChange {
				id,
				name,
				old: Self::stringify(&before),
				new: Self::stringify(&now)
			});
		}
		if !paused {
			self.state = None;
		}
		(!wanted.is_empty()).then_some(wanted)
	}

	/// Reset baselines after debugger evaluations and assignments so their
	/// side effects do not trigger data breakpoints.
	pub(super) fn refresh_watches(&mut self) {
		if self.watches.is_empty() {
			return;
		}
		// The frames this pause has read, which are still there
		let frames: Vec<(u32, *mut raw_types::procs::ExecutionContext)> = match &self.state {
			Some(state) => {
				let asleep = state.suspended.get().map(|stacks| stacks.iter().flatten());
				state
					.active()
					.iter()
					.chain(asleep.into_iter().flatten())
					.map(|frame| (unsafe { instance_id(frame.instance) }, frame.context))
					.collect()
			}
			None => vec![]
		};
		let was_in_eval = std::mem::replace(&mut self.in_eval, true);
		for watch in &mut self.watches {
			let now = match &watch.place {
				Place::Pointer(pointer) => auxtools::pointer::read_raw(pointer),
				Place::Slot { instance, slot, .. } => frames
					.iter()
					.find(|(frame, _)| frame == instance)
					.and_then(|(_, ctx)| unsafe { read_slot(*ctx, *slot) })
			};
			if let Some(now) = now {
				watch.last = unsafe { Value::from_raw(now) };
			}
		}
		self.in_eval = was_in_eval;
	}

	/// A watch is over: its client is told why
	fn end_watch(&mut self, id: u32, reason: &str) {
		let Some(at) = self.watches.iter().position(|watch| watch.id == id) else {
			return;
		};
		let watch = self.watches.remove(at);
		let ended = Response::WatchEnded {
			id,
			reason: reason.to_owned()
		};
		if let Some(to) = self.clients.get_mut(&watch.client) {
			if !Self::deliver(to, &ended) {
				self.remove_client(watch.client);
			}
		}
	}

	pub(super) fn forget_watches_of(&mut self, client: ClientId) {
		self.watches.retain(|watch| watch.client != client);
	}

	/// Evaluate `&(expression)` to resolve a watch pointer.
	fn pointer_to(&mut self, frame_id: Option<u32>, expression: &str, given: Vec<(String, Value)>) -> Result<Value, String> {
		if !auxtools::pointer::readable() {
			return Err("pointer reads are unavailable in this BYOND build".to_owned());
		}
		self.eval_given = given;
		let pointer = self.eval_expr(frame_id, &format!("&({})", expression));
		if let Some(state) = self.state.as_mut() {
			state.invalidate_stacks();
		}
		let pointer = pointer?;
		// Pointers to evaluation-local variables expire when evaluation
		// returns.
		let names_something = pointer.raw.tag == ValueTag::Pointer && pointer.to_string().is_ok_and(|text| !text.is_empty());
		if !names_something || auxtools::pointer::read_raw(&pointer).is_none() {
			return Err("expression has no persistent watch target; select an object variable, list entry or global".to_owned());
		}
		Ok(pointer)
	}

	/// A frame's own var by name: its slot, if it has one of that name
	fn slot_of(frame: &debug::StackFrame, name: &str) -> Result<Option<Slot>, String> {
		if name == "." {
			return Ok(Some(Slot::Dot));
		}
		let mut locals = frame.locals.iter().enumerate().filter(|(_, (local, _))| String::from(local) == name);
		match (locals.next(), locals.next()) {
			(Some((index, _)), None) => return Ok(Some(Slot::Local(index))),
			(Some(_), Some(_)) => return Err(format!("the proc has more than one local called {}", name)),
			_ => {}
		}
		let argument = frame
			.args
			.iter()
			.position(|(argument, _)| argument.as_ref().is_some_and(|argument| String::from(argument) == name));
		Ok(argument.map(Slot::Argument))
	}

	fn slot_place(frame: &debug::StackFrame, slot: Slot) -> Result<(Place, Value), String> {
		let now = unsafe { read_slot(frame.context, slot) }.ok_or("the frame has no such var")?;
		let place = Place::Slot {
			instance: unsafe { instance_id(frame.instance) },
			proc: frame.proc.id,
			slot
		};
		Ok((place, unsafe { Value::from_raw(now) }))
	}

	/// Resolve a watch target and read its current value.
	fn watched_place(&mut self, place: &WatchPlace) -> Result<(Place, String, Value), String> {
		const ITS_VARS: &str = "select a variable of this value to watch";
		let pointed = |pointer: Value, name: String| -> Result<(Place, String, Value), String> {
			let now = auxtools::pointer::read(&pointer).ok_or("it can't be read")?;
			Ok((Place::Pointer(pointer), name, now))
		};

		match place {
			WatchPlace::Expression { frame_id, expression } => {
				let expression = expression.trim();
				// A name alone may be the frame's own var
				let plain = expression == "." || (!expression.is_empty() && expression.chars().all(|c| c.is_alphanumeric() || c == '_'));
				if let (true, Some(frame_id)) = (plain, frame_id) {
					if expression == "src" || expression == "usr" {
						return Err(ITS_VARS.to_owned());
					}
					let frame = self.get_stack_frame(*frame_id).ok_or("frame no longer exists")?;
					if let Some(slot) = Self::slot_of(frame, expression)? {
						let (place, now) = Self::slot_place(frame, slot)?;
						return Ok((place, expression.to_owned(), now));
					}
				}
				let pointer = self.pointer_to(*frame_id, expression, vec![])?;
				pointed(pointer, expression.to_owned())
			}

			WatchPlace::Variable { vars, name } => {
				let container = self
					.state
					.as_ref()
					.and_then(|state| state.get_variables(vars.clone()))
					.ok_or("variable reference expired after execution resumed")?;
				// What's inside something is reached through it, by what
				// it's called in these expressions
				const HOLDER: &str = "__watched";
				const KEY: &str = "__key";
				let within = |holder: &Option<String>, rest: String| match holder {
					Some(holder) => format!("{}{}", holder, rest),
					None => name.clone()
				};
				match container {
					Variables::Locals { frame } | Variables::Arguments { frame } => {
						if name == "src" || name == "usr" {
							return Err(ITS_VARS.to_owned());
						}
						let frame = self.get_stack_frame(frame).ok_or("frame no longer exists")?;
						let slot = Self::slot_of(frame, name)?.ok_or_else(|| format!("the frame has no var called {}", name))?;
						let (place, now) = Self::slot_place(frame, slot)?;
						Ok((place, name.clone(), now))
					}
					Variables::Globals => {
						let pointer = self.pointer_to(None, &format!("global.{}", name), vec![])?;
						pointed(pointer, name.clone())
					}
					Variables::Builtins => Err("BYOND's constants don't change".to_owned()),
					Variables::ObjectVars(object, holder) => {
						let pointer = self.pointer_to(None, &format!("{}.{}", HOLDER, name), vec![(HOLDER.to_owned(), object)])?;
						pointed(pointer, within(&holder, format!(".{}", name)))
					}
					Variables::ListContents(list, holder) => {
						let number = Self::entry_number(name).ok_or_else(|| format!("the list has no entry {}", name))?;
						// An entry with a value for its key: the value is
						// what changes
						let runtime = |e: Runtime| format!("runtime error: {}", e.message);
						let entries = List::from_value(&list).map_err(runtime)?;
						let key = if entries.is_alist() {
							let (key, _) = entries
								.alist_pairs()
								.map_err(runtime)?
								.into_iter()
								.nth(number as usize - 1)
								.ok_or("the list has changed")?;
							Some(key)
						} else {
							let key = entries.get(number).map_err(runtime)?;
							let has_value = key.raw.tag != ValueTag::Number && entries.get(&key).is_ok_and(|of_key| of_key.raw.tag != ValueTag::Null);
							has_value.then_some(key)
						};
						let pointer = match key {
							Some(key) => self.pointer_to(None, &format!("{}[{}]", HOLDER, KEY), vec![
								(HOLDER.to_owned(), list),
								(KEY.to_owned(), key),
							])?,
							None => self.pointer_to(None, &format!("{}[{}]", HOLDER, number), vec![(HOLDER.to_owned(), list)])?
						};
						pointed(pointer, within(&holder, name.clone()))
					}
					Variables::ListPair { key, of, list, .. } => {
						if name != "value" {
							return Err("list keys cannot be watched for value changes".to_owned());
						}
						let shown = list.zip(Self::literal(&key)).map(|(list, key)| format!("{}[{}]", list, key));
						let pointer = self.pointer_to(None, &format!("{}[{}]", HOLDER, KEY), vec![
							(HOLDER.to_owned(), of),
							(KEY.to_owned(), key),
						])?;
						pointed(pointer, shown.unwrap_or_else(|| name.clone()))
					}
					// What a pointer names is the place itself
					Variables::Pointee(pointer, holder) => pointed(pointer, within(&holder, String::new()).trim_start_matches('*').to_owned())
				}
			}
		}
	}

	/// Describe a watch target for the client.
	pub(super) fn watch_info(&mut self, place: &WatchPlace) -> Result<String, String> {
		self.watched_place(place).map(|(_, name, _)| name)
	}

	pub(super) fn watch_set(&mut self, place: &WatchPlace, condition: Option<String>, hit: Option<HitCondition>) -> Result<Watch, String> {
		let client = self.current.ok_or("nobody asked")?;
		let (place, name, now) = self.watched_place(place)?;
		self.next_watch += 1;
		let watch = Watch {
			id: self.next_watch,
			name: name.clone(),
			value: Self::stringify(&now)
		};
		self.watches.push(Watched {
			id: watch.id,
			client,
			name,
			place,
			last: now,
			condition,
			hit,
			hits: 0,
			told: false
		});
		Ok(watch)
	}

	pub(super) fn watch_unset(&mut self, id: u32) {
		let client = self.current;
		self.watches.retain(|watch| watch.id != id || Some(watch.client) != client);
	}
}
