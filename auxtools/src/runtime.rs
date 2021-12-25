use crate::value::Value;
use std::result;

/// Represents a byond runtime, sort of. This will probably drastically in the future.
///
/// These are just simple error messages that our API and hooks can return as failure states.
#[derive(Debug)]
pub struct Runtime {
	pub message: String,
}

impl Runtime {
	pub fn new<S: Into<String>>(message: S) -> Self {
		Self {
			message: message.into(),
		}
	}
}

/// This macro makes instantiating [Runtimes](struct.Runtime.html) a (little bit) easier.
#[macro_export]
macro_rules! runtime {
	($fmt:expr) => {
		$crate::Runtime::new($fmt)
	};
	($fmt: expr, $( $args:expr ),*) => {
		$crate::Runtime::new(format!( $fmt, $( $args, )* ))
	};
}

pub type DMResult<T = Value> = result::Result<T, Runtime>;
