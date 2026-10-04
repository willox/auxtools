use std::{
	io::{Read, Write},
	net::{Ipv4Addr, TcpStream},
	time::{Duration, Instant}
};

use crate::server_types::{BreakpointReason, Request, Response};

// Long enough for a world reboot, short enough that a dead world fails the run
// instead of hanging it
const TIMEOUT: Duration = Duration::from_secs(30);

/// Stands in for dm-langserver: speaks the debug server's protocol over TCP.
pub struct Client {
	stream: TcpStream
}

impl Client {
	/// Keeps trying until the world is listening.
	pub fn connect(port: u16) -> Self {
		let start = Instant::now();

		loop {
			match TcpStream::connect((Ipv4Addr::LOCALHOST, port)) {
				Ok(stream) => {
					stream.set_read_timeout(Some(TIMEOUT)).unwrap();
					return Self { stream };
				}

				Err(e) => {
					assert!(start.elapsed() < TIMEOUT, "couldn't connect to the debug server: {}", e);
					std::thread::sleep(Duration::from_millis(100));
				}
			}
		}
	}

	pub fn send(&mut self, request: Request) {
		let data = bincode::serialize(&request).unwrap();
		self.stream.write_all(&(data.len() as u32).to_le_bytes()).unwrap();
		self.stream.write_all(&data).unwrap();
	}

	fn read_message(&mut self) -> std::io::Result<Response> {
		let mut len_bytes = [0u8; 4];
		self.stream.read_exact(&mut len_bytes)?;

		let mut data = vec![0; u32::from_le_bytes(len_bytes) as usize];
		self.stream.read_exact(&mut data)?;

		Ok(bincode::deserialize(&data).unwrap())
	}

	/// The next message that isn't a notification.
	pub fn recv(&mut self) -> Response {
		loop {
			match self.read_message().expect("debug server stopped answering") {
				Response::Notification { message } => println!("  [debug server] {}", message),
				response => return response
			}
		}
	}

	pub fn request(&mut self, request: Request) -> Response {
		self.send(request);
		self.recv()
	}

	/// Waits for the world to pause and says why it did.
	pub fn expect_pause(&mut self) -> BreakpointReason {
		match self.recv() {
			Response::BreakpointHit { reason } => reason,
			other => panic!("expected the world to pause, got {:?}", other)
		}
	}

	pub fn resume(&mut self) {
		let response = self.request(Request::Continue {
			kind: crate::server_types::ContinueKind::Continue
		});
		assert!(matches!(response, Response::Ack), "expected Ack for Continue, got {:?}", response);
	}

	/// Waits for the server to hang up on us.
	pub fn wait_closed(mut self) {
		loop {
			match self.read_message() {
				Ok(Response::Disconnect) => return,
				Ok(_) => {}
				Err(e) => {
					let timed_out = matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock);
					assert!(!timed_out, "debug server never hung up");
					return;
				}
			}
		}
	}
}
