#![feature(proc_macro_diagnostic)]
use proc_macro::{Diagnostic, Level, TokenStream};
use syn::spanned::Spanned;

use quote::quote;
use syn::{parse_macro_input, Lit};

use std::fs::File;
use std::fs::OpenOptions;
use std::io::prelude::*;
use std::path::Path;

fn from_signature(s: String) -> Vec<Option<u8>> {
	s.trim()
		.split(" ")
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
		Lit::Str(lit) => lit.value().to_string(),
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

	return quote! {
		&[ #( #streams, )* ]
	}
	.into();
}

fn extract_args(a: &syn::FnArg) -> &syn::PatType {
	match a {
		syn::FnArg::Typed(p) => p,
		_ => panic!("Not supported on types with `self`!"),
	}
}

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
			Diagnostic::spanned(
				ty.span().unwrap(),
				Level::Error,
				"Do not specify return type of proc hooks",
			)
			.emit();
		}
	}

	let cthook_prelude = match proc {
		Some(p) => quote! {
			inventory::submit!(
				crate::hooks::CompileTimeHook::new(#p, #func_name)
			);
		},
		None => quote! {},
	};
	let signature = quote! {
		fn #func_name<'a>(
			ctx: &'a DMContext,
			src: Value<'a>,
			usr: Value<'a>,
			args: &mut Vec<Value<'a>>,
		) -> Value <'a>
	};

	let body = &input.block;
	let mut arg_names: syn::punctuated::Punctuated<syn::Ident, syn::Token![,]> =
		syn::punctuated::Punctuated::new();
	let mut proc_arg_unpacker: syn::punctuated::Punctuated<
		proc_macro2::TokenStream,
		syn::Token![,],
	> = syn::punctuated::Punctuated::new();
	for arg in args.iter().map(extract_args) {
		match &*arg.pat {
			syn::Pat::Ident(p) => {
				arg_names.push(p.ident.clone());
				let index = arg_names.len() - 1;
				proc_arg_unpacker.push(
					(quote! {
						&args[#index]
					})
					.into(),
				)
			}
			_ => {}
		};
	}
	let result = quote! {
		#cthook_prelude
		#signature {
			for i in 0..#args_len - args.len() {
				args.push(Value::null())
			}
			let (#arg_names) = (#proc_arg_unpacker);
			#body
		}
	};
	result.into()
}
