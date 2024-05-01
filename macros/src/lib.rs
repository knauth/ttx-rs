extern crate proc_macro;
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemMod};

#[proc_macro_attribute]
pub fn kernel(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let input = parse_macro_input!(item as ItemMod);

    // Get the module name (ident)
    let mod_name = input.ident.to_string();

    // Generate the new module with the cfg attribute
    let output = quote! {
        #[cfg(all(target_vendor = "tenstorrent", kernel_name = #mod_name))]
        #input
    };

    // Convert the output back into TokenStream and return it
    TokenStream::from(output)
}
