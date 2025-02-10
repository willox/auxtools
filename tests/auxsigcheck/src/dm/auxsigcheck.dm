#if DM_VERSION >= 515
#define LIBCALL call_ext
#else
#define LIBCALL call
#endif

/world/New()
	. = ..()
	var/result = run_tests()
	if(!result)
		world.log << "FAILED (unknown, likely runtime)"
	else
		world.log << "[result]"
	del(src)

/proc/run_tests()
	var/dll = world.GetConfig("env", "AUXTOOLS_DLL")
	if(!dll)
		return "FAILED (AUXTOOLS_DLL not set)"
	if(!fexists(dll))
		return "FAILED (AUXTOOLS_DLL path doesn't exist)"
	return LIBCALL(dll, "auxtools_check_signatures")()
