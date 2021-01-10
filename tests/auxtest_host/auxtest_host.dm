/proc/auxtools_test_dll()
	. = world.GetConfig("env", "AUXTEST_DLL")

/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/auxtest_out()
	// Graceful failure

/proc/auxtest_strings()
	CRASH()

/proc/auxtest_inc_counter()
	CRASH()

/proc/concat_strings(a, b)
	return addtext(a, b)

/proc/do_tests()
	var/auxtest_dll = auxtools_test_dll()
	ASSERT(call(auxtest_dll, "auxtools_init")() == "SUCCESS")

	// TODO: More tests
	ASSERT(auxtest_strings() == TRUE)

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
