//! `#[derive(LuaTable)]`: parses a neori config struct from a Lua table, so that a field added to
//! the struct is a config key right away.
//!
//! Each named field becomes a key, accepted in snake_case and kebab-case
//! (`active_color` and `active-color`), and is parsed by its type's `FromLuaConfig`
//! implementation. An `Option<T>` field is parsed as a `T` and set to `Some`; any other field is
//! merged into its current value, so a nested table only changes the keys it has. Keys the
//! struct doesn't have are reported as unknown.
//!
//! Struct attributes:
//! - `#[lua(extra = path::to::fn)]`: handle keys that aren't fields (like `on` for a struct with an
//!   `off` field) with `fn(&mut Self, &str, &Value, &str) -> Result<bool, String>`, which returns
//!   whether it knew the key.
//! - `#[lua(class = "LayerMatch")]`: the LuaLS class name, `neori.config.<class>`, when the
//!   struct's own name is taken by another struct.
//!
//! Field attributes:
//! - `#[lua(rename = "in")]`: use this key instead of the field name.
//! - `#[lua(alias = "match")]`: also accept this key.
//! - `#[lua(skip)]`: not settable from Lua.
//! - `#[lua(with = path::to::fn)]`: parse with `fn(&Value, &str) -> Result<Option<T>, String>`,
//!   where `T` is the field's type.
//! - `#[lua(lua_type = "string|table")]`: the LuaLS type, for a field parsed `with` a function.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    parse_macro_input, Data, DeriveInput, Expr, Fields, GenericArgument, PathArguments, Type,
};

#[proc_macro_derive(LuaTable, attributes(lua))]
pub fn derive_lua_table(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[derive(Default)]
struct FieldAttrs {
    rename: Option<String>,
    aliases: Vec<String>,
    skip: bool,
    with: Option<Expr>,
    lua_type: Option<String>,
}

fn field_attrs(field: &syn::Field) -> syn::Result<FieldAttrs> {
    let mut attrs = FieldAttrs::default();
    for attr in &field.attrs {
        if !attr.path().is_ident("lua") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                attrs.rename = Some(meta.value()?.parse::<syn::LitStr>()?.value());
            } else if meta.path.is_ident("alias") {
                attrs
                    .aliases
                    .push(meta.value()?.parse::<syn::LitStr>()?.value());
            } else if meta.path.is_ident("skip") {
                attrs.skip = true;
            } else if meta.path.is_ident("with") {
                attrs.with = Some(meta.value()?.parse::<Expr>()?);
            } else if meta.path.is_ident("lua_type") {
                attrs.lua_type = Some(meta.value()?.parse::<syn::LitStr>()?.value());
            } else {
                return Err(meta.error("unknown lua attribute"));
            }
            Ok(())
        })?;
    }
    Ok(attrs)
}

/// The doc comment on a field, joined into one line.
fn doc_comment(field: &syn::Field) -> String {
    let mut lines = Vec::new();
    for attr in &field.attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let syn::Meta::NameValue(nv) = &attr.meta {
            if let Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
            {
                lines.push(s.value().trim().to_owned());
            }
        }
    }
    lines.join(" ")
}

/// `T` if `ty` is `Option<T>`.
fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    match args.args.first()? {
        GenericArgument::Type(inner) if args.args.len() == 1 => Some(inner),
        _ => None,
    }
}

/// The struct's `#[lua(extra = …)]` function and `#[lua(class = …)]` name, if any.
fn struct_attrs(input: &DeriveInput) -> syn::Result<(Option<Expr>, Option<String>)> {
    let mut extra = None;
    let mut class = None;
    for attr in &input.attrs {
        if !attr.path().is_ident("lua") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("extra") {
                extra = Some(meta.value()?.parse::<Expr>()?);
                Ok(())
            } else if meta.path.is_ident("class") {
                class = Some(meta.value()?.parse::<syn::LitStr>()?.value());
                Ok(())
            } else {
                Err(meta.error("unknown lua attribute"))
            }
        })?;
    }
    Ok((extra, class))
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let (extra, class) = struct_attrs(input)?;
    let class = format!("neori.config.{}", class.unwrap_or_else(|| name.to_string()));
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(input, "LuaTable needs a struct"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            input,
            "LuaTable needs named fields",
        ));
    };

    let mut arms = Vec::new();
    let mut infos = Vec::new();
    for field in &fields.named {
        let attrs = field_attrs(field)?;
        if attrs.skip {
            continue;
        }
        let ident = field.ident.as_ref().unwrap();
        let key = attrs
            .rename
            .clone()
            .unwrap_or_else(|| ident.to_string().trim_end_matches('_').to_owned());
        let mut keys = vec![key.clone()];
        let kebab = key.replace('_', "-");
        if kebab != key {
            keys.push(kebab);
        }
        keys.extend(attrs.aliases.iter().cloned());

        let ty = &field.ty;
        let (parse, lua_type, lua_class) = if let Some(with) = &attrs.with {
            let lua_type = attrs.lua_type.clone().unwrap_or_else(|| "any".to_owned());
            (
                quote! {
                    if let Some(v) = (#with)(value, &path)? {
                        self.#ident = v;
                    }
                },
                quote! { ::std::string::String::from(#lua_type) },
                quote! { || None },
            )
        } else if let Some(inner) = option_inner(ty) {
            (
                quote! {
                    if let Some(v) = <#inner as crate::lua_table::FromLuaConfig>::from_lua_config(value, &path)? {
                        self.#ident = Some(v);
                    }
                },
                quote! { <#inner as crate::lua_table::FromLuaConfig>::lua_type() },
                quote! { <#inner as crate::lua_table::FromLuaConfig>::lua_class },
            )
        } else {
            (
                quote! {
                    crate::lua_table::FromLuaConfig::merge_from_lua(&mut self.#ident, value, &path)?;
                },
                quote! { <#ty as crate::lua_table::FromLuaConfig>::lua_type() },
                quote! { <#ty as crate::lua_table::FromLuaConfig>::lua_class },
            )
        };

        arms.push(quote! {
            #(#keys)|* => {
                let path = ::std::format!("{}.{}", path, #key);
                #parse
            }
        });
        let doc = doc_comment(field);
        // Aliases are listed too: `match` is how the docs write `matches`.
        for name in std::iter::once(&key).chain(&attrs.aliases) {
            infos.push(quote! {
                crate::lua_table::LuaField {
                    name: #name,
                    lua_type: #lua_type,
                    class: #lua_class,
                    doc: #doc,
                }
            });
        }
    }

    let fallback = match &extra {
        Some(extra) => quote! { (#extra)(self, key, value, path) },
        None => quote! { Ok(false) },
    };

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics crate::lua_table::LuaTable for #name #ty_generics #where_clause {
            fn set_lua_key(
                &mut self,
                key: &str,
                value: &::mlua::Value,
                path: &str,
            ) -> ::std::result::Result<bool, ::std::string::String> {
                match key {
                    #(#arms)*
                    _ => return #fallback,
                }
                Ok(true)
            }

            fn lua_fields() -> ::std::vec::Vec<crate::lua_table::LuaField> {
                ::std::vec![#(#infos),*]
            }
        }

        impl #impl_generics crate::lua_table::FromLuaConfig for #name #ty_generics #where_clause {
            fn from_lua_config(
                value: &::mlua::Value,
                path: &str,
            ) -> ::std::result::Result<Option<Self>, ::std::string::String> {
                crate::lua_table::table_from_lua(value, path)
            }

            fn merge_from_lua(
                &mut self,
                value: &::mlua::Value,
                path: &str,
            ) -> ::std::result::Result<(), ::std::string::String> {
                crate::lua_table::merge_table_from_lua(self, value, path)
            }

            fn lua_type() -> ::std::string::String {
                ::std::string::String::from(#class)
            }

            fn lua_class() -> Option<crate::lua_table::LuaClass> {
                Some(crate::lua_table::LuaClass {
                    name: ::std::string::String::from(#class),
                    fields: <Self as crate::lua_table::LuaTable>::lua_fields(),
                })
            }
        }
    })
}
