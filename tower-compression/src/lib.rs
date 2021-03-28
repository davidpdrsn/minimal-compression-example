use futures::stream::Stream;
use http_body::Body;
use pin_project::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};

pub mod compression;
pub mod decompression;

#[pin_project]
pub(crate) struct BodyIntoStream<B> {
    #[pin]
    body: B,
}

#[allow(dead_code)]
impl<B> BodyIntoStream<B> {
    pub(crate) fn new(body: B) -> Self {
        Self { body }
    }

    /// Get a pinned mutable reference to the inner body
    pub(crate) fn get_pin_mut(self: Pin<&mut Self>) -> Pin<&mut B> {
        self.project().body
    }
}

impl<B> Stream for BodyIntoStream<B>
where
    B: Body,
{
    type Item = Result<B::Data, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project()
            .body
            .poll_data(cx)
            .map_err(|_| panic!("error"))
    }
}
