/proc/start()
	var/auxtest = world.GetConfig("env", "AUXTOOLS_TEST_DLL")
	if (auxtest)
		var/init_res = call(auxtest, "auxtools_init")()
		world.log << "auxtools_init = [init_res]"
		ASSERT(init_res == "SUCCESS")

/proc/end()
	var/auxtest = world.GetConfig("env", "AUXTOOLS_TEST_DLL")
	if (auxtest)
		var/init_res = call(auxtest, "auxtools_shutdown")()

/mob/proc/penis()
	return

/world/New()
	var/x = /datum
	x:penis()
	return
	start()
	. = ..()

/world/Del()
	end()
	. = ..()