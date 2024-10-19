use proc_macro::TokenStream;

extern crate proc_macro;

#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    format!("fn main() {{ {item}; inel::block_on(main()) }}")
        .parse()
        .unwrap()
}

#[proc_macro_attribute]
pub fn test_repeat(attr: TokenStream, item: TokenStream) -> TokenStream {
    let name = item.clone().into_iter().nth(1).unwrap().to_string();
    format!("fn {name}() {{ {item}; for _ in (0..{attr}) {{ {name}(); }} }}")
        .parse()
        .unwrap()
}
