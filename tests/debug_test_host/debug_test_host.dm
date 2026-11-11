// Started with the debug library loaded into it, which adds the procs it
// needs: this has none of auxtools' own.

var/runtimes_left = 0
var/reboot_requested = FALSE

// The test puts its breakpoints on these two. They are called in this order,
// once per tick, so the test can tell which breakpoint it stopped on.
/proc/breakpoint_target()
	return 2

/proc/tick_marker()
	return 1

// 516's own kinds of value. 515 has none of them.
#if DM_VERSION >= 516
/proc/alist_sample()
	return alist("a" = 1, 7 = "seven", "n" = null)

/proc/vector_sample()
	return vector(1, 2, 3)

/proc/vector2_sample()
	return vector(4, 5)

/proc/pixloc_sample()
	return pixloc(1, 1, 1)
#endif

// Two seconds of runtime errors, then quiet again. The debugger is told not
// to pause on them.
/proc/start_runtimes()
	runtimes_left = 20

/proc/make_runtime()
	var/list/nothing
	return nothing.len

/proc/request_reboot()
	reboot_requested = TRUE

/proc/main_loop()
#if DM_VERSION >= 516
	// a callee only reads while its proc is running, so the test looks at this one
	var/callee/this_proc = callee
#endif
	while (!reboot_requested)
		breakpoint_target()
		tick_marker()
		if (runtimes_left > 0)
			runtimes_left--
			spawn()
				make_runtime()
		sleep(1)

	// Rebooting has to happen out here. The debugger evaluates
	// request_reboot() from inside the debug server, which can't shut itself
	// down there.
	world.Reboot()

/world/New()
	// pixloc_sample() needs a real tile to point at
	maxx = 5
	maxy = 5
	maxz = 1

	spawn()
		main_loop()
	. = ..()

// Evaluated expressions call verbs as well as procs
/mob/helper/verb/greet(who as text)
	set name = "Greet Someone"
	return "hi [who]"

var/mob/helper/helper = new
