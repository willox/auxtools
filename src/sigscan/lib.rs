#[cfg(unix)]
pub use linux::Scanner;
#[cfg(windows)]
pub use windows::Scanner;

#[cfg(unix)]
mod linux;
#[cfg(windows)]
mod windows;
