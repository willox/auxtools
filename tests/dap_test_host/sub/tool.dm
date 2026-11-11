// Included by the world's own .dme, so it's "sub/tool.dm" to the world: a
// name that ends with "tool.dm", which is what the world calls lib/tool.dm.

/proc/sub_tool(n)
	return n + 1 // @sub_tool

// A var with no value has a line in the world's code and nothing else: its
// breakpoint is where the next line starts
/proc/sub_declares(n)
	var/unset // @declares_var
	n += 1 // @declares_first
	return n + length(unset) // @declares_second

// A global var with a value, in a file that isn't the world's first: the proc
// that sets the world's global vars has the code of every file's
var/tooled = sub_tool(20) // @tooled
