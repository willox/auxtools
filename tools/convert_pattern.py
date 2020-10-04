import sys

if len(sys.argv) == 1:
	print(f"Usage: {sys.argv[0]} [pattern]\n\nThe pattern does not need to be surrounded in quotes.")
	sys.exit()
	
args = sys.argv
	
args.pop(0)

if len(args) == 1:
	args = args[0].split()

result = ""
for byte in args:
	if byte == "??":
		result += "?"
	else:
		result += "\\x" + byte
		
while result.endswith("?"):
	result = result[:-1]
	
print(result)