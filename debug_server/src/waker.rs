//! Wake idle worlds while debugger clients need service.
//!
//! With `world.sleep_offline`, no DM instructions run and requests cannot be
//! processed. An idle TCP connection keeps BYOND awake. Open one after a period
//! without instructions while clients or requests are waiting, and close it
//! when no clients remain. The debugger keepalive proc supplies regular ticks.

use std::{
	net::{Ipv4Addr, SocketAddr, TcpStream},
	sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering},
	thread,
	time::{Duration, Instant}
};

/// Instruction counter updated by the world thread.
static ALIVE: AtomicU32 = AtomicU32::new(0);
/// Requests queued by network readers but not yet processed.
static WAITING: AtomicU32 = AtomicU32::new(0);
/// Clients connected
static CLIENTS: AtomicU32 = AtomicU32::new(0);
/// The world's port, once it's known
static PORT: AtomicU16 = AtomicU16::new(0);
/// Wake notification pending for clients.
static WOKE: AtomicBool = AtomicBool::new(false);
/// Signal that the event channel may need draining on the world thread.
static EVENTS: AtomicBool = AtomicBool::new(false);

/// Idle interval before waking a world with connected clients.
const PATIENCE: Duration = Duration::from_secs(1);
const CHECK_EVERY: Duration = Duration::from_millis(250);

/// Record instruction activity on the world thread.
pub fn alive() {
	ALIVE.store(ALIVE.load(Ordering::Relaxed).wrapping_add(1), Ordering::Relaxed);
}

pub fn request_waiting() {
	WAITING.fetch_add(1, Ordering::Relaxed);
}

pub fn request_taken() {
	WAITING.fetch_sub(1, Ordering::Relaxed);
}

pub fn client_connected() {
	CLIENTS.fetch_add(1, Ordering::Relaxed);
}

pub fn client_left() {
	CLIENTS.fetch_sub(1, Ordering::Relaxed);
}

pub fn set_port(port: u16) {
	PORT.store(port, Ordering::Relaxed);
}

/// Signal a new event. Call after sending it to the channel.
pub fn event_sent() {
	EVENTS.store(true, Ordering::Release);
}

/// Consume the pending-event flag. If set, drain the channel; later sends
/// set the flag again.
pub fn events_pending() -> bool {
	// Avoid an atomic write on every instruction when the flag is clear.
	EVENTS.load(Ordering::Acquire) && EVENTS.swap(false, Ordering::AcqRel)
}

/// Consume the wake notification.
pub fn woke() -> bool {
	// Asked on every instruction, so it only writes when it was
	WOKE.load(Ordering::Relaxed) && WOKE.swap(false, Ordering::Relaxed)
}

/// Starts watching, for as long as the process runs
pub fn start() -> std::io::Result<()> {
	thread::Builder::new().name("debug server waker".to_owned()).spawn(watch)?;
	Ok(())
}

fn wanted() -> bool {
	WAITING.load(Ordering::Relaxed) > 0 || CLIENTS.load(Ordering::Relaxed) > 0
}

fn watch() {
	let mut last_alive = ALIVE.load(Ordering::Relaxed);
	let mut quiet_since = Instant::now();
	let mut connection: Option<TcpStream> = None;
	loop {
		thread::sleep(CHECK_EVERY);

		if !wanted() {
			// The world may sleep again
			connection = None;
		}
		// BYOND hangs up on a connection that never says anything, in time
		if connection.as_ref().is_some_and(|c| !open(c)) {
			connection = None;
		}

		let alive = ALIVE.load(Ordering::Relaxed);
		if alive != last_alive || !wanted() || connection.is_some() {
			last_alive = alive;
			quiet_since = Instant::now();
			continue;
		}
		if quiet_since.elapsed() < PATIENCE {
			continue;
		}

		let port = PORT.load(Ordering::Relaxed);
		if port != 0 {
			let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
			connection = TcpStream::connect_timeout(&address, Duration::from_secs(1)).ok();
			if connection.is_some() {
				WOKE.store(true, Ordering::Relaxed);
			}
		}
		quiet_since = Instant::now();
	}
}

/// Check whether BYOND closed the wake connection, draining any preceding data.
/// Peeking would keep seeing unread data and miss the close.
fn open(connection: &TcpStream) -> bool {
	use std::io::{ErrorKind, Read};

	if connection.set_nonblocking(true).is_err() {
		return false;
	}
	let mut buffer = [0u8; 256];
	let open = loop {
		match (&mut &*connection).read(&mut buffer) {
			Ok(0) => break false,
			Ok(_) => continue,
			Err(e) if e.kind() == ErrorKind::Interrupted => continue,
			Err(e) => break e.kind() == ErrorKind::WouldBlock
		}
	};
	let _ = connection.set_nonblocking(false);
	open
}
