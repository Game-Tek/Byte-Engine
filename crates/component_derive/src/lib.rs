use proc_macro::TokenStream;
use quote::ToTokens;
use syn::parse::Parser;

#[proc_macro_attribute]
pub fn component(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let mut ast = syn::parse_macro_input!(input as syn::DeriveInput);

	if let syn::Data::Struct(ref mut data) = ast.data {
		if let syn::Fields::Named(ref mut fields) = data.fields {
			fields.named.push(syn::Field::parse_named.parse2(quote::quote!{ pub _internal_data: u32 }).unwrap());

			return ast.into_token_stream().into();
		}
	}

	TokenStream::from(
        syn::Error::new(
            ast.ident.span(),
            "Only structs with named fields can derive `Component`"
        ).to_compile_error()
    )
}