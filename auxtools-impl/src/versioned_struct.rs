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

fn build_dispatch_chain(variants: &[VersionVariant], helper_names: &[Ident]) -> TokenStream {
	let fallback = helper_names.last().unwrap();
	let mut chain = quote! { Self::#fallback };

	for (variant, helper) in variants[..variants.len() - 1].iter().zip(&helper_names[..helper_names.len() - 1]).rev() {
		let cond = variant.condition.as_ref().unwrap();
		chain = quote! {
			if #cond { Self::#helper } else { #chain }
		};
	}

	chain
}

fn generate_field_accessor(struct_ident: &Ident, field: &Field, variants: &[VersionVariant]) -> TokenStream {
	let field_name = field.ident.as_ref().unwrap();
	let field_ty = &field.ty;

	let helper_names: Vec<Ident> = variants
		.iter()
		.map(|v| format_ident!("__versioned_{}_{}", field_name, variant_field_name(v)))
		.collect();

	let dispatch = build_dispatch_chain(variants, &helper_names);

	let helpers = variants.iter().zip(&helper_names).map(|(v, helper_name)| {
		let union_field = variant_field_name(v);
		quote! {
			#[inline(never)]
			fn #helper_name(this: &Self) -> #field_ty {
				unsafe { this.#union_field.#field_name }
			}
		}
	});

	quote! {
		pub fn #field_name(&self) -> #field_ty {
			static REDIRECT: ::std::sync::OnceLock<fn(&#struct_ident) -> #field_ty> =
				::std::sync::OnceLock::new();
			REDIRECT.get_or_init(|| unsafe { #dispatch })(self)
		}
		#(#helpers)*
	}
}

fn generate_field_ptr_accessor(struct_ident: &Ident, field: &Field, variants: &[VersionVariant]) -> TokenStream {
	let field_name = field.ident.as_ref().unwrap();
	let field_ty = &field.ty;
	let ptr_name = format_ident!("{}_ptr", field_name);

	let helper_names: Vec<Ident> = variants
		.iter()
		.map(|v| format_ident!("__versioned_{}_{}_ptr", field_name, variant_field_name(v)))
		.collect();

	let dispatch = build_dispatch_chain(variants, &helper_names);

	let helpers = variants.iter().zip(&helper_names).map(|(v, helper_name)| {
		let union_field = variant_field_name(v);
		quote! {
			#[inline(never)]
			fn #helper_name(this: &mut Self) -> *mut #field_ty {
				unsafe { ::std::ptr::addr_of_mut!(this.#union_field.#field_name) }
			}
		}
	});

	quote! {
		pub fn #ptr_name(&mut self) -> *mut #field_ty {
			static REDIRECT: ::std::sync::OnceLock<fn(&mut #struct_ident) -> *mut #field_ty> =
				::std::sync::OnceLock::new();
			REDIRECT.get_or_init(|| unsafe { #dispatch })(self)
		}
		#(#helpers)*
	}
}

fn generate_impl(struct_ident: &Ident, variants: &[VersionVariant], fields: &[VersionedField]) -> TokenStream {
	let pub_all_fields: Vec<_> = fields
		.iter()
		.filter(|vf| matches!(vf.version_info, FieldVersionInfo::AllVersions) && matches!(vf.field.vis, Visibility::Public(_)))
		.collect();

	let value_accessors = pub_all_fields.iter().map(|vf| generate_field_accessor(struct_ident, &vf.field, variants));
	let ptr_accessors = pub_all_fields
		.iter()
		.map(|vf| generate_field_ptr_accessor(struct_ident, &vf.field, variants));

	quote! {
		impl #struct_ident {
			#(#value_accessors)*
			#(#ptr_accessors)*
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

	let variant_structs = args
		.variants
		.iter()
		.map(|v| generate_variant_struct(&input.ident, &input.attrs, v, &fields));
	let union_def = generate_union(&input.ident, &input.vis, &input.attrs, &args.variants);
	let impl_block = generate_impl(&input.ident, &args.variants, &fields);

	quote! {
		#(#variant_structs)*
		#union_def
		#impl_block
	}
}
