//! Read a compiled world's resource table (resource IDs to CRCs).
//!
//! A `.dmb` has no table lengths. Walk the preceding tables in DreamMaker's
//! order to reach the final resource table. Check the string table's byte
//! count and checksum, and require the resource table to end at EOF.
//! Reject mismatches to avoid resolving an ID to the wrong file.
//!
//! Shared with auxtools-cli, which needs the same IDs for watched files.

/// Why a `.dmb`'s resource table couldn't be read
pub type Error = String;

struct Reader<'a> {
	data: &'a [u8],
	at: usize,
	large: bool
}

impl<'a> Reader<'a> {
	fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
		let end = self.at.checked_add(n).filter(|&end| end <= self.data.len());
		let Some(end) = end else {
			return Err(format!("unexpected end of table at byte {}", self.at));
		};
		let bytes = &self.data[self.at..end];
		self.at = end;
		Ok(bytes)
	}

	fn u8(&mut self) -> Result<u8, Error> {
		Ok(self.take(1)?[0])
	}

	fn u16(&mut self) -> Result<u16, Error> {
		Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
	}

	fn u32(&mut self) -> Result<u32, Error> {
		Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
	}

	/// An id or count: four bytes in a world with large ids, two otherwise
	fn word(&mut self) -> Result<u32, Error> {
		if self.large {
			self.u32()
		} else {
			self.u16().map(u32::from)
		}
	}

	fn words(&mut self, n: usize) -> Result<(), Error> {
		self.take(n * if self.large { 4 } else { 2 }).map(|_| ())
	}
}

/// Read `world bin vMAJOR` and `min compatibility vLHS [RHS]`, skipping
/// an optional executor `#!` line. Return the major and LHS versions,
/// header location and table location.
fn header(data: &[u8]) -> Result<(u32, u32, usize, usize), Error> {
	let bad = || "missing world header".to_owned();
	let line = |from: usize| -> Result<(&str, usize), Error> {
		let end = from + data[from..].iter().position(|&b| b == b'\n').ok_or_else(bad)?;
		Ok((std::str::from_utf8(&data[from..end]).map_err(|_| bad())?, end + 1))
	};
	let start = if data.starts_with(b"#!") { line(0)?.1 } else { 0 };
	let (first, next) = line(start)?;
	let (second, body) = line(next)?;
	let major = first.strip_prefix("world bin v").and_then(|v| v.trim().parse().ok()).ok_or_else(bad)?;
	let lhs = second
		.strip_prefix("min compatibility v")
		.and_then(|v| v.split(' ').next()?.trim().parse().ok())
		.ok_or_else(bad)?;
	Ok((major, lhs, start, body))
}

/// The CRC of each resource id of the world compiled into `data`
pub fn resource_crcs(data: &[u8]) -> Result<Vec<u32>, Error> {
	let (major, lhs, start, body) = header(data)?;
	let mut r = Reader {
		data,
		at: body,
		large: false
	};
	let flags = r.u32()?;
	if flags & 0x8000_0000 != 0 {
		r.u32()?;
	}
	r.large = flags & 0x4000_0000 != 0;

	// The map: its size, then runs of (turf, area, extra turfs, copies)
	let cells = u64::from(r.u16()?) * u64::from(r.u16()?) * u64::from(r.u16()?);
	let mut left = cells;
	while left > 0 {
		r.words(3)?;
		let copies = r.u8()?;
		if copies == 0 {
			return Err(format!("empty map run at byte {}", r.at));
		}
		left = left.saturating_sub(u64::from(copies));
	}
	let string_bytes = r.u32()?;

	// The types
	for _ in 0..r.word()? {
		r.words(6)?; // path, parent, name, desc, icon, icon_state
		r.u8()?; // dir
		if major >= 307 && r.u8()? == 0x0F {
			r.u32()?;
		}
		r.words(1)?; // text
		if lhs >= 494 {
			r.words(1)?; // maptext
			r.take(4)?; // its width and height
		}
		if lhs >= 508 {
			r.take(4)?; // maptext_x, maptext_y
		}
		r.words(1)?; // suffix
		if major >= 306 {
			r.u32()?;
			if lhs >= 514 {
				r.u32()?;
			}
		} else {
			r.u8()?;
		}
		r.words(5)?; // verbs, procs, init proc, initialized vars, vars
		if lhs >= 514 {
			r.words(1)?;
		}
		if major >= 267 {
			r.u32()?; // layer
		}
		if lhs >= 500 && r.u8()? != 0 {
			r.take(6 * 4)?; // transform
		}
		if lhs >= 509 && r.u8()? != 0 {
			r.take(20 * 4)?; // color matrix
		}
		if major >= 306 {
			r.words(1)?; // overriding vars
		}
	}

	// The mobs
	for _ in 0..r.word()? {
		r.words(2)?; // path, key
		if r.u8()? >= 0x80 {
			r.take(6)?; // sight flags, see_in_dark, see_invisible
		}
	}

	// Strings are obscured using their offset from the version header.
	// Length 0xFFFF continues with another length to add.
	let strings = r.word()?;
	let mut hash = 0xFFFF_FFFFu32;
	let mut total = 0u64;
	for _ in 0..strings {
		let mut length = 0usize;
		loop {
			let at = r.at - start;
			let part = r.u16()? ^ (at as u16);
			length += usize::from(part);
			if part != 0xFFFF {
				break;
			}
		}
		let mut key = (r.at - start) as u8;
		for &byte in r.take(length)? {
			hash = crc_byte(hash, byte ^ key);
			key = key.wrapping_add(9);
		}
		hash = crc_byte(hash, 0);
		total += length as u64 + 1;
	}
	if major >= 468 && r.u32()? != hash {
		return Err("string-table checksum mismatch; unsupported .dmb layout".to_owned());
	}
	if total != u64::from(string_bytes) {
		return Err("string-table size mismatch; unsupported .dmb layout".to_owned());
	}

	// The misc lists
	for _ in 0..r.word()? {
		let n = r.u16()?;
		r.words(usize::from(n))?;
	}

	// The procs
	for _ in 0..r.word()? {
		if r.large || major >= 224 {
			r.words(1)?; // path
		}
		r.words(3)?; // name, desc, category
		r.take(2)?; // verb src
		if r.u8()? & 0x80 != 0 {
			r.take(5)?;
		}
		r.words(3)?; // bytecode, locals, parameters
	}

	// The vars: (tag, data, name)
	for _ in 0..r.word()? {
		r.take(5)?;
		r.words(1)?;
	}
	if lhs >= 512 {
		r.u32()?;
	}

	// The world's init procs, then the instances: (tag, data, init proc)
	let init_procs = r.word()? as usize;
	r.words(init_procs)?;
	for _ in 0..r.word()? {
		r.take(5)?;
		r.words(1)?;
	}

	// The map's extra turfs: (cells since the last, turf)
	for _ in 0..r.u32()? {
		r.u16()?;
		r.words(1)?;
	}

	// World settings: mob, turf, area, name, etc. Skin precedes only
	// icon_size and map_format.
	r.words(7)?;
	r.u32()?;
	r.words(2)?;
	if lhs > 514 {
		r.take(4)?;
	}
	r.take(4)?;
	r.u8()?;
	r.words(1)?;
	let n = r.u16()?;
	r.words(usize::from(n))?;
	r.u16()?;
	r.words(2)?;
	r.take(8)?;
	r.u16()?;
	r.words(5)?;
	r.take(6)?;

	// The resources: (crc, type)
	let count = r.word()? as usize;
	let table = r.take(count.checked_mul(5).ok_or("too many resources")?)?;
	if r.at != data.len() {
		return Err(format!(
			"{} trailing bytes after the resource table; unsupported .dmb layout",
			data.len() - r.at
		));
	}
	Ok(table
		.chunks_exact(5)
		.map(|entry| u32::from_le_bytes(entry[..4].try_into().unwrap()))
		.collect())
}

fn crc_table() -> &'static [u32; 256] {
	static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
	TABLE.get_or_init(|| {
		let mut table = [0u32; 256];
		for (i, entry) in table.iter_mut().enumerate() {
			let mut crc = (i as u32) << 24;
			for _ in 0..8 {
				crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0xAF } else { crc << 1 };
			}
			*entry = crc;
		}
		table
	})
}

fn crc_byte(crc: u32, byte: u8) -> u32 {
	(crc << 8) ^ crc_table()[((crc >> 24) as u8 ^ byte) as usize]
}

/// BYOND CRC used for resource UIDs and `.dmb` strings: MSB first,
/// polynomial 0xAF, initialized to all ones, without final inversion.
pub fn byond_crc(data: &[u8]) -> u32 {
	data.iter().fold(0xFFFF_FFFF, |crc, &byte| crc_byte(crc, byte))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Minimal world with only settings, small IDs and `resources`.
	fn world(resources: &[(u32, u8)]) -> Vec<u8> {
		let mut dmb = b"world bin v516\nmin compatibility v514 468\n".to_vec();
		dmb.extend(0u32.to_le_bytes()); // flags
		dmb.extend([0; 6]); // a map of no cells
		dmb.extend(0u32.to_le_bytes()); // string bytes
		dmb.extend([0; 4]); // types, mobs
		dmb.extend([0; 2]); // strings
		dmb.extend(0xFFFF_FFFFu32.to_le_bytes()); // their checksum
		dmb.extend([0; 6]); // misc, procs, vars
		dmb.extend(0u32.to_le_bytes());
		dmb.extend([0; 4]); // init procs, instances
		dmb.extend(0u32.to_le_bytes()); // extra turfs
		dmb.extend([0; 14]);
		dmb.extend(100u32.to_le_bytes());
		dmb.extend([0; 4 + 5 + 2]);
		dmb.extend([0; 2]); // no client procs
		dmb.extend([0; 2 + 4 + 8 + 2 + 10 + 6]);
		dmb.extend((resources.len() as u16).to_le_bytes());
		for (crc, kind) in resources {
			dmb.extend(crc.to_le_bytes());
			dmb.push(*kind);
		}
		dmb
	}

	#[test]
	fn the_resource_table_follows_the_world_settings() {
		let dmb = world(&[(0x3B83BAEA, 3), (0xCA381640, 2)]);
		assert_eq!(resource_crcs(&dmb), Ok(vec![0x3B83BAEA, 0xCA381640]));
	}

	#[test]
	fn a_table_that_doesnt_end_the_file_isnt_read() {
		let mut dmb = world(&[(0x3B83BAEA, 3)]);
		dmb.push(0);
		assert!(resource_crcs(&dmb).unwrap_err().contains("1 trailing bytes after the resource table"));
		let mut dmb = world(&[(0x3B83BAEA, 3)]);
		dmb.pop();
		assert!(resource_crcs(&dmb).is_err());
	}

	#[test]
	fn the_crc_is_byonds() {
		assert_eq!(byond_crc(b""), 0xFFFF_FFFF);
		assert_eq!(crc_table()[1], 0xAF);
	}
}
