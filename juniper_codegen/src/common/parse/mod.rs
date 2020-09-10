pub(crate) mod attr;
pub(crate) mod downcaster;

use std::{
    mem,
    any::TypeId,
    iter::{self, FromIterator as _},
};

use syn::{
    ext::IdentExt as _,
    parse::{Parse, ParseBuffer},
    punctuated::Punctuated,
    token::{self, Token},
    parse_quote,
};

pub(crate) trait ParseBufferExt {
    /// Tries to parse `T` as the next token.
    ///
    /// Doesn't move [`ParseStream`]'s cursor if there is no `T`.
    fn try_parse<T: Default + Parse + Token>(&self) -> syn::Result<Option<T>>;

    /// Checks whether next token is `T`.
    ///
    /// Doesn't move [`ParseStream`]'s cursor.
    fn is_next<T: Default + Token>(&self) -> bool;

    /// Parses next token as [`syn::Ident`] _allowing_ Rust keywords, while default [`Parse`]
    /// implementation for [`syn::Ident`] disallows keywords.
    ///
    /// Always moves [`ParseStream`]'s cursor.
    fn parse_any_ident(&self) -> syn::Result<syn::Ident>;

    /// Checks whether next token is a wrapper `W` and if yes, then parses the wrapped tokens as `T`
    /// [`Punctuated`] with `P`. Otherwise, parses just `T`.
    ///
    /// Always moves [`ParseStream`]'s cursor.
    fn parse_maybe_wrapped_and_punctuated<T, W, P>(&self) -> syn::Result<Punctuated<T, P>>
    where
        T: Parse,
        W: Default + Token + 'static,
        P: Default + Parse + Token;
}

impl<'a> ParseBufferExt for ParseBuffer<'a> {
    fn try_parse<T: Default + Parse + Token>(&self) -> syn::Result<Option<T>> {
        Ok(if self.is_next::<T>() {
            Some(self.parse()?)
        } else {
            None
        })
    }

    fn is_next<T: Default + Token>(&self) -> bool {
        self.lookahead1().peek(|_| T::default())
    }

    fn parse_any_ident(&self) -> syn::Result<syn::Ident> {
        self.call(syn::Ident::parse_any)
    }

    fn parse_maybe_wrapped_and_punctuated<T, W, P>(&self) -> syn::Result<Punctuated<T, P>>
    where
        T: Parse,
        W: Default + Token + 'static,
        P: Default + Parse + Token,
    {
        Ok(if self.is_next::<W>() {
            let inner;
            if TypeId::of::<W>() == TypeId::of::<token::Bracket>() {
                let _ = syn::bracketed!(inner in self);
            } else if TypeId::of::<W>() == TypeId::of::<token::Brace>() {
                let _ = syn::braced!(inner in self);
            } else if TypeId::of::<W>() == TypeId::of::<token::Paren>() {
                let _ = syn::parenthesized!(inner in self);
            } else {
                panic!(
                    "ParseBufferExt::parse_maybe_wrapped_and_punctuated supports only brackets, \
                     braces and parentheses as wrappers.",
                );
            }
            Punctuated::parse_terminated(&inner)?
        } else {
            Punctuated::from_iter(iter::once(self.parse::<T>()?))
        })
    }
}

pub(crate) trait TypeExt {
    /// Retrieves the innermost non-parenthesized [`syn::Type`] from the given one (unwraps nested
    /// [`syn::TypeParen`]s asap).
    fn unparenthesized(&self) -> &Self;

    /// Retrieves the inner [`syn::Type`] from the given reference type, or just returns "as is" if
    /// the type is not a reference.
    ///
    /// Also, unparenthesizes the type, if required.
    fn unreferenced(&self) -> &Self;
}

impl TypeExt for syn::Type {
    fn unparenthesized(&self) -> &Self {
        match self {
            Self::Paren(ty) => ty.elem.unparenthesized(),
            ty => ty,
        }
    }

    fn unreferenced(&self) -> &Self {
        match self.unparenthesized() {
            Self::Reference(ref_ty) => &*ref_ty.elem,
            ty => ty,
        }
    }
}

pub(crate) trait GenericsExt {
    fn remove_defaults(&mut self);

    fn move_bounds_to_where_clause(&mut self);
}

impl GenericsExt for syn::Generics {
    fn remove_defaults(&mut self) {
        use syn::GenericParam as P;

        for p in &mut self.params {
            match p {
                P::Type(p) => {
                    p.eq_token = None;
                    p.default = None;
                }
                P::Lifetime(_) => {}
                P::Const(p) => {
                    p.eq_token = None;
                    p.default = None;
                }
            }
        }
    }

    fn move_bounds_to_where_clause(&mut self) {
        use syn::GenericParam as P;

        let _ = self.make_where_clause();
        let where_clause = self.where_clause.as_mut().unwrap();

        for p in &mut self.params {
            match p {
                P::Type(p) => {
                    if p.colon_token.is_some() {
                        p.colon_token = None;
                        let bounds = mem::take(&mut p.bounds);
                        let ty = &p.ident;
                        where_clause.predicates.push(parse_quote! { #ty: #bounds });
                    }
                }
                P::Lifetime(p) => {
                    if p.colon_token.is_some() {
                        p.colon_token = None;
                        let bounds = mem::take(&mut p.bounds);
                        let lt = &p.lifetime;
                        where_clause.predicates.push(parse_quote! { #lt: #bounds });
                    }
                }
                P::Const(_) => {}
            }
        }
    }
}