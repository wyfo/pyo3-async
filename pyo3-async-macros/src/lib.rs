use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse::Parser, parse_macro_input, parse_quote, parse_quote_spanned, punctuated::Punctuated,
    spanned::Spanned,
};

const MODULES: [&str; 3] = ["asyncio", "trio", "sniffio"];

macro_rules! unwrap {
    ($result:expr) => {
        match $result {
            Ok(ok) => ok,
            Err(err) => return err.into_compile_error().into(),
        }
    };
}

struct Options {
    module: syn::Path,
    allow_threads: bool,
}

fn parse_options(attr: TokenStream) -> syn::Result<Options> {
    let mut allow_threads = false;
    let mut module = None;
    let module_parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("allow_threads") {
            allow_threads = true;
        } else if MODULES.iter().any(|m| meta.path.is_ident(m)) {
            if module.is_some() {
                return Err(meta.error("multiple Python async backend specified"));
            }
            module = Some(meta.path);
        } else {
            return Err(meta.error("invalid option"));
        }
        Ok(())
    });
    module_parser.parse(attr)?;
    Ok(Options {
        module: module.unwrap_or_else(|| parse_quote!(asyncio)),
        allow_threads,
    })
}

fn build_coroutine(
    path: impl ToTokens,
    attrs: &mut Vec<syn::Attribute>,
    sig: &mut syn::Signature,
    block: &mut syn::Block,
    options: &Options,
) -> syn::Result<()> {
    attrs.retain(|attr| attr.meta.path().is_ident("pyo3"));
    let mut has_name = false;
    for attr in attrs.iter() {
        has_name |= attr
            .parse_args_with(Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)?
            .into_iter()
            .any(|meta| matches!(meta, syn::Meta::NameValue(nv) if nv.path.is_ident("name")));
    }
    if !has_name {
        let name = format!("{}", &sig.ident);
        attrs.push(parse_quote!(#[pyo3(name = #name)]));
    }
    let ident = sig.ident.clone();
    sig.ident = format_ident!("async_{ident}");
    sig.asyncness = None;
    let module = &options.module;
    let coro_path = quote!(::pyo3_async::#module::Coroutine);
    let params = sig.inputs.iter().map(|arg| match arg {
        syn::FnArg::Receiver(_) => quote!(self),
        syn::FnArg::Typed(syn::PatType { pat, .. }) => quote!(#pat),
    });
    let mut future = quote!(#path(#(#params),*));
    if matches!(sig.output, syn::ReturnType::Default) {
        future = quote!(async move {#future.await; pyo3::PyResult::Ok(())})
    }
    if options.allow_threads {
        future = quote!(::pyo3_async::AllowThreads(#future));
    }
    // return statement because `parse_quote_spanned` doesn't work otherwise
    block.stmts = vec![parse_quote_spanned! { block.span() =>
        #[allow(clippy::needless_return)]
        return #coro_path::from_future(#future);
    }];
    sig.output = parse_quote_spanned!(sig.output.span() => -> #coro_path);
    Ok(())
}

/// [`pyo3::pyfunction`] with async support.
///
/// Generate a additional function prefixed by `async_`, decorated by [`pyo3::pyfunction`] and
/// `#[pyo3(name = ...)]`.
///
/// Python async backend can be specified using macro argument (default to `asyncio`).
/// If `allow_threads` is passed in arguments, GIL will be released for future polling (see
/// [`AllowThreads`])
///
/// # Example
///
/// ```rust
/// #[pyo3_async::pyfunction(allow_threads)]
/// pub async fn print(s: String) {
///     println!("{s}");
/// }
/// ```
/// generates
/// ```rust
/// pub async fn print(s: String) {
///     println!("{s}");
/// }
/// #[::pyo3::pyfunction]
/// #[pyo3(name = "dumb_add")]
/// pub fn async_print(s: String) -> ::pyo3_async::asyncio::Coroutine {
///     ::pyo3_async::asyncio::Coroutine::from_future(::pyo3_async::AllowThreads(
///         async move { print(s).await; Ok(()) }
///     ))
/// }
/// ```
///
/// [`pyo3::pyfunction`]: https://docs.rs/pyo3/latest/pyo3/attr.pyfunction.html
/// [`AllowThreads`]: https://docs.rs/pyo3-async/latest/pyo3_async/struct.AllowThreads.html
#[proc_macro_attribute]
pub fn pyfunction(attr: TokenStream, input: TokenStream) -> TokenStream {
    let options = unwrap!(parse_options(attr));
    let mut func = parse_macro_input!(input as syn::ItemFn);
    if func.sig.asyncness.is_none() {
        return quote!(#[::pyo3::pyfunction] #func).into();
    }
    let mut coro = func.clone();
    unwrap!(build_coroutine(
        &func.sig.ident,
        &mut coro.attrs,
        &mut coro.sig,
        &mut coro.block,
        &options
    ));
    func.attrs.retain(|attr| !attr.meta.path().is_ident("pyo3"));
    let expanded = quote! {
        #func
        #[::pyo3::pyfunction]
        #coro
    };
    expanded.into()
}

/// [`pyo3::pymethods`] with async support.
///
/// For each async methods, generate a additional function prefixed by `async_`, decorated with
/// `#[pyo3(name = ...)]`. Original async methods are kept in a separate impl, while the original
/// impl is decorated with [`pyo3::pymethods`].
///
/// Python async backend can be specified using macro argument (default to `asyncio`).
/// If `allow_threads` is passed in arguments, GIL will be released for future polling (see
/// [`AllowThreads`])
///
/// # Example
///
/// ```rust
/// #[pyo3::pyclass]
/// struct Counter(usize);
///
/// #[pyo3_async::pymethods(trio)]
/// impl Counter {
///     fn incr_sync(&mut self) -> usize {
///         self.0 += 1;
///         self.0
///     }
///
///     // Arguments needs to implement `Send + 'static`, so `self` must be passed using `Py<Self>`
///     async fn incr_async(self_: pyo3::Py<Self>) -> pyo3::PyResult<usize> {
///         pyo3::Python::with_gil(|gil| {
///             let mut this = self_.borrow_mut(gil);
///             this.0 += 1;
///             Ok(this.0)
///         })
///     }
/// }
/// ```
/// generates
/// ```rust
/// #[pyo3::pyclass]
/// struct Counter(usize);
///
/// #[::pyo3::pymethods]
/// impl Counter {
///     fn incr_sync(&mut self) -> usize {
///         self.0 += 1;
///         self.0
///     }
///
///     #[pyo3(name = "incr_async")]
///     fn async_incr_async(self_: pyo3::Py<Self>) -> ::pyo3_async::trio::Coroutine {
///         ::pyo3_async::trio::Coroutine::from_future(Counter::incr_async(self_))
///     }
/// }
/// impl Counter {
///     async fn incr_async(self_: pyo3::Py<Self>) -> pyo3::PyResult<usize> {
///         pyo3::Python::with_gil(|gil| {
///             let mut this = self_.borrow_mut(gil);
///             this.0 += 1;
///             Ok(this.0)
///         })
///     }
/// }
/// ```
///
/// [`pyo3::pymethods`]: https://docs.rs/pyo3/latest/pyo3/attr.pymethods.html
/// [`AllowThreads`]: https://docs.rs/pyo3-async/latest/pyo3_async/struct.AllowThreads.html
#[proc_macro_attribute]
pub fn pymethods(attr: TokenStream, input: TokenStream) -> TokenStream {
    let options = unwrap!(parse_options(attr));
    let mut r#impl = parse_macro_input!(input as syn::ItemImpl);
    let (async_methods, items) = r#impl.items.into_iter().partition::<Vec<_>, _>(
        |item| matches!(item, syn::ImplItem::Fn(func) if func.sig.asyncness.is_some()),
    );
    r#impl.items = items;
    if async_methods.is_empty() {
        return quote!(#[::pyo3::pymethods] #r#impl).into();
    }
    let mut async_impl = r#impl.clone();
    async_impl.items = async_methods;
    async_impl.attrs.clear();
    for item in &mut async_impl.items {
        let syn::ImplItem::Fn(method) = item else {
            unreachable!()
        };
        let mut coro = method.clone();
        let self_ty = &r#impl.self_ty;
        let method_name = &method.sig.ident;
        unwrap!(build_coroutine(
            quote!(#self_ty::#method_name),
            &mut coro.attrs,
            &mut coro.sig,
            &mut coro.block,
            &options
        ));
        method
            .attrs
            .retain(|attr| !attr.meta.path().is_ident("pyo3"));
        method.attrs.retain(|attr| {
            if ["getter", "classmethod", "staticmethod"]
                .iter()
                .any(|m| attr.meta.path().is_ident(m))
            {
                coro.attrs.push(attr.clone());
                return false;
            }
            true
        });
        r#impl.items.push(syn::ImplItem::Fn(coro));
    }
    let expanded = quote! {
        #[::pyo3::pymethods]
        #r#impl
        #async_impl
    };
    expanded.into()
}
