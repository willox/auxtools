//! Connect to worlds over local sockets (named pipes on Windows).
//! Endpoints are identified by process ID. Reboot disconnects clients; new
//! connections wait for the next world to finish loading.

use std::{
	io::{self, Read, Write},
	path::{Path, PathBuf},
	sync::Arc,
	time::Duration
};

pub use interprocess::local_socket::Stream;
use interprocess::{
	local_socket::{prelude::*, ConnectOptions, GenericNamespaced},
	ConnectWaitMode
};

use crate::server_types::{world_socket_name, world_socket_name_for, Request, Response, PROTOCOL_VERSION};

/// The name a world's process listens on
pub fn world_name(pid: u32) -> String {
	world_socket_name(pid)
}

fn connect_to(name: String) -> io::Result<Stream> {
	// Avoid waiting indefinitely for a named pipe instance during shutdown.
	ConnectOptions::new()
		.name(name.to_ns_name::<GenericNamespaced>()?)
		.wait_mode(ConnectWaitMode::Timeout(Duration::from_secs(1)))
		.connect_sync()
}

/// Connect to the world in `pid`. Report incompatible protocol versions.
pub fn connect(pid: u32) -> io::Result<Stream> {
	connect_to(world_name(pid)).map_err(|e| match other_protocol(pid) {
		Some(theirs) => io::Error::other(format!(
			"its debug library speaks protocol {}, and this auxtools speaks {}: use the auxtools that goes with it (the one that started the \
			 world), or start the world with this one",
			theirs, PROTOCOL_VERSION
		)),
		None => e
	})
}

/// How far past this version to look for a newer debug library
const NEWER_PROTOCOLS: u32 = 16;

/// Find an incompatible endpoint's protocol version without sending messages.
pub fn other_protocol(pid: u32) -> Option<u32> {
	(1..=PROTOCOL_VERSION + NEWER_PROTOCOLS)
		.filter(|version| *version != PROTOCOL_VERSION)
		.find(|version| connect_to(world_socket_name_for(*version, pid)).is_ok())
}

/// Connection supporting reads and writes on separate threads.
#[derive(Clone)]
pub struct Shared(Arc<Stream>);

impl Shared {
	pub fn new(stream: Stream) -> Self {
		Self(Arc::new(stream))
	}
}

impl Read for Shared {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		(&*self.0).read(buf)
	}
}

impl Write for Shared {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		(&*self.0).write(buf)
	}

	fn flush(&mut self) -> io::Result<()> {
		(&*self.0).flush()
	}
}

/// Sends a message to the debug server
pub fn send(stream: &mut impl Write, request: &Request) -> io::Result<()> {
	let data = bincode::serialize(request).map_err(io::Error::other)?;
	stream.write_all(&(data.len() as u32).to_le_bytes())?;
	stream.write_all(&data)?;
	stream.flush()
}

/// Reads a message from the debug server
pub fn receive(stream: &mut impl Read) -> io::Result<Response> {
	// The debug server limits requests to this, and nothing it sends is bigger
	const MAX_MESSAGE_LEN: u32 = 256 * 1024 * 1024;

	let mut len = [0u8; 4];
	stream.read_exact(&mut len)?;
	let len = u32::from_le_bytes(len);
	if len > MAX_MESSAGE_LEN {
		return Err(io::Error::other(format!("message length {} exceeds the protocol limit", len)));
	}
	let mut data = vec![0; len as usize];
	stream.read_exact(&mut data)?;
	bincode::deserialize(&data).map_err(io::Error::other)
}

/// A world that can be attached to
#[derive(Debug, serde::Serialize)]
pub struct World {
	pub pid: u32,
	/// Its program's file name
	pub program: String,
	/// The world it runs, from its command line
	pub dmb: Option<PathBuf>,
	pub args: Vec<String>,
	pub cwd: String,
	/// The version of the protocol its debug library speaks
	pub protocol: u32,
	/// Whether the protocol matches. Incompatible worlds cannot be queried
	/// or attached to.
	pub compatible: bool
}

/// Discover debug servers by querying BYOND processes.
/// List incompatible endpoints without sending protocol messages.
pub fn worlds() -> Vec<World> {
	server_processes()
		.into_iter()
		.filter_map(|(pid, program)| {
			probe(pid, program.clone()).or_else(|| {
				other_protocol(pid).map(|protocol| World {
					pid,
					program,
					dmb: None,
					args: vec![],
					cwd: String::new(),
					protocol,
					compatible: false
				})
			})
		})
		.collect()
}

fn probe(pid: u32, program: String) -> Option<World> {
	let stream = connect_to(world_name(pid)).ok()?;
	let (sender, receiver) = std::sync::mpsc::channel();
	// A world busy with something else answers between DM instructions, and
	// one that has hung doesn't
	std::thread::spawn(move || {
		let mut stream = stream;
		let answer = (|| {
			send(&mut stream, &Request::WorldInfo)?;
			loop {
				match receive(&mut stream)? {
					Response::WorldInfo { args, cwd } => return Ok((args, cwd)),
					Response::Disconnect => return Err(io::Error::other("it hung up")),
					_ => {}
				}
			}
		})();
		let _ = send(&mut stream, &Request::Disconnect);
		let _ = sender.send(answer);
	});
	let (args, cwd) = receiver.recv_timeout(Duration::from_secs(5)).ok()?.ok()?;
	Some(World {
		pid,
		program,
		dmb: world_dmb(&args, &cwd),
		args,
		cwd,
		protocol: PROTOCOL_VERSION,
		compatible: true
	})
}

/// The .dmb a BYOND program's command line runs
pub fn world_dmb(args: &[String], cwd: &str) -> Option<PathBuf> {
	let dmb = args.iter().skip(1).find(|arg| arg.to_ascii_lowercase().ends_with(".dmb"))?;
	Some(Path::new(cwd).join(dmb))
}

/// The processes that might be BYOND servers, by id, with their program's name
#[cfg(unix)]
fn server_processes() -> Vec<(u32, String)> {
	let Ok(entries) = std::fs::read_dir("/proc") else {
		return vec![];
	};
	entries
		.filter_map(|entry| {
			let pid: u32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
			let program = std::fs::read_to_string(format!("/proc/{}/comm", pid)).ok()?.trim().to_owned();
			(program == "DreamDaemon").then_some((pid, program))
		})
		.collect()
}

#[cfg(windows)]
fn server_processes() -> Vec<(u32, String)> {
	use winapi::um::{
		handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
		tlhelp32::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS}
	};

	let mut found = vec![];
	unsafe {
		let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
		if snapshot == INVALID_HANDLE_VALUE {
			return found;
		}
		let mut entry: PROCESSENTRY32W = std::mem::zeroed();
		entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
		let mut more = Process32FirstW(snapshot, &mut entry) != 0;
		while more {
			let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
			let program = String::from_utf16_lossy(&entry.szExeFile[..len]);
			if ["dd.exe", "dreamdaemon.exe", "dreamseeker.exe"].contains(&program.to_ascii_lowercase().as_str()) {
				found.push((entry.th32ProcessID, program));
			}
			more = Process32NextW(snapshot, &mut entry) != 0;
		}
		CloseHandle(snapshot);
	}
	found
}

/// Whether process `pid` is still running
#[cfg(unix)]
pub fn alive(pid: u32) -> bool {
	// Signal 0 only checks
	if unsafe { libc::kill(pid as i32, 0) } != 0 {
		return false;
	}
	// Signal 0 also succeeds for zombies. Check the process state in
	// `stat`, since only our own children are reliably reaped.
	let zombie = std::fs::read_to_string(format!("/proc/{}/stat", pid))
		.ok()
		.and_then(|stat| Some(stat.rsplit_once(") ")?.1.starts_with('Z')));
	zombie != Some(true)
}

#[cfg(windows)]
pub fn alive(pid: u32) -> bool {
	use winapi::{
		shared::winerror::WAIT_TIMEOUT,
		um::{handleapi::CloseHandle, processthreadsapi::OpenProcess, synchapi::WaitForSingleObject, winnt::SYNCHRONIZE}
	};

	unsafe {
		let process = OpenProcess(SYNCHRONIZE, 0, pid);
		if process.is_null() {
			return false;
		}
		let running = WaitForSingleObject(process, 0) == WAIT_TIMEOUT;
		CloseHandle(process);
		running
	}
}

/// Ends process `pid`
pub fn kill(pid: u32) {
	#[cfg(unix)]
	unsafe {
		libc::kill(pid as i32, libc::SIGKILL);
	}

	#[cfg(windows)]
	unsafe {
		use winapi::um::{
			handleapi::CloseHandle,
			processthreadsapi::{OpenProcess, TerminateProcess},
			winnt::PROCESS_TERMINATE
		};

		let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
		if !process.is_null() {
			TerminateProcess(process, 1);
			CloseHandle(process);
		}
	}
}
