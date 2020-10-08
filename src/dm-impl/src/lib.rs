use proc_macro::TokenStream;

use quote::quote;
use syn::{parse_macro_input, Lit};

fn from_signature(s: String) -> Vec<Option<u8>> {
	s.split(" ")
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
		vec![ #( #streams, )* ]
	}
	.into();
}
