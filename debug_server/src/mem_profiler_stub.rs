use std::fmt;

pub struct Error;

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "UnsupportedPlatform")
	}
}

pub fn begin(_: &str) -> Result<(), Error> {
	Err(Error)
}

pub fn end() {}
