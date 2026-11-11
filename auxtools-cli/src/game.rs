//! Launch a BYOND world with the debug library and monitor its exit.

use std::{
	collections::HashMap,
	io::{BufRead, BufReader, Read},
	path::{Path, PathBuf},
	process::{Command, Stdio},
	sync::Arc
};

/// Debug library startup mode: `wait` delays world execution until a debugger
/// is configured; `run` starts immediately.
pub const DEBUG_VAR: &str = "AUXTOOLS_DEBUG";

pub struct Launch<'a> {
	pub program: PathBuf,
	pub dmb: &'a Path,
	pub args: &'a [String],
	pub env: &'a HashMap<String, String>,
	/// The debug library, and whether the world waits for a debugger to set its
	/// breakpoints
	pub debug: Option<(PathBuf, bool)>
}

/// A running world. It's killed if the launcher goes away first.
pub struct Game {
	pid: u32,
	#[cfg(windows)]
	job: windows::Job
}

impl Game {
	/// Launch the world. Send lines to `output` with their stream category
	/// and report the exit code through `exited`.
	pub fn start(
		launch: Launch,
		output: impl Fn(&'static str, String) + Send + Sync + 'static,
		exited: impl FnOnce(Option<i32>) + Send + 'static
	) -> Result<Self, String> {
		let output: Output = Arc::new(output);
		let mut command = Command::new(&launch.program);
		command
			.arg(launch.dmb)
			.args(launch.args)
			.envs(launch.env)
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped());
		if let Some(dir) = launch.dmb.parent().filter(|dir| !dir.as_os_str().is_empty()) {
			command.current_dir(dir);
		}
		// Clear legacy initialization so the world does not load the library
		// twice.
		command.env_remove("AUXTOOLS_DEBUG_DLL");
		if let Some((_, wait)) = &launch.debug {
			command.env(DEBUG_VAR, if *wait { "wait" } else { "run" });
		}

		#[cfg(unix)]
		{
			use std::os::unix::process::CommandExt;

			if let Some(bin) = launch.program.parent() {
				let paths = std::env::var_os("LD_LIBRARY_PATH")
					.map(|old| std::env::join_paths(std::iter::once(bin.to_owned()).chain(std::env::split_paths(&old))));
				match paths {
					Some(Ok(paths)) => command.env("LD_LIBRARY_PATH", paths),
					_ => command.env("LD_LIBRARY_PATH", bin)
				};
			}
			if let Some((library, _)) = &launch.debug {
				command.env("LD_PRELOAD", library);
			}
			// SAFETY: prctl and setpgid are async-signal-safe
			unsafe {
				command.pre_exec(|| {
					libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
					// Use a separate process group so Ctrl-C reaches only the
					// launcher, which requests graceful shutdown.
					libc::setpgid(0, 0);
					Ok(())
				});
			}
		}

		#[cfg(windows)]
		{
			use std::os::windows::process::CommandExt;
			// A process group of its own ignores Ctrl-C in the console: the
			// launcher hears it, and has the world end itself
			command.creation_flags(winapi::um::winbase::CREATE_SUSPENDED | winapi::um::winbase::CREATE_NEW_PROCESS_GROUP);
		}

		let mut child = command
			.spawn()
			.map_err(|e| format!("couldn't start {}: {}", launch.program.display(), e))?;
		let pid = child.id();

		#[cfg(windows)]
		let job = {
			use std::os::windows::io::AsRawHandle;

			let process = child.as_raw_handle() as winapi::um::winnt::HANDLE;
			let started = windows::Job::new(process).and_then(|job| {
				if let Some((library, _)) = &launch.debug {
					crate::inject::attach_on_entry(process, library)?;
				}
				crate::inject::resume(pid)?;
				Ok(job)
			});
			match started {
				Ok(job) => job,
				Err(e) => {
					let _ = child.kill();
					return Err(e);
				}
			}
		};

		output(
			"console",
			format!(
				"Started {} (process {})
",
				launch.program.display(),
				pid
			)
		);
		if let Some(stdout) = child.stdout.take() {
			relay(stdout, output.clone(), "stdout");
		}
		if let Some(stderr) = child.stderr.take() {
			relay(stderr, output.clone(), "stderr");
		}
		std::thread::Builder::new()
			.name("game".to_owned())
			.spawn(move || {
				let code = child.wait().ok().and_then(|status| status.code());
				exited(code);
			})
			.map_err(|e| e.to_string())?;

		Ok(Self {
			pid,
			#[cfg(windows)]
			job
		})
	}

	pub fn kill(&self) {
		#[cfg(unix)]
		unsafe {
			libc::kill(self.pid as i32, libc::SIGKILL);
		}
		#[cfg(windows)]
		{
			self.job.kill();
			// In case it couldn't be put in the job
			crate::local::kill(self.pid);
		}
	}

	pub const fn pid(&self) -> u32 {
		self.pid
	}
}

type Output = Arc<dyn Fn(&'static str, String) + Send + Sync>;

/// Passes each line `stream` has on
fn relay(stream: impl Read + Send + 'static, output: Output, category: &'static str) {
	let _ = std::thread::Builder::new().name(format!("{} relay", category)).spawn(move || {
		let mut reader = BufReader::new(stream);
		let mut line = vec![];
		while matches!(reader.read_until(b'\n', &mut line), Ok(n) if n > 0) {
			output(category, String::from_utf8_lossy(&line).into_owned());
			line.clear();
		}
	});
}

#[cfg(windows)]
mod windows {
	use std::{mem::size_of, ptr::null_mut};

	use winapi::{
		shared::minwindef::FALSE,
		um::{
			handleapi::CloseHandle,
			jobapi2::{AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, TerminateJobObject},
			winnt::{JobObjectExtendedLimitInformation, HANDLE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE}
		}
	};

	/// Windows job that terminates the child when the launcher exits.
	pub struct Job(HANDLE);

	// The handle is only used to kill the job
	unsafe impl Send for Job {}

	impl Job {
		pub fn new(process: HANDLE) -> Result<Self, String> {
			unsafe {
				let job = CreateJobObjectW(null_mut(), null_mut());
				if job.is_null() {
					return Err(format!("couldn't make a job object: {}", std::io::Error::last_os_error()));
				}
				let job = Self(job);

				let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
				limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
				let set = SetInformationJobObject(
					job.0,
					JobObjectExtendedLimitInformation,
					&mut limits as *mut _ as *mut _,
					size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32
				);
				// Assign the process only if the kill-on-close limit was set.
				if set != FALSE {
					AssignProcessToJobObject(job.0, process);
				}
				Ok(job)
			}
		}

		pub fn kill(&self) {
			unsafe {
				TerminateJobObject(self.0, 1);
			}
		}
	}

	impl Drop for Job {
		fn drop(&mut self) {
			unsafe {
				CloseHandle(self.0);
			}
		}
	}
}
