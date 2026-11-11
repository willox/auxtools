//! Accept debug clients on a process-specific local socket or Windows named
//! pipe.

use std::{
	io::{self, Read, Write},
	sync::Arc
};

use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Stream as LocalStream};

use crate::server_types::world_socket_name;

/// The name this world listens on
pub fn world_name() -> String {
	world_socket_name(std::process::id())
}

/// Block in accept for the process lifetime (see `Server::listen`).
/// Nonblocking named pipes do not accept connections under Wine.
pub struct Listener(interprocess::local_socket::Listener);

impl Listener {
	/// Waits for a client
	pub fn accept(&self) -> io::Result<Connection> {
		self.0.accept().map(Connection::new)
	}
}

pub fn listen(name: &str) -> io::Result<Listener> {
	Ok(Listener(
		ListenerOptions::new().name(name.to_ns_name::<GenericNamespaced>()?).create_sync()?
	))
}

/// Connection shared by the server writer and per-client reader thread.
#[derive(Clone)]
pub struct Connection(Arc<LocalStream>);

impl Connection {
	pub fn new(stream: LocalStream) -> Self {
		Self(Arc::new(stream))
	}

	/// Connects to a listener, as a client would
	#[cfg(test)]
	pub fn connect(name: &str) -> io::Result<Self> {
		Ok(Self::new(LocalStream::connect(name.to_ns_name::<GenericNamespaced>()?)?))
	}
}

impl Read for Connection {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		(&*self.0).read(buf)
	}
}

impl Write for Connection {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		(&*self.0).write(buf)
	}

	fn flush(&mut self) -> io::Result<()> {
		(&*self.0).flush()
	}
}
