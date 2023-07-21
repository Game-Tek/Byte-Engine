use proc_macro::TokenStream;
use quote::quote;
use syn;

#[proc_macro_derive(Component, attributes(field))]
pub fn component_macro_derive(input: TokenStream) -> TokenStream {
    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let ast = syn::parse(input).unwrap();

    // Build the trait implementation
    impl_component_macro(&ast)
}

#[proc_macro_attribute]
pub fn field(attr: TokenStream, item: TokenStream) -> TokenStream {
	let gen = quote! {
		impl Mesh {
			fn set_transform(&mut self, value: maths_rs::Mat4f) { self.transform = value; }

			fn get_transform(&self) -> maths_rs::Mat4f { self.transform }

			pub const fn transform() -> Property<(), Self, maths_rs::Mat4f> { Property::Component { getter: Mesh::get_transform, setter: Mesh::set_transform } }
		}
	};
	gen.into()
}

fn impl_component_macro(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let gen = quote! {
    };
    gen.into()
}