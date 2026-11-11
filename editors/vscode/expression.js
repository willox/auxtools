// Find a read-only hover expression ending at the selected identifier.
// Include member chains such as `a.b.c`, `a?.b` and `a:b`, but reject
// called names so hovering cannot invoke a proc.

const NAME = /[A-Za-z_][A-Za-z0-9_]*/y;

// Identifier range containing or immediately preceding zero-based `column`.
function nameAt(line, column) {
	let start = column;
	while (start > 0 && /[A-Za-z0-9_]/.test(line[start - 1])) {
		start--;
	}
	NAME.lastIndex = start;
	const match = NAME.exec(line);
	if (!match || start + match[0].length < column) {
		return undefined;
	}
	return [start, start + match[0].length];
}

// Whether the position is inside a string or line comment.
function inTextOrComment(line, position) {
	let quoted = false;
	for (let i = 0; i < position; i++) {
		const c = line[i];
		if (quoted) {
			if (c === "\\") {
				i++;
			} else if (c === '"') {
				quoted = false;
			}
		} else if (c === '"') {
			quoted = true;
		} else if (c === "/" && line[i + 1] === "/") {
			return true;
		}
	}
	return quoted;
}

// Return the hover range in `line`, or undefined.
function expressionAt(line, column) {
	const name = nameAt(line, column);
	if (!name || inTextOrComment(line, name[0])) {
		return undefined;
	}
	let [start, end] = name;

	// A proc's name, where it's called or declared
	if (/^\s*\(/.test(line.slice(end))) {
		return undefined;
	}
	// A keyword, or a number's letters
	const word = line.slice(start, end);
	if (/^(var|proc|verb|if|else|for|while|do|switch|return|new|del|in|to|as|set|try|catch|throw|goto|break|continue|spawn|step|null|global|static|const|tmp)$/.test(word)) {
		return undefined;
	}
	if (start > 1 && line[start - 1] === "." && /[0-9]/.test(line[start - 2])) {
		return undefined;
	}

	// Extend through member-access operators and identifiers.
	for (;;) {
		const before = /([A-Za-z_][A-Za-z0-9_]*)\s*(\?\.|\?:|\.|:)\s*$/.exec(line.slice(0, start));
		// Exclude ternary colons preceded by whitespace.
		if (!before || (/:$/.test(before[2]) && /\s:\s*$/.test(line.slice(0, start)))) {
			break;
		}
		start = before.index;
	}
	// Reject receivers requiring call or index evaluation.
	const through = /(\?\.|\?:|\.|:)\s*$/.exec(line.slice(0, start));
	if (through && !(through[1] === ":" && /\s:\s*$/.test(line.slice(0, start)))) {
		return undefined;
	}
	// Type paths and declarations are not hover values.
	if (start > 0 && line[start - 1] === "/") {
		return undefined;
	}
	return { start, end };
}

module.exports = { expressionAt };
