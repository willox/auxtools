//! Expand source macros in debugger expressions.
//!
//! The launcher supplies definitions through `Request::Defines`. Expand only
//! macro invocations, preserving surrounding text and line numbers. Cache
//! results for repeated breakpoint conditions.
//!
//! The launcher shares this module to serialize definitions, so it must not
//! depend on a running world.

use std::{
	borrow::Cow,
	collections::{HashMap, HashSet},
	sync::Mutex
};

use dreammaker::{
	lexer::{Punctuation, Token},
	Context, FileId, Location
};

use crate::server_types::Macro;

/// What the world was given: none until a launcher gives them
static MACROS: Mutex<Option<Macros>> = Mutex::new(None);

/// Maximum cached expansions; breakpoint conditions may run at every hit.
const REMEMBERED: usize = 1000;

struct Macros {
	by_name: HashMap<String, Macro>,
	/// Cached expansions; `None` means the expression contains no macros.
	expanded: HashMap<String, Result<Option<String>, String>>
}

/// The code's macros, in place of any the world had
pub fn set(macros: Vec<Macro>) {
	let by_name = macros.into_iter().map(|define| (define.name.clone(), define)).collect();
	*MACROS.lock().unwrap() = Some(Macros {
		by_name,
		expanded: HashMap::new()
	});
}

/// Whether a launcher has given the world its code's macros
pub fn given() -> bool {
	MACROS.lock().unwrap().is_some()
}

/// Expand source macros in `text`, preserving text without macro invocations.
pub fn expand(text: &str) -> Result<Cow<'_, str>, String> {
	let mut macros = MACROS.lock().unwrap();
	let Some(macros) = macros.as_mut() else {
		return Ok(Cow::Borrowed(text));
	};
	let expanded = match macros.expanded.get(text) {
		Some(expanded) => expanded.clone(),
		None => {
			let expanded = expand_with(&macros.by_name, text);
			if macros.expanded.len() >= REMEMBERED {
				macros.expanded.clear();
			}
			macros.expanded.insert(text.to_owned(), expanded.clone());
			expanded
		}
	};
	expanded.map(|expanded| expanded.map_or(Cow::Borrowed(text), Cow::Owned))
}

/// Expand `macros` in `text`; return `None` if none are invoked.
pub fn expand_with(macros: &HashMap<String, Macro>, text: &str) -> Result<Option<String>, String> {
	if macros.is_empty() {
		return Ok(None);
	}
	let ctx = Context::default();
	let tokens: Vec<_> = dreammaker::lexer::Lexer::new(&ctx, FileId::BUILTINS, text.as_bytes()).collect();
	// Where each line starts, for the tokens' places as offsets
	let lines: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(at, _)| at + 1)).collect();
	let offset = |at: Location| -> Option<usize> { Some(lines.get((at.line as usize).checked_sub(1)?)? + (at.column as usize).checked_sub(1)?) };

	let mut out = String::new();
	let mut copied = 0;
	let mut any = false;
	let mut i = 0;
	while i < tokens.len() {
		let Token::Ident(name, _) = &tokens[i].token else {
			i += 1;
			continue;
		};
		let Some(define) = macros.get(name.as_str()) else {
			i += 1;
			continue;
		};
		// A use is the name, and a macro with parameters needs its arguments
		let (last, length) = if define.params.is_empty() {
			(i, name.len())
		} else {
			match closing(&tokens[i + 1..]) {
				Some(after) => (i + 1 + after, 1),
				None if is(tokens.get(i + 1).map(|t| &t.token), Punctuation::LParen) => return Err(format!("{}( has no ) to end it", name)),
				// Not a use: the name alone
				None => {
					i += 1;
					continue;
				}
			}
		};
		let (Some(start), Some(end)) = (offset(tokens[i].start), offset(tokens[last].start).map(|at| at + length)) else {
			return Err(format!("couldn't find where {} is in the text", name));
		};
		let (Some(used), Some(before)) = (text.get(start..end), text.get(copied..start)) else {
			return Err(format!("couldn't find where {} is in the text", name));
		};
		let expanded = expand_use(macros, used)?;
		out.push_str(before);
		// The expansion's tokens stay apart from those around it
		if out.chars().next_back().zip(expanded.chars().next()).is_some_and(|(a, b)| glued(a, b)) {
			out.push(' ');
		}
		out.push_str(&expanded);
		if expanded
			.chars()
			.next_back()
			.zip(text[end..].chars().next())
			.is_some_and(|(a, b)| glued(a, b))
		{
			out.push(' ');
		}
		copied = end;
		any = true;
		i = last + 1;
	}
	if !any {
		return Ok(None);
	}
	out.push_str(&text[copied..]);
	Ok(Some(out))
}

/// Whether two characters next to each other would be read as one token
fn glued(a: char, b: char) -> bool {
	let word = |c: char| c.is_alphanumeric() || c == '_';
	let operator = |c: char| "+-*/%<>=!&|^~?:.".contains(c);
	(word(a) && word(b)) || (operator(a) && operator(b))
}

fn is(token: Option<&Token>, punctuation: Punctuation) -> bool {
	matches!(token, Some(Token::Punct(p)) if *p == punctuation)
}

/// Where the `)` is that closes the `(` these tokens start with
fn closing(tokens: &[dreammaker::lexer::LocatedToken]) -> Option<usize> {
	if !is(tokens.first().map(|t| &t.token), Punctuation::LParen) {
		return None;
	}
	let mut depth = 0;
	for (at, token) in tokens.iter().enumerate() {
		if is(Some(&token.token), Punctuation::LParen) {
			depth += 1;
		} else if is(Some(&token.token), Punctuation::RParen) {
			depth -= 1;
			if depth == 0 {
				return Some(at);
			}
		}
	}
	None
}

/// The names in some text
fn names(text: &str) -> Vec<String> {
	let ctx = Context::default();
	dreammaker::lexer::Lexer::new(&ctx, FileId::BUILTINS, text.as_bytes())
		.filter_map(|token| match token.token {
			Token::Ident(name, _) => Some(name.to_string()),
			_ => None
		})
		.collect()
}

/// Expand one invocation with the preprocessor, including transitive
/// macro dependencies.
fn expand_use(macros: &HashMap<String, Macro>, used: &str) -> Result<String, String> {
	let mut needed = vec![];
	let mut seen = HashSet::new();
	let mut names_left = names(used);
	while let Some(name) = names_left.pop() {
		if let Some(define) = macros.get(&name) {
			if seen.insert(name) {
				names_left.extend(names(&define.text));
				needed.push(define);
			}
		}
	}

	let mut buffer = String::new();
	for define in needed {
		buffer += &define_line(define);
	}
	buffer += used;
	buffer.push('\n');
	let ctx = Context::default();
	let expanded: Vec<Token> = dreammaker::Preprocessor::from_buffer(&ctx, "<expression>".into(), buffer)
		.map(|token| token.token)
		.collect();
	if let Some(error) = ctx.errors().iter().find(|e| e.severity() == dreammaker::Severity::Error) {
		return Err(format!("couldn't expand the macros in {}: {}", used, error.description()));
	}
	Ok(text_of(&expanded))
}

/// A macro as the code would define it
fn define_line(define: &Macro) -> String {
	let mut line = format!("#define {}", define.name);
	if !define.params.is_empty() {
		line += &format!("({})", define.params.join(", "));
	}
	line.push(' ');
	line += &define.text;
	line.push('\n');
	line
}

/// Tokens as text on one line, as the lexer would read them back
pub fn text_of<'a>(tokens: impl IntoIterator<Item = &'a Token>) -> String {
	let mut text = String::new();
	let mut previous: Option<&Token> = None;
	for token in tokens {
		if matches!(token, Token::Punct(Punctuation::Newline) | Token::DocComment(_)) {
			continue;
		}
		if previous.is_some_and(|previous| token.separate_from(previous)) {
			text.push(' ');
		}
		text += &token.to_string();
		previous = Some(token);
	}
	text
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;

	use super::{expand_with, Macro};

	fn macros() -> HashMap<String, Macro> {
		let define = |name: &str, params: &[&str], text: &str| Macro {
			name: name.to_owned(),
			params: params.iter().map(|p| p.to_string()).collect(),
			text: text.to_owned()
		};
		[
			define("DOUBLE", &["x"], "((x) * 2)"),
			define("ANSWER", &[], "42"),
			define("MINUS_ONE", &[], "-1"),
			define("TWICE_ANSWER", &[], "DOUBLE(ANSWER)"),
			define("NAMED", &["x"], "#x")
		]
		.into_iter()
		.map(|define| (define.name.clone(), define))
		.collect()
	}

	fn expanded(text: &str) -> Option<String> {
		expand_with(&macros(), text).unwrap()
	}

	#[test]
	fn text_without_macros_is_left_alone() {
		assert_eq!(expanded("1 + double(2)"), None);
		assert_eq!(expanded("\"ANSWER\""), None);
		// A name of a macro with parameters isn't a use without them
		assert_eq!(expanded("DOUBLE + 1"), None);
	}

	#[test]
	fn uses_are_expanded_in_place() {
		assert_eq!(expanded("DOUBLE(ANSWER) + 1").as_deref(), Some("((42) * 2) + 1"));
		assert_eq!(expanded("TWICE_ANSWER").as_deref(), Some("((42) * 2)"));
		assert_eq!(expanded("x-MINUS_ONE").as_deref(), Some("x- - 1"));
		assert_eq!(expanded("\"[ANSWER]!\"").as_deref(), Some("\"[42]!\""));
		// (a raw text, as the preprocessor makes it)
		assert_eq!(expanded("NAMED(a + b)").as_deref(), Some("@\"a + b\""));
		// Lines, and what isn't a macro, stay as they were
		assert_eq!(
			expanded("var/x = ANSWER\n\tif (x)\n\t\treturn DOUBLE(f(1, 2))").as_deref(),
			Some("var/x = 42\n\tif (x)\n\t\treturn ((f(1, 2)) * 2)")
		);
	}

	#[test]
	fn a_use_without_its_end_is_an_error() {
		assert!(expand_with(&macros(), "DOUBLE(1").is_err());
	}
}
