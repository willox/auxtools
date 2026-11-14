//! Update appearances after resource reloads.
//!
//! Clients retain resource IDs for a whole session, so edited files need
//! new IDs. Update appearances referencing the old IDs and resend them to
//! every connection to trigger a redraw. This scans the appearance table.
//!
//! Appearances are interned in a content-ordered tree. Remove changed nodes
//! with BYOND's erase function and reinsert them using its comparator and
//! node layout. BYOND's insert allocates a new slot, so it cannot reinsert
//! an existing appearance. Recalculate bounds for changed appearances and
//! their transitive overlay/underlay users; bounds do not affect tree order.
//!
//! Validate all required functions, globals and tree invariants before
//! changing anything, then validate the tree again afterwards. Patterns
//! were checked on Windows and Linux 515.1637, 515.1647, 516.1659, 516.1685
//! and 516.1688. Reject newer builds until checked.

use std::{
	collections::{HashMap, HashSet},
	sync::OnceLock
};

use byond_scan::{Anchor, Extract, OperandSelect, Recipe, SignatureTreatment, VersionRange};

/// Appearance fields, the same on Windows and Linux: the tree links (left,
/// right, parent, then the next larger and next smaller in content order), the
/// id, the tree color (0 red, 1 black, 0xFFFF a free slot), the icon and the
/// overlays and underlays.
const LEFT: usize = 0x0;
const RIGHT: usize = 0x4;
const PARENT: usize = 0x8;
const LARGER: usize = 0xC;
const SMALLER: usize = 0x10;
const ID: usize = 0x14;
const COLOR: usize = 0x18;
const ICON: usize = 0x30;
/// Misc IDs of overlay and underlay appearance-ID lists.
const OVERLAYS: usize = 0x38;
const UNDERLAYS: usize = 0x3C;
/// Reserved appearance ID. Once reached, BYOND inserts a blank tree node
/// with this ID but leaves it out of the ID table.
const UNUSED_ID: usize = 0xFFFF;
const RED: u16 = 0;
const BLACK: u16 = 1;
const FREE: u16 = 0xFFFF;

/// The manager's intern tree header (root, smallest, largest), its id table
/// and the table's capacity
const TREE: usize = 0x14;
const TABLE: usize = 0x40;
const CAPACITY: usize = 0x44;

/// `mouse_over_pointer`, `mouse_drag_pointer` and `mouse_drop_pointer`, three
/// Values (a tag byte, then the data at +4) that can each be an icon. 516.1648
/// added `icon_w` and `icon_z` before them.
const POINTERS_515: usize = 0x70;
const POINTERS_516: usize = 0x74;
const ICON_W_BUILD: u32 = 1648;
/// Newest validated build. Unlink and rebalance are identified by call
/// position in erase, so matching patterns alone is insufficient for
/// newer builds. Reject them until checked.
const NEWEST_CHECKED_BUILD: u32 = 1688;
const RESOURCE_TAG: u8 = 0x0C;

#[cfg(windows)]
mod recipes {
	use super::*;

	const OLD: VersionRange = VersionRange { min: 1637, max: 1647 };
	const NEW: VersionRange = VersionRange { min: 1648, max: 1688 };
	const ALL: VersionRange = VersionRange { min: 1637, max: 1688 };

	// appearance_send_to_client(u16 connection, u32 appearance, int force),
	// cdecl. Looks up the connection, then the appearance.
	const SEND_OLD: &str = "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 81 EC E4 08 00 00 A1 ?? ?? ?? ?? 33 C5 89 45 F0 53 56 57 50 8D 45 F4 \
	                        64 A3 00 00 00 00 0F B7";
	const SEND_NEW: &str = "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 81 EC E4 08 00 00 A1 ?? ?? ?? ?? 33 C5 89 45 F0 53 56 57 50 8D 45 F4";
	// The comparator, thiscall (a, b) -> int, starts with the name and the icon
	const COMPARE_OLD: &str =
		"55 8B EC 56 8B F1 57 8B 7D 08 8B 56 1C 2B 57 1C 0F 85 ?? ?? ?? ?? 8B 56 30 2B 57 30 0F 85 ?? ?? ?? ?? 8B 56 34 2B 57 34";
	const COMPARE_NEW: &str =
		"55 8B EC 53 56 8B F1 57 8B 7D 08 8B 56 1C 2B 57 1C 0F 85 ?? ?? ?? ?? 8B 56 30 2B 57 30 0F 85 ?? ?? ?? ?? 8B 56 34 2B 57";
	// appearance_update_bounds, thiscall (appearance)
	const BOUNDS_OLD: &str =
		"55 8B EC 83 EC 0C 8D 45 F4 56 57 6A 00 68 FF FF 00 00 50 8B F1 E8 ?? ?? ?? ?? 80 BE B1 00 00 00 00 8D BE C8 00 00 00 F3";
	const BOUNDS_NEW: &str =
		"55 8B EC 83 EC 0C 8D 45 F4 56 57 6A 00 68 FF FF 00 00 50 8B F1 E8 ?? ?? ?? ?? 80 BE B5 00 00 00 00 8D BE CC 00 00 00 F3";
	// After the "unique atomic images" error in appearance_intern, slot
	// release calls unlink first, then rebalance. Both are thiscall;
	// unlink(tree, node) returns a node, rebalance(tree, node) returns void.
	const RELEASE: &str = "68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 0C 8D 4F 14 56 E8 ?? ?? ?? ?? B8 FF FF 00 00 EB 50 8B 47 44 8B 5E 14 89 85 F8 FE FF";

	pub const SEND: [Recipe; 2] = [
		Recipe {
			name: "appearance_send",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_OLD),
			hops: &[],
			extract: Extract::Entry
		},
		Recipe {
			name: "appearance_send",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_NEW),
			hops: &[],
			extract: Extract::Entry
		}
	];
	pub const CONNECTION_COUNT: [Recipe; 2] = [
		Recipe {
			name: "connection_count",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_OLD),
			hops: &[0],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
		},
		Recipe {
			name: "connection_count",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_NEW),
			hops: &[0],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
		}
	];
	pub const CONNECTIONS: [Recipe; 2] = [
		Recipe {
			name: "connections",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_OLD),
			hops: &[0],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
		},
		Recipe {
			name: "connections",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_NEW),
			hops: &[0],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
		}
	];
	pub const MANAGER: [Recipe; 2] = [
		Recipe {
			name: "appearance_manager",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_OLD),
			hops: &[1],
			extract: Extract::AbsMem(OperandSelect::FirstAbsMem)
		},
		Recipe {
			name: "appearance_manager",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_NEW),
			hops: &[1],
			extract: Extract::AbsMem(OperandSelect::FirstAbsMem)
		}
	];
	pub const COMPARE: [Recipe; 2] = [
		Recipe {
			name: "appearance_compare",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, COMPARE_OLD),
			hops: &[],
			extract: Extract::Entry
		},
		Recipe {
			name: "appearance_compare",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, COMPARE_NEW),
			hops: &[],
			extract: Extract::Entry
		}
	];
	pub const BOUNDS: [Recipe; 2] = [
		Recipe {
			name: "appearance_update_bounds",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, BOUNDS_OLD),
			hops: &[],
			extract: Extract::Entry
		},
		Recipe {
			name: "appearance_update_bounds",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, BOUNDS_NEW),
			hops: &[],
			extract: Extract::Entry
		}
	];
	pub const UNLINK: [Recipe; 1] = [Recipe {
		name: "appearance_tree_unlink",
		versions: ALL,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RELEASE),
		hops: &[1, 0],
		extract: Extract::Entry
	}];
	pub const REBALANCE: [Recipe; 1] = [Recipe {
		name: "appearance_tree_rebalance",
		versions: ALL,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RELEASE),
		hops: &[1, 1],
		extract: Extract::Entry
	}];
}

#[cfg(unix)]
mod recipes {
	use super::*;

	const OLD: VersionRange = VersionRange { min: 1637, max: 1643 };
	const NEW: VersionRange = VersionRange { min: 1647, max: 1688 };

	// Send ABI: EAX = u16 connection, EDX = appearance, ECX = force.
	// Looks up the connection, then the appearance.
	const SEND_OLD: &str = "55 0F B7 C0 89 E5 57 56 53 89 D3 81 EC 2C 09 00 00 89 04 24 89 8D 74 F7 FF FF 89 85 4C F7 FF FF E8 ?? ?? ?? ?? 85 C0 89";
	const SEND_NEW: &str = "55 0F B7 C0 89 E5 57 56 89 CE 53 89 D3 81 EC F8 04 00 00 89 85 30 FB FF FF 50 E8 ?? ?? ?? ?? 83 C4 10 89 85 24 FB FF FF";
	// The comparator, cdecl (a, b) -> int, starts with the name and the icon
	const COMPARE_OLD: &str =
		"55 89 E5 83 EC 28 89 75 F8 8B 75 08 89 5D F4 8B 5D 0C 89 7D FC 8B 46 1C 2B 43 1C 75 08 8B 46 30 2B 43 30 74 13 8B 5D F4";
	const COMPARE_NEW: &str =
		"55 57 56 53 83 EC 1C 8B 74 24 30 8B 5C 24 34 8B 46 1C 2B 43 1C 75 42 8B 46 30 2B 43 30 75 3A 8B 46 34 2B 43 34 75 32 0F";
	// appearance_update_bounds, cdecl (appearance)
	const BOUNDS_OLD: &str =
		"55 89 E5 83 EC 38 89 5D F4 8B 5D 08 8D 45 D8 89 04 24 89 75 F8 89 7D FC C7 44 24 0C 00 00 00 00 C7 44 24 08 FF FF 00 00";
	const BOUNDS_NEW: &str =
		"55 57 56 53 83 EC 1C 8B 5C 24 30 89 E0 6A 00 68 FF FF 00 00 53 50 E8 ?? ?? ?? ?? 8B 4C 24 0C 89 8B ?? 00 00 00 8B 54 24";
	// After the "unique atomic images" error in appearance_intern, slot
	// release calls unlink first, then rebalance second (515) or fourth
	// (later). Both are cdecl; unlink(tree, node) returns a node,
	// rebalance(tree, node) returns void.
	const RELEASE_OLD: &str =
		"C7 04 24 ?? ?? ?? ?? E8 ?? ?? ?? ?? 8B 95 F4 FE FF FF 89 34 24 89 54 24 04 E8 ?? ?? ?? ?? 89 F8 8B 5D F4 8B 75 F8 8B 7D";
	const RELEASE_NEW: &str =
		"68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 58 5A 53 57 E8 ?? ?? ?? ?? 83 C4 10 C7 85 F4 FE FF FF FF FF 00 00 8B 85 F4 FE FF FF EB 28";

	pub const SEND: [Recipe; 2] = [
		Recipe {
			name: "appearance_send",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_OLD),
			hops: &[],
			extract: Extract::Entry
		},
		Recipe {
			name: "appearance_send",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_NEW),
			hops: &[],
			extract: Extract::Entry
		}
	];
	pub const CONNECTION_COUNT: [Recipe; 2] = [
		Recipe {
			name: "connection_count",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_OLD),
			hops: &[0],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
		},
		Recipe {
			name: "connection_count",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_NEW),
			hops: &[0],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
		}
	];
	pub const CONNECTIONS: [Recipe; 2] = [
		Recipe {
			name: "connections",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_OLD),
			hops: &[0],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
		},
		Recipe {
			name: "connections",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_NEW),
			hops: &[0],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
		}
	];
	pub const MANAGER: [Recipe; 2] = [
		Recipe {
			name: "appearance_manager",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_OLD),
			hops: &[1],
			extract: Extract::AbsMem(OperandSelect::FirstAbsMem)
		},
		Recipe {
			name: "appearance_manager",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SEND_NEW),
			hops: &[1],
			extract: Extract::AbsMem(OperandSelect::FirstAbsMem)
		}
	];
	pub const COMPARE: [Recipe; 2] = [
		Recipe {
			name: "appearance_compare",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, COMPARE_OLD),
			hops: &[],
			extract: Extract::Entry
		},
		Recipe {
			name: "appearance_compare",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, COMPARE_NEW),
			hops: &[],
			extract: Extract::Entry
		}
	];
	pub const BOUNDS: [Recipe; 2] = [
		Recipe {
			name: "appearance_update_bounds",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, BOUNDS_OLD),
			hops: &[],
			extract: Extract::Entry
		},
		Recipe {
			name: "appearance_update_bounds",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, BOUNDS_NEW),
			hops: &[],
			extract: Extract::Entry
		}
	];
	pub const UNLINK: [Recipe; 2] = [
		Recipe {
			name: "appearance_tree_unlink",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, RELEASE_OLD),
			hops: &[1, 0],
			extract: Extract::Entry
		},
		Recipe {
			name: "appearance_tree_unlink",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, RELEASE_NEW),
			hops: &[1, 0],
			extract: Extract::Entry
		}
	];
	pub const REBALANCE: [Recipe; 2] = [
		Recipe {
			name: "appearance_tree_rebalance",
			versions: OLD,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, RELEASE_OLD),
			hops: &[1, 1],
			extract: Extract::Entry
		},
		Recipe {
			name: "appearance_tree_rebalance",
			versions: NEW,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, RELEASE_NEW),
			hops: &[1, 3],
			extract: Extract::Entry
		}
	];
}

#[cfg(windows)]
type Compare = extern "thiscall" fn(*const u8, *const u8) -> i32;
#[cfg(windows)]
type TreeFn = extern "thiscall" fn(*mut u8, *mut u8) -> *mut u8;
#[cfg(windows)]
type Bounds = extern "thiscall" fn(*mut u8);
#[cfg(unix)]
type Compare = extern "C" fn(*const u8, *const u8) -> i32;
#[cfg(unix)]
type TreeFn = extern "C" fn(*mut u8, *mut u8) -> *mut u8;
#[cfg(unix)]
type Bounds = extern "C" fn(*mut u8);

/// Functions and globals required for resource reloads, resolved once.
struct Tables {
	/// The global holding the appearance manager
	manager: usize,
	connection_count: usize,
	connections: usize,
	send: usize,
	compare: Compare,
	unlink: TreeFn,
	rebalance: TreeFn,
	bounds: Bounds,
	pointers: usize
}

static TABLES: OnceLock<Result<Tables, String>> = OnceLock::new();

fn find() -> Result<Tables, String> {
	use recipes::*;
	let build = crate::byond_build()?;
	if build > NEWEST_CHECKED_BUILD {
		return Err(format!(
			"build {build} is newer than the latest validated resource-reload build ({NEWEST_CHECKED_BUILD})"
		));
	}
	let find = |recipes: &[Recipe]| crate::find_recipe_of(recipes);
	let tables = unsafe {
		Tables {
			manager: find(&MANAGER)?,
			connection_count: find(&CONNECTION_COUNT)?,
			connections: find(&CONNECTIONS)?,
			send: find(&SEND)?,
			compare: std::mem::transmute::<usize, Compare>(find(&COMPARE)?),
			unlink: std::mem::transmute::<usize, TreeFn>(find(&UNLINK)?),
			rebalance: std::mem::transmute::<usize, TreeFn>(find(&REBALANCE)?),
			bounds: std::mem::transmute::<usize, Bounds>(find(&BOUNDS)?),
			pointers: if build >= ICON_W_BUILD { POINTERS_516 } else { POINTERS_515 }
		}
	};
	if tables.unlink as usize == tables.rebalance as usize {
		return Err("Could not distinguish appearance-tree unlink and rebalance functions".to_owned());
	}
	Ok(tables)
}

fn tables() -> Result<&'static Tables, String> {
	TABLES
		.get_or_init(find)
		.as_ref()
		.map_err(|e| format!("resource reload unavailable on this BYOND build: {e}"))
}

/// Check required functions, globals and the appearance tree layout.
pub fn check() -> Result<(), String> {
	let tables = tables()?;
	unsafe { Appearances::new(tables)?.check().map_err(unexpected) }
}

/// Report a tree validation failure.
fn unexpected(why: String) -> String {
	format!("BYOND's appearance tree isn't laid out as expected ({why}); the resource reload changed nothing")
}

unsafe fn word(at: *mut u8, offset: usize) -> *mut *mut u8 {
	at.add(offset) as *mut *mut u8
}
unsafe fn link(node: *mut u8, offset: usize) -> *mut u8 {
	*word(node, offset)
}
unsafe fn set_link(node: *mut u8, offset: usize, to: *mut u8) {
	*word(node, offset) = to;
}
unsafe fn color(node: *mut u8) -> u16 {
	if node.is_null() {
		BLACK
	} else {
		*(node.add(COLOR) as *const u16)
	}
}
unsafe fn set_color(node: *mut u8, color: u16) {
	*(node.add(COLOR) as *mut u16) = color;
}

/// The appearance table: the manager, its tree header and its id table
struct Appearances<'a> {
	tables: &'a Tables,
	tree: *mut *mut u8,
	table: *mut *mut u8,
	capacity: usize
}

impl<'a> Appearances<'a> {
	unsafe fn new(tables: &'a Tables) -> Result<Self, String> {
		let manager = *(tables.manager as *const *mut u8);
		if manager.is_null() {
			return Err("the appearance manager isn't set up yet".to_owned());
		}
		Ok(Appearances {
			tables,
			tree: manager.add(TREE) as *mut *mut u8,
			table: *(manager.add(TABLE) as *const *mut *mut u8),
			capacity: *(manager.add(CAPACITY) as *const u32) as usize
		})
	}

	unsafe fn live(&self) -> impl Iterator<Item = *mut u8> + '_ {
		(0..self.capacity).filter_map(move |id| {
			let node = *self.table.add(id);
			(!node.is_null() && color(node) != FREE).then_some(node)
		})
	}

	unsafe fn root(&self) -> *mut u8 {
		*self.tree
	}

	/// Validate live nodes, parent links, threaded traversal, header ends,
	/// comparator order and red-black invariants (black root, no adjacent red
	/// nodes, equal black height). Allow the reserved blank node absent
	/// from the ID table; never modify it. Return an error on failure.
	unsafe fn check(&self) -> Result<(), String> {
		let live = self.live().count();
		let bad = |why: &str| Err(why.to_owned());
		let root = self.root();
		if !root.is_null() && !link(root, PARENT).is_null() {
			return bad("the root has a parent");
		}
		if color(root) != BLACK {
			return bad("the root isn't black");
		}
		let mut black_height = None;
		// In order, without recursion
		let mut count = 0usize;
		let mut skipped_id = 0usize;
		let mut previous: *mut u8 = std::ptr::null_mut();
		let mut node = root;
		while !node.is_null() && !link(node, LEFT).is_null() {
			node = link(node, LEFT);
		}
		if node != *self.tree.add(1) {
			return bad("the smallest node isn't the header's");
		}
		while !node.is_null() {
			count += 1;
			if count > live + 1 {
				return bad("the tree has more nodes than there are appearances");
			}
			if link(node, SMALLER) != previous {
				return bad("the threaded list doesn't follow the tree");
			}
			let node_color = color(node);
			if node_color != RED && node_color != BLACK {
				return bad("a node is neither red nor black");
			}
			let mut leaf = false;
			for child in [LEFT, RIGHT] {
				let child = link(node, child);
				leaf |= child.is_null();
				if !child.is_null() && link(child, PARENT) != node {
					return bad("a child doesn't name its parent");
				}
				if node_color == RED && color(child) == RED {
					return bad("a red node has a red child");
				}
			}
			if leaf {
				// Deeper than any balanced tree with 2^32 nodes.
				let (mut blacks, mut depth, mut at) = (0, 0, node);
				while !at.is_null() && depth < 128 {
					blacks += usize::from(color(at) == BLACK);
					depth += 1;
					at = link(at, PARENT);
				}
				if *black_height.get_or_insert(blacks) != blacks || depth == 128 {
					return bad("the tree isn't balanced as a red-black tree is");
				}
			}
			if !previous.is_null() && self.compare(previous, node) > 0 {
				return bad("the nodes aren't in the comparator's order");
			}
			let id = *(node.add(ID) as *const u32) as usize;
			let listed = id < self.capacity && *self.table.add(id) == node;
			if !listed {
				let unlisted = id == UNUSED_ID && (id >= self.capacity || (*self.table.add(id)).is_null());
				if !unlisted || skipped_id > 0 {
					return bad("a node's id doesn't lead back to it");
				}
				skipped_id += 1;
			}
			previous = node;
			// In-order successor: the right subtree's leftmost node, or the
			// first ancestor reached from its left child.
			if !link(node, RIGHT).is_null() {
				node = link(node, RIGHT);
				while !link(node, LEFT).is_null() {
					node = link(node, LEFT);
				}
			} else {
				let mut child = node;
				node = link(node, PARENT);
				while !node.is_null() && link(node, RIGHT) == child {
					child = node;
					node = link(node, PARENT);
				}
			}
			if !node.is_null() && link(previous, LARGER) != node {
				return bad("the threaded list doesn't follow the tree");
			}
		}
		if previous != *self.tree.add(2) {
			return bad("the largest node isn't the header's");
		}
		if count - skipped_id != live {
			return bad("some appearances aren't in the tree");
		}
		Ok(())
	}

	unsafe fn compare(&self, a: *mut u8, b: *mut u8) -> i32 {
		(self.tables.compare)(a, b)
	}

	unsafe fn rotate(&self, x: *mut u8, left: bool) {
		let (toward, away) = if left { (LEFT, RIGHT) } else { (RIGHT, LEFT) };
		let y = link(x, away);
		set_link(x, away, link(y, toward));
		if !link(y, toward).is_null() {
			set_link(link(y, toward), PARENT, x);
		}
		let parent = link(x, PARENT);
		set_link(y, PARENT, parent);
		if parent.is_null() {
			*self.tree = y;
		} else if link(parent, LEFT) == x {
			set_link(parent, LEFT, y);
		} else {
			set_link(parent, RIGHT, y);
		}
		set_link(y, toward, x);
		set_link(x, PARENT, y);
	}

	/// Reinsert `node` by content: compare down the tree (equal goes left),
	/// update the threaded list beside its parent, then rebalance.
	unsafe fn insert(&self, node: *mut u8) {
		set_link(node, LEFT, std::ptr::null_mut());
		set_link(node, RIGHT, std::ptr::null_mut());
		set_color(node, RED);
		let root = self.root();
		if root.is_null() {
			set_link(node, PARENT, std::ptr::null_mut());
			set_link(node, LARGER, std::ptr::null_mut());
			set_link(node, SMALLER, std::ptr::null_mut());
			*self.tree = node;
			*self.tree.add(1) = node;
			*self.tree.add(2) = node;
			set_color(node, BLACK);
			return;
		}
		let mut parent = root;
		let mut order;
		loop {
			order = self.compare(node, parent);
			let child = link(parent, if order > 0 { RIGHT } else { LEFT });
			if child.is_null() {
				break;
			}
			parent = child;
		}
		set_link(node, PARENT, parent);
		if order > 0 {
			set_link(parent, RIGHT, node);
			let mut at = parent;
			while !link(at, LARGER).is_null() && self.compare(node, link(at, LARGER)) > 0 {
				at = link(at, LARGER);
			}
			let larger = link(at, LARGER);
			set_link(node, SMALLER, at);
			set_link(node, LARGER, larger);
			if larger.is_null() {
				*self.tree.add(2) = node;
			} else {
				set_link(larger, SMALLER, node);
			}
			set_link(at, LARGER, node);
		} else {
			set_link(parent, LEFT, node);
			let mut at = parent;
			while !link(at, SMALLER).is_null() && self.compare(link(at, SMALLER), node) > 0 {
				at = link(at, SMALLER);
			}
			let smaller = link(at, SMALLER);
			set_link(node, LARGER, at);
			set_link(node, SMALLER, smaller);
			if smaller.is_null() {
				*self.tree.add(1) = node;
			} else {
				set_link(smaller, LARGER, node);
			}
			set_link(at, SMALLER, node);
		}

		let mut z = node;
		while color(link(z, PARENT)) == RED {
			let parent = link(z, PARENT);
			let grand = link(parent, PARENT);
			let on_left = link(grand, LEFT) == parent;
			let uncle = link(grand, if on_left { RIGHT } else { LEFT });
			if color(uncle) == RED {
				set_color(parent, BLACK);
				set_color(uncle, BLACK);
				set_color(grand, RED);
				z = grand;
				continue;
			}
			if z == link(parent, if on_left { RIGHT } else { LEFT }) {
				z = parent;
				self.rotate(z, on_left);
			}
			let parent = link(z, PARENT);
			let grand = link(parent, PARENT);
			set_color(parent, BLACK);
			set_color(grand, RED);
			self.rotate(grand, !on_left);
		}
		set_color(self.root(), BLACK);
	}
}

/// Appearance updates made by `repoint`.
pub struct Repointed {
	/// Appearance IDs referencing new resources.
	pub appearances: Vec<u32>,
	/// Appearance count per new resource.
	pub counts: HashMap<u32, u32>,
	/// Mouse pointers among them
	pub pointers: usize,
	/// Appearances now identical to another node, leaving duplicate tree
	/// entries.
	pub warnings: Vec<String>
}

/// Replace resource references using `(old, new)` mappings, retaining
/// appearance IDs. Validate the tree before and after. An error leaves
/// appearances unchanged.
pub fn repoint(mappings: &[(u32, u32)]) -> Result<Repointed, String> {
	let tables = tables()?;
	let mut new_of: HashMap<u32, u32> = HashMap::new();
	for &(old, new) in mappings {
		if old != new {
			new_of.entry(old).or_insert(new);
		}
	}
	unsafe {
		let appearances = Appearances::new(tables)?;
		appearances.check().map_err(unexpected)?;
		let pointer = |node: *mut u8, n: usize| node.add(tables.pointers + 8 * n);
		let points = |node: *mut u8, n: usize| -> Option<u32> {
			let value = pointer(node, n);
			if *value != RESOURCE_TAG {
				return None;
			}
			new_of.get(&*(value.add(4) as *const u32)).copied()
		};
		let changing: Vec<*mut u8> = appearances
			.live()
			.filter(|&node| new_of.contains_key(&*(node.add(ICON) as *const u32)) || (0..3).any(|n| points(node, n).is_some()))
			.collect();
		if changing.is_empty() {
			return Ok(Repointed {
				appearances: vec![],
				counts: HashMap::new(),
				pointers: 0,
				warnings: vec![]
			});
		}
		// Find overlay and underlay users before changing appearances.
		let shown_in = appearances.shown_in();

		let tree = appearances.tree as *mut u8;
		for &node in &changing {
			let unlinked = (tables.unlink)(tree, node);
			if color(unlinked) != RED {
				(tables.rebalance)(tree, unlinked);
			}
		}
		let mut counts: HashMap<u32, u32> = HashMap::new();
		let mut pointers = 0;
		for &node in &changing {
			let icon = node.add(ICON) as *mut u32;
			let mut to = None;
			if let Some(&new) = new_of.get(&*icon) {
				*icon = new;
				to = Some(new);
			}
			for n in 0..3 {
				if let Some(new) = points(node, n) {
					*(pointer(node, n).add(4) as *mut u32) = new;
					pointers += 1;
					to = to.or(Some(new));
				}
			}
			if let Some(to) = to {
				*counts.entry(to).or_default() += 1;
			}
		}
		// Update all icons first, then bounds in dependency order.
		for node in appearances.bounds_order(&changing, &shown_in) {
			(tables.bounds)(node);
		}
		let mut warnings = vec![];
		for &node in &changing {
			appearances.insert(node);
			if let Some(same) = appearances.equal_neighbour(node) {
				warnings.push(format!(
					"appearance {} now duplicates appearance {}; both IDs are retained with their existing client appearances",
					*(node.add(ID) as *const u32),
					*(same.add(ID) as *const u32)
				));
			}
		}
		if let Err(why) = appearances.check() {
			warnings.push(format!(
				"appearance-tree validation failed after reload ({why}); restart the world to avoid incorrect behavior"
			));
		}
		Ok(Repointed {
			appearances: changing.iter().map(|&node| *(node.add(ID) as *const u32)).collect(),
			counts,
			pointers,
			warnings
		})
	}
}

impl Appearances<'_> {
	/// Index overlay and underlay users in one pass over the table.
	unsafe fn shown_in(&self) -> HashMap<u32, Vec<*mut u8>> {
		let mut shown_in: HashMap<u32, Vec<*mut u8>> = HashMap::new();
		for node in self.live() {
			let mut shown: Vec<u32> = [OVERLAYS, UNDERLAYS]
				.iter()
				.flat_map(|&list| crate::runtime_tables::misc_words(*(node.add(list) as *const u32)).unwrap_or_default())
				.collect();
			shown.sort_unstable();
			shown.dedup();
			for id in shown {
				shown_in.entry(id).or_default().push(node);
			}
		}
		shown_in
	}

	/// Topologically order changed appearances and their transitive users
	/// so each appearance follows its overlays and underlays.
	unsafe fn bounds_order(&self, changed: &[*mut u8], shown_in: &HashMap<u32, Vec<*mut u8>>) -> Vec<*mut u8> {
		let id = |node: *mut u8| *(node.add(ID) as *const u32);
		let mut affected: Vec<*mut u8> = changed.to_vec();
		let mut seen: HashSet<*mut u8> = affected.iter().copied().collect();
		let mut at = 0;
		while at < affected.len() {
			for &parent in shown_in.get(&id(affected[at])).into_iter().flatten() {
				if seen.insert(parent) {
					affected.push(parent);
				}
			}
			at += 1;
		}
		// Count affected dependencies and process those with none left first.
		let mut waiting: HashMap<*mut u8, usize> = affected.iter().map(|&node| (node, 0)).collect();
		for &node in &affected {
			for &parent in shown_in.get(&id(node)).into_iter().flatten() {
				*waiting.get_mut(&parent).unwrap() += 1;
			}
		}
		let mut ready: Vec<*mut u8> = affected.iter().copied().filter(|node| waiting[node] == 0).collect();
		let mut order = Vec::with_capacity(affected.len());
		while let Some(node) = ready.pop() {
			order.push(node);
			for &parent in shown_in.get(&id(node)).into_iter().flatten() {
				let count = waiting.get_mut(&parent).unwrap();
				*count -= 1;
				if *count == 0 {
					ready.push(parent);
				}
			}
		}
		// Include remaining nodes even if an unexpected cycle exists.
		if order.len() < affected.len() {
			let placed: HashSet<*mut u8> = order.iter().copied().collect();
			order.extend(affected.iter().copied().filter(|node| !placed.contains(node)));
		}
		order
	}

	/// Adjacent node with identical content, if any.
	unsafe fn equal_neighbour(&self, node: *mut u8) -> Option<*mut u8> {
		[link(node, SMALLER), link(node, LARGER)]
			.into_iter()
			.find(|&other| !other.is_null() && self.compare(node, other) == 0)
	}
}

/// Resend appearances to every connection, replacing client entries under
/// those IDs and triggering redraws. Return the connections reached.
pub fn resend(appearances: &[u32]) -> Result<usize, String> {
	let tables = tables()?;
	if appearances.is_empty() {
		return Ok(0);
	}
	unsafe {
		let count = *(tables.connection_count as *const u16) as u32;
		let connections = *(tables.connections as *const *const *mut u8);
		let mut reached = 0;
		for connection in 0..count {
			if connections.is_null() || (*connections.add(connection as usize)).is_null() {
				continue;
			}
			for &id in appearances {
				send(tables.send, connection, id);
			}
			reached += 1;
		}
		Ok(reached)
	}
}

#[cfg(windows)]
unsafe fn send(function: usize, connection: u32, appearance: u32) {
	let send: extern "cdecl" fn(u32, u32, i32) = std::mem::transmute(function);
	send(connection, appearance, 1);
}

/// EAX the connection, EDX the appearance, ECX force; the callee keeps EBX,
/// ESI, EDI and EBP. The stack is aligned for it as GCC expects.
#[cfg(unix)]
unsafe fn send(function: usize, connection: u32, appearance: u32) {
	std::arch::asm!(
		"mov edi, esp",
		"and esp, -16",
		"call {function}",
		"mov esp, edi",
		function = in(reg) function,
		out("edi") _,
		inout("eax") connection => _,
		inout("edx") appearance => _,
		inout("ecx") 1u32 => _,
		clobber_abi("C"),
	);
}
