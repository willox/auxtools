//! BYOND function and global patterns for `byondcore.dll`.
//!
//! Patterns were checked on every build in their ranges. Before 516.1659,
//! each recipe was checked by resolving it against those builds.

use byond_scan::{Anchor, Extract, OperandSelect, Recipe, SignatureTreatment, VersionRange};

// Checked on every build in this range. Older builds are refused by the
// resolver, newer ones are tried and can still fail.
const SUPPORTED: VersionRange = VersionRange { min: 1637, max: 1688 };
// The patterns that changed with 516 (and alists, which came with it)
const SINCE_516: VersionRange = VersionRange { min: 1648, max: 1688 };
// What 515 has in their place. Those for the variables and lists are from
// auxtools before byond-scan, which found them on 515.
const BYOND_515: VersionRange = VersionRange { min: 1637, max: 1647 };

// One loader mask finds the variable-name table. Operands that don't use a
// register are count=0, global_values=1, the sizing lea=3 and variable_names=4.
// The lea is why variable_names is not index 3.
const GLOBAL_VARS_SIG: &str = "A3 ?? ?? ?? ?? 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 4B 01 00 00 A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? 8D 34 85 00 00 00 \
                               00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 2D 01 00 00 33 DB A3 ?? ?? ?? ?? 39 1D";

// The prologue tells this queue access apart from its dequeue clone. Operands
// are front=0, back=1, buffer=2.
const SUSPENDED_SIG: &str =
	"55 8B EC 56 8B 75 08 56 80 4E 04 04 E8 ?? ?? ?? ?? 8B 0D ?? ?? ?? ?? 83 C4 04 3B 0D ?? ?? ?? ?? 73 ?? A1 ?? ?? ?? ?? 8B 04 88";

// The procdef accessor has a 44-byte stride, which sets it apart from the other
// table accessors.
const PROCDEF_ACCESSOR_SIG: &str = "55 8B EC 8B 45 08 3B 05 ?? ?? ?? ?? 72 04 33 C0 5D C3 6B C0 2C 03 05";
// The branch distance varies, so it is wildcarded and the mask keeps a longer
// tail.
const STRING_ACCESSOR_SIG: &str = "55 8B EC 8B 4D 08 3B 0D ?? ?? ?? ?? 73 ?? A1 ?? ?? ?? ?? 8B 04 88 85 C0 0F 85";

const ALIST_SIG: &str = "3B 35 ?? ?? ?? ?? 73 ?? A1 ?? ?? ?? ?? C7 04 B0 00 00 00 00 46 3B 35 ?? ?? ?? ?? 72 EB";

pub(crate) const RECIPES: &[Recipe] = &[
	Recipe {
		name: "get_proc_array_entry",
		versions: SUPPORTED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, PROCDEF_ACCESSOR_SIG),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_table_entry",
		versions: SUPPORTED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, STRING_ACCESSOR_SIG),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_id",
		versions: SUPPORTED,
		// mkstr, where string interning goes in
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 8B 45 ?? 83 EC ?? 53 56 8B 35 ?? ?? ?? ?? 57 85 C0 75 ?? 68 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_proc_by_id",
		versions: SUPPORTED,
		// the frame size is wildcarded because the compiler picks it
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 81 EC ?? ?? ?? ?? A1 ?? ?? ?? ?? 33 C5 89 45 ?? 8B 55 ?? 8B 45 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_datum_proc_by_name",
		versions: SUPPORTED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC ?? 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 75 1C 8D 45 \
			 F3 8B 7D 18 8B 5D 10 6A 00"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_variable",
		versions: SINCE_516,
		// the 0x0C frame size has to stay pinned: without it the mask matches
		// one of several common SEH prologues. A changed frame should fail as
		// NotFound.
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 0C 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 5D ?? 8B 75 \
			 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_variable",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 24 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 4D ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "inc_ref_count",
		versions: SUPPORTED,
		// no prologue worth masking, so anchor on a call site and follow it
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? FF ?? ?? FF ?? ?? E8 ?? ?? ?? ?? 8D ?? ?? 56 E8"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "dec_ref_count",
		versions: SUPPORTED,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 C4 0C 81 FF FF FF 00 00 74 ?? 85 FF 74"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_assoc_element",
		versions: SUPPORTED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 51 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 5D 08 80"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_assoc_element",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC ?? 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 5D ?? 80 FB \
			 0F"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "create_list",
		versions: SUPPORTED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 8B 0D ?? ?? ?? ?? 56 85 C9 74 1B A1 ?? ?? ?? ?? 49 89 0D ?? ?? ?? ?? 8B 34 88 81 FE FF FF 00 00"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "append_to_list",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 8B 4D ?? 0F B6 C1 48 56 57 83 F8 54 0F 87 ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "remove_from_list",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 83 EC 08 53 8B 5D ?? 0F B6 C3 48 56 57 83 F8 54 0F 87 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_length",
		versions: SUPPORTED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 8B 4D ?? 83 EC ?? 0F B6 C1 48 53 56 57 83 F8 ?? 0F 87 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "value_is_list",
		versions: SINCE_516,
		// this is the function the `islist()` opcode calls. The exported
		// `ByondValue_IsList` is not used because on 1659 it says no to a
		// `filters` list. The prologue is a common one, and the tag-range check
		// at the end (`83 C0 F1 83 F8 46`) is what makes it unique
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC ?? 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 89 65 ?? 8B 45 \
			 08 0F B6 C0 83 C0 F1 83 F8 46"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_misc_by_id",
		versions: SUPPORTED,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 C4 04 85 C0 74 08 0F B7 38 8B 70 08 EB 04 33 FF 33 F6 0F B7 C7 50 89 45 F8 E8 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "to_string",
		versions: SUPPORTED,
		// the prologue changes at 1686 and any shared prefix is ambiguous, so
		// follow this caller instead. It is unique across the whole range.
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 C4 08 8B F0 EB ?? 80 FB 2A 75 ?? 66 83 7D F8"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "runtime",
		versions: SUPPORTED,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 C4 04 8B 85 ?? ?? ?? ?? 0F B6 C0 51 66 0F 6E C0 0F 5B C0"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "current_execution_context",
		versions: SUPPORTED,
		// the `A1` load from an absolute address has no base register, which
		// makes it operand 0
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"A1 ?? ?? ?? ?? 56 53 6A 00 8B 00 57 6A ?? 89 4D ?? FF 70 ?? 8B 4D ?? FF"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	// BYOND reallocates this per world load, so it is looked up again on every
	// partial init.
	Recipe {
		name: "variable_names",
		versions: SUPPORTED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, GLOBAL_VARS_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(4))
	},
	// the start of the sleeping-proc queue: front, then back, then a capacity
	Recipe {
		name: "suspended_procs",
		versions: SUPPORTED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, SUSPENDED_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "suspended_procs_buffer",
		versions: SUPPORTED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, SUSPENDED_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	},
	// the alist accessor's base-less operands are count=0, table=1, and the
	// repeated count=2
	Recipe {
		name: "alist_table_count",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, ALIST_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "alist_table_ptr",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, ALIST_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "get_variable",
		versions: BYOND_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 28 A1 ?? ?? ?? ?? 33 C5 89 45 F0 53 56 57 50 8D 45 F4 64 A3 00 00 00 00 8B 5D"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_variable",
		versions: BYOND_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 0C 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 4D"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_assoc_element",
		versions: BYOND_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 14 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 5D 08 80"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "append_to_list",
		versions: BYOND_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 8B 4D 08 0F B6 C1 48 56 57 83 F8 53 0F 87 AA 00 00 00 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 0C E8 ?? ?? ?? ?? 8B F0 \
			 83 C4 04 85 F6 0F 84 94 00 00 00 8B 46 0C 40 50 56 E8 ?? ?? ?? ?? 8B 7E 0C 83 C4 08 85"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "remove_from_list",
		versions: BYOND_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 83 EC 08 53 8B 5D 08 0F B6 C3 48 56 57 83 F8 53 0F 87 23 01 00 00 0F B6 80 ?? ?? ?? ?? 8B 4D 10 FF 24 85 ?? ?? ?? ?? 8B 7D 0C \
			 6A 0F 57 53 E8 ?? ?? ?? ?? 50 E8 ?? ?? ?? ?? 83 C4 10 85 C0 0F 84 02 01 00 00 8B 08 8B 40 0C 85 C0 0F 84 F5 00 00 00 8B 75 14 8B 5D 10"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "value_is_list",
		versions: BYOND_515,
		// what the `islist()` opcode calls in 515: the same shape as 516's,
		// with one list tag fewer (`83 F8 45`)
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 51 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 89 65 F0 8B 4D 08 0F \
			 B6 C1 83 C0 F1 83 F8 45"
		),
		hops: &[],
		extract: Extract::Entry
	}
];
