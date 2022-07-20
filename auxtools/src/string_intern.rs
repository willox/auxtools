use ahash::AHashMap;
use once_cell::unsync::OnceCell;
use std::cell::UnsafeCell;

use crate::inventory;
use crate::StringRef;

#[macro_export]
macro_rules! byond_string {
	($s:literal) => {{
		$crate::inventory::submit!($crate::string_intern::InternedString { 0: $s });
		let _x = $crate::string_intern::INTERNED_STRINGS.with(|cell| {
			let map = cell
				.get()
				.expect("Interned string is uninitialized, is this access on the main thread?");
			map.get($s).unwrap().get()
		});
		unsafe { _x.as_ref().unwrap().as_ref().unwrap() }
	}};
}

thread_local! {
	pub static INTERNED_STRINGS: OnceCell<AHashMap<&'static str, UnsafeCell<Option<StringRef>>>> = OnceCell::new()
}

#[doc(hidden)]
pub struct InternedString(pub &'static str);

inventory::collect!(InternedString);

pub fn setup_interned_strings() {
	INTERNED_STRINGS.with(|thing| {
		let map = thing.get_or_init(|| {
			let mut map = AHashMap::new();
			for info in inventory::iter::<InternedString> {
				map.insert(info.0, UnsafeCell::new(None));
			}
			map
		});
		for (k, v) in map.iter() {
			let string = StringRef::new(k).expect("failed to create interned string");
			unsafe {
				*v.get().as_mut().unwrap() = Some(string);
			}
		}
	});
}

pub fn destroy_interned_strings() {
	INTERNED_STRINGS.with(|thing| {
		let map = thing.get().unwrap();
		for (_, v) in map.iter() {
			unsafe {
				*v.get().as_mut().unwrap() = None;
			}
		}
	});
}
