use proc_macro::TokenStream;
use quote::quote;
use syn::DeriveInput;
use syn::parse_macro_input;

/// Derive macro that automatically generates JSON schema from a struct definition.
///
/// This macro generates an implementation of the `ToJsonSchema` trait that returns
/// the corresponding JSON schema for OpenAI tool calls.
///
/// Field descriptions can be provided using doc comments:
///
/// # Example
/// ```rust
/// #[derive(ToolSchema)]
/// struct WeatherParams {
///     /// The city and country, e.g. "Bogotá, Colombia"
///     city: String,
///     /// Optional temperature reading
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
                use crate::openai_tools::Property;
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

                // Extract description from doc comments
                let description = extract_doc_comment(&field.attrs);

                // Map Rust types to JSON schema types
                let property = map_rust_type_to_property(field_type, description.as_deref());

                let field_name_str = field_name.to_string();
                properties.push(quote! {
                    properties.insert(#field_name_str.to_string(), #property);
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

/// Extract documentation comment from field attributes
fn extract_doc_comment(attrs: &[syn::Attribute]) -> Option<String> {
    let mut doc_parts = Vec::new();

    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let syn::Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit_str),
                    ..
                }) = &meta.value
                {
                    let comment = lit_str.value();
                    // Remove leading/trailing whitespace and common doc comment prefixes
                    let trimmed = comment
                        .trim()
                        .trim_start_matches("//")
                        .trim_start_matches("///")
                        .trim();
                    if !trimmed.is_empty() {
                        doc_parts.push(trimmed.to_string());
                    }
                }
            }
        }
    }

    if doc_parts.is_empty() {
        None
    } else {
        Some(doc_parts.join(" "))
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

fn map_rust_type_to_property(
    ty: &syn::Type,
    description: Option<&str>,
) -> proc_macro2::TokenStream {
    let desc = description
        .map(|d| quote! { Some(#d) })
        .unwrap_or_else(|| quote! { None });

    match ty {
        syn::Type::Path(type_path) => {
            let path = &type_path.path;
            let segments: Vec<_> = path.segments.iter().map(|s| &s.ident).collect();

            if segments.len() == 1 {
                match segments[0].to_string().as_str() {
                    "String" => quote! {
                        Property::WithDescription {
                            schema: JsonSchema::String,
                            description: #desc,
                            enum_values: None,
                        }
                    },
                    "str" => quote! {
                        Property::WithDescription {
                            schema: JsonSchema::String,
                            description: #desc,
                            enum_values: None,
                        }
                    },
                    "i32" | "i64" | "u32" | "u64" | "f32" | "f64" => quote! {
                        Property::WithDescription {
                            schema: JsonSchema::Number,
                            description: #desc,
                            enum_values: None,
                        }
                    },
                    "bool" => quote! {
                        Property::WithDescription {
                            schema: JsonSchema::Boolean,
                            description: #desc,
                            enum_values: None,
                        }
                    },
                    "Vec" => {
                        quote! {
                            Property::WithDescription {
                                schema: JsonSchema::Array {
                                    items: Box::new(JsonSchema::String),
                                },
                                description: #desc,
                                enum_values: None,
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
                                    return map_rust_type_to_property(inner_type, description);
                                }
                            }
                        }
                        // Fallback if we can't extract inner type
                        quote! {
                            Property::WithDescription {
                                schema: JsonSchema::String,
                                description: #desc,
                                enum_values: None,
                            }
                        }
                    }
                    _ => quote! {
                        Property::WithDescription {
                            schema: JsonSchema::String,
                            description: #desc,
                            enum_values: None,
                        }
                    }, // Default to string
                }
            } else if segments.len() == 2 && segments[0].to_string() == "Vec" {
                quote! {
                    Property::WithDescription {
                        schema: JsonSchema::Array {
                            items: Box::new(JsonSchema::String),
                        },
                        description: #desc,
                        enum_values: None,
                    }
                }
            } else {
                quote! {
                    Property::WithDescription {
                        schema: JsonSchema::String,
                        description: #desc,
                        enum_values: None,
                    }
                } // Default fallback
            }
        }
        syn::Type::Reference(_) => quote! {
            Property::WithDescription {
                schema: JsonSchema::String,
                description: #desc,
                enum_values: None,
            }
        }, // &str, &String, etc.
        _ => quote! {
            Property::WithDescription {
                schema: JsonSchema::String,
                description: #desc,
                enum_values: None,
            }
        }, // Default fallback
    }
}
