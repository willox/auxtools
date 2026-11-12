//! Serialize values recursively for `Request::Render` and CLI JSON output.
//! Unlike pause-local variable references, rendered values are self-contained
//! and can be returned while the world is running.

use super::*;

impl Server {
	/// What an expression gives, written out
	pub(super) fn handle_render(&mut self, frame_id: Option<u32>, expression: &str, depth: u32) {
		let rendered = match expression.strip_prefix('#') {
			// Debugger commands return text and usually do not evaluate code.
			// Do not attach warnings left over from another evaluation.
			Some(command) => {
				self.eval_callee_errors.clear();
				self.eval_callee_more = 0;
				Ok(Rendered::Text(self.handle_command(frame_id, command)))
			}
			None => self.eval_expr(frame_id, expression).map(|value| Self::render(&value, depth))
		};
		self.send_callee_errors();
		self.send_or_disconnect(Response::Render(rendered));
	}

	pub(super) fn handle_render_variables(&mut self, vars: VariablesRef, depth: u32) {
		let rendered = self
			.values_in(vars)
			.map(|values| values.iter().map(|(name, value)| (name.clone(), Self::render(value, depth))).collect());
		self.send_or_disconnect(Response::RenderVariables(rendered));
	}

	/// The values `vars` opens to, by name, as `Variables` lists them
	fn values_in(&self, vars: VariablesRef) -> Result<Vec<(String, Value)>, String> {
		let container = self
			.state
			.as_ref()
			.and_then(|state| state.get_variables(vars))
			.ok_or("variable reference expired after execution resumed")?;
		let gone = || "frame no longer exists".to_owned();
		match container {
			Variables::Arguments { frame } => {
				let frame = self.get_stack_frame(frame).ok_or_else(gone)?;
				let mut values = vec![("src".to_owned(), frame.src.clone()), ("usr".to_owned(), frame.usr.clone())];
				let mut unnamed = 0;
				for (name, value) in &frame.args {
					let name = match name {
						Some(name) => String::from(name),
						None => {
							unnamed += 1;
							format!("undefined argument #{}", unnamed)
						}
					};
					values.push((name, value.clone()));
				}
				Ok(values)
			}
			Variables::Locals { frame } => {
				let frame = self.get_stack_frame(frame).ok_or_else(gone)?;
				let mut values = vec![(".".to_owned(), frame.dot.clone())];
				values.extend(
					Self::visible_locals(&frame)
						.into_iter()
						.zip(&frame.locals)
						.filter_map(|(name, (_, value))| Some((name?, value.clone())))
				);
				Ok(values)
			}
			Variables::Globals | Variables::Builtins => {
				let builtin = matches!(container, Variables::Builtins);
				let builtins = Self::builtin_constants();
				let names = List::from_value(&unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) }).map_err(|e| e.message)?;
				let mut values = vec![];
				for i in 1..=names.len() {
					let name = names.get(i).and_then(|name| name.as_string()).map_err(|e| e.message)?;
					if builtins.contains(&name) != builtin {
						continue;
					}
					let value = StringRef::new(name.as_str())
						.and_then(|var| Value::GLOBAL.get(var))
						.map_err(|e| e.message)?;
					values.push((name, value));
				}
				values.sort_by_key(|(name, _)| name.to_lowercase());
				Ok(values)
			}
			Variables::ObjectVars(object, _) => Ok(Self::object_values(&object)),
			_ => Err("rendering supports frame variables, globals and object variables".to_owned())
		}
	}

	/// Serialize object variables, excluding the recursive `vars` member.
	fn object_values(object: &Value) -> Vec<(String, Value)> {
		let names: Vec<String> = match Self::primitive_fields(object) {
			Some(fields) => fields.iter().map(|&name| name.to_owned()).collect(),
			None => {
				let Ok(vars) = object.get(byond_string!("vars")).and_then(|vars| List::from_value(&vars)) else {
					return vec![];
				};
				let mut names: Vec<String> = (1..=vars.len())
					.filter_map(|i| vars.get(i).and_then(|name| name.as_string()).ok())
					.collect();
				names.sort_by_key(|name| name.to_lowercase());
				names
			}
		};
		names
			.into_iter()
			.filter(|name| name != "vars")
			.filter_map(|name| {
				let value = StringRef::new(name.as_str()).and_then(|var| object.get(var)).ok()?;
				Some((name, value))
			})
			.collect()
	}

	/// Serialize a value and its children to `depth`.
	pub(super) fn render(value: &Value, depth: u32) -> Rendered {
		match value.raw.tag {
			ValueTag::Null => return Rendered::Null,
			ValueTag::Number => return Rendered::Number(unsafe { value.raw.data.number }),
			ValueTag::String => return Rendered::Text(value.as_string().unwrap_or_default()),
			_ => {}
		}

		if List::is_list(value) {
			let Ok(list) = List::from_value(value) else {
				return Rendered::Other {
					kind: "/list".to_owned(),
					text: Self::stringify(value)
				};
			};
			let entries = (depth > 0).then(|| Self::entries_of(&list, depth - 1));
			return Rendered::List { len: list.len(), entries };
		}

		if Self::is_object(value) {
			let name = value.get(byond_string!("name")).ok().and_then(|name| name.as_string().ok());
			let vars = (depth > 1).then(|| {
				Self::object_values(value)
					.iter()
					.map(|(name, value)| (name.clone(), Self::render(value, depth - 1)))
					.collect()
			});
			return Rendered::Object {
				reference: Self::reference(value),
				kind: Self::kind(value).unwrap_or_default(),
				name,
				vars
			};
		}

		Rendered::Other {
			kind: Self::kind(value).unwrap_or_default(),
			text: Self::stringify(value)
		}
	}

	/// Serialize list entries and associated values to `depth`.
	fn entries_of(list: &List, depth: u32) -> Vec<(Rendered, Option<Rendered>)> {
		if list.is_alist() {
			return list
				.alist_pairs()
				.unwrap_or_default()
				.iter()
				.map(|(key, value)| (Self::render(key, depth), Some(Self::render(value, depth))))
				.collect();
		}
		let mut entries = vec![];
		for i in 1..=list.len() {
			let Ok(key) = list.get(i) else {
				continue;
			};
			let value = (key.raw.tag != ValueTag::Number)
				.then(|| list.get(&key).ok())
				.flatten()
				.filter(|value| value.raw.tag != ValueTag::Null)
				.map(|value| Self::render(&value, depth));
			entries.push((Self::render(&key, depth), value));
		}
		entries
	}

	/// What `ref()` gives for a value: its kind and id, in hex
	pub(super) fn reference(value: &Value) -> String {
		let id = unsafe { value.raw.data.id };
		format!("[0x{:x}]", (value.raw.tag as u32) << 24 | id)
	}
}
