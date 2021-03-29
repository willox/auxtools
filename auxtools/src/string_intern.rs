use std::cell::UnsafeCell;

use crate::inventory;
use crate::StringRef;

#[macro_export]
macro_rules! byond_string {
	($s:literal) => {
		unsafe {
			static mut STORE: $crate::InternedString =
				$crate::InternedString($s, std::cell::UnsafeCell::new(None));
			$crate::inventory::submit!(unsafe { &STORE });
			let x = &*STORE.1.get();
			x.as_ref().unwrap()
		}
	};
}

#[doc(hidden)]
pub struct InternedString(pub &'static str, pub UnsafeCell<Option<StringRef>>);

inventory::collect!(&'static InternedString);

pub fn setup_interned_strings() {
	for info in inventory::iter::<&'static InternedString> {
		let string = StringRef::new(info.0).expect("failed to create interned string");

		unsafe {
			let dst = &mut *info.1.get();
			*dst = Some(string);
		}
	}
}

pub fn destroy_interned_strings() {
	for info in inventory::iter::<&'static InternedString> {
		unsafe {
			let dst = &mut *info.1.get();
			*dst = None;
		}
	}
}
