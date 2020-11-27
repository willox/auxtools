#include <stdint.h>
#include "hooks.h"

// The type of the func defined in Byond
using Runtime_Ptr = void(*)(char *pError);
using CallProcById_Ptr = Value(LINUX_REGPARM3 *)(Value, uint32_t, uint32_t, uint32_t, Value, Value*, uint32_t, uint32_t, uint32_t);

// The type of the hook defined in hooks.rs
using CallProcById_Hook_Ptr = Value(*)(Value, uint32_t, uint32_t, uint32_t, Value, Value*, uint32_t, uint32_t, uint32_t);

extern "C" {
	// The ptr everybody else sees
	Runtime_Ptr runtime_byond = nullptr;

	// The original function - set by rust after hooking
	Runtime_Ptr runtime_original = nullptr;
	CallProcById_Ptr call_proc_by_id_original = nullptr;
}

// If the top of this stack is true, we replace byond's runtime exceptions with our own
std::stack<bool> runtime_contexts({false});

extern "C" void on_runtime(const char* pError);

extern "C" void runtime_hook(char* pError) {
	const char* pErrorCorrected = (pError != nullptr) ? pError : "<null>";
	if (runtime_contexts.top()) {
#ifdef USE_SJLJ
		longjmp(*current_jmp, 1);
#else
		throw AuxtoolsException(pErrorCorrected);
#endif
		return;
	}

	on_runtime(pErrorCorrected);
	return runtime_original(pError);
}

extern "C" uint8_t call_proc_by_id_hook(
	Value* ret,
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
	Value ret;

	if (call_proc_by_id_hook(&ret, usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2)) {
		clean(ret);
		return ret;
	} else {
		return call_proc_by_id_original(usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2);
	}
	//return call_proc_by_id_hook(usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2);
}
