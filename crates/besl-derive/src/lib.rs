//! Derive macros for declaring Rust structs as portable BESL shader structures.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
	Attribute, Data, DeriveInput, Expr, ExprLit, Fields, GenericArgument, Lit, LitStr, PathArguments, Type, parse_macro_input,
};

#[proc_macro_derive(BeslStruct, attributes(besl, besl_name, besl_type))]
pub fn derive_besl_struct(input: TokenStream) -> TokenStream {
	match derive_besl_struct_impl(parse_macro_input!(input as DeriveInput)) {
		Ok(tokens) => tokens,
		Err(error) => error.to_compile_error().into(),
	}
}

fn derive_besl_struct_impl(input: DeriveInput) -> syn::Result<TokenStream> {
	let ident = input.ident;
	let struct_name = parse_name_override(&input.attrs)?.unwrap_or_else(|| ident.to_string());

	let fields = match input.data {
		Data::Struct(data) => data.fields,
		_ => {
			return Err(syn::Error::new_spanned(
				ident,
				"Invalid BESL struct derive target. The most likely cause is that `BeslStruct` was used on a non-struct item.",
			));
		}
	};

	let named_fields = match fields {
		Fields::Named(fields) => fields.named,
		_ => {
			return Err(syn::Error::new_spanned(
				ident,
				"Invalid BESL struct fields. The most likely cause is that `BeslStruct` requires named struct fields.",
			));
		}
	};

	let field_nodes = named_fields
		.iter()
		.map(|field| {
			let field_ident = field.ident.as_ref().ok_or_else(|| {
				syn::Error::new_spanned(field, "Missing field name. The most likely cause is an unnamed struct field.")
			})?;
			let field_name = parse_name_override(&field.attrs)?.unwrap_or_else(|| field_ident.to_string());
			let field_type = parse_type_override(&field.attrs)?.unwrap_or(type_to_besl(&field.ty)?);
			let field_name_literal = LitStr::new(&field_name, field_ident.span());
			let field_type_literal = LitStr::new(&field_type, field_ident.span());

			Ok(quote! {
				::besl::ParserNode::member(#field_name_literal, #field_type_literal)
			})
		})
		.collect::<syn::Result<Vec<_>>>()?;

	let struct_name_literal = LitStr::new(&struct_name, ident.span());

	Ok(TokenStream::from(quote! {
		impl ::besl::BeslStructDefinition for #ident {
			fn besl_struct_node() -> ::besl::ParserNode<'static> {
				::besl::ParserNode::r#struct(#struct_name_literal, vec![#(#field_nodes),*])
			}
		}
	}))
}

fn parse_name_override(attributes: &[Attribute]) -> syn::Result<Option<String>> {
	parse_override(
		attributes,
		"besl_name",
		"name",
		"besl_type",
		"Duplicate BESL name override. The most likely cause is multiple `#[besl_name = ...]` attributes.",
		"Duplicate BESL name override. The most likely cause is multiple `name = ...` attributes.",
	)
}

fn parse_type_override(attributes: &[Attribute]) -> syn::Result<Option<String>> {
	parse_override(
		attributes,
		"besl_type",
		"besl_type",
		"name",
		"Duplicate BESL type override. The most likely cause is multiple `#[besl_type = ...]` attributes.",
		"Duplicate BESL type override. The most likely cause is multiple `besl_type = ...` attributes.",
	)
}

// Parses one override form while accepting the other supported nested key.
fn parse_override(
	attributes: &[Attribute],
	direct_key: &str,
	nested_key: &str,
	ignored_nested_key: &str,
	duplicate_direct_error: &'static str,
	duplicate_nested_error: &'static str,
) -> syn::Result<Option<String>> {
	let mut result = None;

	for attribute in attributes {
		if attribute.path().is_ident(direct_key) {
			if result.is_some() {
				return Err(syn::Error::new_spanned(attribute, duplicate_direct_error));
			}

			result = Some(parse_name_value_attribute(attribute, direct_key)?);
			continue;
		}

		if !attribute.path().is_ident("besl") {
			continue;
		}

		attribute.parse_nested_meta(|meta| {
			parse_nested_override(meta, nested_key, ignored_nested_key, duplicate_nested_error, &mut result)
		})?;
	}

	Ok(result)
}

// Parses the selected nested key and rejects unknown BESL attribute keys.
fn parse_nested_override(
	meta: syn::meta::ParseNestedMeta<'_>,
	nested_key: &str,
	ignored_nested_key: &str,
	duplicate_error: &'static str,
	result: &mut Option<String>,
) -> syn::Result<()> {
	if meta.path.is_ident(ignored_nested_key) {
		return Ok(());
	}
	if !meta.path.is_ident(nested_key) {
		return Err(meta.error("Unknown BESL attribute. The most likely cause is an unsupported `#[besl(...)]` key."));
	}
	if result.is_some() {
		return Err(meta.error(duplicate_error));
	}

	let value = if meta.input.peek(syn::token::Paren) {
		let content;
		syn::parenthesized!(content in meta.input);
		content.parse::<LitStr>()?
	} else {
		meta.value()?.parse::<LitStr>()?
	};
	*result = Some(value.value());
	Ok(())
}

fn parse_name_value_attribute(attribute: &Attribute, attribute_name: &str) -> syn::Result<String> {
	match &attribute.meta {
		syn::Meta::NameValue(name_value) => match &name_value.value {
			Expr::Lit(ExprLit {
				lit: Lit::Str(value), ..
			}) => Ok(value.value()),
			_ => Err(syn::Error::new_spanned(
				attribute,
				format!("Invalid {attribute_name} attribute. The most likely cause is a non-string literal value."),
			)),
		},
		_ => Err(syn::Error::new_spanned(
			attribute,
			format!("Invalid {attribute_name} attribute. The most likely cause is missing `= \"...\"` syntax."),
		)),
	}
}

fn type_to_besl(ty: &Type) -> syn::Result<String> {
	match ty {
		Type::Path(path) => path_to_besl(path),
		Type::Array(array) => {
			let element_type = type_to_besl(&array.elem)?;
			let count = match &array.len {
				Expr::Lit(ExprLit {
					lit: Lit::Int(value), ..
				}) => value.base10_digits().to_string(),
				_ => {
					return Err(syn::Error::new_spanned(
						&array.len,
						"Invalid BESL array length. The most likely cause is a non-literal array size.",
					));
				}
			};

			Ok(format!("{element_type}[{count}]"))
		}
		_ => Err(syn::Error::new_spanned(
			ty,
			"Unsupported BESL field type. The most likely cause is that the field type is not a path or fixed-size array.",
		)),
	}
}

fn path_to_besl(path: &syn::TypePath) -> syn::Result<String> {
	let mut segments = Vec::new();

	for segment in &path.path.segments {
		match &segment.arguments {
			PathArguments::None => segments.push(segment.ident.to_string()),
			PathArguments::AngleBracketed(arguments) => {
				let generic_arguments = arguments
					.args
					.iter()
					.map(|argument| match argument {
						GenericArgument::Type(ty) => type_to_besl(ty),
						_ => Err(syn::Error::new_spanned(
							argument,
							"Unsupported BESL generic argument. The most likely cause is a non-type generic argument.",
						)),
					})
					.collect::<syn::Result<Vec<_>>>()?;

				segments.push(format!("{}<{}>", segment.ident, generic_arguments.join(",")));
			}
			_ => {
				return Err(syn::Error::new_spanned(
					segment,
					"Unsupported BESL path arguments. The most likely cause is parenthesized path arguments.",
				));
			}
		}
	}

	segments.last().cloned().ok_or_else(|| {
		syn::Error::new_spanned(
			path,
			"Invalid BESL type path. The most likely cause is an empty Rust type path.",
		)
	})
}
