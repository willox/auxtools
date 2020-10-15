use crate::value::Value;
use std::result;

/// This struct represents a DM runtime error. Since we don't have exceptions in rust, we
/// have to handle errors using [DMResult]. Some operations, such as attempting to read
/// nonexistent variables, can return a Runtime; It can be either handled or bubbled up using the ? operator.
///
/// # Examples
///
/// ```ignore
/// #[hook("/datum/proc/errorist")]
/// fn throw() {
///		let v = src.get("nonexistent_var")?;
/// 	Ok(v)
/// }
/// ```
///
/// This hook attempts to read a variable that does not exist.
///
///
///
///
///
///
///
///
///
///
///

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

#[macro_export]
macro_rules! runtime {
	($fmt:expr) => {
		return Err($crate::runtime::Runtime::new($fmt));
	};
	($fmt: expr, $( $args:expr ),*) => {
		return Err($crate::runtime::Runtime::new(format!( $fmt, $( $args, )* )));
	};
}

pub type DMResult<'a> = result::Result<Value<'a>, Runtime>;
pub type ConversionResult<T> = result::Result<T, Runtime>;
