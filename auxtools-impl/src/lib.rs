#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, spanned::Spanned, Lit};

fn from_signature(s: String) -> Vec<Option<u8>> {
	s.trim()
		.split(' ')
		.map(|byte| {
			let byte = byte.trim();
			match byte.len() {
				2 => {
					if byte == "??" {
						None
					} else {
						hex::decode(byte).map(|decoded_byte| decoded_byte[0]).ok()
					}
				}
				_ => None,
			}
		})
		.collect()
}

#[proc_macro]
pub fn convert_signature(input: TokenStream) -> TokenStream {
	let string = parse_macro_input!(input as Lit);
	let string = match string {
		Lit::Str(lit) => lit.value(),
		_ => panic!("not string input"),
	};

	let streams: Vec<proc_macro2::TokenStream> = from_signature(string)
		.into_iter()
		.map(|x| match x {
			Some(byte) => {
				quote! {
					Some(#byte as u8)
				}
			}
			None => {
				quote! { None }
			}
		})
		.collect();

	let result = quote! {
		&[ #( #streams, )* ]
	};
	result.into()
}

fn extract_args(a: &syn::FnArg) -> &syn::PatType {
	match a {
		syn::FnArg::Typed(p) => p,
		_ => panic!("Not supported on types with `self`!"),
	}
}

#[proc_macro_attribute]
pub fn init(attr: TokenStream, item: TokenStream) -> TokenStream {
	let init_type = syn::parse_macro_input!(attr as syn::Ident);
	let func = syn::parse_macro_input!(item as syn::ItemFn);
	let func_name = &func.sig.ident;

	let func_type = match init_type.to_string().as_str() {
		"full" => quote! { auxtools::FullInitFunc },
		"partial" => quote! { auxtools::PartialInitFunc },
		_ => {
			return syn::Error::new(init_type.span(), "invalid init type")
				.to_compile_error()
				.into()
		}
	};

	let inventory_define = quote! {
		auxtools::inventory::submit!(
			#![crate = auxtools]
			#func_type(#func_name)
		);
	};

	let code = quote! {
		#func
		#inventory_define
	};

	code.into()
}

#[proc_macro_attribute]
pub fn runtime_handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
	let func = syn::parse_macro_input!(item as syn::ItemFn);
	let func_name = &func.sig.ident;

	let inventory_define = quote! {
		auxtools::inventory::submit!(
			#![crate = auxtools]
			auxtools::RuntimeHook(#func_name)
		);
	};

	let code = quote! {
		#func
		#inventory_define
	};

	code.into()
}

#[proc_macro_attribute]
pub fn shutdown(_: TokenStream, item: TokenStream) -> TokenStream {
	let func = syn::parse_macro_input!(item as syn::ItemFn);
	let func_name = &func.sig.ident;

	let inventory_define = quote! {
		auxtools::inventory::submit!(
			#![crate = auxtools]
			auxtools::PartialShutdownFunc(#func_name)
		);
	};

	let code = quote! {
		#func
		#inventory_define
	};

	code.into()
}

/// The `hook` attribute is used to define functions that may be used as proc hooks,
/// and to optionally hook those procs upon library initialization.
///
/// # Examples
///
/// Here we define a hook that multiplies a number passed to it by two.
/// It can now be used to hook procs, for example `hooks::hook("/proc/double_up", double_up);`
/// ```ignore
/// #[hook]
/// fn double_up(num: Value) {
///     if let Some(num) = num.as_number() {
///         Value::from(num * 2.0);
///     }
///     Value::null()
/// }
/// ```
///
/// This function is used to hook `/mob/proc/on_honked`.
/// By specifying the proc path, we hook the proc immediately upon startup.
///
/// ```ignore
/// #[hook("/mob/proc/on_honked")]
/// fn on_honked(honker: Value) {
///     src.call("gib", &[]);
///     honker.call("laugh", &[]);
///     Value::null()
/// }
/// ```

#[proc_macro_attribute]
pub fn hook(attr: TokenStream, item: TokenStream) -> TokenStream {
	let input = syn::parse_macro_input!(item as syn::ItemFn);
	let proc = syn::parse_macro_input!(attr as Option<syn::Lit>);
	let func_name = &input.sig.ident;
	let args = &input.sig.inputs;
	let args_len = args.len();

	match &input.sig.output {
		syn::ReturnType::Default => {} //

		syn::ReturnType::Type(_, ty) => {
			return syn::Error::new(ty.span(), "Do not specify the return value of hooks")
				.to_compile_error()
				.into()
		}
	}

	let cthook_prelude = match proc {
		Some(p) => quote! {
			auxtools::inventory::submit!(
				#![crate = auxtools]
				auxtools::CompileTimeHook::new(#p, #func_name)
			);
		},
		None => quote! {},
	};
	let signature = quote! {
		fn #func_name(
			src: &auxtools::Value,
			usr: &auxtools::Value,
			args: &mut Vec<auxtools::Value>,
		) -> auxtools::DMResult
	};

	let body = &input.block;
	let mut arg_names: syn::punctuated::Punctuated<syn::Ident, syn::Token![,]> =
		syn::punctuated::Punctuated::new();
	let mut proc_arg_unpacker: syn::punctuated::Punctuated<
		proc_macro2::TokenStream,
		syn::Token![,],
	> = syn::punctuated::Punctuated::new();
	for arg in args.iter().map(extract_args) {
		if let syn::Pat::Ident(p) = &*arg.pat {
			arg_names.push(p.ident.clone());
			let index = arg_names.len() - 1;
			proc_arg_unpacker.push(
				(quote! {
					&args[#index]
				})
				.into(),
			);
		}
	}
	let _default_null = quote! {
		#[allow(unreachable_code)]
		auxtools::Value::null()
	};
	let result = quote! {
		#cthook_prelude
		#signature {
			if #args_len > args.len() {
				for i in 0..#args_len - args.len() {
					args.push(auxtools::Value::null())
				}
			}
			let (#arg_names) = (#proc_arg_unpacker);
			#body
		}
	};
	result.into()
}
