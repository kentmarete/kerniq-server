//! Procedural macros for Kerniq Server.
//!
//! Use them through `kerniq-server` with the `macros` feature; this crate's
//! expansions refer to `::kerniq_server` paths.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    BinOp, Expr, ExprCall, ExprLit, FnArg, GenericArgument, ItemFn, Lit, LitStr, Path,
    PathArguments, Token, Type, UnOp, parse_macro_input, parse_quote,
};

/// Protects a handler with a role or permission rule, in the spirit of
/// Spring's `@PreAuthorize`.
///
/// ```rust,ignore
/// use kerniq_server::prelude::*;
/// use kerniq_server::middleware::auth::Identity;
///
/// #[authorize(has_any_role("USER", "ADMIN"))]
/// async fn profile(Identity(user): Identity<User>) -> Json<Profile> {
///     Json(Profile::of(&user))
/// }
/// ```
///
/// The identity type must implement `Authorities` and be produced by the
/// `Auth` middleware. Requests without an identity are rejected with
/// `401 UNAUTHORIZED`; identities that fail the rule with `403 FORBIDDEN`,
/// before any other extractor (including the body) runs.
///
/// # Rules
///
/// | Rule | Passes when the identity |
/// |---|---|
/// | `authenticated()` | exists |
/// | `has_role("A")` | has role `A` |
/// | `has_any_role("A", "B", ..)` | has at least one of the roles |
/// | `has_all_roles("A", "B", ..)` | has every role |
/// | `has_permission("p")` | has permission `p` |
/// | `has_any_permission("p", "q", ..)` | has at least one of the permissions |
/// | `has_all_permissions("p", "q", ..)` | has every permission |
/// | `check(path::to::function)` | makes `fn(&T) -> bool` return `true` |
///
/// Rules combine with `&&`, `||`, `!` and parentheses:
/// `has_role("ADMIN") || (has_role("USER") && has_permission("reports:read"))`.
///
/// # Options
///
/// After the rule, comma-separated:
///
/// * `identity = Type` — the identity type. Optional when the handler has an
///   `Identity<T>` parameter, from which it is inferred.
/// * `message = "..."` — the `403` error message (default
///   `"Insufficient permissions"`).
/// * `crate = path` — path to `kerniq_server` if it was renamed.
///
/// # Expansion
///
/// The attribute adds a leading `Authorized<T, P>` parameter, where `P` is a
/// generated policy type, and leaves the body and return type untouched.
/// Since it emits that type next to the function, it supports free
/// functions only, not methods.
#[proc_macro_attribute]
pub fn authorize(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as AuthorizeArgs);
    let item = parse_macro_input!(item as ItemFn);
    expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

struct AuthorizeArgs {
    rule: Expr,
    identity: Option<Type>,
    message: Option<LitStr>,
    krate: Option<Path>,
}

impl Parse for AuthorizeArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Err(
                input.error("expected a rule, for example `#[authorize(has_role(\"ADMIN\"))]`")
            );
        }
        let rule: Expr = input.parse()?;
        let mut args = Self {
            rule,
            identity: None,
            message: None,
            krate: None,
        };
        while !input.is_empty() {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            if input.peek(Token![crate]) {
                input.parse::<Token![crate]>()?;
                input.parse::<Token![=]>()?;
                set_once(&mut args.krate, input.parse()?, "crate")?;
                continue;
            }
            let key: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "identity" => set_once(&mut args.identity, input.parse()?, "identity")?,
                "message" => set_once(&mut args.message, input.parse()?, "message")?,
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        "unknown option; expected `identity`, `message` or `crate`",
                    ));
                }
            }
        }
        Ok(args)
    }
}

fn set_once<T: Spanned>(slot: &mut Option<T>, value: T, name: &str) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new(
            value.span(),
            format!("`{name}` given more than once"),
        ));
    }
    *slot = Some(value);
    Ok(())
}

fn expand(args: AuthorizeArgs, mut item: ItemFn) -> syn::Result<TokenStream2> {
    let sig = &item.sig;
    if let Some(receiver) = sig.receiver() {
        return Err(syn::Error::new(
            receiver.span(),
            "#[authorize] supports free functions only, not methods",
        ));
    }
    if sig.generics.type_params().next().is_some() || sig.generics.const_params().next().is_some() {
        return Err(syn::Error::new(
            sig.generics.span(),
            "#[authorize] does not support generic handlers",
        ));
    }

    let identity_ty = match args.identity {
        Some(ty) => ty,
        None => infer_identity(&item).ok_or_else(|| {
            syn::Error::new(
                sig.ident.span(),
                "cannot infer the identity type: add an `Identity<T>` parameter \
                 or pass `identity = T`, e.g. `#[authorize(has_role(\"ADMIN\"), identity = User)]`",
            )
        })?,
    };
    let krate: Path = args.krate.unwrap_or_else(|| parse_quote!(::kerniq_server));
    let message = args
        .message
        .unwrap_or_else(|| LitStr::new("Insufficient permissions", Span::call_site()));

    let identity = format_ident!("identity");
    let condition = translate(&args.rule, &krate, &identity)?;

    let vis = &item.vis;
    let policy = format_ident!("__kerniq_authorize_{}", item.sig.ident);
    let auth = quote!(#krate::middleware::auth);

    item.sig
        .inputs
        .insert(0, parse_quote!(_: #auth::Authorized<#identity_ty, #policy>));

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        #vis struct #policy;

        impl #auth::Policy<#identity_ty> for #policy {
            #[allow(unused_variables)]
            fn check(#identity: &#identity_ty) -> ::core::result::Result<(), #krate::Error> {
                if #condition {
                    ::core::result::Result::Ok(())
                } else {
                    ::core::result::Result::Err(#krate::Error::forbidden(#message))
                }
            }
        }

        #item
    })
}

/// Finds `T` in a parameter typed `Identity<T>` (any path ending in
/// `Identity`).
fn infer_identity(item: &ItemFn) -> Option<Type> {
    item.sig.inputs.iter().find_map(|arg| {
        let FnArg::Typed(arg) = arg else { return None };
        let Type::Path(ty) = &*arg.ty else {
            return None;
        };
        let last = ty.path.segments.last()?;
        if last.ident != "Identity" {
            return None;
        }
        let PathArguments::AngleBracketed(generics) = &last.arguments else {
            return None;
        };
        match generics.args.first()? {
            GenericArgument::Type(t) if generics.args.len() == 1 => Some(t.clone()),
            _ => None,
        }
    })
}

const RULES: &str = "`authenticated()`, `has_role`, `has_any_role`, `has_all_roles`, \
    `has_permission`, `has_any_permission`, `has_all_permissions` or `check`";

fn translate(expr: &Expr, krate: &Path, identity: &syn::Ident) -> syn::Result<TokenStream2> {
    match expr {
        Expr::Paren(e) => translate(&e.expr, krate, identity),
        Expr::Group(e) => translate(&e.expr, krate, identity),
        Expr::Unary(e) if matches!(e.op, UnOp::Not(_)) => {
            let inner = translate(&e.expr, krate, identity)?;
            Ok(quote!((!#inner)))
        }
        Expr::Binary(e) if matches!(e.op, BinOp::And(_) | BinOp::Or(_)) => {
            let left = translate(&e.left, krate, identity)?;
            let right = translate(&e.right, krate, identity)?;
            let op = &e.op;
            Ok(quote!((#left #op #right)))
        }
        Expr::Call(call) => translate_call(call, krate, identity),
        other => Err(syn::Error::new(
            other.span(),
            format!("expected a rule ({RULES}) combined with `&&`, `||` or `!`"),
        )),
    }
}

fn translate_call(
    call: &ExprCall,
    krate: &Path,
    identity: &syn::Ident,
) -> syn::Result<TokenStream2> {
    let name = match &*call.func {
        Expr::Path(p) if p.qself.is_none() => p.path.get_ident(),
        _ => None,
    }
    .ok_or_else(|| syn::Error::new(call.func.span(), format!("expected one of {RULES}")))?;

    let authorities = quote!(#krate::middleware::auth::Authorities);
    let check = |method: &str, joiner: TokenStream2, exactly_one: bool| {
        let method = format_ident!("{}", method);
        let mut checks = string_args(call, exactly_one)?
            .into_iter()
            .map(|n| quote!(#authorities::#method(#identity, #n)));
        let first = checks.next().expect("string_args returns at least one");
        let joined = checks.fold(first, |acc, c| quote!(#acc #joiner #c));
        Ok::<_, syn::Error>(quote!((#joined)))
    };

    match name.to_string().as_str() {
        "authenticated" => {
            if !call.args.is_empty() {
                return Err(syn::Error::new(
                    call.args.span(),
                    "`authenticated()` takes no arguments",
                ));
            }
            Ok(quote!(true))
        }
        "has_role" => check("has_role", quote!(&&), true),
        "has_any_role" => check("has_role", quote!(||), false),
        "has_all_roles" => check("has_role", quote!(&&), false),
        "has_permission" => check("has_permission", quote!(&&), true),
        "has_any_permission" => check("has_permission", quote!(||), false),
        "has_all_permissions" => check("has_permission", quote!(&&), false),
        "check" => match single(&call.args) {
            Some(Expr::Path(path)) => Ok(quote!((#path(#identity)))),
            _ => Err(syn::Error::new(
                call.span(),
                "`check` takes the path of a `fn(&T) -> bool`, e.g. `check(is_owner)`",
            )),
        },
        _ => Err(syn::Error::new(
            name.span(),
            format!("unknown rule `{name}`; expected one of {RULES}"),
        )),
    }
}

fn single(args: &Punctuated<Expr, Token![,]>) -> Option<&Expr> {
    if args.len() == 1 { args.first() } else { None }
}

fn string_args(call: &ExprCall, exactly_one: bool) -> syn::Result<Vec<&LitStr>> {
    if call.args.is_empty() || (exactly_one && call.args.len() != 1) {
        let expected = if exactly_one {
            "exactly one string literal"
        } else {
            "one or more string literals"
        };
        return Err(syn::Error::new(call.span(), format!("expected {expected}")));
    }
    call.args
        .iter()
        .map(|arg| match arg {
            Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) => Ok(s),
            other => Err(syn::Error::new(other.span(), "expected a string literal")),
        })
        .collect()
}
