/datum
	var/__auxtools_weakref_id

/proc/auxtools_test_dll()
	. = world.GetConfig("env", "AUXTEST_DLL")

/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/auxtest_out()
	// Graceful failure

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

// Tests
/proc/auxtest_lists()
	CRASH()

/proc/auxtest_strings()
	CRASH()

/proc/auxtest_weak_values()
	CRASH()

/proc/auxtest_value_from()
	CRASH()

/proc/do_tests()
	var/auxtest_dll = auxtools_test_dll()
	ASSERT(call(auxtest_dll, "auxtools_init")() == "SUCCESS")

	// Tests
	ASSERT(auxtest_lists() == TRUE)
	ASSERT(auxtest_strings() == TRUE)
	ASSERT(auxtest_value_from() == TRUE)

	var/datum/weak_test = new
	ASSERT(auxtest_weak_values(weak_test) == TRUE)
	ASSERT(weak_test == null)

	// Stop testing after the 8th reboot
	if (auxtest_inc_counter() == 8)
		auxtest_out("SUCCESS: Finished")
		call(auxtest_dll, "auxtools_shutdown")()
		shutdown()
	else
		call(auxtest_dll, "auxtools_shutdown")()
		world.Reboot()

/world/New()
	do_tests()
	. = ..()

/world/Error(exception/e)
	auxtest_out("FAILED: world/Error([e])")
	. = ..()
	shutdown()
