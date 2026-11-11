// Started by auxtools' launcher without debugging, then attached to

var/rounds = 0

// Macros for the command line's eval, and one that's gone by the end
#define DOUBLE_IT(x) ((x) * 2)
#define ANSWER 42
#define GONE 1
#undef GONE

// An object for the command line's eval --src
var/datum/held/held = new

/datum/held
	var/count = 3

/datum/held/proc/double(n)
	return n * 2

// For the command line's eval --spawn: procs that sleep
/proc/slow_double(n, wait = 2)
	sleep(wait)
	return n * 2

/proc/slow_fail()
	sleep(1)
	return fail_deeper(5)

/world/New()
	..()
	spawn(1)
		run_until_stopped()

// A world whose shutdown code doesn't shut it down, when the test asks for one
/world/Del()
	world.log << "attach test: world/Del"
	if (fexists("refuse_del"))
		return
	..()

/proc/run_until_stopped()
	var/said = 0
	while (!fexists("stop"))
		world.log << "attach test: [value()]"
		tally()
		rounds++
		said++
		// A runtime error when the test asks for one
		if (fexists("fail"))
			fdel("fail")
			spawn(0)
				fail()
		// A reboot when the test asks for one
		if (fexists("reboot"))
			fdel("reboot")
			world.Reboot()
		// A call for the command line to step through
		if (fexists("call"))
			fdel("call")
			spawn(0)
				fail_in(1)
		sleep(5)
	world.log << "attach test: done"
	del(world)

/proc/value()
	return 1

// Calls tick() from two places: every round, and every other round, for
// counting hits by caller
/proc/tally()
	tick(1)
	if (rounds % 2 == 0)
		tick(2)

/proc/tick(n)
	return n

/proc/fail()
	var/list/L = null
	return L[1]

// Embedded text macro for debugger output checks.
/obj/macro_named
	name = "\improper AI Chamber"

// For the command line's eval: a runtime error two procs down from it, and
// one a proc catches itself
/proc/fail_in(n)
	var/got = fail_deeper(n)
	return got

/proc/fail_deeper(n)
	var/list/L = list()
	return L[n]

/proc/caught_inside()
	try
		fail()
	catch
		return "caught"

// /client as compiled code has it, for the command line's eval to match
/proc/client_type()
	return /client

// A proc with two definitions, for the command line's proc and dis
/datum/twice/proc/act()
	return "first"

/datum/twice/act()
	. = ..()
	return "second"
