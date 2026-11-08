//! Stands in for dm-langserver against a real codebase: waits for a
//! DreamDaemon launched with `AUXTOOLS_DEBUG_MODE=LAUNCHED`, sets one
//! breakpoint, and when it hits does what VS Code does on a stop (stacks,
//! frames, scopes, arguments, locals) and then evaluates the given watches.
//! Every message is printed, so a crash shows which request caused it.

// The debug server's message types aren't exported from its cdylib, so they
// are pulled in as source, like tests/debug_test does
#[allow(dead_code)]
#[path = "../../../debug_server/src/server_types.rs"]
mod server_types;

use std::{
	io::{Read, Write},
	net::{TcpListener, TcpStream},
	path::Path,
	time::Duration
};

use server_types::{ContinueKind, EvalResponse, InstructionRef, ProcRef, Request, Response, DEFAULT_PORT};

const USAGE: &str = "\
usage: debug_client --proc PATH --line N [options] [WATCH...]

Listens for DreamDaemon started with AUXTOOLS_DEBUG_MODE=LAUNCHED and
AUXTOOLS_DEBUG_PORT set to the same port.

  --port N            port to listen on (default 2448)
  --proc PATH         proc to break in, as the debugger names it, e.g. /obj/item/Initialize
  --override N        override id of that proc (default 0)
  --line N            source line to break on
  --condition EXPR    only stop when EXPR is true
  --skip-inspect      go straight to the watches, without the requests VS Code sends on a stop
  --wait-file PATH    before the watches, wait until PATH exists (time to attach a debugger)

A WATCH is evaluated in the top frame like a VS Code watch; prefix it with
`repl:` to evaluate it like the debug console does.";

struct Options {
	port: u16,
	proc: ProcRef,
	line: u32,
	condition: Option<String>,
	inspect: bool,
	wait_file: Option<String>,
	watches: Vec<String>
}

fn parse_args() -> Result<Options, String> {
	let mut port = DEFAULT_PORT;
	let mut proc = None;
	let mut override_id = 0;
	let mut line = None;
	let mut condition = None;
	let mut inspect = true;
	let mut wait_file = None;
	let mut watches = vec![];

	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = |name: &str| args.next().ok_or_else(|| format!("{} needs a value", name));
		match arg.as_str() {
			"--port" => port = value("--port")?.parse().map_err(|e| format!("--port: {}", e))?,
			"--proc" => proc = Some(value("--proc")?),
			"--override" => override_id = value("--override")?.parse().map_err(|e| format!("--override: {}", e))?,
			"--line" => line = Some(value("--line")?.parse().map_err(|e| format!("--line: {}", e))?),
			"--condition" => condition = Some(value("--condition")?),
			"--skip-inspect" => inspect = false,
			"--wait-file" => wait_file = Some(value("--wait-file")?),
			"-h" | "--help" => return Err(String::new()),
			_ if arg.starts_with("--") => return Err(format!("unknown option {}", arg)),
			_ => watches.push(arg)
		}
	}

	Ok(Options {
		port,
		proc: ProcRef {
			path: proc.ok_or("--proc is required")?,
			override_id
		},
		line: line.ok_or("--line is required")?,
		condition,
		inspect,
		wait_file,
		watches
	})
}

struct Connection(TcpStream);

impl Connection {
	fn send(&mut self, request: &Request) {
		let data = bincode::serialize(request).unwrap();
		self.0.write_all(&(data.len() as u32).to_le_bytes()).unwrap();
		self.0.write_all(&data).unwrap();
		println!(">> {:?}", request);
	}

	/// The next message, or None once DreamDaemon has gone
	fn recv_any(&mut self) -> Option<Response> {
		let mut len = [0u8; 4];
		if let Err(e) = self.0.read_exact(&mut len) {
			println!("!! connection lost: {}", e);
			return None;
		}
		let mut data = vec![0u8; u32::from_le_bytes(len) as usize];
		if let Err(e) = self.0.read_exact(&mut data) {
			println!("!! connection lost mid-message: {}", e);
			return None;
		}
		Some(bincode::deserialize(&data).unwrap())
	}

	/// The next message that isn't a notification, printing those on the way
	fn recv(&mut self) -> Option<Response> {
		loop {
			match self.recv_any()? {
				Response::Notification { message } => println!("<< notification: {}", message.trim_end()),
				response => {
					let text = format!("{:?}", response);
					if text.len() > 2000 {
						println!("<< {}... ({} chars)", &text[..2000], text.len());
					} else {
						println!("<< {}", text);
					}
					return Some(response);
				}
			}
		}
	}

	fn request(&mut self, request: Request) -> Option<Response> {
		self.send(&request);
		self.recv()
	}
}

fn on_stop(conn: &mut Connection, options: &Options) -> Option<()> {
	if options.inspect {
		conn.request(Request::Stacks)?;
		conn.request(Request::StackFrames {
			stack_id: 0,
			start_frame: Some(0),
			count: Some(20)
		})?;
		if let Response::Scopes { arguments, locals, .. } = conn.request(Request::Scopes { frame_id: 0 })? {
			for vars in [arguments, locals].into_iter().flatten() {
				conn.request(Request::Variables { vars })?;
			}
		}
	}

	if let Some(path) = &options.wait_file {
		println!("== ready for watches, waiting for {}", path);
		while !Path::new(path).exists() {
			std::thread::sleep(Duration::from_millis(200));
		}
	}

	for watch in &options.watches {
		let (context, command) = match watch.strip_prefix("repl:") {
			Some(command) => ("repl", command),
			None => ("watch", watch.as_str())
		};
		let response = conn.request(Request::Eval {
			frame_id: Some(0),
			command: command.to_owned(),
			context: Some(context.to_owned())
		})?;
		if let Response::Eval(EvalResponse { variables: Some(vars), .. }) = response {
			conn.request(Request::Variables { vars })?;
		}
	}

	Some(())
}

fn main() {
	let options = match parse_args() {
		Ok(options) => options,
		Err(message) => {
			if !message.is_empty() {
				eprintln!("{}\n", message);
			}
			eprintln!("{}", USAGE);
			std::process::exit(2);
		}
	};

	let listener = TcpListener::bind(("127.0.0.1", options.port)).unwrap();
	println!("== listening on {}", options.port);
	let (stream, _) = listener.accept().unwrap();
	let mut conn = Connection(stream);
	println!("== DreamDaemon connected");

	let offset = match conn.request(Request::Offset {
		proc: options.proc.clone(),
		line: options.line
	}) {
		Some(Response::Offset { offset: Some(offset) }) => offset,
		other => {
			eprintln!("no instruction on line {} of {}: {:?}", options.line, options.proc.path, other);
			std::process::exit(1);
		}
	};
	let instruction = InstructionRef {
		proc: options.proc.clone(),
		offset
	};
	conn.request(Request::BreakpointSet {
		instruction: instruction.clone(),
		condition: options.condition.clone()
	});
	// Runtimes would stop us somewhere else. This one gets no reply.
	conn.send(&Request::CatchRuntimes { should_catch: false });
	conn.request(Request::Configured);

	loop {
		let Some(response) = conn.recv() else {
			std::process::exit(1);
		};
		if let Response::BreakpointHit { .. } = response {
			break;
		}
	}

	if on_stop(&mut conn, &options).is_none() {
		std::process::exit(1);
	}

	println!("== watches done, continuing");
	conn.request(Request::BreakpointUnset { instruction });
	conn.request(Request::Continue {
		kind: ContinueKind::Continue
	});
	// Long enough to see the world carry on before we let go of it
	std::thread::sleep(Duration::from_secs(3));
	conn.send(&Request::Disconnect);
}
