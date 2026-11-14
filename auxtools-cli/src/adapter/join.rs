//! World addresses and DreamSeeker connections.

use std::process::{Child, Command, Stdio};

use super::*;

/// World-ready event, sent for each new process (`started`) and reboot.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorldEvent {
	/// Join address, if the world has a listening port.
	address: Option<String>,
	/// New process, rather than a reboot.
	started: bool,
	/// Whether `auxtoolsJoin` can open DreamSeeker on this platform.
	can_join: bool,
	/// Whether DreamSeeker hosts this world.
	hosted: bool
}

impl Event for WorldEvent {
	const EVENT: &'static str = "auxtoolsWorld";
}

/// Arguments for `auxtoolsJoin`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JoinArguments {
	/// Open DreamSeeker; otherwise return only the address.
	#[serde(default = "yes")]
	open: bool,
	/// Close the opened DreamSeeker process when the world ends.
	#[serde(default)]
	close_with_world: bool
}

/// DreamSeeker processes opened by `auxtoolsJoin`.
#[derive(Default)]
pub(super) struct Joined {
	/// Processes and their close-with-world flags.
	clients: Vec<(Child, bool)>,
	/// Last process announced by `WorldEvent`.
	announced: Option<u32>
}

impl Joined {
	/// Close tracked DreamSeeker processes whose close-with-world flag is set.
	pub(super) fn world_ended(&mut self) {
		for (mut client, close) in std::mem::take(&mut self.clients) {
			if close {
				let _ = client.kill();
			}
			let _ = client.try_wait();
		}
	}
}

impl Adapter {
	/// Join address, or none if the world has no listening port.
	fn address(&mut self) -> Result<Option<String>, String> {
		let port = match self.request(&AuxRequest::WorldInfo)? {
			AuxResponse::WorldInfo { port, .. } => port,
			other => return Err(unexpected(other))
		};
		Ok((port != 0).then(|| format!("byond://127.0.0.1:{}", port)))
	}

	fn hosted(&self) -> bool {
		self.launched
			.as_ref()
			.is_some_and(|launched| launched.server == Some(Server::DreamSeeker))
	}

	/// Report the world's address and whether this is a new process.
	pub(super) fn announce_world(&mut self) {
		let process = self.world_process();
		let started = process != self.joined.announced;
		self.joined.announced = process;
		let Ok(address) = self.address() else {
			return;
		};
		let hosted = self.hosted();
		self.out.event(WorldEvent {
			can_join: cfg!(windows) && address.is_some(),
			address,
			started,
			hosted
		});
	}

	/// Open DreamSeeker or return the address on platforms without it.
	pub(super) fn join(&mut self, args: JoinArguments) -> Handled {
		let address = self
			.address()?
			.ok_or("world has no listening port; set \"port\" in the launch configuration")?;
		if !args.open || !cfg!(windows) {
			return Ok(serde_json::json!({ "address": address, "opened": false }));
		}
		let byond = self.launched.as_ref().and_then(|launched| launched.byond_path.clone());
		let program = byond::bin_dir(byond.as_deref())?.join(Server::DreamSeeker.program());
		let client = Command::new(&program)
			.arg(&address)
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.map_err(|e| format!("couldn't start {}: {}", program.display(), e))?;
		self.joined.clients.retain_mut(|(client, _)| matches!(client.try_wait(), Ok(None)));
		self.joined.clients.push((client, args.close_with_world));
		Ok(serde_json::json!({ "address": address, "opened": true }))
	}
}
