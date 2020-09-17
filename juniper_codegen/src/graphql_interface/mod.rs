//! Code generation for [GraphQL interface][1].
//!
//! [1]: https://spec.graphql.org/June2018/#sec-Interfaces

pub mod attr;

use std::collections::{HashMap, HashSet};

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, ToTokens, TokenStreamExt as _};
use syn::{
    parse::{Parse, ParseStream},
    parse_quote,
    spanned::Spanned as _,
    token,
};

use crate::{
    common::{
        gen,
        parse::{
            attr::{err, OptionExt as _},
            GenericsExt as _, ParseBufferExt as _,
        },
        ScalarValueType,
    },
    util::{filter_attrs, get_deprecated, get_doc_comment, span_container::SpanContainer},
};

/// Helper alias for the type of [`InterfaceMeta::external_downcasts`] field.
type InterfaceMetaDowncasts = HashMap<syn::Type, SpanContainer<syn::ExprPath>>;

/// Available metadata (arguments) behind `#[graphql]` (or `#[graphql_interface]`) attribute placed
/// on a trait definition, when generating code for [GraphQL interface][1] type.
///
/// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
#[derive(Debug, Default)]
struct InterfaceMeta {
    /// Explicitly specified name of [GraphQL interface][1] type.
    ///
    /// If absent, then Rust type name is used by default.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    pub name: Option<SpanContainer<String>>,

    /// Explicitly specified [description][2] of [GraphQL interface][1] type.
    ///
    /// If absent, then Rust doc comment is used as [description][2], if any.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    /// [2]: https://spec.graphql.org/June2018/#sec-Descriptions
    pub description: Option<SpanContainer<String>>,

    pub as_enum: Option<SpanContainer<syn::Ident>>,

    pub as_dyn: Option<SpanContainer<syn::Ident>>,

    /// Explicitly specified Rust types of [GraphQL objects][2] implementing this
    /// [GraphQL interface][1] type.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    /// [2]: https://spec.graphql.org/June2018/#sec-Objects
    pub implementers: HashSet<SpanContainer<syn::Type>>,

    /// Explicitly specified type of `juniper::Context` to use for resolving this
    /// [GraphQL interface][1] type with.
    ///
    /// If absent, then unit type `()` is assumed as type of `juniper::Context`.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    pub context: Option<SpanContainer<syn::Type>>,

    /// Explicitly specified type of `juniper::ScalarValue` to use for resolving this
    /// [GraphQL interface][1] type with.
    ///
    /// If absent, then generated code will be generic over any `juniper::ScalarValue` type, which,
    /// in turn, requires all [interface][1] implementers to be generic over any
    /// `juniper::ScalarValue` type too. That's why this type should be specified only if one of the
    /// implementers implements `juniper::GraphQLType` in a non-generic way over
    /// `juniper::ScalarValue` type.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    pub scalar: Option<SpanContainer<syn::Type>>,

    pub asyncness: Option<SpanContainer<syn::Ident>>,

    /// Explicitly specified external downcasting functions for [GraphQL interface][1] implementers.
    ///
    /// If absent, then macro will try to auto-infer all the possible variants from the type
    /// declaration, if possible. That's why specifying an external resolver function has sense,
    /// when some custom [union][1] variant resolving logic is involved, or variants cannot be
    /// inferred.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    pub external_downcasts: InterfaceMetaDowncasts,

    /// Indicator whether the generated code is intended to be used only inside the `juniper`
    /// library.
    pub is_internal: bool,
}

impl Parse for InterfaceMeta {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut output = Self::default();

        while !input.is_empty() {
            let ident = input.parse_any_ident()?;
            match ident.to_string().as_str() {
                "name" => {
                    input.parse::<token::Eq>()?;
                    let name = input.parse::<syn::LitStr>()?;
                    output
                        .name
                        .replace(SpanContainer::new(
                            ident.span(),
                            Some(name.span()),
                            name.value(),
                        ))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "desc" | "description" => {
                    input.parse::<token::Eq>()?;
                    let desc = input.parse::<syn::LitStr>()?;
                    output
                        .description
                        .replace(SpanContainer::new(
                            ident.span(),
                            Some(desc.span()),
                            desc.value(),
                        ))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "ctx" | "context" | "Context" => {
                    input.parse::<token::Eq>()?;
                    let ctx = input.parse::<syn::Type>()?;
                    output
                        .context
                        .replace(SpanContainer::new(ident.span(), Some(ctx.span()), ctx))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "scalar" | "Scalar" | "ScalarValue" => {
                    input.parse::<token::Eq>()?;
                    let scl = input.parse::<syn::Type>()?;
                    output
                        .scalar
                        .replace(SpanContainer::new(ident.span(), Some(scl.span()), scl))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "for" | "implementers" => {
                    input.parse::<token::Eq>()?;
                    for impler in input.parse_maybe_wrapped_and_punctuated::<
                        syn::Type, token::Bracket, token::Comma,
                    >()? {
                        let impler_span = impler.span();
                        output
                            .implementers
                            .replace(SpanContainer::new(ident.span(), Some(impler_span), impler))
                            .none_or_else(|_| err::dup_arg(impler_span))?;
                    }
                }
                "dyn" => {
                    input.parse::<token::Eq>()?;
                    let alias = input.parse::<syn::Ident>()?;
                    output
                        .as_dyn
                        .replace(SpanContainer::new(ident.span(), Some(alias.span()), alias))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "enum" => {
                    input.parse::<token::Eq>()?;
                    let alias = input.parse::<syn::Ident>()?;
                    output
                        .as_enum
                        .replace(SpanContainer::new(ident.span(), Some(alias.span()), alias))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "async" => {
                    let span = ident.span();
                    output
                        .asyncness
                        .replace(SpanContainer::new(span, Some(span), ident))
                        .none_or_else(|_| err::dup_arg(span))?;
                }
                "on" => {
                    let ty = input.parse::<syn::Type>()?;
                    input.parse::<token::Eq>()?;
                    let dwncst = input.parse::<syn::ExprPath>()?;
                    let dwncst_spanned = SpanContainer::new(ident.span(), Some(ty.span()), dwncst);
                    let dwncst_span = dwncst_spanned.span_joined();
                    output
                        .external_downcasts
                        .insert(ty, dwncst_spanned)
                        .none_or_else(|_| err::dup_arg(dwncst_span))?
                }
                "internal" => {
                    output.is_internal = true;
                }
                name => {
                    return Err(err::unknown_arg(&ident, name));
                }
            }
            input.try_parse::<token::Comma>()?;
        }

        Ok(output)
    }
}

impl InterfaceMeta {
    /// Tries to merge two [`InterfaceMeta`]s into a single one, reporting about duplicates, if any.
    fn try_merge(self, mut another: Self) -> syn::Result<Self> {
        Ok(Self {
            name: try_merge_opt!(name: self, another),
            description: try_merge_opt!(description: self, another),
            context: try_merge_opt!(context: self, another),
            scalar: try_merge_opt!(scalar: self, another),
            implementers: try_merge_hashset!(implementers: self, another => span_joined),
            as_dyn: try_merge_opt!(as_dyn: self, another),
            as_enum: try_merge_opt!(as_enum: self, another),
            asyncness: try_merge_opt!(asyncness: self, another),
            external_downcasts: try_merge_hashmap!(
                external_downcasts: self, another => span_joined
            ),
            is_internal: self.is_internal || another.is_internal,
        })
    }

    /// Parses [`InterfaceMeta`] from the given multiple `name`d [`syn::Attribute`]s placed on a
    /// trait definition.
    pub fn from_attrs(name: &str, attrs: &[syn::Attribute]) -> syn::Result<Self> {
        let mut meta = filter_attrs(name, attrs)
            .map(|attr| attr.parse_args())
            .try_fold(Self::default(), |prev, curr| prev.try_merge(curr?))?;

        if let Some(as_dyn) = &meta.as_dyn {
            if meta.as_enum.is_some() {
                return Err(syn::Error::new(
                    as_dyn.span(),
                    "`dyn` attribute argument is not composable with `enum` attribute argument",
                ));
            }
        }

        if meta.description.is_none() {
            meta.description = get_doc_comment(attrs);
        }

        Ok(meta)
    }
}

/// Available metadata (arguments) behind `#[graphql_interface]` attribute placed on a trait
/// implementation block, when generating code for [GraphQL interface][1] type.
///
/// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
#[derive(Debug, Default)]
struct ImplementerMeta {
    pub scalar: Option<SpanContainer<syn::Type>>,
    pub asyncness: Option<SpanContainer<syn::Ident>>,
    pub as_dyn: Option<SpanContainer<syn::Ident>>,
}

impl Parse for ImplementerMeta {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut output = Self::default();

        while !input.is_empty() {
            let ident = input.parse_any_ident()?;
            match ident.to_string().as_str() {
                "scalar" | "Scalar" | "ScalarValue" => {
                    input.parse::<token::Eq>()?;
                    let scl = input.parse::<syn::Type>()?;
                    output
                        .scalar
                        .replace(SpanContainer::new(ident.span(), Some(scl.span()), scl))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "dyn" => {
                    let span = ident.span();
                    output
                        .as_dyn
                        .replace(SpanContainer::new(span, Some(span), ident))
                        .none_or_else(|_| err::dup_arg(span))?;
                }
                "async" => {
                    let span = ident.span();
                    output
                        .asyncness
                        .replace(SpanContainer::new(span, Some(span), ident))
                        .none_or_else(|_| err::dup_arg(span))?;
                }
                name => {
                    return Err(err::unknown_arg(&ident, name));
                }
            }
            input.try_parse::<token::Comma>()?;
        }

        Ok(output)
    }
}

impl ImplementerMeta {
    /// Tries to merge two [`ImplementerMeta`]s into a single one, reporting about duplicates, if
    /// any.
    fn try_merge(self, mut another: Self) -> syn::Result<Self> {
        Ok(Self {
            scalar: try_merge_opt!(scalar: self, another),
            as_dyn: try_merge_opt!(as_dyn: self, another),
            asyncness: try_merge_opt!(asyncness: self, another),
        })
    }

    /// Parses [`ImplementerMeta`] from the given multiple `name`d [`syn::Attribute`]s placed on a
    /// trait implementation block.
    pub fn from_attrs(name: &str, attrs: &[syn::Attribute]) -> syn::Result<Self> {
        filter_attrs(name, attrs)
            .map(|attr| attr.parse_args())
            .try_fold(Self::default(), |prev, curr| prev.try_merge(curr?))
    }
}

#[derive(Debug, Default)]
struct TraitMethodMeta {
    pub name: Option<SpanContainer<syn::LitStr>>,
    pub description: Option<SpanContainer<syn::LitStr>>,
    pub deprecated: Option<SpanContainer<Option<syn::LitStr>>>,
    pub ignore: Option<SpanContainer<syn::Ident>>,
    pub downcast: Option<SpanContainer<syn::Ident>>,
}

impl Parse for TraitMethodMeta {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut output = Self::default();

        while !input.is_empty() {
            let ident = input.parse::<syn::Ident>()?;
            match ident.to_string().as_str() {
                "name" => {
                    input.parse::<token::Eq>()?;
                    let name = input.parse::<syn::LitStr>()?;
                    output
                        .name
                        .replace(SpanContainer::new(ident.span(), Some(name.span()), name))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "desc" | "description" => {
                    input.parse::<token::Eq>()?;
                    let desc = input.parse::<syn::LitStr>()?;
                    output
                        .description
                        .replace(SpanContainer::new(ident.span(), Some(desc.span()), desc))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "deprecated" => {
                    let mut reason = None;
                    if input.is_next::<token::Eq>() {
                        input.parse::<token::Eq>()?;
                        reason = Some(input.parse::<syn::LitStr>()?);
                    }
                    output
                        .deprecated
                        .replace(SpanContainer::new(
                            ident.span(),
                            reason.as_ref().map(|r| r.span()),
                            reason,
                        ))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "ignore" | "skip" => output
                    .ignore
                    .replace(SpanContainer::new(ident.span(), None, ident.clone()))
                    .none_or_else(|_| err::dup_arg(&ident))?,
                "downcast" => output
                    .downcast
                    .replace(SpanContainer::new(ident.span(), None, ident.clone()))
                    .none_or_else(|_| err::dup_arg(&ident))?,
                name => {
                    return Err(err::unknown_arg(&ident, name));
                }
            }
            input.try_parse::<token::Comma>()?;
        }

        Ok(output)
    }
}

impl TraitMethodMeta {
    /// Tries to merge two [`FieldMeta`]s into a single one, reporting about duplicates, if any.
    fn try_merge(self, mut another: Self) -> syn::Result<Self> {
        Ok(Self {
            name: try_merge_opt!(name: self, another),
            description: try_merge_opt!(description: self, another),
            deprecated: try_merge_opt!(deprecated: self, another),
            ignore: try_merge_opt!(ignore: self, another),
            downcast: try_merge_opt!(downcast: self, another),
        })
    }

    /// Parses [`FieldMeta`] from the given multiple `name`d [`syn::Attribute`]s placed on a
    /// function/method definition.
    pub fn from_attrs(name: &str, attrs: &[syn::Attribute]) -> syn::Result<Self> {
        let mut meta = filter_attrs(name, attrs)
            .map(|attr| attr.parse_args())
            .try_fold(Self::default(), |prev, curr| prev.try_merge(curr?))?;

        if let Some(ignore) = &meta.ignore {
            if meta.name.is_some()
                || meta.description.is_some()
                || meta.deprecated.is_some()
                || meta.downcast.is_some()
            {
                return Err(syn::Error::new(
                    ignore.span(),
                    "`ignore` attribute argument is not composable with any other arguments",
                ));
            }
        }

        if let Some(downcast) = &meta.downcast {
            if meta.name.is_some()
                || meta.description.is_some()
                || meta.deprecated.is_some()
                || meta.ignore.is_some()
            {
                return Err(syn::Error::new(
                    downcast.span(),
                    "`downcast` attribute argument is not composable with any other arguments",
                ));
            }
        }

        if meta.description.is_none() {
            meta.description = get_doc_comment(attrs).map(|sc| {
                let span = sc.span_ident();
                sc.map(|desc| syn::LitStr::new(&desc, span))
            });
        }

        if meta.deprecated.is_none() {
            meta.deprecated = get_deprecated(attrs).map(|sc| {
                let span = sc.span_ident();
                sc.map(|depr| depr.reason.map(|rsn| syn::LitStr::new(&rsn, span)))
            });
        }

        Ok(meta)
    }
}

#[derive(Debug, Default)]
struct ArgumentMeta {
    pub name: Option<SpanContainer<syn::LitStr>>,
    pub description: Option<SpanContainer<syn::LitStr>>,
    pub default: Option<SpanContainer<Option<syn::Expr>>>,
    pub context: Option<SpanContainer<syn::Ident>>,
    pub executor: Option<SpanContainer<syn::Ident>>,
}

impl Parse for ArgumentMeta {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut output = Self::default();

        while !input.is_empty() {
            let ident = input.parse::<syn::Ident>()?;
            match ident.to_string().as_str() {
                "name" => {
                    input.parse::<token::Eq>()?;
                    let name = input.parse::<syn::LitStr>()?;
                    output
                        .name
                        .replace(SpanContainer::new(ident.span(), Some(name.span()), name))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "desc" | "description" => {
                    input.parse::<token::Eq>()?;
                    let desc = input.parse::<syn::LitStr>()?;
                    output
                        .description
                        .replace(SpanContainer::new(ident.span(), Some(desc.span()), desc))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "default" => {
                    let mut expr = None;
                    if input.is_next::<token::Eq>() {
                        input.parse::<token::Eq>()?;
                        expr = Some(input.parse::<syn::Expr>()?);
                    } else if input.is_next::<token::Paren>() {
                        let inner;
                        let _ = syn::parenthesized!(inner in input);
                        expr = Some(inner.parse::<syn::Expr>()?);
                    }
                    output
                        .default
                        .replace(SpanContainer::new(
                            ident.span(),
                            expr.as_ref().map(|e| e.span()),
                            expr,
                        ))
                        .none_or_else(|_| err::dup_arg(&ident))?
                }
                "ctx" | "context" | "Context" => {
                    let span = ident.span();
                    output
                        .context
                        .replace(SpanContainer::new(span, Some(span), ident))
                        .none_or_else(|_| err::dup_arg(span))?
                }
                "exec" | "executor" => {
                    let span = ident.span();
                    output
                        .executor
                        .replace(SpanContainer::new(span, Some(span), ident))
                        .none_or_else(|_| err::dup_arg(span))?
                }
                name => {
                    return Err(err::unknown_arg(&ident, name));
                }
            }
            input.try_parse::<token::Comma>()?;
        }

        Ok(output)
    }
}

impl ArgumentMeta {
    /// Tries to merge two [`ArgumentMeta`]s into a single one, reporting about duplicates, if any.
    fn try_merge(self, mut another: Self) -> syn::Result<Self> {
        Ok(Self {
            name: try_merge_opt!(name: self, another),
            description: try_merge_opt!(description: self, another),
            default: try_merge_opt!(default: self, another),
            context: try_merge_opt!(context: self, another),
            executor: try_merge_opt!(executor: self, another),
        })
    }

    /// Parses [`ArgumentMeta`] from the given multiple `name`d [`syn::Attribute`]s placed on a
    /// function argument.
    pub fn from_attrs(name: &str, attrs: &[syn::Attribute]) -> syn::Result<Self> {
        let meta = filter_attrs(name, attrs)
            .map(|attr| attr.parse_args())
            .try_fold(Self::default(), |prev, curr| prev.try_merge(curr?))?;

        if let Some(context) = &meta.context {
            if meta.name.is_some()
                || meta.description.is_some()
                || meta.default.is_some()
                || meta.executor.is_some()
            {
                return Err(syn::Error::new(
                    context.span(),
                    "`context` attribute argument is not composable with any other arguments",
                ));
            }
        }

        if let Some(executor) = &meta.executor {
            if meta.name.is_some()
                || meta.description.is_some()
                || meta.default.is_some()
                || meta.context.is_some()
            {
                return Err(syn::Error::new(
                    executor.span(),
                    "`executor` attribute argument is not composable with any other arguments",
                ));
            }
        }

        Ok(meta)
    }
}

struct InterfaceFieldArgumentDefinition {
    pub name: String,
    pub ty: syn::Type,
    pub description: Option<String>,
    pub default: Option<Option<syn::Expr>>,
}

enum MethodArgument {
    Regular(InterfaceFieldArgumentDefinition),
    Context(syn::Type),
    Executor,
}

impl MethodArgument {
    #[must_use]
    pub fn as_regular(&self) -> Option<&InterfaceFieldArgumentDefinition> {
        if let Self::Regular(arg) = self {
            Some(arg)
        } else {
            None
        }
    }

    #[must_use]
    fn context_ty(&self) -> Option<&syn::Type> {
        if let Self::Context(ty) = self {
            Some(ty)
        } else {
            None
        }
    }

    fn meta_method_tokens(&self) -> Option<TokenStream> {
        let arg = self.as_regular()?;

        let (name, ty) = (&arg.name, &arg.ty);

        let description = arg
            .description
            .as_ref()
            .map(|desc| quote! { .description(#desc) });

        let method = if let Some(val) = &arg.default {
            let val = val
                .as_ref()
                .map(|v| quote! { (#v).into() })
                .unwrap_or_else(|| quote! { <#ty as Default>::default() });
            quote! { .arg_with_default::<#ty>(#name, &#val, info) }
        } else {
            quote! { .arg::<#ty>(#name, info) }
        };

        Some(quote! { .argument(registry#method#description) })
    }

    fn resolve_field_method_tokens(&self) -> TokenStream {
        match self {
            Self::Regular(arg) => {
                let (name, ty) = (&arg.name, &arg.ty);
                let err_text = format!(
                    "Internal error: missing argument `{}` - validation must have failed",
                    &name,
                );

                quote! {
                    args.get::<#ty>(#name).expect(#err_text)
                }
            }

            Self::Context(_) => quote! {
                ::juniper::FromContext::from(executor.context())
            },

            Self::Executor => quote! { &executor },
        }
    }
}

struct InterfaceFieldDefinition {
    pub name: String,
    pub ty: syn::Type,
    pub trait_ty: syn::Type,
    pub description: Option<String>,
    pub deprecated: Option<Option<String>>,
    pub method: syn::Ident,
    pub arguments: Vec<MethodArgument>,
    pub is_async: bool,
}

impl InterfaceFieldDefinition {
    fn meta_method_tokens(&self) -> TokenStream {
        let (name, ty) = (&self.name, &self.ty);

        let description = self
            .description
            .as_ref()
            .map(|desc| quote! { .description(#desc) });

        let deprecated = self.deprecated.as_ref().map(|reason| {
            let reason = reason
                .as_ref()
                .map(|rsn| quote! { Some(#rsn) })
                .unwrap_or_else(|| quote! { None });
            quote! { .deprecated(#reason) }
        });

        let arguments = self
            .arguments
            .iter()
            .filter_map(MethodArgument::meta_method_tokens);

        quote! {
            registry.field_convert::<#ty, _, Self::Context>(#name, info)
                #( #arguments )*
                #description
                #deprecated
        }
    }

    fn resolve_field_method_tokens(&self) -> Option<TokenStream> {
        if self.is_async {
            return None;
        }

        let (name, ty, method) = (&self.name, &self.ty, &self.method);
        let interface_ty = &self.interface_ty;

        let arguments = self
            .arguments
            .iter()
            .map(MethodArgument::resolve_field_method_tokens);

        let resolving_code = gen::sync_resolving_code();

        Some(quote! {
            #name => {
                let res: #ty = <Self as #interface_ty>::#method(self #( , #arguments )*);
                #resolving_code
            }
        })
    }

    fn resolve_field_async_method_tokens(&self) -> TokenStream {
        let (name, ty, method) = (&self.name, &self.ty, &self.method);
        let interface_ty = &self.interface_ty;

        let arguments = self
            .arguments
            .iter()
            .map(MethodArgument::resolve_field_method_tokens);

        let mut fut = quote! { <Self as #interface_ty>::#method(self #( , #arguments )*) };
        if !self.is_async {
            fut = quote! { ::juniper::futures::future::ready(#fut) };
        }

        let resolving_code = gen::async_resolving_code(Some(ty));

        quote! {
            #name => {
                let fut = #fut;
                #resolving_code
            }
        }
    }
}

#[derive(Clone)]
enum ImplementerDowncastDefinition {
    Method {
        name: syn::Ident,
        with_context: bool,
    },
    External {
        path: syn::ExprPath,
    },
}

/// Definition of [GraphQL interface][1] implementer for code generation.
///
/// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
#[derive(Clone)]
struct ImplementerDefinition {
    /// Rust type that this [GraphQL interface][1] implementer resolves into.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    pub ty: syn::Type,

    pub downcast: Option<ImplementerDowncastDefinition>,

    /// Rust type of `juniper::Context` that this [GraphQL interface][1] implementer requires for
    /// downcasting.
    ///
    /// It's available only when code generation happens for Rust traits and a trait method contains
    /// context argument.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    pub context_ty: Option<syn::Type>,

    pub scalar: ScalarValueType,

    pub interface_ty: syn::Type,
}

impl ImplementerDefinition {
    fn downcast_call_tokens(&self) -> Option<TokenStream> {
        let interface_ty = &self.interface_ty;

        let mut ctx_arg = Some(quote! { , ::juniper::FromContext::from(context) });

        let fn_path = match self.downcast.as_ref()? {
            ImplementerDowncastDefinition::Method { name, with_context } => {
                if !with_context {
                    ctx_arg = None;
                }
                quote! { #interface_ty::#name }
            }
            ImplementerDowncastDefinition::External { path } => {
                quote! { #path }
            }
        };

        Some(quote! {
            #fn_path(self #ctx_arg)
        })
    }

    fn concrete_type_name_method_tokens(&self) -> Option<TokenStream> {
        if self.downcast.is_none() {
            return None;
        }

        let ty = &self.ty;
        let scalar_ty = self.scalar.ty_tokens_default();

        let downcast = self.downcast_call_tokens();

        // Doing this may be quite an expensive, because resolving may contain some heavy
        // computation, so we're preforming it twice. Unfortunately, we have no other options here,
        // until the `juniper::GraphQLType` itself will allow to do it in some cleverer way.
        Some(quote! {
            if (#downcast as ::std::option::Option<&#ty>).is_some() {
                return <#ty as ::juniper::GraphQLType<#scalar_ty>>::name(info).unwrap().to_string();
            }
        })
    }

    fn resolve_into_type_method_tokens(&self) -> Option<TokenStream> {
        if self.downcast.is_none() {
            return None;
        }

        let ty = &self.ty;
        let scalar_ty = self.scalar.ty_tokens_default();

        let downcast = self.downcast_call_tokens();

        let resolving_code = gen::sync_resolving_code();

        Some(quote! {
            if type_name == <#ty as ::juniper::GraphQLType<#scalar_ty>>::name(info).unwrap() {
                let res = #downcast;
                return #resolving_code;
            }
        })
    }

    fn resolve_into_type_async_method_tokens(&self) -> Option<TokenStream> {
        if self.downcast.is_none() {
            return None;
        }

        let ty = &self.ty;
        let scalar_ty = self.scalar.ty_tokens_default();

        let downcast = self.downcast_call_tokens();

        let resolving_code = gen::async_resolving_code(None);

        Some(quote! {
            if type_name == <#ty as ::juniper::GraphQLType<#scalar_ty>>::name(info).unwrap() {
                let fut = ::juniper::futures::future::ready(#downcast);
                return #resolving_code;
            }
        })
    }
}

struct Definition {
    /// Rust type that this [GraphQL interface][1] is represented with.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    ty: Type,

    trait_ident: syn::Ident,

    trait_generics: syn::Generics,

    /// Name of this [GraphQL interface][1] in GraphQL schema.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    name: String,

    /// Description of this [GraphQL interface][1] to put into GraphQL schema.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    description: Option<String>,

    /// Rust type of `juniper::Context` to generate `juniper::GraphQLType` implementation with
    /// for this [GraphQL interface][1].
    ///
    /// If [`None`] then generated code will use unit type `()` as `juniper::Context`.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    context: Option<syn::Type>,

    /// Rust type of `juniper::ScalarValue` to generate `juniper::GraphQLType` implementation with
    /// for this [GraphQL interface][1].
    ///
    /// If [`None`] then generated code will be generic over any `juniper::ScalarValue` type, which,
    /// in turn, requires all [interface][1] implementers to be generic over any
    /// `juniper::ScalarValue` type too. That's why this type should be specified only if one of the
    /// implementers implements `juniper::GraphQLType` in a non-generic way over
    /// `juniper::ScalarValue` type.
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    scalar: ScalarValueType,

    fields: Vec<InterfaceFieldDefinition>,

    /// Implementers definitions of this [GraphQL interface][1].
    ///
    /// [1]: https://spec.graphql.org/June2018/#sec-Interfaces
    implementers: Vec<ImplementerDefinition>,
}

impl Definition {
    fn trait_ty(&self) -> syn::Type {
        let ty = &self.trait_ident;
        let (_, generics, _) = self.trait_generics.split_for_impl();

        parse_quote! { #ty#generics }
    }

    fn no_field_panic_tokens(&self) -> TokenStream {
        let scalar_ty = self.scalar.ty_tokens_default();

        quote! {
            panic!(
                "Field `{}` not found on type `{}`",
                field,
                <Self as ::juniper::GraphQLType<#scalar_ty>>::name(info).unwrap(),
            )
        }
    }

    fn impl_graphql_type_tokens(&self) -> TokenStream {
        let scalar_ty = self.scalar.ty_tokens_default();

        let generics = self.ty.impl_generics(&self.scalar);
        let (impl_generics, _, where_clause) = generics.split_for_impl();

        let ty = self.ty.ty_tokens();

        let name = &self.name;
        let description = self
            .description
            .as_ref()
            .map(|desc| quote! { .description(#desc) });

        // Sorting is required to preserve/guarantee the order of implementers registered in schema.
        let mut impler_tys: Vec<_> = self.implementers.iter().map(|impler| &impler.ty).collect();
        impler_tys.sort_unstable_by(|a, b| {
            let (a, b) = (quote!(#a).to_string(), quote!(#b).to_string());
            a.cmp(&b)
        });

        let fields_meta = self
            .fields
            .iter()
            .map(InterfaceFieldDefinition::meta_method_tokens);

        quote! {
            #[automatically_derived]
            impl#impl_generics ::juniper::GraphQLType<#scalar_ty> for #ty #where_clause
            {
                fn name(_ : &Self::TypeInfo) -> Option<&'static str> {
                    Some(#name)
                }

                fn meta<'r>(
                    info: &Self::TypeInfo,
                    registry: &mut ::juniper::Registry<'r, #scalar_ty>
                ) -> ::juniper::meta::MetaType<'r, #scalar_ty>
                where #scalar_ty: 'r,
                {
                    // Ensure all implementer types are registered.
                    #( let _ = registry.get_type::<#impler_tys>(info); )*

                    let fields = [
                        #( #fields_meta, )*
                    ];
                    registry.build_interface_type::<#ty>(info, &fields)
                        #description
                        .into_meta()
                }
            }
        }
    }

    fn impl_graphql_value_tokens(&self) -> TokenStream {
        let scalar_ty = self.scalar.ty_tokens_default();

        let generics = self.ty.impl_generics(&self.scalar);
        let (impl_generics, _, where_clause) = generics.split_for_impl();

        let ty = self.ty.ty_tokens();
        let context_ty = self.context.clone().unwrap_or_else(|| parse_quote! { () });

        let fields_resolvers = self
            .fields
            .iter()
            .filter_map(InterfaceFieldDefinition::resolve_field_method_tokens);
        let async_fields_panic = {
            let names = self
                .fields
                .iter()
                .filter_map(|field| {
                    if field.is_async {
                        Some(&field.name)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            if names.is_empty() {
                None
            } else {
                Some(quote! {
                    #( #names )|* => panic!(
                        "Tried to resolve async field `{}` on type `{}` with a sync resolver",
                        field,
                        <Self as ::juniper::GraphQLType<#scalar_ty>>::name(info).unwrap(),
                    ),
                })
            }
        };
        let no_field_panic = self.no_field_panic_tokens();

        let custom_downcast_checks = self
            .implementers
            .iter()
            .filter_map(ImplementerDefinition::concrete_type_name_method_tokens);
        let regular_downcast_check = self.ty.concrete_type_name_method_tokens();

        let custom_downcasts = self
            .implementers
            .iter()
            .filter_map(ImplementerDefinition::resolve_into_type_method_tokens);
        let regular_downcast = self.ty.resolve_into_type_method_tokens();

        quote! {
            #[automatically_derived]
            impl#impl_generics ::juniper::GraphQLValue<#scalar_ty> for #ty #where_clause
            {
                type Context = #context_ty;
                type TypeInfo = ();

                fn type_name<'__i>(&self, info: &'__i Self::TypeInfo) -> Option<&'__i str> {
                    <Self as ::juniper::GraphQLType<#scalar_ty>>::name(info)
                }

                fn resolve_field(
                    &self,
                    info: &Self::TypeInfo,
                    field: &str,
                    args: &::juniper::Arguments<#scalar_ty>,
                    executor: &::juniper::Executor<Self::Context, #scalar_ty>,
                ) -> ::juniper::ExecutionResult<#scalar_ty> {
                    match field {
                        #( #fields_resolvers )*
                        #async_fields_panic
                        _ => #no_field_panic,
                    }
                }

                fn concrete_type_name(
                    &self,
                    context: &Self::Context,
                    info: &Self::TypeInfo,
                ) -> String {
                    #( #custom_downcast_checks )*
                    #regular_downcast_check
                }

                fn resolve_into_type(
                    &self,
                    info: &Self::TypeInfo,
                    type_name: &str,
                    _: Option<&[::juniper::Selection<#scalar>]>,
                    executor: &::juniper::Executor<Self::Context, #scalar>,
                ) -> ::juniper::ExecutionResult<#scalar> {
                    #( #custom_downcasts )*
                    #regular_downcast
                }
            }
        }
    }

    fn impl_graphql_value_async_tokens(&self) -> TokenStream {
        let scalar_ty = self.scalar.ty_tokens_default();

        let generics = self.ty.impl_generics(&self.scalar);
        let (impl_generics, _, where_clause) = generics.split_for_impl();
        let mut where_clause = where_clause
            .cloned()
            .unwrap_or_else(|| parse_quote! { where });
        where_clause.predicates.push(parse_quote! { Self: Sync });
        if self.scalar.is_generic() {
            where_clause
                .predicates
                .push(parse_quote! { #scalar_ty: Send + Sync });
        }

        let ty = self.ty.ty_tokens();
        let context_ty = self.context.clone().unwrap_or_else(|| parse_quote! { () });

        let fields_resolvers = self
            .fields
            .iter()
            .map(InterfaceFieldDefinition::resolve_field_async_method_tokens);
        let no_field_panic = self.no_field_panic_tokens();

        let custom_downcasts = self
            .implementers
            .iter()
            .filter_map(ImplementerDefinition::resolve_into_type_async_method_tokens);
        let regular_downcast = self.ty.resolve_into_type_async_method_tokens();

        quote! {
            #[automatically_derived]
            impl#impl_generics ::juniper::GraphQLValue<#scalar_ty> for #ty #where_clause
            {
                fn resolve_field_async<'b>(
                    &'b self,
                    info: &'b Self::TypeInfo,
                    field: &'b str,
                    args: &'b ::juniper::Arguments<#scalar_ty>,
                    executor: &'b ::juniper::Executor<Self::Context, #scalar_ty>,
                ) -> ::juniper::BoxFuture<'b, ::juniper::ExecutionResult<#scalar_ty>> {
                    match field {
                        #( #fields_resolvers )*
                        _ => #no_field_panic,
                    }
                }

                fn resolve_into_type_async<'b>(
                    &'b self,
                    info: &'b Self::TypeInfo,
                    type_name: &str,
                    _: Option<&'b [::juniper::Selection<'b, #scalar_ty>]>,
                    executor: &'b ::juniper::Executor<'b, 'b, Self::Context, #scalar_ty>
                ) -> ::juniper::BoxFuture<'b, ::juniper::ExecutionResult<#scalar_ty>> {
                    #( #custom_downcasts )*
                    #regular_downcast
                }
            }
        }
    }

    fn impl_graphql_interface_tokens(&self) -> TokenStream {
        let scalar_ty = self.scalar.ty_tokens_default();

        let generics = self.ty.impl_generics(&self.scalar);
        let (impl_generics, _, where_clause) = generics.split_for_impl();

        let ty = self.ty.ty_tokens();

        let impler_tys: Vec<_> = self.implementers.iter().map(|impler| &impler.ty).collect();

        let all_implers_unique = if impler_types.len() > 1 {
            Some(quote! { ::juniper::sa::assert_type_ne_all!(#( #impler_tys ),*); })
        } else {
            None
        };

        quote! {
            #[automatically_derived]
            impl#impl_generics ::juniper::marker::GraphQLInterface<#scalar_ty> for #ty #where_clause
            {
                fn mark() {
                    #all_implers_unique

                    #( <#impler_tys as ::juniper::marker::GraphQLObjectType<#scalar_ty>>::mark(); )*
                }
            }
        }
    }

    fn impl_output_type_tokens(&self) -> TokenStream {
        let scalar_ty = self.scalar.ty_tokens_default();

        let generics = self.ty.impl_generics(&self.scalar);
        let (impl_generics, _, where_clause) = generics.split_for_impl();

        let ty = self.ty.ty_tokens();

        let fields_marks = self.fields.iter().map(|field| {
            let arguments_marks = field.arguments.iter().filter_map(|arg| {
                let arg_ty = &arg.as_regular()?.ty;
                Some(quote! { <#arg_ty as ::juniper::marker::IsInputType<#scalar_ty>>::mark(); })
            });

            let field_ty = &field.ty;
            let resolved_ty = quote! {
                <#field_ty as ::juniper::IntoResolvable<
                    '_, #scalar_ty, _, <Self as ::juniper::GraphQLValue<#scalar_ty>>::Context,
                >>::Type
            };

            quote! {
                #( #arguments_marks )*
                <#resolved_ty as ::juniper::marker::IsOutputType<#scalar_ty>>::mark();
            }
        });

        let impler_tys = self.implementers.iter().map(|impler| &impler.ty);

        quote! {
            #[automatically_derived]
            impl#impl_generics ::juniper::marker::IsOutputType<#scalar_ty> for #ty #where_clause
            {
                fn mark() {
                    #( #fields_marks )*
                    #( <#impler_tys as ::juniper::marker::IsOutputType<#scalar_ty>>::mark(); )*
                }
            }
        }
    }
}

impl ToTokens for Definition {
    fn to_tokens(&self, into: &mut TokenStream) {
        into.append_all(&[
            self.ty.to_token_stream(),
            self.impl_graphql_interface_tokens(),
            self.impl_output_type_tokens(),
            self.impl_graphql_type_tokens(),
            self.impl_graphql_value_tokens(),
            self.impl_graphql_value_async_tokens(),
        ]);
    }
}

struct EnumType {
    ident: syn::Ident,
    visibility: syn::Visibility,
    variants: Vec<syn::Type>,
    trait_ident: syn::Ident,
    trait_generics: syn::Generics,
    trait_types: Vec<(syn::Ident, syn::Generics)>,
    trait_consts: Vec<(syn::Ident, syn::Type)>,
    trait_methods: Vec<syn::Signature>,
}

impl EnumType {
    fn new(
        r#trait: &syn::ItemTrait,
        meta: &InterfaceMeta,
        implers: &Vec<ImplementerDefinition>,
    ) -> Self {
        Self {
            ident: meta
                .as_enum
                .as_ref()
                .map(SpanContainer::as_ref)
                .cloned()
                .unwrap_or_else(|| format_ident!("{}Value", r#trait.ident)),
            visibility: r#trait.vis.clone(),
            variants: implers.iter().map(|impler| impler.ty.clone()).collect(),
            trait_ident: r#trait.ident.clone(),
            trait_generics: r#trait.generics.clone(),
            trait_types: r#trait
                .items
                .iter()
                .filter_map(|i| {
                    if let syn::TraitItem::Type(ty) = i {
                        Some((ty.ident.clone(), ty.generics.clone()))
                    } else {
                        None
                    }
                })
                .collect(),
            trait_consts: r#trait
                .items
                .iter()
                .filter_map(|i| {
                    if let syn::TraitItem::Const(cnst) = i {
                        Some((cnst.ident.clone(), cnst.ty.clone()))
                    } else {
                        None
                    }
                })
                .collect(),
            trait_methods: r#trait
                .items
                .iter()
                .filter_map(|i| {
                    if let syn::TraitItem::Method(m) = i {
                        Some(m.sig.clone())
                    } else {
                        None
                    }
                })
                .collect(),
        }
    }

    fn variant_ident(num: usize) -> syn::Ident {
        format_ident!("Impl{}", num)
    }

    fn impl_generics(&self, scalar: &ScalarValueType) -> syn::Generics {
        let mut generics = syn::Generics::default();
        if scalar.is_generic() {
            let scalar_ty = scalar.ty_tokens_default();
            generics.params.push(parse_quote! { #scalar_ty });
            generics
                .make_where_clause()
                .predicates
                .push(parse_quote! { #scalar_ty: ::juniper::ScalarValue });
        }
        generics
    }

    fn ty_tokens(&self) -> TokenStream {
        let ty = &self.ident;

        quote! { #ty }
    }

    fn type_definition_tokens(&self) -> TokenStream {
        let enum_ty = &self.ident;
        let vis = &self.visibility;

        let doc = format!(
            "Type implementing [GraphQL interface][1] represented by `{}` trait.\
             \n\n\
             [1]: https://spec.graphql.org/June2018/#sec-Interfaces",
            self.trait_ident,
        );

        let variants = self.variants.iter().enumerate().map(|(n, ty)| {
            let variant = Self::variant_ident(n);

            quote! { #variant(#ty), }
        });

        quote! {
            #[automatically_derived]
            #[doc = #doc]
            #vis enum #enum_ty {
                #( #variants )*
            }
        }
    }

    fn impl_from_tokens(&self) -> impl Iterator<Item = TokenStream> + '_ {
        let enum_ty = &self.ident;

        self.variants.iter().enumerate().map(move |(n, ty)| {
            let variant = Self::variant_ident(n);

            quote! {
                #[automatically_derived]
                impl From<#ty> for #enum_ty {
                    fn from(v: #ty) -> Self {
                        Self::#variant(v)
                    }
                }
            }
        })
    }

    fn impl_trait_tokens(&self) -> TokenStream {
        let enum_ty = &self.ident;

        let trait_ident = &self.trait_ident;
        let (trait_params, trait_generics, where_clause) = self.trait_generics.split_for_impl();

        let var_ty = self.variants.first().unwrap();

        let assoc_types = self.trait_types.iter().map(|(ty, ty_gen)| {
            quote! {
                type #ty#ty_gen = <#var_ty as #trait_ident#trait_generics>::#ty#ty_gen;
            }
        });

        let assoc_consts = self.trait_consts.iter().map(|(ident, ty)| {
            quote! {
                const #ident: #ty = <#var_ty as #trait_ident#trait_generics>::#ident;
            }
        });

        let methods = self.trait_methods.iter().map(|sig| {
            let method = &sig.ident;

            let args = sig.inputs.iter().filter_map(|arg| match arg {
                syn::FnArg::Receiver(_) => None,
                syn::FnArg::Typed(a) => Some(&a.pat),
            });

            let and_await = if sig.asyncness.is_some() {
                Some(quote! { .await })
            } else {
                None
            };

            let match_arms = self.variants.iter().enumerate().map(|(n, ty)| {
                let variant = Self::variant_ident(n);
                let args = args.clone();

                quote! {
                    Self::#variant(v) =>
                        <#ty as #trait_ident#trait_generics>::#method(v #( , #args )* )#and_await,
                }
            });

            quote! {
                #sig {
                    match self {
                        #( #match_arms )*
                    }
                }
            }
        });

        let mut impl_tokens = quote! {
            #[automatically_derived]
            impl#trait_params #trait_ident#trait_generics for #enum_ty #where_clause {
                #( #assoc_types )*

                #( #assoc_consts )*

                #( #methods )*
            }
        };

        if self
            .trait_methods
            .iter()
            .find(|sig| sig.asyncness.is_some())
            .is_some()
        {
            let mut ast: syn::ItemImpl = parse_quote! { #impl_tokens };
            inject_async_trait(
                &mut ast.attrs,
                ast.items.iter_mut().filter_map(|i| {
                    if let syn::ImplItem::Method(m) = i {
                        Some(&mut m.sig)
                    } else {
                        None
                    }
                }),
                &ast.generics,
            );
            impl_tokens = quote! { #ast };
        }

        impl_tokens
    }

    fn concrete_type_name_method_tokens(&self) -> TokenStream {
        let match_arms = self.variants.iter().enumerate().map(|(n, ty)| {
            let variant = Self::variant_ident(n);

            quote! {
                Self::#variant(v) =>
                    <#ty as ::juniper::GraphQLValue<_>>::concrete_type_name(v, context, info),
            }
        });

        quote! {
            match self {
                #( #match_arms )*
            }
        }
    }

    fn resolve_into_type_method_tokens(&self) -> TokenStream {
        let resolving_code = gen::sync_resolving_code();

        let match_arms = self.variants.iter().enumerate().map(|(n, _)| {
            let variant = Self::variant_ident(n);

            quote! {
                Self::#variant(res) => #resolving_code,
            }
        });

        quote! {
            match self {
                #( #match_arms )*
            }
        }
    }

    fn to_resolve_into_type_async_method_tokens(&self) -> TokenStream {
        let resolving_code = gen::async_resolving_code(None);

        let match_arms = self.variants.iter().enumerate().map(|(n, _)| {
            let variant = Self::variant_ident(n);

            quote! {
                Self::#variant(v) => {
                    let fut = ::juniper::futures::future::ready(v);
                    #resolving_code
                }
            }
        });

        quote! {
            match self {
                #( #match_arms )*
            }
        }
    }
}

impl ToTokens for EnumType {
    fn to_tokens(&self, into: &mut TokenStream) {
        into.append_all(&[self.type_definition_tokens()]);
        into.append_all(self.impl_from_tokens());
        into.append_all(&[self.impl_trait_tokens()]);
    }
}

struct TraitObjectType {
    pub ident: syn::Ident,
    pub visibility: syn::Visibility,
    pub trait_ident: syn::Ident,
    pub trait_generics: syn::Generics,
    pub context: Option<syn::Type>,
}

impl TraitObjectType {
    fn new(r#trait: &syn::ItemTrait, meta: &InterfaceMeta, context: Option<syn::Type>) -> Self {
        Self {
            ident: meta.as_dyn.as_ref().unwrap().as_ref().clone(),
            visibility: r#trait.vis.clone(),
            trait_ident: r#trait.ident.clone(),
            trait_generics: r#trait.generics.clone(),
            context,
        }
    }

    fn impl_generics(&self, scalar: &ScalarValueType) -> syn::Generics {
        let mut generics = self.trait_generics.clone();
        generics.params.push(parse_quote! { '__obj });
        if scalar.is_generic() {
            let scalar_ty = scalar.ty_tokens_default();
            generics
                .make_where_clause()
                .predicates
                .push(parse_quote! { #scalar_ty: ::juniper::ScalarValue });
        }
        generics
    }

    fn ty_tokens(&self) -> TokenStream {
        let ty = &self.trait_ident;

        let mut generics = self.trait_generics.clone();
        generics.remove_defaults();
        generics.move_bounds_to_where_clause();
        let ty_params = &generics.params;

        let context_ty = self.context.clone().unwrap_or_else(|| parse_quote! { () });

        quote! {
            dyn #ty<#ty_params, Context = #context_ty, TypeInfo = ()> + '__obj + Send + Sync
        }
    }

    fn concrete_type_name_method_tokens(&self) -> TokenStream {
        quote! {
            self.as_dyn_graphql_value().concrete_type_name(context, info)
        }
    }

    fn resolve_into_type_method_tokens(&self) -> TokenStream {
        let resolving_code = gen::sync_resolving_code();

        quote! {
            let res = self.as_dyn_graphql_value();
            #resolving_code
        }
    }

    fn to_resolve_into_type_async_method_tokens(&self) -> TokenStream {
        let resolving_code = gen::async_resolving_code(None);

        quote! {
            let fut = ::juniper::futures::future::ready(self.as_dyn_graphql_value_async());
            #resolving_code
        }
    }
}

impl ToTokens for TraitObjectType {
    fn to_tokens(&self, into: &mut TokenStream) {
        let dyn_ty = &self.ident;
        let vis = &self.visibility;

        let doc = format!(
            "Helper alias for the `{}` [trait object][2] implementing [GraphQL interface][1].\
             \n\n\
             [1]: https://spec.graphql.org/June2018/#sec-Interfaces\n\
             [2]: https://doc.rust-lang.org/reference/types/trait-object.html",
            self.trait_ident,
        );

        let trait_ident = &self.trait_ident;

        let (mut ty_params_left, mut ty_params_right) = (None, None);
        if !self.trait_generics.params.is_empty() {
            // We should preserve defaults for left side.
            let mut generics = self.trait_generics.clone();
            generics.move_bounds_to_where_clause();
            let params = &generics.params;
            ty_params_left = Some(quote! { , #params });

            generics.remove_defaults();
            let params = &generics.params;
            ty_params_right = Some(quote! { #params, });
        };

        let context_ty = self.context.clone().unwrap_or_else(|| parse_quote! { () });

        let dyn_alias = quote! {
            #[automatically_derived]
            #[doc = #doc]
            #vis type #dyn_ty<'a #ty_params_left> =
                dyn #trait_ident<#ty_params_right Context = #context_ty, TypeInfo = ()> +
                    'a + Send + Sync;
        };

        into.append_all(&[dyn_alias]);
    }
}

enum Type {
    Enum(EnumType),
    TraitObject(TraitObjectType),
}

impl Type {
    fn is_trait_object(&self) -> bool {
        matches!(self, Self::TraitObject(_))
    }

    fn impl_generics(&self, scalar: &ScalarValueType) -> syn::Generics {
        match self {
            Self::Enum(e) => e.impl_generics(scalar),
            Self::TraitObject(o) => o.impl_generics(scalar),
        }
    }

    fn ty_tokens(&self) -> TokenStream {
        match self {
            Self::Enum(e) => e.ty_tokens(),
            Self::TraitObject(o) => o.ty_tokens(),
        }
    }

    fn concrete_type_name_method_tokens(&self) -> TokenStream {
        match self {
            Self::Enum(e) => e.concrete_type_name_method_tokens(),
            Self::TraitObject(o) => o.concrete_type_name_method_tokens(),
        }
    }

    fn resolve_into_type_method_tokens(&self) -> TokenStream {
        match self {
            Self::Enum(e) => e.resolve_into_type_method_tokens(),
            Self::TraitObject(o) => o.resolve_into_type_method_tokens(),
        }
    }

    fn to_resolve_into_type_async_method_tokens(&self) -> TokenStream {
        match self {
            Self::Enum(e) => e.to_resolve_into_type_async_method_tokens(),
            Self::TraitObject(o) => o.to_resolve_into_type_async_method_tokens(),
        }
    }
}

impl ToTokens for Type {
    fn to_tokens(&self, into: &mut TokenStream) {
        match self {
            Self::Enum(e) => e.to_tokens(into),
            Self::TraitObject(o) => o.to_tokens(into),
        }
    }
}

fn inject_async_trait<'m, M>(attrs: &mut Vec<syn::Attribute>, methods: M, generics: &syn::Generics)
where
    M: IntoIterator<Item = &'m mut syn::Signature>,
{
    attrs.push(parse_quote! { #[::juniper::async_trait] });

    for method in methods.into_iter() {
        if method.asyncness.is_some() {
            let where_clause = &mut method.generics.make_where_clause().predicates;
            for p in &generics.params {
                let ty_param = match p {
                    syn::GenericParam::Type(t) => {
                        let ty_param = &t.ident;
                        quote! { #ty_param }
                    }
                    syn::GenericParam::Lifetime(l) => {
                        let ty_param = &l.lifetime;
                        quote! { #ty_param }
                    }
                    syn::GenericParam::Const(_) => continue,
                };
                where_clause.push(parse_quote! { #ty_param: 'async_trait });
            }
        }
    }
}
