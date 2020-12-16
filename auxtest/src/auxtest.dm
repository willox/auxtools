/proc/auxtools_test_dll()
	. = world.GetConfig("env", "AUXTOOLS_TEST_DLL")
	if (!.)
		. = "E:\\auxtools\\target\\i686-pc-windows-msvc\\debug\\auxtest.dll"

/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/test_strings()
	CRASH()

/proc/concat_strings(a, b)
	return addtext(a, b)

/proc/start()
	var/auxtest = auxtools_test_dll()
	var/init_res = call(auxtest, "auxtools_init")()
	world.log << "auxtools_init = [init_res]"
	ASSERT(init_res == "SUCCESS")
	world.log << "[test_strings() ? "TEST SUCCESS" : "TEST FAILED"]"
	world.Reboot()

/proc/end()
	var/auxtest = auxtools_test_dll()
	call(auxtest, "auxtools_shutdown")()

/world/New()
	start()
	. = ..()

/world/Del()
	end()
	. = ..()
