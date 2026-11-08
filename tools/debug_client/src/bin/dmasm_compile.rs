//! Prints the dmasm the debug server builds for an expression, before string
//! and proc names become ids. Pass the names of the frame's args and locals
//! after the expression, in the order the debug server would:
//! `. usr src ARGS... LOCALS...`

fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let Some((expr, params)) = args.split_first() else {
		eprintln!("usage: dmasm_compile EXPR [PARAM...]");
		std::process::exit(2);
	};
	let params: Vec<&str> = params.iter().map(String::as_str).collect();

	match dmasm::compiler::compile_expr(expr, &params) {
		Ok(nodes) => {
			for node in nodes {
				println!("{:?}", node);
			}
		}
		Err(e) => {
			eprintln!("{}", e);
			std::process::exit(1);
		}
	}
}
