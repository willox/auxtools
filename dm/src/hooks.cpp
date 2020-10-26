#include <stdint.h>

#ifdef _WIN32
#define LINUX_REGPARM3
#else
#define LINUX_REGPARM3 __attribute__((regparm(3)))
#endif

struct Value {
	uint32_t type;
	uint32_t value;
};

// The type of the func defined in Byond
using CallProcById_Ptr = Value(LINUX_REGPARM3 *)(Value, uint32_t, uint32_t, uint32_t, Value, Value*, uint32_t, uint32_t, uint32_t);

// The type of the hook defined in hooks.rs
using CallProcById_Hook_Ptr = Value(*)(Value, uint32_t, uint32_t, uint32_t, Value, Value*, uint32_t, uint32_t, uint32_t);

// The original function - set by rust after hooking
extern "C" CallProcById_Ptr call_proc_by_id_original = nullptr;

// A little function to handle the odd calling convention on Linux and pass-through to the original byond function
// Used on Windows too
extern "C" Value LINUX_REGPARM3 call_proc_by_id_original_trampoline(
	Value usr,
	uint32_t proc_type,
	uint32_t proc_id,
	uint32_t unk_0,
	Value src,
	Value* args,
	uint8_t args_count,
	uint32_t unk_1,
	uint32_t unk_2
) {
	return call_proc_by_id_original(usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2);
}

extern "C" Value call_proc_by_id_hook(
		Value usr,
	uint32_t proc_type,
	uint32_t proc_id,
	uint32_t unk_0,
	Value src,
	Value* args,
	uint8_t args_count,
	uint32_t unk_1,
	uint32_t unk_2);

// A little function to handle the odd calling convention on Linux and pass-through to our rust hook
// Used on Windows too
extern "C" Value LINUX_REGPARM3 call_proc_by_id_hook_trampoline(
	Value usr,
	uint32_t proc_type,
	uint32_t proc_id,
	uint32_t unk_0,
	Value src,
	Value* args,
	uint8_t args_count,
	uint32_t unk_1,
	uint32_t unk_2
) {
	return call_proc_by_id_hook(usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2);
}
