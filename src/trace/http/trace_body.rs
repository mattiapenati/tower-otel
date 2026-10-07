use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use pin_project::{pin_project, pinned_drop};
use tracing::Span;

/// Response body that records errors and early drops on the request span.
#[pin_project(PinnedDrop)]
pub struct TraceBody<B>
where
    B: http_body::Body,
{
    #[pin]
    body: Option<B>,
    span: Option<Span>,
}

impl<B> TraceBody<B>
where
    B: http_body::Body,
{
    /// Drops the body intentionally without recording an early-drop error.
    pub fn cancel(mut self) {
        self.span = None;
    }

    /// Extracts the inner body without recording an early-drop error.
    pub fn into_inner(mut self) -> B {
        self.span = None;
        self.body.take().unwrap()
    }

    /// Associates a response body with its request span.
    pub(super) fn new(body: B, span: Span) -> Self {
        Self {
            body: Some(body),
            span: Some(span),
        }
    }
}

impl<B> http_body::Body for TraceBody<B>
where
    B: http_body::Body,
    B::Error: std::fmt::Display,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.project();

        let body = this.body.as_pin_mut().unwrap();

        let result = match ready!(body.poll_frame(cx)) {
            Some(Ok(frame)) => Some(Ok(frame)),
            Some(Err(err)) => {
                if let Some(span) = this.span.as_ref() {
                    super::record_error(span, &err);
                }
                *this.span = None;
                Some(Err(err))
            }
            None => {
                *this.span = None;
                None
            }
        };
        Poll::Ready(result)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.body.as_ref().unwrap().is_end_stream()
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        self.body.as_ref().unwrap().size_hint()
    }
}

#[pinned_drop]
impl<B> PinnedDrop for TraceBody<B>
where
    B: http_body::Body,
{
    fn drop(self: Pin<&mut Self>) {
        let this = self.project();

        let Some(span) = this.span.as_ref() else {
            return;
        };

        let body = this.body.as_ref().as_pin_ref().unwrap();

        // Consumers may finish without polling for `None`.
        if body.is_end_stream() {
            return;
        }

        super::record_error(span, &"connection closed while transmitting body");
    }
}
