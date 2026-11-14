//! Handle DAP data breakpoints. `dataBreakpointInfo` resolves watch targets;
//! `setDataBreakpoints` replaces the client's breakpoint list.

use super::*;

/// Where the records of the data breakpoints set in the world are kept
pub(super) const DATA_BREAKPOINTS: &str = "<data>";

#[derive(Default)]
pub(super) struct DataBreakpoints {
	/// Watch targets indexed by client-visible data ID. Frame and variable
	/// references also record the pause in which they were resolved.
	places: HashMap<String, (aux::WatchPlace, String, Option<u32>)>,
	asked: u32,
	/// Current client data breakpoints.
	set: Vec<DataBreakpoint>
}

struct DataBreakpoint {
	data_id: String,
	condition: Option<String>,
	hit: Option<String>,
	/// Client-visible ID.
	id: i64,
	/// Description of the watch target.
	name: String,
	/// Active server watch, if set.
	watch: Option<u32>,
	/// Reason the watch is not set.
	message: Option<String>
}

impl DataBreakpoint {
	fn shown(&self) -> Breakpoint {
		Breakpoint {
			id: Some(self.id),
			verified: self.watch.is_some(),
			message: self.message.clone(),
			..Default::default()
		}
	}
}

/// Arguments for `dataBreakpointInfo`, which dap-types does not define.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DataBreakpointInfoArguments {
	variables_reference: Option<i64>,
	name: String,
	frame_id: Option<i64>
}

#[derive(Deserialize)]
pub(super) struct SetDataBreakpointsArguments {
	breakpoints: Vec<DataBreakpointWanted>
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DataBreakpointWanted {
	data_id: String,
	condition: Option<String>,
	hit_condition: Option<String>
}

impl Adapter {
	/// Resolve a variable or expression to a watch target.
	/// Frame and object references are valid only during the current pause.
	pub(super) fn data_breakpoint_info(&mut self, args: DataBreakpointInfoArguments) -> Handled {
		let unwatched = |why: &str| Ok(serde_json::json!({ "dataId": null, "description": why }));
		if !self.debugging {
			return unwatched("debugging is disabled");
		}
		if self.debuggee.is_none() {
			return unwatched(self.not_connected());
		}
		let place = match args.variables_reference.filter(|&reference| reference > 0) {
			Some(reference) => aux::WatchPlace::Variable {
				vars: aux::VariablesRef(reference as i32),
				name: args.name
			},
			None => aux::WatchPlace::Expression {
				frame_id: args.frame_id.map(|id| id as u32),
				expression: args.name
			}
		};
		match self.request(&AuxRequest::WatchInfo { place: place.clone() })? {
			AuxResponse::WatchInfo(Ok(name)) => {
				self.data.asked += 1;
				let data_id = format!("watch {}: {}", self.data.asked, name);
				let of_stop = match &place {
					aux::WatchPlace::Expression { frame_id: None, .. } => None,
					_ => self.debuggee.as_ref().map(Debuggee::stop_number)
				};
				self.data.places.insert(data_id.clone(), (place, name.clone(), of_stop));
				Ok(serde_json::json!({
					"dataId": data_id,
					"description": name,
					// Only value changes are detected, not reads or writes of the same value.
					"accessTypes": ["write"],
					"canPersist": false
				}))
			}
			AuxResponse::WatchInfo(Err(why)) => unwatched(&why),
			other => Err(unexpected(other))
		}
	}

	/// Replace the client's data breakpoints. Preserve existing watches,
	/// including frame targets that can no longer be resolved.
	pub(super) fn set_data_breakpoints(&mut self, args: SetDataBreakpointsArguments) -> Handled {
		let mut had = std::mem::take(&mut self.data.set);
		let mut set = vec![];
		for wanted in args.breakpoints {
			let same = |b: &DataBreakpoint| b.data_id == wanted.data_id && b.condition == wanted.condition && b.hit == wanted.hit_condition;
			if let Some(at) = had.iter().position(same) {
				set.push(had.remove(at));
				continue;
			}

			let id = self.next_breakpoint_id;
			self.next_breakpoint_id += 1;
			let known = self.data.places.get(&wanted.data_id).cloned();
			let stop = self.debuggee.as_ref().map(Debuggee::stop_number);
			const OVER: &str = "watch target expired; select it again";
			let watched: Result<aux::Watch, String> = match &known {
				_ if !self.debugging => Err("debugging is disabled".to_owned()),
				_ if self.debuggee.is_none() => Err(self.not_connected().to_owned()),
				None => Err(OVER.to_owned()),
				Some((_, _, Some(of_stop))) if Some(*of_stop) != stop || !self.debuggee.as_ref().is_some_and(Debuggee::stopped) => {
					Err(OVER.to_owned())
				}
				Some((place, ..)) => hit_condition(wanted.hit_condition.as_deref()).and_then(|hit| {
					match self.request(&AuxRequest::WatchSet {
						place: place.clone(),
						condition: wanted.condition.clone().filter(|condition| !condition.trim().is_empty()),
						hit
					})? {
						AuxResponse::WatchSet(result) => result,
						other => Err(unexpected(other))
					}
				})
			};
			let name = known.map_or_else(|| wanted.data_id.clone(), |(_, name, _)| name);
			set.push(match watched {
				Ok(watch) => DataBreakpoint {
					data_id: wanted.data_id,
					condition: wanted.condition,
					hit: wanted.hit_condition,
					id,
					name: watch.name,
					watch: Some(watch.id),
					message: None
				},
				Err(why) => DataBreakpoint {
					data_id: wanted.data_id,
					condition: wanted.condition,
					hit: wanted.hit_condition,
					id,
					name,
					watch: None,
					message: Some(why)
				}
			});
		}
		for gone in had {
			if let Some(id) = gone.watch {
				let _ = self.request(&AuxRequest::WatchUnset { id });
			}
		}
		let breakpoints: Vec<Breakpoint> = set.iter().map(DataBreakpoint::shown).collect();
		self.data.set = set;
		self.record_data_breakpoints();
		json(SetDataBreakpointsResponse { breakpoints })
	}

	/// Translate server watch IDs to client breakpoint IDs.
	fn record_data_breakpoints(&mut self) {
		let records = self
			.data
			.set
			.iter()
			.filter(|breakpoint| breakpoint.watch.is_some())
			.map(|breakpoint| SetBreakpoint {
				asked: InstructionRef {
					proc: aux::ProcRef {
						path: String::new(),
						override_id: 0
					},
					offset: 0
				},
				at: None,
				watch: breakpoint.watch,
				id: breakpoint.id
			})
			.collect();
		self.set.lock().unwrap().insert(DATA_BREAKPOINTS.to_owned(), records);
	}

	/// Invalidate a watch after its proc returns or object is deleted.
	pub(super) fn watch_ended(&mut self, watch: u32, reason: &str) {
		let Some(breakpoint) = self.data.set.iter_mut().find(|breakpoint| breakpoint.watch == Some(watch)) else {
			return;
		};
		breakpoint.watch = None;
		breakpoint.message = Some(reason.to_owned());
		self.out.console(format!("No longer watching {}: {}", breakpoint.name, reason));
		self.out.event(dap::BreakpointEvent {
			reason: "changed".to_owned(),
			breakpoint: breakpoint.shown()
		});
		self.record_data_breakpoints();
	}

	/// Invalidate watches after reboot. Frame and object references cannot
	/// be restored; `rewatch` restores targets that can be resolved globally.
	pub(super) fn watches_lost(&mut self) {
		self.data.places.retain(|_, (_, _, of_stop)| of_stop.is_none());
		for breakpoint in &mut self.data.set {
			if breakpoint.watch.take().is_some() && !self.data.places.contains_key(&breakpoint.data_id) {
				breakpoint.message = Some("watch expired after reboot; select it again".to_owned());
				self.out.event(dap::BreakpointEvent {
					reason: "changed".to_owned(),
					breakpoint: breakpoint.shown()
				});
			}
		}
	}

	/// Restore watches whose expressions can be evaluated without a frame.
	pub(super) fn rewatch(&mut self) {
		for at in 0..self.data.set.len() {
			let breakpoint = &self.data.set[at];
			let Some((place, ..)) = self.data.places.get(&breakpoint.data_id).cloned() else {
				continue;
			};
			if breakpoint.watch.is_some() || breakpoint.message.is_some() {
				continue;
			}
			let condition = breakpoint.condition.clone().filter(|condition| !condition.trim().is_empty());
			let watched =
				hit_condition(breakpoint.hit.as_deref()).and_then(|hit| match self.request(&AuxRequest::WatchSet { place, condition, hit })? {
					AuxResponse::WatchSet(result) => result,
					other => Err(unexpected(other))
				});
			let breakpoint = &mut self.data.set[at];
			match watched {
				Ok(watch) => breakpoint.watch = Some(watch.id),
				Err(why) => {
					breakpoint.message = Some(why);
					self.out.event(dap::BreakpointEvent {
						reason: "changed".to_owned(),
						breakpoint: breakpoint.shown()
					});
				}
			}
		}
		self.record_data_breakpoints();
	}
}

/// Build a `setDataBreakpoints` response.
#[derive(serde::Serialize)]
struct SetDataBreakpointsResponse {
	breakpoints: Vec<Breakpoint>
}
