#[cfg(unix)]
mod linux;
#[cfg(windows)]
mod win32;

use std::ops::{RangeBounds, Bound};

#[cfg(unix)]
pub use linux::Scanner;
#[cfg(windows)]
pub use win32::Scanner;

pub use auxtools_impl::convert_signature;

#[macro_export]
macro_rules! signature {
	($sig:tt) => {
		sigscan::convert_signature!($sig)
	};
}

#[macro_export]
macro_rules! signatures {
	( $( $name:ident => $sig:expr ),*) => {
		struct Signatures {
			$( pub $name: SignatureMap, )*
		}

		static SIGNATURES0: Lazy<Signatures> = Lazy::new(|| Signatures {
			$( $name: $sig, )*
		});
	};
}

#[macro_export]
macro_rules! signature_struct {
	(call, $sig:tt) => {
		Signature{ treatment: SignatureTreatment::OffsetByCall, bytes: signature!($sig) }
	};
	($offset:literal, $sig:tt) => {
		Signature{ treatment: SignatureTreatment::OffsetByInt($offset), bytes: signature!($sig) }
	};
	(($spec:tt, $sig:tt)) => {
		signature_struct!($spec, $sig)
	};
	($sig:tt) => {
		Signature{ treatment: SignatureTreatment::NoOffset, bytes: signature!($sig) }
	};
}

#[macro_export]
macro_rules! version_dependent_signature {
	( $($range:expr => $sig:tt),* ) => {
		SignatureMap::VersionDependent(vec![
			$((($range.start_bound(), $range.end_bound()), signature_struct!($sig)),)*
		])
	};
}

#[macro_export]
macro_rules! universal_signature {
	(call, $sig:tt) => {
		SignatureMap::AllVersions(signature_struct!(call, $sig))
	};
	($offset:literal, $sig:tt) => {
		SignatureMap::AllVersions(signature_struct!($offset, $sig))
	};
	($sig:tt) => {
		SignatureMap::AllVersions(signature_struct!($sig))
	};
}

#[macro_export]
macro_rules! find_signature_inner {
	($scanner:ident, $name:ident, $type:ty) => {
		let $name: $type;
		if let Some(ptr) = SIGNATURES0.$name.find(&$scanner, version::get().1) {
			$name = ptr as $type;
		} else {
			return Some(format!("FAILED (Couldn't find {})", stringify!($name)));
		}
	};
}

#[macro_export]
macro_rules! find_signature {
	($scanner:ident, $name:ident as $type:ty) => {
		find_signature_inner!($scanner, $name, $type);
	};

	($scanner:ident, ($name:ident as $type:ty)) => {
		find_signature_inner!($scanner, $name, $type);
	};

	($scanner:ident, $name:ident) => {
		find_signature_inner!($scanner, $name, *const c_void);
	};
}

#[macro_export]
macro_rules! find_signatures {
	($scanner:ident, $($sig:tt),* ) => {
		$(
			find_signature!($scanner, $sig);
		)*
	};
}


pub(crate) enum SignatureTreatment {
	NoOffset,
	OffsetByInt(isize),
	OffsetByCall
}

pub(crate) struct Signature {
	pub treatment: SignatureTreatment,
	pub bytes: &'static [Option<u8>]
}

impl Signature {
	pub fn find(&self, scanner: &Scanner) -> Option<*const std::ffi::c_void> {
		scanner.find(&self.bytes).map(|address| unsafe {
			match self.treatment {
				SignatureTreatment::NoOffset | SignatureTreatment::OffsetByInt(0) => std::mem::transmute(address as *const std::ffi::c_void),
				SignatureTreatment::OffsetByInt(i) => *(address.offset(i) as *const *const std::ffi::c_void),
				SignatureTreatment::OffsetByCall => {
					let offset = *(address.offset(1) as *const isize);
					address.offset(5).offset(offset) as *const () as *const std::ffi::c_void
				}
			}
		})
	}
}

pub(crate) enum SignatureMap {
	AllVersions(Signature),
	VersionDependent(Vec<((Bound<&'static u32>, Bound<&'static u32>), Signature)>)
}

impl SignatureMap {
	pub fn find(&self, scanner: &Scanner, version: u32) -> Option<*const std::ffi::c_void> {
		match self {
			Self::AllVersions(signature) => signature.find(scanner),
			Self::VersionDependent(map) => {
				map.iter().find(|(version_range, _)| {
					version_range.contains(&version)
				}).and_then(|(_, signature)| {
					signature.find(scanner)
				})
			}
		}
	}
}
