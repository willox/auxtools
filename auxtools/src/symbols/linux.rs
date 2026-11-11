//! BYOND function and global patterns for `libbyond.so`.
//!
//! Patterns were checked on every build in their ranges. Before 516.1659,
//! each recipe was checked by resolving it against those builds.

use byond_scan::{Anchor, Extract, OperandSelect, Recipe, SignatureTreatment, VersionRange};

// Checked on every build in this range. Older builds are refused by the
// resolver, newer ones are tried and can still fail.
//
// Before 1664 GCC inlined `call_proc_by_id` into the interpreter, so a DM
// proc call never enters the function the hook patches: proc hooks there go
// through the debug server's instruction hook instead (see
// `hooks::calls_bypass_detour`).
const SUPPORTED: VersionRange = VersionRange { min: 1648, max: 1688 };
// The patterns that also match 515 from 515.1644, which GCC compiled much as
// it did 516
const FROM_515_1644: VersionRange = VersionRange { min: 1644, max: 1688 };
// What 515.1644 to 515.1647 have in place of the others
const LATE_515: VersionRange = VersionRange { min: 1644, max: 1647 };
// 515.1637 to 515.1643, which GCC compiled with frame pointers. The patterns
// marked "auxtools before byond-scan" are the ones it had for these builds.
const EARLY_515: VersionRange = VersionRange { min: 1637, max: 1643 };

// The first build where `remove_from_list` takes its arguments on the stack.
// Every build before it wants the list in `eax:edx`. Read off the prologue of
// every build from 1669 to 1675.
pub(crate) const REMOVE_FROM_LIST_STACK_BUILD: u32 = 1674;

// One loader mask finds the variable-name table. Operands that don't use a
// register are count=0, global_values=1 and variable_names=2.
const GLOBAL_VARS_SIG: &str = "89 1D ?? ?? ?? ?? 57 E8 ?? ?? ?? ?? 83 C4 10 85 FF 74 08 85 C0 0F 84 ?? ?? ?? ?? 83 EC 0C C1 E3 02 A3 ?? ?? ?? ?? 53 \
                               E8 ?? ?? ?? ?? 83 C4 10 85 DB 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ??";

// The prefix tells this enqueue apart from the dequeue clones. Operands are
// back=0, front=1, buffer=2. The two index globals sit the other way round from
// Windows, so back is 4 bytes below front.
// The resume time is at +0x8C of a ProcInstance from 516, +0x88 before.
const SUSPENDED_SIG: &str = "55 57 89 C7 56 53 83 EC ?? 8B 0D ?? ?? ?? ?? 80 48 04 04 8B A8 ?? 00 00 00 A1 ?? ?? ?? ?? 89 4C 24 ?? 8B 35 ?? ?? ?? ??";

// `runtime` opens with a load of the current execution context, so the same
// mask finds both.
const RUNTIME_SIG: &str =
	"55 89 E5 56 53 83 EC ?? 8B 15 ?? ?? ?? ?? 85 D2 0F 84 ?? ?? ?? ?? 0F B6 42 6F 3C 01 76 ?? 31 C9 84 C0 C7 42 10 ?? ?? ?? ?? 0F 94 C3";

// 515's frame flag byte is at +0x6D (`0F B6 42 6D`), 516's at +0x6F
const RUNTIME_SIG_515: &str = "55 89 E5 56 53 83 EC 30 8B 15 ?? ?? ?? ?? 85 D2 0F 84 ?? ?? ?? ?? 0F B6 42 6D";

const PROCDEF_ACCESSOR_SIG: &str = "8B 44 24 04 39 05 ?? ?? ?? ?? 76 ?? 6B C0 ?? 03 05 ?? ?? ?? ?? C3";
const STRING_ACCESSOR_SIG: &str = "56 53 83 EC 14 8B 44 24 20 39 05 ?? ?? ?? ?? 76 ?? 8B 15 ?? ?? ?? ?? 8B 04 82";

// The 0x0F/0x55 tag dispatch identifies the accessor that reads the alist
// table. Operands are count=0 and pointer=1.
const ALIST_SIG: &str =
	"8B 54 24 04 8B 4C 24 08 80 FA 0F 74 ?? 31 C0 80 FA 55 75 ?? 3B 0D ?? ?? ?? ?? 73 ?? A1 ?? ?? ?? ?? 8B 04 88 85 C0 74 ?? 8B 00 C3";

pub(crate) const RECIPES: &[Recipe] = &[
	Recipe {
		name: "get_proc_array_entry",
		versions: FROM_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, PROCDEF_ACCESSOR_SIG),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_table_entry",
		versions: FROM_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, STRING_ACCESSOR_SIG),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_id",
		versions: FROM_515_1644,
		// frameless mkstr, pinned by the comparison against 0x01
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 57 56 89 CE 53 89 D3 83 EC ?? 8B 54 24 ?? 88 54 24 ?? 85 C0 0F 84 ?? ?? ?? ?? 89 44 24 ?? 84 DB 0F 88 ?? ?? ?? ?? 88 5C 24 ?? 80 FB \
			 01 0F 84 ?? ?? ?? ?? 89 F0 84 C0"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_proc_by_id",
		versions: FROM_515_1644,
		// the frame that builds a call record and drops into the interpreter.
		// Every `mov reg,[esp+disp]` argument load has its displacement
		// wildcarded because they all move with the frame.
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 31 C0 57 56 53 81 EC ?? ?? ?? ?? F3 0F 7E 8C 24 ?? ?? ?? ?? 8B 8C 24 ?? ?? ?? ?? 8A 84 24 ?? ?? ?? ?? 8B 9C 24 ?? ?? ?? ?? F3 0F 7E \
			 84 24"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_datum_proc_by_name",
		versions: FROM_515_1644,
		// positive `ebp` displacements are incoming arguments and stay pinned.
		// The negative ones are locals and move with the frame.
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 31 C9 89 E5 57 56 8D 45 ?? 53 83 EC ?? F3 0F 7E 45 0C 8B 5D 14 C6 45 ?? 00 8B 55 18 8B 75 08 6A 00 66 0F D6 45 ?? 0F B6 FB 66 0F 7E \
			 45 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_variable",
		versions: SUPPORTED,
		// `3C 58` is `cmp al, 58h`, the tag bound of the 89-case switch, and it
		// is what makes this unique
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC ?? F3 0F 7E 4D 0C 8B 75 08 8B 5D 14 66 0F 6F C1 66 0F 7E CA 66 0F 73 D0 20 0F B6 C2 66 0F 7E 45 ?? 3C 58 77 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_variable",
		versions: SUPPORTED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC ?? F3 0F 7E 55 08 8B 7D 10 66 0F 6F C2 66 0F 7E D2 66 0F 73 D0 20 89 D3 66 0F 7E C6 F3 0F 7E 45 14 89 75 ?? 66 \
			 0F 6F C8"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "inc_ref_count",
		versions: FROM_515_1644,
		// `80 F9 4A` is `cmp cl, 4Ah`, the tag table bound. The `66 0F 7E C0`
		// before it is the only thing telling this apart from dec_ref_count in
		// the first 48 bytes.
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 56 53 83 EC ?? F3 0F 7E 4D 08 66 0F 6F C1 66 0F 7E C9 66 0F 73 D0 20 66 0F 7E C0 80 F9 4A 77 ?? 0F B6 C9 FF 24 8D ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "dec_ref_count",
		versions: SUPPORTED,
		// The fifth call from exported ByondValue_DecRef. A shifted call chain
		// fails silently, so this index was also checked against a direct mask.
		anchor: Anchor::Export(c"ByondValue_DecRef"),
		hops: &[4],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_assoc_element",
		versions: SUPPORTED,
		// splits from set_assoc_element at +0x0E: `8B 7D` here against the
		// setter's `66 0F 6F`. Everything before that is identical or
		// wildcarded.
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC ?? F3 0F 7E 4D 0C 8B 7D 08 66 0F 6F C1 66 0F 7E C8 66 0F 7E 4D ?? 66 0F 73 D0 20 66 0F 7E 45 ?? 3C 3C 0F 84 ?? \
			 ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_assoc_element",
		versions: FROM_515_1644,
		// the prologue changes after 1659, so follow this caller. It is unique
		// across the range, and its alignment padding is wildcarded.
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 C4 20 E9 ?? ?? ?? ?? ?? ?? ?? ?? 31 C9 EB ?? D9 43 04 BA 01 00 00 00"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "create_list",
		versions: FROM_515_1644,
		// `81 FB FF FF 00 00` is `cmp ebx, 0FFFFh`, the failure sentinel. It is
		// what tells this apart from the `new` dispatcher, which opens the same
		// way.
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 57 56 53 83 EC ?? A1 ?? ?? ?? ?? 8B 7C 24 ?? 85 C0 0F 84 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 83 E8 01 A3 ?? ?? ?? ?? 8B 1C 82 81 FB FF FF \
			 00 00"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "append_to_list",
		versions: SUPPORTED,
		// the Value arrives in registers, so the head is a run of
		// movd/punpckldq instead of stack loads. It opens with `56` where
		// remove_from_list opens with `55`.
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"56 66 0F 6E CA 66 0F 6E C0 53 66 0F 62 C1 66 0F 7E C6 66 0F 6F D0 66 0F 73 D0 20 89 F1 83 EC ?? 66 0F 7E C0 F3 0F 7E 4C 24 ?? 66 0F 6F \
			 C1"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "remove_from_list",
		versions: SUPPORTED,
		// the prologue mask only covers up to 1669 and the head changes at
		// 1675, so follow this caller, which is unique on every build. Its
		// `ebp` displacements are wildcarded.
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 85 ?? ?? ?? ?? 01 8B 8D ?? ?? ?? ?? 83 C4 10 83 85 ?? ?? ?? ?? 08 39"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_length",
		versions: SUPPORTED,
		// the tag-switch gate of the real length helper. An older mask resolved
		// value_is_list instead.
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC ?? F3 0F 7E 4D 0C 8B 5D 08 66 0F 7E CA 83 EA 0C 80 FA 4B 77 ?? 66 0F 6F C1 0F B6 D2 66 0F 73 D0 20 66 0F 7E C0 \
			 FF 24 95 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "value_is_list",
		versions: SUPPORTED,
		// this is the function the `islist()` opcode calls. The exported
		// `ByondValue_IsList` is not used because on Windows 1659 it says no to
		// a `filters` list, and both platforms should ask the same function.
		// `80 FA 55` is `cmp dl, 55h`, the highest tag it accepts
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 56 53 83 EC ?? F3 0F 7E 4D 08 8B 75 10 66 0F 7E CA 80 FA 55 0F 87 ?? ?? ?? ?? 80 FA 3B 76 ?? 8D 4A C4 B8 01 00 00 00 D3 E0 89 \
			 C1"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_misc_by_id",
		versions: FROM_515_1644,
		// the plain accessor body matches 17 times, so follow a unique caller
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 89 85 ?? ?? ?? ?? 83 C4 10 31 DB 85 C0 74 03 0F B7 18 8B 85 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "to_string",
		versions: SUPPORTED,
		// the 89-case interning switch. Stops before unstable alignment
		// padding.
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC ?? 0F B6 45 08 3C 45 77 ?? 3C 27 77 ?? 8D 50 FF 80 FA 26 77 ?? FF 24 85 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "runtime",
		versions: SUPPORTED,
		// throws through __cxa_throw here, where Windows uses
		// _CxxThrowException
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RUNTIME_SIG),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "current_execution_context",
		versions: SUPPORTED,
		// `runtime` opens with a load of this global, and nothing touches
		// memory before that, so it is operand 0
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RUNTIME_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	// BYOND reallocates this per world load, so it is looked up again on every
	// partial init.
	Recipe {
		name: "variable_names",
		versions: FROM_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, GLOBAL_VARS_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	},
	// the start of the sleeping-proc queue: back, then front
	Recipe {
		name: "suspended_procs",
		versions: FROM_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, SUSPENDED_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "suspended_procs_buffer",
		versions: FROM_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, SUSPENDED_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	},
	Recipe {
		name: "alist_table_count",
		versions: SUPPORTED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, ALIST_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "alist_table_ptr",
		versions: SUPPORTED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, ALIST_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "get_variable",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 5C F3 0F 7E 4D 0C 8B 75 08 66 0F 6F C1 66 0F 7E C8"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_variable",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 7C F3 0F 7E 55 08 8B 75 10 66 0F 6F C2 66 0F 7E D0"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_assoc_element",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 3C F3 0F 7E 45 0C 8B 7D 08 0F 29 45 B8 66 0F 7E C0"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "append_to_list",
		versions: LATE_515,
		// the list in `eax:edx`, as on 516
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"56 66 0F 6E CA 66 0F 6E C0 53 66 0F 62 C1 66 0F 6F C8 66 0F 7E C6 66 0F 73 D1 20"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "remove_from_list",
		versions: LATE_515,
		// the list in `eax:edx`, as on 516 before 1674
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 66 0F 6E CA 66 0F 6E C0 57 66 0F 62 C1 56 66 0F 6F C8 66 0F 7E C7 53"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_length",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 2C F3 0F 7E 4D 0C 8B 5D 08 66 0F 7E C8 3C 39 77 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "value_is_list",
		versions: LATE_515,
		// one list tag fewer than 516 (`80 FA 54`)
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 56 53 83 EC 20 F3 0F 7E 4D 08 8B 75 10 66 0F 7E CA 80 FA 54 0F 87 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "runtime",
		versions: LATE_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RUNTIME_SIG_515),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "current_execution_context",
		versions: LATE_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RUNTIME_SIG_515),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "to_string",
		versions: LATE_515,
		// the same interning switch, over 515's tags
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 3C 0F B6 45 08 3C 23 77 ?? 84 C0 74 ?? 3C 23 77 ?? FF 24 85 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "dec_ref_count",
		versions: LATE_515,
		// 515's ByondValue_DecRef doesn't call it, so by its prologue: the
		// 0x4A tag bound of its switch, as on 516
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 56 53 83 EC 20 F3 0F 7E 4D 08 66 0F 6F C1 66 0F 7E C9 66 0F 73 D0 20 66 0F 7E C3 80 F9 4A"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_proc_array_entry",
		versions: EARLY_515,
		// auxtools before byond-scan
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 0F B7 F6 89 C7 89 B5 ?? ?? ?? ?? 89 34 24 E8 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_table_entry",
		versions: EARLY_515,
		// auxtools before byond-scan
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 18 8B 45 ?? 39 05 ?? ?? ?? ?? 76 ?? 8B 15 ?? ?? ?? ?? 8B 04 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_id",
		versions: EARLY_515,
		// auxtools before byond-scan
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 89 CE 53 89 D3 83 EC 5C 8B 55 ?? 85 C0 88 55 ?? 0F 84 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_proc_by_id",
		versions: EARLY_515,
		// what CallByNameWithCallback calls with a proc id
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 81 EC ?? ?? 00 00 0F B6 45 14 81 7D ?? ?? ?? ?? 00 8B 55 0C 8B 4D 10"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_datum_proc_by_name",
		versions: EARLY_515,
		// CallByNameWithCallback, which the build has two identical copies of:
		// the one the export's helper calls
		anchor: Anchor::Export(c"Byond_CallProcByStrId"),
		hops: &[11, 2],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_variable",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 81 EC ?? ?? 00 00 8B 55 0C 89 5D F4 8B 5D 10 89 75 F8 8B 75 08 89 7D FC 8B 7D 14"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_variable",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 81 EC ?? ?? 00 00 89 5D F4 8B 5D 08 8B 45 0C 89 75 F8 8B 55 18 89 7D FC 8B 75 10"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "inc_ref_count",
		versions: EARLY_515,
		// auxtools before byond-scan
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 8B 43 ?? 80 48 ?? 04 8B 5D ?? 8B 75 ?? 8B 7D ?? 89 EC 5D"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "dec_ref_count",
		versions: EARLY_515,
		// auxtools before byond-scan
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? C7 06 00 00 00 00 C7 46 ?? 00 00 00 00 A1 ?? ?? ?? ?? 0F B7 50 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_assoc_element",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 58 89 5D F4 8B 5D 0C 89 75 F8 8B 75 10 89 7D FC 80 FB 3C 89 DF 0F 84 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_assoc_element",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 68 89 5D F4 8B 5D 08 89 75 F8 8B 75 10 8B 45 14 89 7D FC 8B 7D 0C 80 FB 3C"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "create_list",
		versions: EARLY_515,
		// auxtools before byond-scan
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 2C A1 ?? ?? ?? ?? 8B 75 ?? 85 C0 0F 84 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "append_to_list",
		versions: EARLY_515,
		// auxtools before byond-scan; the list in `eax:edx`
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 38 3C 54 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ?? 89 7D ?? 76 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "remove_from_list",
		versions: EARLY_515,
		// auxtools before byond-scan; the list in `eax:edx`
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 48 3C 54 89 5D ?? 89 C3 89 75 ?? 8B 75 ?? 89 7D ?? 8B 7D ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_length",
		versions: EARLY_515,
		// auxtools before byond-scan; returns the count, not a Value
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 6C 8B 45 ?? 8B 5D ?? 3C 54 76 ?? 31 F6 8D 65 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "value_is_list",
		versions: EARLY_515,
		// what the list exports check first, as 516's do
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 48 89 5D F4 8B 5D 08 89 75 F8 ?? ?? ?? ?? 89 7D FC 8B 7D 0C 89 DE 8D 46 F1"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_misc_by_id",
		versions: EARLY_515,
		// auxtools before byond-scan
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 0F B7 55 ?? 03 1F 0F B7 4B ?? 89 8D ?? ?? ?? ?? 0F B7 5B ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "to_string",
		versions: EARLY_515,
		// auxtools before byond-scan
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 ?? ?? 89 5D F4 8D ?? ?? 89 75 F8 89 7D FC 80 ?? ?? ?? ?? ?? B8"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "runtime",
		versions: EARLY_515,
		// auxtools before byond-scan
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 31 C0 8D B4 26 00 00 00 00 8B 5D ?? 8B 75 ?? 8B 7D ?? 89 EC"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "current_execution_context",
		versions: EARLY_515,
		// auxtools before byond-scan
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"A1 ?? ?? ?? ?? C7 44 24 ?? 00 00 00 00 C7 44 24 ?? 00 00 00 00 89 74 24"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "variable_names",
		versions: EARLY_515,
		// auxtools before byond-scan
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"A1 ?? ?? ?? ?? 8B 13 8B 39 8B 75 ?? 8B 14 ?? 89 7D ?? 8B 3C ?? 83 EE 02"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "suspended_procs",
		versions: EARLY_515,
		// a proc made ready, checked against the queue: front=0, back=1,
		// buffer=2
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"E8 ?? ?? ?? ?? A1 ?? ?? ?? ?? 3B 05 ?? ?? ?? ?? 0F 83 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 3B 34 82 0F 85 ?? ?? ?? ?? 8B 8E 88 00 00 00"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "suspended_procs_buffer",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"E8 ?? ?? ?? ?? A1 ?? ?? ?? ?? 3B 05 ?? ?? ?? ?? 0F 83 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 3B 34 82 0F 85 ?? ?? ?? ?? 8B 8E 88 00 00 00"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	}
];
