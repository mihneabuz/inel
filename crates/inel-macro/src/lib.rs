use proc_macro::TokenStream;

extern crate proc_macro;

#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    format!("fn main() {{ {item}; inel::block_on(main()) }}")
        .parse()
        .unwrap()
}
