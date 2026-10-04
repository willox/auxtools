//! Where auxtools finds its BYOND functions and globals in `libbyond.so`.
//!
//! Copied from `byond_catalog` in the byond-re repo, where `verify_recipes`
//! checks each pattern against every local build. Keep the two in step.

use byond_scan::{Anchor, Extract, OperandSelect, Recipe, SignatureTreatment, VersionRange};

// Checked on every build in this range. Older builds are refused by the
// resolver, newer ones are tried and can still fail.
//
// The floor is 1664 and not Windows' 1659 because of proc hooks. On 1659 GCC
// inlined `call_proc_by_id` into the interpreter, so a DM proc call never enters
// the function the hook patches and no `#[hook]` ever fires. 1660 to 1663 were
// never checked.
const SUPPORTED: VersionRange = VersionRange { min: 1664, max: 1688 };

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
const SUSPENDED_SIG: &str = "55 57 89 C7 56 53 83 EC ?? 8B 0D ?? ?? ?? ?? 80 48 04 04 8B A8 8C 00 00 00 A1 ?? ?? ?? ?? 89 4C 24 ?? 8B 35 ?? ?? ?? ??";

// `runtime` opens with a load of the current execution context, so the same
// mask finds both.
const RUNTIME_SIG: &str =
	"55 89 E5 56 53 83 EC ?? 8B 15 ?? ?? ?? ?? 85 D2 0F 84 ?? ?? ?? ?? 0F B6 42 6F 3C 01 76 ?? 31 C9 84 C0 C7 42 10 ?? ?? ?? ?? 0F 94 C3";

const PROCDEF_ACCESSOR_SIG: &str = "8B 44 24 04 39 05 ?? ?? ?? ?? 76 ?? 6B C0 ?? 03 05 ?? ?? ?? ?? C3";
const STRING_ACCESSOR_SIG: &str = "56 53 83 EC 14 8B 44 24 20 39 05 ?? ?? ?? ?? 76 ?? 8B 15 ?? ?? ?? ?? 8B 04 82";

// The 0x0F/0x55 tag dispatch identifies the accessor that reads the alist
// table. Operands are count=0 and pointer=1.
const ALIST_SIG: &str = "8B 54 24 04 8B 4C 24 08 80 FA 0F 74 ?? 31 C0 80 FA 55 75 ?? 3B 0D ?? ?? ?? ?? 73 ?? A1 ?? ?? ?? ?? 8B 04 88 85 C0 74 ?? 8B 00 C3";

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
		versions: SUPPORTED,
		// the frame that builds a call record and drops into the interpreter. Every
		// `mov reg,[esp+disp]` argument load has its displacement wildcarded because
		// they all move with the frame.
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
		versions: SUPPORTED,
		// positive `ebp` displacements are incoming arguments and stay pinned. The
		// negative ones are locals and move with the frame.
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
		// `3C 58` is `cmp al, 58h`, the tag bound of the 89-case switch, and it is
		// what makes this unique
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
		versions: SUPPORTED,
		// `80 F9 4A` is `cmp cl, 4Ah`, the tag table bound. The `66 0F 7E C0` before
		// it is the only thing telling this apart from dec_ref_count in the first 48
		// bytes.
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
		// the fifth call out of the exported ByondValue_DecRef. A shifted call chain
		// fails silently, so byond-re checks this index with a direct mask as well.
		anchor: Anchor::Export(c"ByondValue_DecRef"),
		hops: &[4],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_assoc_element",
		versions: SUPPORTED,
		// splits from set_assoc_element at +0x0E: `8B 7D` here against the setter's
		// `66 0F 6F`. Everything before that is identical or wildcarded.
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
		versions: SUPPORTED,
		// the prologue changes after 1659, so follow this caller. It is unique across
		// the range, and its alignment padding is wildcarded.
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 C4 20 E9 ?? ?? ?? ?? ?? ?? ?? ?? 31 C9 EB ?? D9 43 04 BA 01 00 00 00"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "create_list",
		versions: SUPPORTED,
		// `81 FB FF FF 00 00` is `cmp ebx, 0FFFFh`, the failure sentinel. It is what
		// tells this apart from the `new` dispatcher, which opens the same way.
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
		// the Value arrives in registers, so the head is a run of movd/punpckldq
		// instead of stack loads. It opens with `56` where remove_from_list opens
		// with `55`.
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
		// the prologue mask only covers up to 1669 and the head changes at 1675, so
		// follow this caller, which is unique on every build. Its `ebp`
		// displacements are wildcarded.
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
		// `ByondValue_IsList` is not used because on Windows 1659 it says no to a
		// `filters` list, and both platforms should ask the same function.
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
		versions: SUPPORTED,
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
		// the 89-case interning switch. Stops before unstable alignment padding.
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
		// throws through __cxa_throw here, where Windows uses _CxxThrowException
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RUNTIME_SIG),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "current_execution_context",
		versions: SUPPORTED,
		// `runtime` opens with a load of this global, and nothing touches memory
		// before that, so it is operand 0
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RUNTIME_SIG),
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
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	},
	// the start of the sleeping-proc queue: back, then front
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
	}
];
