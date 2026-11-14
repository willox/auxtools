//! Stop or reboot a world by evaluating `world.Del()` or `world.Reboot()`.
//! Successful calls close the connection without a response. Wait for process
//! exit or reconnection to confirm completion.

use std::time::{Duration, Instant};

use auxtools_cli::{
	local,
	server_types::{Reconnected, Request, Response}
};
use serde_json::json;

use super::{args, seconds_text, Failure, Result, World, HUNG_UP, TIMED_OUT};

/// How often to look again
const INTERVAL: Duration = Duration::from_millis(50);

/// Prefix identifying an evaluation's runtime error.
const RUNTIME_ERROR: &str = "runtime error: ";

/// Has the world evaluate `expression`, without waiting for an answer
fn ask(world: &mut World, expression: &str) -> Result<()> {
	let request = Request::Eval {
		frame_id: None,
		command: expression.to_owned(),
		context: None
	};
	local::send(&mut world.stream, &request).map_err(|_| super::hung_up())
}

/// Whether process `pid` is gone by `until`
fn gone_by(pid: u32, until: Instant) -> bool {
	loop {
		if !local::alive(pid) {
			return true;
		}
		if Instant::now() >= until {
			return false;
		}
		std::thread::sleep(INTERVAL);
	}
}

/// Wait for process exit until `until`. A successful world.Del() closes
/// the debugger without a response. A runtime-error response means the
/// world is still running. A sleep response ends only the evaluation: keep
/// waiting for world.Del() to finish.
fn ended_by(world: &mut World, until: Instant) -> Result<bool> {
	loop {
		if !local::alive(world.pid) {
			return Ok(true);
		}
		let now = Instant::now();
		if now >= until {
			return Ok(false);
		}
		match world.next(Some(until.min(now + INTERVAL))) {
			// The evaluation ended at a sleep, but world.Del() is still
			// running.
			Ok(Some(Response::Eval(Err(error)))) if !error.starts_with(RUNTIME_ERROR) => {}
			// The runtime error left the world and debugger running.
			Ok(Some(Response::Eval(_))) => {
				return Err(format!(
					"world.Del() failed with a runtime error (see the world's log), so the world (pid {}) is still running. Fix world/Del() or \
					 include --kill in the stop request.",
					world.pid
				)
				.into())
			}
			Ok(_) => {}
			// No debugger is available to request world.Del().
			Err(e) if world.unavailable.is_some() => return Err(e),
			// Debugger disconnection can precede process exit.
			Err(_) => return Ok(gone_by(world.pid, until))
		}
	}
}

/// What a command that's done says: a line, or with `--json` the same as JSON
fn done(as_json: bool, document: serde_json::Value, line: String) {
	if as_json {
		say!("{}", document);
	} else {
		say!("{}", line);
	}
}

pub fn stop(world: &mut World, args: &args::Stop) -> Result<()> {
	let timeout = args.timeout.unwrap_or(Duration::from_secs(10));
	let kill_timeout = args.kill_timeout.unwrap_or(Duration::from_secs(5));
	let pid = world.pid;
	// (a world that went away before it was asked has stopped all the same)
	if ask(world, "world.Del()").is_err() && local::alive(pid) {
		return Err(format!("cannot send the shutdown request: the connection to the world with pid {} ended.", pid).into());
	}
	// With --kill, force termination on timeout or world.Del() failure.
	// An unavailable debugger cannot call world.Del().
	let failed = match ended_by(world, Instant::now() + timeout) {
		Ok(true) => {
			done(
				args.json,
				json!({ "pid": pid, "stopped": true, "killed": false }),
				format!("pid {}: stopped", pid)
			);
			return Ok(());
		}
		Ok(false) => None,
		Err(_) if world.unavailable.is_some() => None,
		Err(e) if args.kill => Some(e),
		Err(e) => return Err(e)
	};
	if !args.kill && world.unavailable.is_none() {
		return Err(Failure(
			TIMED_OUT,
			format!(
				"timed out after {} waiting for the world process (pid {}) to exit. world.Del() shut down its debugger, so auxtools can no longer \
				 reach it; include --kill in the stop request for a forced fallback.",
				seconds_text(timeout),
				pid
			)
		));
	}
	local::kill(pid);
	if gone_by(pid, Instant::now() + kill_timeout) {
		match (&world.unavailable, failed) {
			(Some(why), _) => eprintln!(
				"process terminated because debugger initialization failed: {}. world.Del() could not be called.",
				why
			),
			(None, Some(_)) => eprintln!("world.Del() failed with a runtime error; the process was terminated."),
			(None, None) => eprintln!("graceful shutdown timed out after {}; the process was terminated.", seconds_text(timeout))
		}
		done(
			args.json,
			json!({ "pid": pid, "stopped": true, "killed": true }),
			format!("pid {}: stopped (killed)", pid)
		);
		Ok(())
	} else {
		Err(Failure(
			TIMED_OUT,
			format!(
				"timed out after {} waiting for the terminated process (pid {}) to exit.",
				seconds_text(kill_timeout),
				pid
			)
		))
	}
}

pub fn reconnect(world: &mut World, args: &args::Reconnect) -> Result<()> {
	match world.request(Request::Reconnect)? {
		Response::Reconnect(Ok(Reconnected::Clients(clients))) => {
			done(
				args.json,
				json!({ "pid": world.pid, "reconnected": clients }),
				format!(
					"pid {}: asked {} {} to reconnect",
					world.pid,
					clients,
					if clients == 1 { "client" } else { "clients" }
				)
			);
			Ok(())
		}
		Response::Reconnect(Ok(Reconnected::Hosted)) => {
			if !args.json {
				eprintln!("Dream Seeker hosts the world: rebooting it instead.");
			}
			reboot(world, &args::Reboot {
				timeout: args.timeout,
				json: args.json
			})
		}
		Response::Reconnect(Err(e)) => Err(format!("cannot reconnect the clients: {}.", e).into()),
		other => Err(format!("unexpected response: {:?}", other).into())
	}
}

pub fn reboot(world: &mut World, args: &args::Reboot) -> Result<()> {
	let timeout = args.timeout.unwrap_or(Duration::from_secs(60));
	let pid = world.pid;
	let until = Instant::now() + timeout;
	ask(world, "world.Reboot()")?;

	// The world lets its clients go as it reboots
	loop {
		match world.next(Some(until)) {
			Ok(Some(_)) => {}
			Ok(None) => {
				return Err(Failure(
					TIMED_OUT,
					format!("timed out after {} waiting for the world to reboot.", seconds_text(timeout))
				))
			}
			Err(Failure(HUNG_UP, _)) => break,
			Err(e) => return Err(e)
		}
	}

	// Reconnect after loading, then wait for the launcher to restore reloads
	// and release the world.
	let mut again: Option<World> = None;
	loop {
		if !local::alive(pid) {
			return Err(Failure(HUNG_UP, format!("the world process (pid {}) exited instead of rebooting.", pid)));
		}
		if Instant::now() >= until {
			return Err(Failure(
				TIMED_OUT,
				format!(
					"timed out after {} waiting for the rebooted world to start running.",
					seconds_text(timeout)
				)
			));
		}
		match again.as_mut() {
			None => again = World::connect(pid).ok(),
			Some(world) => match world.started(Some(until)) {
				Ok(Some(true)) => {
					world.leave();
					done(args.json, json!({ "pid": pid, "rebooted": true }), format!("pid {}: rebooted", pid));
					return Ok(());
				}
				// Retry response timeouts until the reboot deadline.
				Ok(_) => {}
				// Reconnect if the connection closes, including during another
				// reboot.
				Err(_) => again = None
			}
		}
		std::thread::sleep(INTERVAL);
	}
}
