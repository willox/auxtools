#include <stdint.h>
#include "hooks.h"

#ifdef USE_SJLJ
jmp_buf *current_jmp;
#endif

//
// BYOND likes to use C++ exceptions for some stuff (like runtimes) - Rust can't catch them and code will just unroll back to before our hooks
// We use these wrappers to hackily handle that and let Rust know an exception happened instead of letting it propagate
//

#define DEFINE_byond(name, ret_type, params)     \
	using Fn##name##_byond = ret_type(*) params; \
	Fn##name##_byond name##_byond = nullptr;

#define DEFINE_byond_REGPARM2(name, ret_type, params)           \
	using Fn##name##_byond = ret_type(LINUX_REGPARM2 *) params; \
	Fn##name##_byond name##_byond = nullptr;

#define DEFINE_byond_REGPARM3(name, ret_type, params)           \
	using Fn##name##_byond = ret_type(LINUX_REGPARM3 *) params; \
	Fn##name##_byond name##_byond = nullptr;

#ifdef __MINGW32__

struct RestoreJmpBuf
{
	jmp_buf *to_restore;

	RestoreJmpBuf() : to_restore(current_jmp) {}
	~RestoreJmpBuf() { current_jmp = to_restore; }
	RestoreJmpBuf(const RestoreJmpBuf &) = delete;
	RestoreJmpBuf(RestoreJmpBuf &&) = delete;
	RestoreJmpBuf &operator=(const RestoreJmpBuf &) = delete;
	RestoreJmpBuf &operator=(RestoreJmpBuf &&) = delete;
};

#define BYOND_TRY              \
	RestoreJmpBuf restore;     \
	jmp_buf jmp;               \
	current_jmp = &jmp;        \
	int jmp_val = setjmp(jmp); \
	if (jmp_val == 0)
#define BYOND_CATCH \
	else

#else

#define BYOND_TRY try
#define BYOND_CATCH catch (AuxtoolsException _)

#endif

// The REGPARM markers are real on Linux and expand to nothing on Windows, so a
// wrong one here is a Linux-only bug that no Windows test can catch. These were
// read off libbyond.so's function prologues on 516.1669 and 516.1687.
//
// How to check one: does the function read `eax` or `edx` before writing it in
// its first handful of instructions? If so it takes arguments in registers and
// needs a marker. If it opens by reading `[esp+arg_N]` it is plain cdecl and must
// not have one. The export table can't tell you, none of these are in it.
//
// Two things make this easy to get wrong:
//
//   * A Value-returning regparm function spends `eax` on the hidden return
//     buffer, so its first real argument starts at `edx`. Count the buffer.
//   * The convention is per-function and per-build. `remove_from_list` changed
//     from registers to the stack at 1674 with no change to its signature,
//     while its identically-shaped sibling `append_to_list` did not move. Don't
//     infer one row from another.
extern "C"
{
	DEFINE_byond(call_proc_by_id, Value, (Value, uint32_t, uint32_t, uint32_t, Value, const Value *, uint32_t, uint32_t, uint32_t));
	DEFINE_byond(call_datum_proc_by_name, Value, (Value, uint32_t, uint32_t, Value, const Value *, uint32_t, uint32_t, uint32_t));
	DEFINE_byond(get_proc_array_entry, void *, (uint32_t));
	DEFINE_byond_REGPARM3(get_string_id, uint32_t, (const char *, uint8_t, uint8_t, uint8_t));
	DEFINE_byond(get_variable, Value, (Value, uint32_t));
	DEFINE_byond(set_variable, void, (Value, uint32_t, Value));
	DEFINE_byond(get_string_table_entry, void *, (uint32_t));
	DEFINE_byond(inc_ref_count, void, (Value));
	DEFINE_byond(dec_ref_count, void, (Value));
	DEFINE_byond(get_assoc_element, Value, (Value, Value));
	DEFINE_byond(set_assoc_element, void, (Value, Value, Value));
	DEFINE_byond(create_list, uint32_t, (uint32_t));
	DEFINE_byond_REGPARM2(append_to_list, void, (Value, Value));
	// The one function known to have changed convention mid-range, so it gets
	// both declarations. Linux builds up to 516.1673 take the list in `eax:edx`,
	// and 516.1674 on take everything on the stack. Rust sets the flag from the
	// running build, and the `remove_from_list` wrapper below picks the matching
	// pointer type.
	DEFINE_byond(remove_from_list, bool, (Value, Value));
	using Fnremove_from_list_regparm_byond = bool(LINUX_REGPARM2 *)(Value, Value);
	bool remove_from_list_in_registers = false;
	// The one function here whose *return* differs by platform, not just its
	// argument placement. Windows hands the count straight back in `eax`. Linux
	// returns a DM Value through a caller-supplied buffer, so the count has to be
	// unpacked out of it. See the `get_length` wrapper below.
#ifdef _WIN32
	DEFINE_byond(get_length, uint32_t, (Value));
#else
	DEFINE_byond(get_length, Value, (Value));
#endif
	// BYOND's own `islist()`. The last argument says whether a pointer to a list
	// counts as a list.
	DEFINE_byond(value_is_list, bool, (Value, uint8_t));
	DEFINE_byond(get_misc_by_id, void *, (uint32_t));
	DEFINE_byond(to_string, uint32_t, (Value));
}

#ifndef _WIN32
// Unpack the DM Value Linux's `get_length` returns into the plain count every
// caller here wants. Tag 0x2A is Number and its data half is an `f32`. Anything
// else means the argument had no length, which BYOND itself reports as 0.
//
// The tag is masked to a byte on purpose: BYOND leaves the three padding bytes
// above a tag indeterminate, so comparing the whole dword answers wrong at
// random.
static uint32_t length_value_to_count(Value length)
{
	if ((length.type & 0xFF) != 0x2A)
	{
		return 0;
	}
	float count;
	__builtin_memcpy(&count, &length.value, sizeof(count));
	if (!(count > 0.0f))
	{
		return 0;
	}
	return static_cast<uint32_t>(count);
}
#endif

extern "C" uint8_t call_proc_by_id(
	Value *out,
	Value usr,
	uint32_t proc_type,
	uint32_t proc_id,
	uint32_t override_depth,
	Value src,
	const Value *args,
	uint32_t args_count,
	uint32_t callback,
	uint32_t callback_value)
{
	RuntimeContext ctx(false);

	BYOND_TRY
	{
		*out = call_proc_by_id_byond(usr, proc_type, proc_id, override_depth, src, args, args_count, callback, callback_value);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t call_datum_proc_by_name(
	Value *out,
	Value usr,
	uint32_t proc_type,
	uint32_t proc_name,
	Value src,
	Value *args,
	uint32_t args_count,
	uint32_t callback,
	uint32_t callback_value)
{
	// Intercepts, unlike `call_proc_by_id` above. The errors this function raises
	// itself (an unknown proc name, a null receiver) happen before any DM code
	// runs, and with `false` BYOND's own exception flew straight past the catch and
	// killed the process at the first Rust frame. The proc it ends up calling still
	// reports its own errors normally: `call_proc_by_id_hook_trampoline` pushes
	// `false` around every DM call.
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(usr);
		clean(src);
		for (uint32_t i = 0; i < args_count; i++)
		{
			clean(args[i]);
		}
		*out = call_datum_proc_by_name_byond(usr, proc_type, proc_name, src, args, args_count, callback, callback_value);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t get_proc_array_entry(void **out, uint32_t id)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		*out = get_proc_array_entry_byond(id);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t get_string_id(uint32_t *out, const char *data)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		*out = get_string_id_byond(data, 0, 0, 1);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t get_variable(Value *out, Value datum, uint32_t string_id)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(datum);
		*out = get_variable_byond(datum, string_id);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t set_variable(Value datum, uint32_t string_id, Value value)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(datum);
		clean(value);
		set_variable_byond(datum, string_id, value);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t get_string_table_entry(void **out, uint32_t string_id)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		*out = get_string_table_entry_byond(string_id);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t inc_ref_count(Value value)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(value);
		inc_ref_count_byond(value);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t dec_ref_count(Value value)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(value);
		dec_ref_count_byond(value);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t get_assoc_element(Value *out, Value datum, Value index)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(datum);
		clean(index);
		*out = get_assoc_element_byond(datum, index);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t set_assoc_element(Value datum, Value index, Value value)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(datum);
		clean(index);
		clean(value);
		set_assoc_element_byond(datum, index, value);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t create_list(uint32_t *out, uint32_t reserve_capacity)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		*out = create_list_byond(reserve_capacity);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t append_to_list(Value list, Value value)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(list);
		clean(value);
		append_to_list_byond(list, value);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t remove_from_list(Value list, Value value)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(list);
		clean(value);
		if (remove_from_list_in_registers)
		{
			reinterpret_cast<Fnremove_from_list_regparm_byond>(remove_from_list_byond)(list, value);
		}
		else
		{
			remove_from_list_byond(list, value);
		}
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t get_length(uint32_t *out, Value value)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(value);
#ifdef _WIN32
		*out = get_length_byond(value);
#else
		*out = length_value_to_count(get_length_byond(value));
#endif
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t value_is_list(uint8_t *out, Value value)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(value);
		// 0 is what the `islist()` opcode passes, so a pointer to a list is not a list
		*out = value_is_list_byond(value, 0);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t get_misc_by_id(void **out, uint32_t index)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		*out = get_misc_by_id_byond(index);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}

extern "C" uint8_t to_string(uint32_t *out, Value value)
{
	RuntimeContext ctx(true);

	BYOND_TRY
	{
		clean(value);
		*out = to_string_byond(value);
		return 1;
	}
	BYOND_CATCH
	{
		return 0;
	}
}
