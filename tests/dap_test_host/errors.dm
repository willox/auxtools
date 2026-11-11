// Runtime errors of each kind

/proc/fail_null()
	var/list/L = null
	return L[1] // @fail_null

/proc/fail_deep()
	return fail_null() + 1 // @fail_deep

/proc/fail_index()
	var/list/L = list(1)
	return L[5] // @fail_index

/proc/fail_divide()
	var/zero = 0
	return 1 / zero // @fail_divide

/proc/fail_crash()
	CRASH("crashed on purpose") // @fail_crash

/proc/fail_caught()
	try
		fail_null()
	catch (var/exception/e)
		return "caught [e.name]"

/proc/fail_throw()
	throw EXCEPTION("thrown on purpose") // @fail_throw

/proc/sc_fail_null()
	fail_deep()
	world.log << "dap: after the failure" // @after_failure

/proc/sc_fail_index()
	fail_index()

/proc/sc_fail_divide()
	fail_divide()

/proc/sc_fail_crash()
	fail_crash()

/proc/sc_fail_caught()
	world.log << "dap: [fail_caught()]"

/proc/sc_fail_throw()
	fail_throw()

/proc/sc_fail_spawned()
	spawn(0)
		fail_null()
	sleep(1)

/proc/sc_fail_twice()
	spawn(0)
		fail_index()
	fail_divide()

// A runtime error in the try block itself
/proc/fail_caught_here()
	try
		var/list/L = null
		return L[1] // @fail_caught_here
	catch (var/exception/e)
		return "caught here [e.name]"

/proc/sc_fail_caught_here()
	world.log << "dap: [fail_caught_here()]"

/proc/throw_caught()
	try
		throw EXCEPTION("thrown and caught") // @throw_caught
	catch (var/exception/e)
		return "caught [e.name]"

/proc/sc_throw_caught()
	world.log << "dap: [throw_caught()]"

/proc/sc_throw_text()
	throw "just text" // @throw_text
