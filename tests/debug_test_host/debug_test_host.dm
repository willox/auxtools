#if DM_VERSION < 515
#define call_ext call
#endif

/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/auxtools_expr_stub()
	CRASH("auxtools not loaded")

/proc/enable_debugging(mode, port)
	CRASH("auxtools not loaded")

var/runtimes_left = 0
var/reboot_requested = FALSE

// The test puts its breakpoints on these two. They are called in this order,
// once per tick, so the test can tell which breakpoint it stopped on.
/proc/breakpoint_target()
	return 2

/proc/tick_marker()
	return 1

/proc/alist_sample()
	return alist("a" = 1, 7 = "seven", "n" = null)

// Two seconds of runtime errors, then quiet again. The test detaches while
// they are going and comes back after they stop.
/proc/start_runtimes()
	runtimes_left = 20

/proc/make_runtime()
	var/list/nothing
	return nothing.len

/proc/request_reboot()
	reboot_requested = TRUE

/proc/main_loop(debug_dll)
	while (!reboot_requested)
		breakpoint_target()
		tick_marker()
		if (runtimes_left > 0)
			runtimes_left--
			spawn()
				make_runtime()
		sleep(1)

	// Shutting down has to happen out here. The debugger evaluates
	// request_reboot() from inside the debug server, which can't free itself.
	call_ext(debug_dll, "auxtools_shutdown")()
	world.Reboot()

/world/New()
	var/debug_dll = world.GetConfig("env", "AUXTOOLS_DEBUG_DLL")
	var/init_result = call_ext(debug_dll, "auxtools_init")()
	world.log << "init_result = [init_result]"
	ASSERT(init_result == "SUCCESS")

	// No port given, so it comes from AUXTOOLS_DEBUG_PORT
	enable_debugging("BACKGROUND")

	spawn()
		main_loop(debug_dll)
	. = ..()
