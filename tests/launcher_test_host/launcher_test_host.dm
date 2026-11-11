// Started by auxtools' launcher, which loads the debugger into it. It has
// nothing of auxtools' own: no auxtools_init, no enable_debugging.

/world/New()
	..()
	world.log << "launcher test: world/New"
	spawn(1)
		run_test()

/proc/run_test()
	world.log << "launcher test: [target(21)]"
	second_target()
	// A runtime error ends the proc it's in and its callers
	spawn(0)
		fail()
	sleep(2)
	// The first world reboots, which loads the world again
	if (fexists("rebooted"))
		fdel("rebooted")
		world.log << "launcher test: done"
		del(world)
	else
		text2file("yes", "rebooted")
		world.log << "launcher test: rebooting"
		world.Reboot()

/proc/target(n)
	var/doubled = n * 2
	return doubled

/proc/second_target()
	return 1

/proc/fail()
	var/list/L = null
	return L[1]
