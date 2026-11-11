// Places for the debugger to watch, and the code that changes them

var/watched_number = 1
var/list/watched_list = list("a", "b", "c")
var/list/watched_pairs = list("one" = 1, "two" = 2)

/proc/change_number(by)
	watched_number += by // @change_number
	return watched_number // @change_number_after

/datum/thing/proc/rename(called)
	name = called // @rename
	return src // @rename_after

/proc/sc_watch()
	var/datum/thing/kept = new("kept")
	var/steps // @watch_start
	steps = 1 // @watch_local_1
	change_number(2) // @watch_call
	watched_list[2] = "B" // @watch_list
	watched_pairs["two"] = 22 // @watch_pair
	kept.rename("renamed") // @watch_object
	kept.n = kept.n // @watch_same
	steps += 10 // @watch_local_2
	sleep(1) // @watch_sleep
	change_number(3) // @watch_after_sleep
	steps += 100 // @watch_local_3
	del(kept) // @watch_del
	return steps // @watch_end

/proc/churn(depth)
	if (depth > 0)
		return churn(depth - 1) + 1
	return 0

// A proc's own var, while the proc sleeps again and again and others come
// and go
/proc/sc_watch_long()
	var/patient = 1 // @watch_long_start
	for (var/i in 1 to 30)
		spawn(0)
			churn(5)
		churn(3)
		sleep(1)
	patient = 2 // @watch_long_change
	return patient // @watch_long_end
