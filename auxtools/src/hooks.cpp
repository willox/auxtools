#include <stdint.h>
#include "hooks.h"

// The type of the func defined in Byond
using Runtime_Ptr = void(*)(char *pError);
// Plain cdecl on both platforms. This carried regparm(3) on Linux, which was
// never right: `call_proc_by_id` reads all twelve of its dwords off the stack, the
// first being the hidden buffer the Value return is written through, and ends
// `retn 4` to pop that dword itself. Declaring regparm(3) made GCC pass the buffer
// in eax and `usr` in edx:ecx, which shifted every argument the trampoline read
// (the proc id it looked up was really the output pointer, so no #[hook] ever
// matched) and left nothing on the stack for `retn 4` to pop, so the stack drifted
// until an SSE store faulted somewhere inside BYOND. Read byte-identical on
// 516.1669 and 516.1687.
using CallProcById_Ptr = Value(*)(Value, uint32_t, uint32_t, uint32_t, Value, Value*, uint32_t, uint32_t, uint32_t);

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

// Stands in for `call_proc_by_id` itself, so its shape has to match that function
// exactly (see the note on `CallProcById_Ptr` above). Passes through to our rust hook.
// Used on Windows too
extern "C" Value call_proc_by_id_hook_trampoline(
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

	// A shim running with `RuntimeContext(true)` wants the errors its own BYOND
	// function raises, not the ones raised inside a DM proc that function calls.
	// Those have to take BYOND's normal path so they get reported and reach the
	// callee's own `try`. Every DM proc call passes through here, so this is the
	// one place that can hand the called proc a clean context.
	RuntimeContext runtime_scope(false);

	if (call_proc_by_id_hook(&ret, usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2)) {
		clean(ret);
		return ret;
	} else {
		return call_proc_by_id_original(usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2);
	}
	//return call_proc_by_id_hook(usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2);
}
