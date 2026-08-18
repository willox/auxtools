#if DM_VERSION < 515
#define call_ext call
#endif

/datum
	var/__auxtools_weakref_id
	var/auxtest_number

/proc/auxtools_test_dll()
	. = world.GetConfig("env", "AUXTEST_DLL")
	
/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/auxtest_out()
	// Graceful failure

/proc/auxtest_marker(msg)
	world.log << msg
	auxtest_out(msg)

/proc/auxtest_run_ffi(auxtest_dll, name)
	auxtest_marker("START: " + name)
	var/result = call_ext(auxtest_dll, "auxtest_ffi_" + name)()
	if (result != "SUCCESS")
		auxtest_marker("FAILED: " + name + ": " + result)
		CRASH(name)
	auxtest_marker("PASS: " + name)

/proc/auxtest_inc_counter()
	CRASH()

/proc/concat_strings(a, b)
	return addtext(a, b)

/proc/del_value(v)
	del v

// We create a new datum after del'ing the one we passed into the test function.
// This causes the new datum to take on the internal ID of the old one, and we can test if auxtools
// can properly deal with this situation.
var/datum/weak_test_datum
/proc/create_datum_for_weak()
	weak_test_datum = new

/proc/auxtest_proc_metadata_subject(alpha, beta)
	var/local_one = alpha
	return beta || local_one

#define RUN_AUXTEST(NAME, EXPR) auxtest_marker("START: " + NAME); if ((EXPR) != TRUE) { auxtest_marker("FAILED: " + NAME); CRASH(NAME) }; auxtest_marker("PASS: " + NAME)
#define RUN_AUXTEST_FFI(NAME) auxtest_run_ffi(auxtest_dll, NAME)

// Tests
/proc/auxtest_lists()
	CRASH()

/proc/auxtest_strings()
	CRASH()

/proc/auxtest_weak_values()
	CRASH()

/proc/auxtest_value_from()
	CRASH()

/proc/auxtest_hook_basic()
	CRASH()

/proc/auxtest_runtime_globals()
	CRASH()

/proc/auxtest_string_id_and_entry()
	CRASH()

/proc/auxtest_string_value_refcount()
	CRASH()

/proc/auxtest_string_value_roundtrip()
	CRASH()

/proc/auxtest_proc_find()
	CRASH()

/proc/auxtest_proc_call_global()
	CRASH()

/proc/auxtest_proc_metadata()
	CRASH()

/proc/auxtest_variables_get_set()
	CRASH()

/proc/auxtest_value_to_string()
	CRASH()

/proc/auxtest_list_create_empty()
	CRASH()

/proc/auxtest_list_append_len()
	CRASH()

/proc/auxtest_list_index_get()
	CRASH()

/proc/auxtest_list_assoc_set_get()
	CRASH()

/proc/auxtest_list_remove()
	CRASH()

/proc/auxtest_list_with_size()
	CRASH()

/proc/auxtest_value_from_number()
	CRASH()

/proc/auxtest_value_from_vec()
	CRASH()

/proc/auxtest_value_from_hashmap_value_key()
	CRASH()

/proc/auxtest_value_from_hashmap_string_key()
	CRASH()

/proc/do_tests()
	world.log << "auxtest: do_tests start"
	var/auxtest_dll = auxtools_test_dll()
	world.log << "auxtest: dll=[auxtest_dll]"
	var/signature_result = call_ext(auxtest_dll, "auxtools_check_signatures")()
	world.log << "signature_result = [signature_result]"
	ASSERT(signature_result == "SUCCESS")
	world.log << "auxtest: before auxtools_init"
	var/init_result = call_ext(auxtest_dll, "auxtools_init")()
	world.log << "auxtest: after auxtools_init"
	world.log << "init_result = [init_result]"
	ASSERT(init_result == "SUCCESS")

	// These tests use call_ext instead of hooks, so broken hook dispatch does not
	// hide lower-level signature, structure, and wrapper failures.
	RUN_AUXTEST_FFI("runtime_globals")
	RUN_AUXTEST_FFI("string_id_and_entry")
	RUN_AUXTEST_FFI("string_value_refcount")
	RUN_AUXTEST_FFI("string_value_roundtrip")
	RUN_AUXTEST_FFI("proc_find")
	RUN_AUXTEST_FFI("proc_metadata")
	RUN_AUXTEST_FFI("list_create_empty")
	RUN_AUXTEST_FFI("list_append_len")
	RUN_AUXTEST_FFI("list_index_get")
	RUN_AUXTEST_FFI("list_assoc_set_get")
	RUN_AUXTEST_FFI("list_remove")
	RUN_AUXTEST_FFI("list_with_size")
	RUN_AUXTEST_FFI("value_from_number")
	RUN_AUXTEST_FFI("value_from_vec")
	RUN_AUXTEST_FFI("value_from_hashmap_value_key")
	RUN_AUXTEST_FFI("value_from_hashmap_string_key")
	RUN_AUXTEST_FFI("proc_call_global")

	// Hook tests are ordered so each case adds the smallest practical new dependency.
	RUN_AUXTEST("hook_basic", auxtest_hook_basic())
	RUN_AUXTEST("runtime_globals", auxtest_runtime_globals())
	RUN_AUXTEST("string_id_and_entry", auxtest_string_id_and_entry())
	RUN_AUXTEST("string_value_refcount", auxtest_string_value_refcount())
	RUN_AUXTEST("string_value_roundtrip", auxtest_string_value_roundtrip())
	RUN_AUXTEST("proc_find", auxtest_proc_find())
	RUN_AUXTEST("proc_call_global", auxtest_proc_call_global())

	var/datum/proc_test = new
	RUN_AUXTEST("proc_metadata", auxtest_proc_metadata())
	RUN_AUXTEST("variables_get_set", auxtest_variables_get_set(proc_test))
	RUN_AUXTEST("value_to_string", auxtest_value_to_string(proc_test))
	RUN_AUXTEST("list_create_empty", auxtest_list_create_empty())
	RUN_AUXTEST("list_append_len", auxtest_list_append_len())
	RUN_AUXTEST("list_index_get", auxtest_list_index_get())
	RUN_AUXTEST("list_assoc_set_get", auxtest_list_assoc_set_get())
	RUN_AUXTEST("list_remove", auxtest_list_remove())
	RUN_AUXTEST("list_with_size", auxtest_list_with_size())
	RUN_AUXTEST("value_from_number", auxtest_value_from_number())
	RUN_AUXTEST("value_from_vec", auxtest_value_from_vec())
	RUN_AUXTEST("value_from_hashmap_value_key", auxtest_value_from_hashmap_value_key())
	RUN_AUXTEST("value_from_hashmap_string_key", auxtest_value_from_hashmap_string_key())

	var/datum/weak_test = new
	RUN_AUXTEST("weak_values", auxtest_weak_values(weak_test))
	ASSERT(weak_test == null)

	// Stop testing after the 8th reboot
	if (auxtest_inc_counter() == 8)
		auxtest_out("SUCCESS: Finished")
		call_ext(auxtest_dll, "auxtools_shutdown")()
		shutdown()
	else
		call_ext(auxtest_dll, "auxtools_shutdown")()
		world.Reboot()

/world/New()
	do_tests()
	. = ..()

/world/Error(exception/e)
	auxtest_out("FAILED: world/Error([e])")
	. = ..()
	shutdown()
