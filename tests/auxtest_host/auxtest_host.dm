#if DM_VERSION < 515
#define call_ext call
#endif

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

/mob/verb/auxtest_verb()
	return

// A spread of values for `List::is_list` to agree with `islist()` on: plain
// lists, the special lists that hang off an object, and things that are not
// lists at all. `M.filters[1]` is the mean one, since a single filter has the
// same tag as the filters list it came out of.
/proc/auxtest_islist_samples()
	var/mob/M = new
	M.filters += filter(type = "blur")
	var/savefile/F = new
	var/list/L = list(1, 2)
	return list(
		null, 1, "string", M, /list, &L,
		L, alist("a" = 1), L.vars, args,
		M.verbs, M.contents, M.vars, M.overlays, M.group, M.vis_contents,
		M.filters, M.filters[1],
		F.dir, world.contents, world.vars, global.vars,
	)

/proc/auxtest_islist(value)
	return islist(value)

/proc/auxtest_length(value)
	return length(value)

// New() is hooked and hands back a string that nothing here ever reads
/datum/auxtest_discard/New()
	CRASH()

/proc/auxtest_new_discard()
	new /datum/auxtest_discard

// Tests
/proc/auxtest_hooks()
	CRASH()

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
	var/init_result = call_ext(auxtest_dll, "auxtools_init")()
	world.log << "init_result = [init_result]"
	ASSERT(init_result == "SUCCESS")

	// Tests
	ASSERT(auxtest_hooks() == TRUE)
	ASSERT(auxtest_lists() == TRUE)
	ASSERT(auxtest_strings() == TRUE)
	ASSERT(auxtest_value_from() == TRUE)

	var/datum/weak_test = new
	ASSERT(auxtest_weak_values(weak_test) == TRUE)
	ASSERT(weak_test == null)

	// Stop testing after the 8th reboot
	if (auxtest_inc_counter() == 8)
		auxtest_out("SUCCESS: Finished")
		call_ext(auxtest_dll, "auxtools_shutdown")()
		shutdown()
	else
		call_ext(auxtest_dll, "auxtools_shutdown")()
		world.Reboot()

/world/New()
	do_tests()
	. = ..()

/world/Error(exception/e)
	auxtest_out("FAILED: world/Error([e])")
	. = ..()
	shutdown()
