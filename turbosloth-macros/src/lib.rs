extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::*;

#[proc_macro_derive(IntoLazy)]
pub fn derive_into_lazy(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ItemStruct);
    let name_ident = input.ident.clone();

    //let name = input.sig.ident.clone();

    /*let mut inputs: Vec<FnArg> = input.sig.inputs.iter().skip(1).cloned().collect();
    for mut arg in inputs.iter_mut() {
        if let FnArg::Typed(arg) = &mut arg {
            if let Pat::Ident(ident) = &mut *arg.pat {
                // "patterns aren't allowed in functions without bodies"
                ident.mutability = None;
            }
        }
    }

    let input_forwards: Vec<_> = input
        .sig
        .inputs
        .iter()
        .skip(1)
        .map(|arg| {
            if let FnArg::Typed(ref arg) = arg {
                let ident = if let Pat::Ident(ref ident) = *arg.pat {
                    ident.ident.clone()
                } else {
                    panic!("onoz, non-ident fn arg");
                };

                ident.clone()
            } else {
                panic!("onoz, non-Typed fn arg");
            }
        })
        .collect();*/

    proc_macro::TokenStream::from(quote! {
        impl IntoLazy for #name_ident {}
    })
}
