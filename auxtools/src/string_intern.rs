use std::cell::UnsafeCell;

use crate::inventory;
use crate::StringRef;

#[macro_export]
macro_rules! byond_string {
	($s:literal) => {
		unsafe {
			static mut store: auxtools::InternedString =
				auxtools::InternedString($s, std::cell::UnsafeCell::new(None));
			auxtools::inventory::submit!(unsafe { &store });
			let x = &*store.1.get();
			x.clone().unwrap()
		}
	};
}

#[doc(hidden)]
pub struct InternedString(pub &'static str, pub UnsafeCell<Option<StringRef>>);

inventory::collect!(&'static InternedString);

pub fn setup_interned_strings() {
	for info in inventory::iter::<&'static InternedString> {
		let string = StringRef::new(info.0);

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

/*
impl From<&InternedString> for StringRef {
	fn from(interned: &InternedString) -> StringRef {
		unsafe {
			let inner = &*interned.1.get();
			match &inner {
				Some(inner) => inner.clone(),
				None => panic!("uninitialized InternedString"),
			}
		}
	}
}
*/
