use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;
use pin_project::pin_project;

/// Wrapper for [`Future`]/[`Stream`] that releases GIL while polling in
/// [`PyFuture`](crate::PyFuture)/[`PyStream`](crate::PyStream).
///
/// Can be instantiated with [`AllowThreadsExt::allow_threads`].
///
/// [`Stream`]: https://docs.rs/futures/latest/futures/stream/trait.Stream.html
#[derive(Debug)]
#[repr(transparent)]
#[pin_project]
pub struct AllowThreads<T>(#[pin] pub T);

impl<F: Future> Future for AllowThreads<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        this.0.poll(cx)
    }
}

impl<S: Stream> Stream for AllowThreads<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        this.0.poll_next(cx)
    }
}

/// Extension trait to allow threads while polling [`Future`] or [`Stream`].
///
/// It is implemented for every types.
///
/// [`Stream`]: https://docs.rs/futures/latest/futures/stream/trait.Stream.html
pub trait AllowThreadsExt: Sized {
    fn allow_threads(self) -> AllowThreads<Self> {
        AllowThreads(self)
    }
}

impl<T> AllowThreadsExt for T {}
