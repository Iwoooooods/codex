use proc_macro::TokenStream;
use quote::quote;
use syn::DeriveInput;
use syn::parse_macro_input;

/// Derive macro that automatically generates JSON schema from a struct definition.
///
/// This macro generates an implementation of the `ToJsonSchema` trait that returns
/// the corresponding JSON schema for OpenAI tool calls.
///
/// # Example
/// ```rust
/// #[derive(ToolSchema)]
/// struct WeatherParams {
///     city: String,
///     temperature: Option<i32>,
/// }
/// ```
#[proc_macro_derive(ToolSchema)]
pub fn derive_tool_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();

    // Generate the schema from struct fields
    let schema = generate_schema_from_struct(&input);

    let expanded = quote! {
        impl ToJsonSchema for #name {
            fn to_json_schema() -> JsonSchema {
                use std::collections::BTreeMap;
                #schema
            }
        }
    };

    TokenStream::from(expanded)
}

fn generate_schema_from_struct(input: &DeriveInput) -> proc_macro2::TokenStream {
    let mut properties = Vec::new();
    let mut required = Vec::new();

    if let syn::Data::Struct(data) = &input.data {
        for field in &data.fields {
            if let Some(ident) = &field.ident {
                let field_name = ident;
                let field_type = &field.ty;

                // Map Rust types to JSON schema types
                let schema_type = map_rust_type_to_schema(field_type);

                let field_name_str = field_name.to_string();
                properties.push(quote! {
                    properties.insert(#field_name_str.to_string(), #schema_type);
                });

                // Check if field is optional (Option<T>)
                if is_option_type(field_type) {
                    // Optional fields are not required
                } else {
                    required.push(quote! {
                        #field_name_str
                    });
                }
            }
        }
    }

    quote! {
        JsonSchema::Object {
            properties: {
                let mut properties = BTreeMap::new();
                #(#properties)*
                properties
            },
            required: &[#(#required),*],
            additional_properties: false,
        }
    }
}

fn is_option_type(ty: &syn::Type) -> bool {
    if let syn::Type::Path(type_path) = ty {
        let path = &type_path.path;
        if path.segments.len() == 1 {
            return path.segments[0].ident == "Option";
        }
    }
    false
}

fn map_rust_type_to_schema(ty: &syn::Type) -> proc_macro2::TokenStream {
    match ty {
        syn::Type::Path(type_path) => {
            let path = &type_path.path;
            let segments: Vec<_> = path.segments.iter().map(|s| &s.ident).collect();

            if segments.len() == 1 {
                match segments[0].to_string().as_str() {
                    "String" => quote! { JsonSchema::String },
                    "str" => quote! { JsonSchema::String },
                    "i32" | "i64" | "u32" | "u64" | "f32" | "f64" => quote! { JsonSchema::Number },
                    "bool" => quote! { JsonSchema::Boolean },
                    "Vec" => {
                        quote! {
                            JsonSchema::Array {
                                items: Box::new(JsonSchema::String),
                            }
                        }
                    }
                    "Option" => {
                        // Handle Option<T> by extracting the inner type from type arguments
                        if let Some(segment) = path.segments.first() {
                            if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                                if let Some(syn::GenericArgument::Type(inner_type)) =
                                    args.args.first()
                                {
                                    return map_rust_type_to_schema(inner_type);
                                }
                            }
                        }
                        // Fallback if we can't extract inner type
                        quote! { JsonSchema::String }
                    }
                    _ => quote! { JsonSchema::String }, // Default to string
                }
            } else if segments.len() == 2 && segments[0].to_string() == "Vec" {
                quote! {
                    JsonSchema::Array {
                        items: Box::new(JsonSchema::String),
                    }
                }
            } else {
                quote! { JsonSchema::String } // Default fallback
            }
        }
        syn::Type::Reference(_) => quote! { JsonSchema::String }, // &str, &String, etc.
        _ => quote! { JsonSchema::String },                       // Default fallback
    }
}
