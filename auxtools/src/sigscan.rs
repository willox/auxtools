#[cfg(unix)]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use linux::Scanner;
#[cfg(windows)]
pub use windows::Scanner;

pub use auxtools_impl::convert_signature;

#[macro_export]
macro_rules! signature {
	($sig:tt) => {
		sigscan::convert_signature!($sig)
	};
}

#[macro_export]
macro_rules! signatures {
	( $( $name:ident => $sig:tt ),* ) => {
		struct Signatures {
			$( $name: &'static [Option<u8>], )*
		}

		static SIGNATURES: Signatures = Signatures {
			$( $name: signature!($sig), )*
		};
	}
}
