// One of every kind of value, for the test to look at

/proc/values(first, second)
	var/whole = 42
	var/fraction = 1.5
	var/past_24_bits = 16777217
	var/huge = 1e30
	var/negative = -0.25
	var/infinite = 1.#INF
	var/text = "hello \"quoted\"\nline two"
	var/empty_text = ""
	var/nothing = null
	var/path = /datum/thing
	var/proc_path = /proc/leaf
	var/datum/thing/named = new("A")
	named.other = new /datum/thing/sub("B")
	named.items = list(1, 2, 3)
	var/datum/made/unnamed = new(1)
	var/list/empty = list()
	var/list/plain = list(1, "b", null, named)
	var/list/assoc = list("a" = 1, "b" = named, "c" = null)
	var/list/nested = list(list(1, 2), list("x" = list(3)))
	var/list/dupes = list("a", "a", 1, 1)
	var/list/big = new(3000)
	var/file = 'values.dm'
	var/turf/T = locate(1, 1, 1)
	var/obj/O = new(T)
	var/mob/player/M = new(T)
	var/area/A = T.loc
	var/the_world = world
	var/matrix/mx = matrix()
	var/pointer = &whole
	var/var_list = named.vars
	var/contents = T.contents
	var/image/img = image(null, T)
	var/appearance = M.appearance
	var/regex/re = regex("a+")
	var/overlays = M.overlays
	var/verbs = M.verbs
	var/filters = M.filters
	var/world_contents = world.contents
	var/datum/thing/deleted = new("D")
	del(deleted)
	var/savefile/save = new("dap_test.sav")
	var/exception/error = new("boom", "file.dm", 3)
#if DM_VERSION >= 516
	var/alist/pairs = alist("a" = 1, 7 = "seven")
	var/vec = vector(1, 2, 3)
	var/pix = pixloc(1, 1, 1)
	var/callee/this_proc = callee
#endif
	// Two locals of one name, each in a block of its own
	if (whole)
		var/twin = 1
		whole += twin - 1
	if (fraction)
		var/twin = 2
		whole += twin - 2
	. = "dot"
	whole += length(args) // @values_stop
	fdel("dap_test.sav")
	del(O)
	del(M)
	return whole

/proc/sc_values()
	values(1, "two", 3)

// A stop inside a datum's proc, with src, a list and a caller to look at
/datum/thing/proc/inside(by)
	var/list/local_list = list(10, 20, 30)
	n += by // @inside_stop
	return local_list

/proc/sc_inside()
	var/datum/thing/sub/s = new("S")
	s.items = list("x", "y")
	var/in_caller = 77
	s.inside(4)
	return in_caller
