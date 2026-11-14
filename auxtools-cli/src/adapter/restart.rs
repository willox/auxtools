//! Restart by rebooting in place, preserving the port and client connections.
//! Relaunch in a new process if configuration changed or the debugger
//! disconnected.

use super::*;

/// Time allowed for a requested reboot before its progress ends.
const REBOOT_PATIENCE: Duration = Duration::from_secs(60);

/// Current client launch configuration for `restart`.
#[derive(Deserialize)]
pub(super) struct RestartArguments {
	arguments: Option<Value>
}

impl Adapter {
	/// Reboot if launch settings are unchanged and the debugger is connected;
	/// otherwise stop and relaunch the process.
	pub(super) fn restart(&mut self, args: RestartArguments) -> Handled {
		if self.ending {
			return Err("the world is already stopping".to_owned());
		}
		let Some(launched) = self.launched.clone() else {
			// Attached sessions can reboot but cannot relaunch the process.
			return self.reboot();
		};
		let wanted = match args.arguments {
			Some(config) => arguments::<LaunchArguments>(Some(config))?,
			None => launched.clone()
		};
		if wanted == launched && self.debuggee.is_some() && !self.unavailable {
			return self.reboot();
		}
		// Validate the replacement configuration before stopping the world.
		let prepared = Prepared::of(&wanted)?;
		self.relaunching = Some((wanted, prepared));
		self.progress_start(RESTARTING, "Restarting the world".to_owned());
		self.terminate()
	}

	/// Call world.Reboot() in the current process. Clients reconnect
	/// automatically.
	pub(super) fn reboot(&mut self) -> Handled {
		if self.ending {
			return Err("the world is already stopping".to_owned());
		}
		// Reboot closes the connection without responding.
		self.debuggee()?.eval_unanswered("world.Reboot()".to_owned())?;
		self.rebooting = true;
		self.out.console("Rebooting the world");
		self.progress_start(RESTARTING, "Rebooting the world".to_owned());
		// The world's Reboot() may decline or delay.
		let (inputs, generation) = (self.inputs.clone(), self.debuggee()?.generation);
		std::thread::spawn(move || {
			std::thread::sleep(REBOOT_PATIENCE);
			let _ = inputs.send(Input::RebootOverdue { generation });
		});
		Ok(Value::Null)
	}

	/// End reboot progress if connection `generation` is still open. Report
	/// later disconnections normally.
	pub(super) fn reboot_overdue(&mut self, generation: u64) {
		if self.rebooting && self.debuggee.as_ref().is_some_and(|d| d.generation == generation) {
			self.rebooting = false;
			self.progress_end(RESTARTING);
			self.out.console(format!(
				"The world hasn't rebooted after {} seconds: its world/Reboot() may be waiting, or have declined",
				REBOOT_PATIENCE.as_secs()
			));
		}
	}

	/// Relaunch after the previous process exits.
	pub(super) fn relaunched(&mut self, args: LaunchArguments, prepared: Prepared) {
		if let Some(mut debuggee) = self.debuggee.take() {
			debuggee.gone();
		}
		// DreamSeeker cannot reconnect to a replacement process.
		self.joined.world_ended();
		self.pid = None;
		self.ending = false;
		self.unavailable = false;
		self.shutting_down = false;
		self.waiting_for_code = false;
		self.set.lock().unwrap().clear();
		// Fetch stddef.dm again; the BYOND build may have changed.
		self.stddef = None;
		if let Err(e) = self.start(args, prepared) {
			self.out.console(format!("Could not restart the world: {}", e));
			self.ended(-1);
		}
	}
}
