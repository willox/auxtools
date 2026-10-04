mod versioned_struct;

use proc_macro::TokenStream;
use quote::quote;
use syn::{spanned::Spanned, Lit};

#[proc_macro_attribute]
pub fn versioned(attr: TokenStream, item: TokenStream) -> TokenStream {
	versioned_struct::versioned(attr.into(), item.into()).into()
}

fn extract_args(a: &syn::FnArg) -> &syn::PatType {
	match a {
		syn::FnArg::Typed(p) => p,
		_ => panic!("Not supported on types with `self`!")
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
		_ => return syn::Error::new(init_type.span(), "invalid init type").to_compile_error().into()
	};

	let inventory_define = quote! {
		auxtools::inventory::submit!(
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
			auxtools::RuntimeErrorHook(#func_name)
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
			auxtools::PartialShutdownFunc(#func_name)
		);
	};

	let code = quote! {
		#func
		#inventory_define
	};

	code.into()
}

#[proc_macro_attribute]
pub fn full_shutdown(_: TokenStream, item: TokenStream) -> TokenStream {
	let func = syn::parse_macro_input!(item as syn::ItemFn);
	let func_name = &func.sig.ident;

	let inventory_define = quote! {
		auxtools::inventory::submit!(
			#![crate = auxtools]
			auxtools::FullShutdownFunc(#func_name)
		);
	};

	let code = quote! {
		#func
		#inventory_define
	};

	code.into()
}

/// The `pin_dll!` macro is used to determine whether the dll handle auxtools
/// takes on Windows is pinned. For reference, a dll with a pinned handle cannot
/// be unloaded during execution of the host process - termination of the host
/// is the only way to unload the dll and release the lock on the corresponding
/// file.
///
/// This has very limited use cases - for instance, if a .dmb is hosted on a
/// live server whose Dream Daemon process is kept running between runs, keeping
/// a pinned handle to the dll will prevent the corresponding file from being
/// updated by automatic updaters such as tgs. You shouldn't use this unless you
/// very specifically need it for your particular use case.
///
/// Libraries that unpin the dll using this macro should ensure that no spawned
/// threads are running when calling `auxtools_full_shutdown` from DM, or else
/// Dream Daemon will crash.
#[proc_macro]
pub fn pin_dll(attr: TokenStream) -> TokenStream {
	let flag = syn::parse_macro_input!(attr as syn::LitBool);
	let code = quote! {
		use std::sync::atomic::{AtomicBool, Ordering};

		#[auxtools::ctor::ctor]
		#[cfg(windows)]
		fn set_pin_dll() {
			auxtools::PIN_DLL.store(#flag, Ordering::Relaxed);
		}
	};

	code.into()
}

/// The `hook` attribute is used to define functions that may be used as proc
/// hooks, and to optionally hook those procs upon library initialization.
///
/// # Examples
///
/// Here we define a hook that multiplies a number passed to it by two.
/// It can now be used to hook procs, for example
/// `Proc::find("/proc/double_up").unwrap().hook(double_up);`
/// ```ignore
/// #[hook]
/// fn double_up(num: Value) {
///     Ok(Value::from(num.as_number()? * 2.0))
/// }
/// ```
///
/// This function is used to hook `/mob/proc/on_honked`.
/// By specifying the proc path, we hook the proc immediately upon startup.
/// ```ignore
/// #[hook("/mob/proc/on_honked")]
/// fn on_honked(honker: Value) {
///     src.call("gib", &[])?;
///     honker.call("laugh", &[])?;
///     Ok(Value::NULL)
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
		Some(Lit::Str(p)) => {
			quote! {
				auxtools::inventory::submit!({
					auxtools::CompileTimeHook{ proc_path: #p, hook: #func_name }
				});
			}
		}
		Some(other_literal) => {
			return syn::Error::new(other_literal.span(), "Hook attributes must be a string literal")
				.to_compile_error()
				.into()
		}
		None => quote! {}
	};
	let signature = quote! {
		fn #func_name(
			src: &auxtools::Value,
			usr: &auxtools::Value,
			mut args: Vec<auxtools::Value>,
		) -> auxtools::DMResult
	};

	let body = &input.block;
	let mut arg_names: syn::punctuated::Punctuated<syn::Ident, syn::Token![,]> = syn::punctuated::Punctuated::new();
	let mut proc_arg_unpacker: syn::punctuated::Punctuated<proc_macro2::TokenStream, syn::Token![,]> = syn::punctuated::Punctuated::new();

	for arg in args.iter().map(extract_args) {
		if let syn::Pat::Ident(p) = &*arg.pat {
			arg_names.push(p.ident.clone());
			let index = arg_names.len() - 1;
			proc_arg_unpacker.push(quote! {
				&args[#index]
			});
		}
	}
	let _default_null = quote! {
		#[allow(unreachable_code)]
		auxtools::Value::NULL
	};
	let result = quote! {
		#cthook_prelude
		#signature {
			if #args_len > args.len() {
				for i in 0..#args_len - args.len() {
					args.push(auxtools::Value::NULL)
				}
			}
			let (#arg_names) = (#proc_arg_unpacker);
			#body
		}
	};
	result.into()
}
