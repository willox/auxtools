// Started with the debug library loaded into it, which adds the procs it
// needs. It hooks this one when it's the world's own, so new code for it never
// runs.
/proc/auxtools_launched_world_del()
	CRASH("auxtools not loaded")

/world/New()
	spawn()
		sleeper()
	spawn()
		main_loop()
	. = ..()

var/reboot_requested = FALSE

/proc/request_reboot()
	reboot_requested = TRUE

// The debug server only answers while DM code runs
/proc/main_loop()
	while (!reboot_requested)
		tick()
		sleep(1)

	// Rebooting has to happen out here. The test asks for it through the
	// debug server, which can't shut itself down there.
	world.Reboot()

// A type's const is compiled into the code that uses it
/datum/limits
	var/const/MAX = 3

/datum/limits/proc/max_doubled()
	return MAX * 2

// A global const, read alone and folded
var/const/LIMIT_ALL = 10

/proc/limit_all()
	return list(LIMIT_ALL, LIMIT_ALL + 1)

var/datum/limits/kept_limits = new

// A var declared with another type later
/datum/limits/var/list/things = null

/datum/limits/proc/thing_count()
	return length(things)

// A proc whose `set name` changes, which call() by name follows
/datum/limits/proc/label()
	set name = "Label"
	return "labelled"

// Definitions of one proc, whose `..()` calls make a chain
/datum/chain/proc/say()
	return "1"

/datum/chain/say()
	return ..() + "3"

var/datum/chain/the_chain = new

// Definitions of a verb, of a world proc and of a global proc, which come and
// go among the others too
/mob/chain_verbs/verb/speak()
	return "a"

/mob/chain_verbs/verb/other()
	return "o"

/mob/chain_verbs/speak()
	return ..() + "c"

var/mob/chain_verbs/verb_chain = new

/proc/verb_names(mob/m)
	var/list/names = list()
	for (var/v in m.verbs)
		names += "[v:name]"
	return jointext(names, ",")

/world/proc/chant()
	return "x"

/world/chant()
	return ..() + "z"

/proc/solo()
	return "declared"

/solo()
	return "another"

// Vars declared another way later
/datum/declared
	var/count = 1
	var/shared = 10
	var/many = null

/datum/declared/sub

/datum/declared/proc/read_count()
	return count

/datum/declared/proc/read_shared()
	return shared

/datum/declared/proc/many_count()
	return length(many)

var/datum/declared/declared_one = new

// Changed to code the compiler learnt late
/proc/compiled_late()
	return 0

/proc/keyed(a, b)
	return b

// Static sharing a built-in sound var name absent from the build.

/datum/hr_named
	var/static/falloff = 9

/proc/hr_named_falloff(sound/s)
	var/datum/hr_named/named = new
	. = "[s.falloff] "
	s.falloff = 7
	. += "[s.falloff] [named.falloff]"

// Reparented sound types use var-table built-ins. Avoid repeat and volume,
// which new() sets from arguments.
/sound/hr_sa
	pan = 10

/sound/hr_sb
	priority = 5

/sound/hr_st
	parent_type = /sound/hr_sa
	frequency = 3
