// The test asks for a scenario by setting `scenario` through the debugger. The
// loop below runs /proc/sc_NAME in a proc of its own and says when it's over.
// A `// @name` comment marks a line the test looks for.

var/scenario
var/ticks = 0
var/idle_ticks = 0
var/global_number = 5
// What the world's shutdown code does: "refuse" doesn't shut it down
var/on_shutdown
var/list/global_list = list(1, "two", 3)
// Worked out while the world loads, before world/New
var/early_global = early_value()

/proc/early_value()
	// A runtime error while the world loads, when the test asks for one
	if (fexists("fail_early"))
		var/list/L = null
		return L[1] // @early_failure
	return 5 // @early_value

/world/New()
	// pixlocs and turfs need a map
	maxx = 3
	maxy = 3
	maxz = 1
	..()
	world.log << "dap: world/New" // @world_new
	spawn(1)
		main_loop()

/world/Del()
	world.log << "dap: world/Del" // @world_del
	if (on_shutdown == "refuse")
		return
	..()

/proc/main_loop()
	while (TRUE)
		ticks++
		if (scenario)
			var/name = scenario
			scenario = null
			world.log << "dap: running [name]"
			spawn(0)
				call("/proc/sc_[name]")()
				world.log << "dap: finished [name]"
		if (idle_ticks)
			// Nothing of the world's runs for a while
			var/idle = idle_ticks
			idle_ticks = 0
			sleep(idle)
			world.log << "dap: idle over"
		sleep(1)

/proc/sc_quit()
	world.log << "dap: quitting"
	del(world)

/proc/sc_reboot()
	world.log << "dap: rebooting"
	world.Reboot()

/proc/sc_stderr()
	world.log << "dap: to the log"
