extern crate proc_macro;

/*use proc_macro::TokenStream;
use quote::quote;
use syn::*;

#[proc_macro_derive(IntoLazy)]
pub fn derive_into_lazy(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ItemStruct);
    let name_ident = input.ident;

    proc_macro::TokenStream::from(quote! {
        impl IntoLazy for #name_ident {
            // nothing right now
        }
    })
}
*/
