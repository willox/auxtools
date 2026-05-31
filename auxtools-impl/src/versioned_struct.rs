use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{
	parse::{Parse, ParseStream},
	punctuated::Punctuated,
	spanned::Spanned,
	Attribute, Expr, Field, Fields, Ident, ItemStruct, Token, Visibility
};

// --- Input types ---

struct VersionVariant {
	name: Ident,
	condition: Option<Expr>
}

struct VersionedArgs {
	variants: Vec<VersionVariant>
}

// --- Field annotation types ---

enum FieldVersionInfo {
	AllVersions,
	OnlyIn(Ident)
}

struct VersionedField {
	field: Field,
	version_info: FieldVersionInfo
}

// --- Parsing ---

impl Parse for VersionVariant {
	fn parse(input: ParseStream) -> syn::Result<Self> {
		let name: Ident = input.parse()?;
		let condition = if input.peek(Token![if]) {
			input.parse::<Token![if]>()?;
			Some(input.parse()?)
		} else {
			None
		};
		Ok(Self { name, condition })
	}
}

impl Parse for VersionedArgs {
	fn parse(input: ParseStream) -> syn::Result<Self> {
		let variants = Punctuated::<VersionVariant, Token![,]>::parse_terminated(input)?.into_iter().collect();
		Ok(Self { variants })
	}
}

// --- Field processing ---

fn extract_field_version_info(field: &mut Field) -> FieldVersionInfo {
	let mut result = FieldVersionInfo::AllVersions;
	field.attrs.retain(|attr| {
		if attr.path().is_ident("only_in") {
			if let Ok(variant) = attr.parse_args::<Ident>() {
				result = FieldVersionInfo::OnlyIn(variant);
				return false;
			}
		}
		true
	});
	result
}

fn extract_versioned_fields(input: &mut ItemStruct) -> Result<Vec<VersionedField>, syn::Error> {
	let Fields::Named(ref mut fields) = input.fields else {
		return Err(syn::Error::new(
			input.fields.span(),
			"#[versioned] only supports structs with named fields"
		));
	};
	Ok(fields
		.named
		.iter_mut()
		.map(|field| {
			let version_info = extract_field_version_info(field);
			VersionedField {
				field: field.clone(),
				version_info
			}
		})
		.collect())
}

// --- Validation ---

fn validate_variants(variants: &[VersionVariant]) -> Option<syn::Error> {
	if variants.len() < 2 {
		return Some(syn::Error::new(Span::call_site(), "#[versioned] requires at least 2 variants"));
	}
	if variants.last().unwrap().condition.is_some() {
		return Some(syn::Error::new(
			variants.last().unwrap().name.span(),
			"the last variant must be the fallback (no `if` condition)"
		));
	}
	for v in &variants[..variants.len() - 1] {
		if v.condition.is_none() {
			return Some(syn::Error::new(v.name.span(), "only the last variant can omit the `if` condition"));
		}
	}
	None
}

// --- Code generation ---

fn field_to_tokens(field: &Field) -> TokenStream {
	let attrs = &field.attrs;
	let vis = &field.vis;
	let ident = &field.ident;
	let ty = &field.ty;
	quote! {
		#(#attrs)*
		#vis #ident: #ty
	}
}

fn variant_inner_name(struct_ident: &Ident, variant: &VersionVariant) -> Ident {
	format_ident!("{}{}", struct_ident, variant.name)
}

fn variant_field_name(variant: &VersionVariant) -> Ident {
	format_ident!("{}", variant.name.to_string().to_lowercase())
}

fn generate_variant_struct(struct_ident: &Ident, struct_attrs: &[Attribute], variant: &VersionVariant, fields: &[VersionedField]) -> TokenStream {
	let inner_name = variant_inner_name(struct_ident, variant);
	let variant_name = &variant.name;

	let variant_fields = fields.iter().filter_map(|vf| match &vf.version_info {
		FieldVersionInfo::AllVersions => Some(field_to_tokens(&vf.field)),
		FieldVersionInfo::OnlyIn(v) if v == variant_name => Some(field_to_tokens(&vf.field)),
		FieldVersionInfo::OnlyIn(_) => None
	});

	quote! {
		#[allow(dead_code)]
		#[derive(Copy, Clone)]
		#(#struct_attrs)*
		struct #inner_name {
			#(#variant_fields,)*
		}
	}
}

fn generate_union(struct_ident: &Ident, vis: &Visibility, attrs: &[Attribute], variants: &[VersionVariant]) -> TokenStream {
	let union_fields = variants.iter().map(|v| {
		let field_name = variant_field_name(v);
		let inner_name = variant_inner_name(struct_ident, v);
		quote! { #field_name: #inner_name }
	});

	quote! {
		#(#attrs)*
		#vis union #struct_ident {
			#(#union_fields,)*
		}
	}
}

fn build_dispatch_chain(struct_ident: &Ident, variants: &[VersionVariant], helper_names: &[Ident]) -> TokenStream {
	let fallback = helper_names.last().unwrap();
	let mut chain = quote! { #struct_ident::#fallback };

	for (variant, helper) in variants[..variants.len() - 1].iter().zip(&helper_names[..helper_names.len() - 1]).rev() {
		let cond = variant.condition.as_ref().unwrap();
		chain = quote! {
			if #cond { #struct_ident::#helper } else { #chain }
		};
	}

	chain
}

fn generate_field_statics(struct_ident: &Ident, field: &Field, variants: &[VersionVariant]) -> TokenStream {
	let field_name = field.ident.as_ref().unwrap();
	let field_ty = &field.ty;
	let static_name = format_ident!("__VERSIONED_{}_{}", struct_ident, field_name);
	let static_name_mut = format_ident!("__VERSIONED_MUT_{}_{}", struct_ident, field_name);
	let fallback = variants.last().unwrap();
	let fallback_fn = format_ident!("__versioned_{}_{}", field_name, variant_field_name(fallback));
	let fallback_fn_mut = format_ident!("__versioned_{}_{}_mut", field_name, variant_field_name(fallback));

	quote! {
		#[allow(non_upper_case_globals)]
		static mut #static_name: fn(&#struct_ident) -> &#field_ty = #struct_ident::#fallback_fn;
		#[allow(non_upper_case_globals)]
		static mut #static_name_mut: fn(&mut #struct_ident) -> &mut #field_ty = #struct_ident::#fallback_fn_mut;
	}
}

fn generate_init_fn(struct_ident: &Ident, pub_fields: &[&VersionedField], variants: &[VersionVariant]) -> TokenStream {
	let init_fn_name = format_ident!("__versioned_init_{}", struct_ident.to_string().to_lowercase());

	let assignments: Vec<_> = pub_fields
		.iter()
		.map(|vf| {
			let field_name = vf.field.ident.as_ref().unwrap();
			let static_name = format_ident!("__VERSIONED_{}_{}", struct_ident, field_name);
			let static_name_mut = format_ident!("__VERSIONED_MUT_{}_{}", struct_ident, field_name);

			let helper_names: Vec<Ident> = variants
				.iter()
				.map(|v| format_ident!("__versioned_{}_{}", field_name, variant_field_name(v)))
				.collect();
			let helper_names_mut: Vec<Ident> = variants
				.iter()
				.map(|v| format_ident!("__versioned_{}_{}_mut", field_name, variant_field_name(v)))
				.collect();

			let dispatch = build_dispatch_chain(struct_ident, variants, &helper_names);
			let dispatch_mut = build_dispatch_chain(struct_ident, variants, &helper_names_mut);

			quote! {
				#static_name = #dispatch;
				#static_name_mut = #dispatch_mut;
			}
		})
		.collect();

	quote! {
		fn #init_fn_name() -> Result<(), String> {
			unsafe {
				#(#assignments)*
			}
			Ok(())
		}
		crate::inventory::submit!(crate::init::PartialInitFunc(#init_fn_name));
	}
}

fn generate_impl(struct_ident: &Ident, variants: &[VersionVariant], pub_fields: &[&VersionedField]) -> TokenStream {
	let methods = pub_fields.iter().map(|vf| {
		let field = &vf.field;
		let field_name = field.ident.as_ref().unwrap();
		let field_ty = &field.ty;
		let field_name_mut = format_ident!("{}_mut", field_name);
		let static_name = format_ident!("__VERSIONED_{}_{}", struct_ident, field_name);
		let static_name_mut = format_ident!("__VERSIONED_MUT_{}_{}", struct_ident, field_name);

		let helpers = variants.iter().map(|v| {
			let helper_name = format_ident!("__versioned_{}_{}", field_name, variant_field_name(v));
			let helper_name_mut = format_ident!("__versioned_{}_{}_mut", field_name, variant_field_name(v));
			let union_field = variant_field_name(v);
			quote! {
				fn #helper_name(this: &Self) -> &#field_ty {
					unsafe { &this.#union_field.#field_name }
				}
				fn #helper_name_mut(this: &mut Self) -> &mut #field_ty {
					unsafe { &mut this.#union_field.#field_name }
				}
			}
		});

		quote! {
			#(#helpers)*
			pub fn #field_name(&self) -> &#field_ty {
				unsafe { #static_name(self) }
			}
			pub fn #field_name_mut(&mut self) -> &mut #field_ty {
				unsafe { #static_name_mut(self) }
			}
		}
	});

	quote! {
		impl #struct_ident {
			#(#methods)*
		}
	}
}

// --- Entry point ---

pub fn versioned(attr: TokenStream, item: TokenStream) -> TokenStream {
	let args = match syn::parse2::<VersionedArgs>(attr) {
		Ok(a) => a,
		Err(e) => return e.to_compile_error()
	};
	let mut input = match syn::parse2::<ItemStruct>(item) {
		Ok(i) => i,
		Err(e) => return e.to_compile_error()
	};

	if let Some(err) = validate_variants(&args.variants) {
		return err.to_compile_error();
	}

	let fields = match extract_versioned_fields(&mut input) {
		Ok(f) => f,
		Err(e) => return e.to_compile_error()
	};

	let pub_all_fields: Vec<&VersionedField> = fields
		.iter()
		.filter(|vf| matches!(vf.version_info, FieldVersionInfo::AllVersions) && matches!(vf.field.vis, Visibility::Public(_)))
		.collect();

	let variant_structs = args
		.variants
		.iter()
		.map(|v| generate_variant_struct(&input.ident, &input.attrs, v, &fields));
	let union_def = generate_union(&input.ident, &input.vis, &input.attrs, &args.variants);
	let field_statics = pub_all_fields
		.iter()
		.map(|vf| generate_field_statics(&input.ident, &vf.field, &args.variants));
	let init_fn = if pub_all_fields.is_empty() {
		quote! {}
	} else {
		generate_init_fn(&input.ident, &pub_all_fields, &args.variants)
	};
	let impl_block = generate_impl(&input.ident, &args.variants, &pub_all_fields);

	quote! {
		#(#variant_structs)*
		#union_def
		#(#field_statics)*
		#init_fn
		#impl_block
	}
}
