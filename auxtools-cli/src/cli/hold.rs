//! Limit pauses retained after a CLI command disconnects.
//! `--hold-max` prevents an interrupted debugging workflow from leaving the
//! world paused indefinitely without a connected debugger.

use std::time::Duration;

use auxtools_cli::server_types::{Request, Response};

use super::{
	args::{Command, Wait, WaitFor},
	unexpected, Failure, Result, World
};

/// What `--hold-max` is when it isn't given
pub const HOLD_MAX_VAR: &str = "AUXTOOLS_HOLD_MAX";

/// Resolve `--hold-max`, falling back to `AUXTOOLS_HOLD_MAX`.
/// Returns `None` for commands that do not retain a pause, or for no limit.
pub fn max(command: &Command) -> Result<Option<Duration>> {
	let given = match command {
		Command::Pause(pause) => pause.hold_max,
		Command::StepOver(step) => step.hold_max,
		Command::StepIn(step) => step.hold_max,
		Command::StepOut(step) => step.hold_max,
		Command::Wait(Wait {
			event: Some(WaitFor::Pause(wait))
		}) if !wait.no_hold => wait.hold_max,
		_ => return Ok(None)
	};
	match given {
		Some(limit) => Ok(Some(limit)),
		None => from_environment(std::env::var(HOLD_MAX_VAR).ok().as_deref())
	}
}

fn from_environment(value: Option<&str>) -> Result<Option<Duration>> {
	let Some(text) = value.map(str::trim).filter(|text| !text.is_empty()) else {
		return Ok(None);
	};
	match text.parse::<f64>() {
		Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => Ok(Some(Duration::from_secs_f64(seconds.min(super::args::MANY_SECONDS)))),
		_ => Err(Failure(
			1,
			format!(
				"invalid value '{}' for {}: expected a non-negative number of seconds.",
				text, HOLD_MAX_VAR
			)
		))
	}
}

impl World {
	/// Set the maximum pause duration while no debugger is connected.
	pub fn hold_max(&mut self, limit: Duration) -> Result<()> {
		match self.request(Request::HoldMax {
			seconds: Some(limit.as_secs_f64())
		})? {
			Response::Ack => Ok(()),
			other => Err(unexpected(&other))
		}
	}
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::from_environment;

	#[test]
	fn the_environment_gives_a_limit_when_its_a_time() {
		assert_eq!(from_environment(Some("2.5")).ok().unwrap(), Some(Duration::from_millis(2500)));
		assert_eq!(from_environment(Some(" ")).ok().unwrap(), None);
		assert_eq!(from_environment(None).ok().unwrap(), None);
		for wrong in ["soon", "-1", "inf"] {
			assert!(from_environment(Some(wrong)).is_err(), "{}", wrong);
		}
	}
}
