#[cfg(unix)]
mod linux;
#[cfg(windows)]
mod windows;

use std::ops::{Bound, RangeBounds};

pub use auxtools_impl::convert_signature;
#[cfg(unix)]
pub use linux::Scanner;
pub use once_cell;
#[cfg(windows)]
pub use windows::Scanner;

#[macro_export]
macro_rules! signature {
	($sig:tt) => {
		$crate::sigscan::convert_signature!($sig)
	};
}

#[macro_export]
macro_rules! count {
    () => (0usize);
    ( $x:tt $($xs:tt)* ) => (1_usize + count!($($xs)*));
}

#[macro_export]
macro_rules! signatures {
	( $( $name:ident => $sig:expr ),*) => {
		struct Signatures {
			$( pub $name: $crate::sigscan::SignatureMap, )*
		}

		impl Signatures {
			#[allow(dead_code)]
			pub fn check_all(&self, scanner: &$crate::sigscan::Scanner) -> [(&'static str, bool); count!($($name)*)] {
				let version = $crate::version::get().1;
				[$(
					(stringify!($name), self.$name.find(scanner, version).is_some()),
				)*]
			}
		}

		static SIGNATURES0: $crate::sigscan::once_cell::sync::Lazy<Signatures> = $crate::sigscan::once_cell::sync::Lazy::new(|| Signatures {
			$( $name: $sig, )*
		});
	};
}

#[macro_export]
macro_rules! signature_struct {
	(call, $sig:tt) => {
		$crate::sigscan::Signature {
			treatment: $crate::sigscan::SignatureTreatment::OffsetByCall,
			bytes: signature!($sig)
		}
	};
	($offset:literal, $sig:tt) => {
		$crate::sigscan::Signature {
			treatment: $crate::sigscan::SignatureTreatment::OffsetByInt($offset),
			bytes: signature!($sig)
		}
	};
	(($spec:tt, $sig:tt)) => {
		signature_struct!($spec, $sig)
	};
	($sig:tt) => {
		$crate::sigscan::Signature {
			treatment: $crate::sigscan::SignatureTreatment::NoOffset,
			bytes: signature!($sig)
		}
	};
}

#[macro_export]
macro_rules! version_dependent_signature {
	( $($range:expr => $sig:tt),* ) => {
		$crate::sigscan::SignatureMap::VersionDependent(vec![
			$(((std::ops::RangeBounds::start_bound(&$range), std::ops::RangeBounds::end_bound(&$range)), signature_struct!($sig)),)*
		])
	};
}

#[macro_export]
macro_rules! universal_signature {
	(call, $sig:tt) => {
		$crate::sigscan::SignatureMap::AllVersions(signature_struct!(call, $sig))
	};
	($offset:literal, $sig:tt) => {
		$crate::sigscan::SignatureMap::AllVersions(signature_struct!($offset, $sig))
	};
	($sig:tt) => {
		$crate::sigscan::SignatureMap::AllVersions(signature_struct!($sig))
	};
}

#[macro_export]
macro_rules! find_signature_inner {
	($scanner:ident, $name:ident, $type:ty) => {
		let $name: $type;
		if let Some(ptr) = SIGNATURES0.$name.find(&$scanner, $crate::version::get().1) {
			$name = ptr as $type;
		} else {
			return Some(format!("FAILED (Couldn't find {})", stringify!($name)));
		}
	};
}

#[macro_export]
macro_rules! find_signature_inner_result {
	($scanner:ident, $name:ident, $type:ty) => {
		let $name: $type;
		if let Some(ptr) = SIGNATURES0.$name.find(&$scanner, $crate::version::get().1) {
			$name = ptr as $type;
		} else {
			return Err(format!("FAILED (Couldn't find {})", stringify!($name)));
		}
	};
}

#[macro_export]
macro_rules! find_signature {
	($scanner:ident, $name:ident as $type:ty) => {
		find_signature_inner!($scanner, $name, $type);
	};

	($scanner:ident,($name:ident as $type:ty)) => {
		find_signature_inner!($scanner, $name, $type);
	};

	($scanner:ident, $name:ident) => {
		find_signature_inner!($scanner, $name, *const c_void);
	};
}

#[macro_export]
macro_rules! find_signature_result {
	($scanner:ident, $name:ident as $type:ty) => {
		find_signature_inner_result!($scanner, $name, $type);
	};

	($scanner:ident,($name:ident as $type:ty)) => {
		find_signature_inner_result!($scanner, $name, $type);
	};

	($scanner:ident, $name:ident) => {
		find_signature_inner_result!($scanner, $name, *const c_void);
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

#[macro_export]
macro_rules! find_signatures_result {
	($scanner:ident, $($sig:tt),* ) => {
		$(
			find_signature_result!($scanner, $sig);
		)*
	};
}

pub enum SignatureTreatment {
	NoOffset,
	OffsetByInt(isize),
	OffsetByCall
}

pub struct Signature {
	pub treatment: SignatureTreatment,
	pub bytes: &'static [Option<u8>]
}

impl Signature {
	pub fn find(&self, scanner: &Scanner) -> Option<*const std::ffi::c_void> {
		scanner.find(self.bytes).map(|address| unsafe {
			match self.treatment {
				SignatureTreatment::NoOffset | SignatureTreatment::OffsetByInt(0) => address as *const std::ffi::c_void,
				SignatureTreatment::OffsetByInt(i) => (address.offset(i) as *const *const std::ffi::c_void).read_unaligned(),
				SignatureTreatment::OffsetByCall => {
					let offset = (address.offset(1) as *const isize).read_unaligned();
					address.offset(5).offset(offset) as *const () as *const std::ffi::c_void
				}
			}
		})
	}
}

pub enum SignatureMap {
	AllVersions(Signature),
	VersionDependent(Vec<((Bound<&'static u32>, Bound<&'static u32>), Signature)>)
}

impl SignatureMap {
	pub fn find(&self, scanner: &Scanner, version: u32) -> Option<*const std::ffi::c_void> {
		match self {
			Self::AllVersions(signature) => signature.find(scanner),
			Self::VersionDependent(map) => map
				.iter()
				.find(|(version_range, _)| version_range.contains(&version))
				.and_then(|(_, signature)| signature.find(scanner))
		}
	}
}
