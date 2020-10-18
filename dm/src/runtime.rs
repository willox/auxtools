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
		Err($crate::runtime::Runtime::new($fmt));
	};
	($fmt: expr, $( $args:expr ),*) => {
		Err($crate::runtime::Runtime::new(format!( $fmt, $( $args, )* )));
	};
}

/// Used as a result for hooks and calls into BYOND.
pub type DMResult<'a> = result::Result<Value<'a>, Runtime>;

/// Used as a result for conversions between DM values and rust values
pub type ConversionResult<T> = result::Result<T, Runtime>;
