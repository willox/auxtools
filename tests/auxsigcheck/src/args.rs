use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
	/// Path to the auxtools DLL to use.
	#[arg(short = 'i', long = "dll")]
	pub auxtools_path: PathBuf,

	/// Version of BYOND to use.
	pub version: u16,

	/// Build of BYOND to use.
	pub build: u16
}

impl Args {
	#[inline]
	pub fn new() -> Self {
		Self::parse()
	}
}
