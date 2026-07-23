#include <stdint.h>
#include <chrono>
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

extern "C" uint8_t call_proc_by_id_profile_enabled = 0;

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

extern "C" uint64_t call_proc_by_id_profile_begin(uint32_t proc_id);
extern "C" void call_proc_by_id_profile_end(
	uint32_t proc_id,
	uint8_t handled,
	uint64_t sample_token,
	uint64_t elapsed_ns);

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
	uint64_t sample_token = 0;
	std::chrono::steady_clock::time_point sample_start;

	if (call_proc_by_id_profile_enabled) {
		sample_token = call_proc_by_id_profile_begin(proc_id);
		if (sample_token) {
			sample_start = std::chrono::steady_clock::now();
		}
	}

	if (call_proc_by_id_hook(&ret, usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2)) {
		clean(ret);
		if (call_proc_by_id_profile_enabled) {
			uint64_t elapsed_ns = 0;
			if (sample_token) {
				elapsed_ns = static_cast<uint64_t>(
					std::chrono::duration_cast<std::chrono::nanoseconds>(
						std::chrono::steady_clock::now() - sample_start
					).count()
				);
			}
			call_proc_by_id_profile_end(proc_id, 1, sample_token, elapsed_ns);
		}
		return ret;
	} else {
		ret = call_proc_by_id_original(usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2);
		if (call_proc_by_id_profile_enabled) {
			uint64_t elapsed_ns = 0;
			if (sample_token) {
				elapsed_ns = static_cast<uint64_t>(
					std::chrono::duration_cast<std::chrono::nanoseconds>(
						std::chrono::steady_clock::now() - sample_start
					).count()
				);
			}
			call_proc_by_id_profile_end(proc_id, 0, sample_token, elapsed_ns);
		}
		return ret;
	}
	//return call_proc_by_id_hook(usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2);
}
