use crate::value::Value;
use std::result;

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

#[macro_export]
macro_rules! runtime {
	($fmt:expr) => {
		Err($crate::runtime::Runtime::new($fmt));
	};
	($fmt: expr, $( $args:expr ),*) => {
		Err($crate::runtime::Runtime::new(format!( $fmt, $( $args, )* )));
	};
}

pub type DMResult<'a> = result::Result<Value<'a>, Runtime>;
pub type ConversionResult<T> = result::Result<T, Runtime>;
