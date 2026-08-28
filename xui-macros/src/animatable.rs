//! `#[derive(Animatable)]` — a field-wise `Animatable` impl.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Error, Fields, Result, parse_quote};

use crate::krate;

pub fn expand_derive_animatable(input: &DeriveInput) -> Result<TokenStream2> {
    let Data::Struct(data) = &input.data else {
        return Err(Error::new(
            input.span(),
            "Animatable can only be derived for structs",
        ));
    };

    let animatable_path = krate::animatable()?;
    let field_types = data
        .fields
        .iter()
        .map(|field| field.ty.clone())
        .collect::<Vec<_>>();

    let mut generics = input.generics.clone();
    if !field_types.is_empty() {
        let where_clause = generics.make_where_clause();
        for field_type in &field_types {
            where_clause
                .predicates
                .push(parse_quote!(#field_type: #animatable_path::Animatable));
        }
    }

    let ident = &input.ident;
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let body = expand_animatable_struct_body(&data.fields, &animatable_path)?;

    Ok(quote! {
        impl #impl_generics #animatable_path::Animatable for #ident #type_generics #where_clause {
            fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
                #body
            }
        }
    })
}


fn expand_animatable_struct_body(
    fields: &Fields,
    animatable_path: &TokenStream2,
) -> Result<TokenStream2> {
    match fields {
        Fields::Named(fields) => {
            let field_values = fields
                .named
                .iter()
                .map(|field| {
                    let ident = field
                        .ident
                        .as_ref()
                        .expect("named fields always have identifiers");
                    let ty = &field.ty;
                    quote! {
                        #ident: <#ty as #animatable_path::Animatable>::interpolate(
                            &from.#ident,
                            &to.#ident,
                            progress,
                        )
                    }
                })
                .collect::<Vec<_>>();

            Ok(quote! {
                Self {
                    #(#field_values),*
                }
            })
        }
        Fields::Unnamed(fields) => {
            let field_values = fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let index = syn::Index::from(index);
                    let ty = &field.ty;
                    quote! {
                        <#ty as #animatable_path::Animatable>::interpolate(
                            &from.#index,
                            &to.#index,
                            progress,
                        )
                    }
                })
                .collect::<Vec<_>>();

            Ok(quote! {
                Self(#(#field_values),*)
            })
        }
        Fields::Unit => Ok(quote!(Self)),
    }
}

