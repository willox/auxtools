use std::{
	io::{Read, Write},
	path::{Path, PathBuf},
	sync::{mpsc, Arc, Mutex},
	time::{Duration, Instant}
};

use auxtools_cli::{
	game::{Game, Launch},
	local::{self, Shared}
};

use crate::server_types::{BreakpointReason, Request, Response};

// Long enough for a world reboot, short enough that a dead world fails the run
// instead of hanging it
const TIMEOUT: Duration = Duration::from_secs(30);

/// A world in DreamDaemon with the debug library loaded into it, as the
/// launcher starts one to debug: it waits for a client to say it's configured.
/// It's ended when this is dropped.
pub struct World {
	game: Game,
	output: Arc<Mutex<String>>
}

impl World {
	/// `env` is the world's environment, beyond what it inherits
	pub fn start(dmb: &Path, env: &[(&str, String)]) -> Self {
		let env = env.iter().map(|(name, value)| (name.to_string(), value.clone())).collect();
		let args = ["-trusted".to_owned(), "-close".to_owned()];
		let output = Arc::new(Mutex::new(String::new()));
		let printed = output.clone();
		let game = Game::start(
			Launch {
				program: crate::paths::find_dreamdaemon(),
				dmb,
				args: &args,
				env: &env,
				debug: Some((find_debug_server(), true))
			},
			move |_, line| {
				print!("  | {}", line);
				printed.lock().unwrap().push_str(&line);
			},
			|code| println!("  | the world exited ({:?})", code)
		)
		.unwrap();

		Self { game, output }
	}

	/// Everything the world has printed
	pub fn output(&self) -> String {
		self.output.lock().unwrap().clone()
	}

	/// Connects to the world's debug server, once it's listening (when the
	/// world loads, and again after each reboot), and lets the world run
	pub fn connect(&self) -> Client {
		self.connect_as(false)
	}

	/// Connects as a hot reload watcher does, which doesn't hear about the
	/// world pausing
	pub fn connect_watcher(&self) -> Client {
		self.connect_as(true)
	}

	/// Connects and says nothing: the world doesn't wait for this client, and
	/// isn't told it's configured
	pub fn connect_silent(&self) -> Client {
		let start = Instant::now();
		loop {
			assert!(start.elapsed() < TIMEOUT, "timed out connecting to debug server");
			if let Ok(stream) = local::connect(self.game.pid()) {
				let stream = Shared::new(stream);
				return Client::new(Box::new(stream.clone()), Box::new(stream), false);
			}
			std::thread::sleep(Duration::from_millis(100));
		}
	}

	fn connect_as(&self, watcher: bool) -> Client {
		let start = Instant::now();
		loop {
			assert!(start.elapsed() < TIMEOUT, "timed out connecting to debug server");
			let Ok(stream) = local::connect(self.game.pid()) else {
				std::thread::sleep(Duration::from_millis(100));
				continue;
			};
			let stream = Shared::new(stream);
			let mut client = Client::new(Box::new(stream.clone()), Box::new(stream), watcher);
			// It waits for the breakpoints the launcher has, and there are
			// none. A world on its way to a reboot hangs up instead.
			if client.try_send(&Request::Configured).is_err() {
				std::thread::sleep(Duration::from_millis(100));
				continue;
			}
			match client.messages.recv_timeout(TIMEOUT) {
				Ok(Response::Ack) => {
					if watcher {
						let response = client.request(Request::Watcher);
						assert!(matches!(response, Response::Ack), "expected Ack for Watcher, got {:?}", response);
					}
					return client;
				}
				Ok(Response::Disconnect) | Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(Duration::from_millis(100)),
				other => panic!("expected Ack for Configured, got {:?}", other)
			}
		}
	}
}

impl Drop for World {
	fn drop(&mut self) {
		self.game.kill();
	}
}

fn find_debug_server() -> PathBuf {
	let mut path = std::env::current_exe().unwrap();
	path.pop();

	#[cfg(unix)]
	path.push("libdebug_server.so");

	#[cfg(windows)]
	path.push("debug_server.dll");

	assert!(path.is_file(), "couldn't find debug_server");
	path
}

/// Stands in for the launcher: speaks the debug server's protocol over the
/// world's local socket
pub struct Client {
	writer: Box<dyn Write + Send>,
	/// What the server sends, read on a thread of its own so a quiet server
	/// can time out. It closes when the server hangs up.
	messages: mpsc::Receiver<Response>,
	/// Whether to pass over the world pausing and carrying on
	watcher: bool
}

impl Client {
	fn new(mut reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>, watcher: bool) -> Self {
		let (sender, messages) = mpsc::channel();
		std::thread::spawn(move || {
			while let Ok(response) = read_message(&mut reader) {
				if sender.send(response).is_err() {
					return;
				}
			}
		});
		Self { writer, messages, watcher }
	}

	pub fn send(&mut self, request: Request) {
		self.try_send(&request).unwrap();
	}

	fn try_send(&mut self, request: &Request) -> std::io::Result<()> {
		let data = bincode::serialize(request).unwrap();
		self.writer.write_all(&(data.len() as u32).to_le_bytes())?;
		self.writer.write_all(&data)
	}

	/// Sends bytes as they are
	pub fn send_bytes(&mut self, bytes: &[u8]) {
		self.writer.write_all(bytes).unwrap();
	}

	/// Says goodbye, and waits for the server to have let this client go:
	/// what another client does next comes after
	pub fn leave(mut self) {
		let _ = self.try_send(&Request::Disconnect);
		self.wait_closed();
	}

	/// The next message that isn't a notification, or another client
	/// letting the world carry on
	pub fn recv(&mut self) -> Response {
		loop {
			match self.messages.recv_timeout(TIMEOUT).expect("debug server stopped answering") {
				Response::Notification { message } => println!("  [debug server] {}", message),
				Response::Resumed | Response::CodeChanged | Response::StepEnded => {}
				Response::BreakpointHit { .. } if self.watcher => {}
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
			Response::BreakpointHit(stop) => stop.reason,
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
	pub fn wait_closed(self) {
		loop {
			match self.messages.recv_timeout(TIMEOUT) {
				Ok(Response::Disconnect) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
				Ok(_) => {}
				Err(mpsc::RecvTimeoutError::Timeout) => panic!("timed out waiting for debug server disconnect")
			}
		}
	}
}

fn read_message(stream: &mut impl Read) -> std::io::Result<Response> {
	let mut len_bytes = [0u8; 4];
	stream.read_exact(&mut len_bytes)?;

	let mut data = vec![0; u32::from_le_bytes(len_bytes) as usize];
	stream.read_exact(&mut data)?;

	Ok(bincode::deserialize(&data).unwrap())
}
