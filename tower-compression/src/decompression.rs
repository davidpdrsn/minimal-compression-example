use super::BodyIntoStream;
use async_compression::tokio::bufread::GzipDecoder;
use bytes::{Bytes, BytesMut};
use futures::ready;
use http::{Request, Response};
use http_body::Body;
use pin_project::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_util::io::poll_read_buf;
use tokio_util::io::StreamReader;
use tower_service::Service;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Copy)]
pub struct Decompression<S> {
    pub(crate) inner: S,
}

impl<S> Decompression<S> {
    pub fn new(service: S) -> Self {
        Self { inner: service }
    }
}

impl<ReqBody, ResBody, S> Service<Request<ReqBody>> for Decompression<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: Body<Data = Bytes>,
    ResBody::Error: Into<BoxError>,
{
    type Response = Response<DecompressionBody<ResBody>>;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        req.headers_mut()
            .insert("accept-encoding", "gzip".parse().unwrap());

        ResponseFuture {
            inner: self.inner.call(req),
        }
    }
}

#[pin_project]
#[derive(Debug)]
pub struct ResponseFuture<F> {
    #[pin]
    pub(crate) inner: F,
}

impl<F, B, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<B>, E>>,
    B: Body<Data = Bytes>,
{
    type Output = Result<Response<DecompressionBody<B>>, E>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = ready!(self.as_mut().project().inner.poll(cx)?);

        let (mut parts, body) = res.into_parts();

        let stream = BodyIntoStream::new(body);
        let read = StreamReader::new(stream);
        let read = GzipDecoder::new(read);
        let body = DecompressionBody(read);

        parts.headers.remove("content-encoding");
        parts.headers.remove("content-length");

        let res = Response::from_parts(parts, body);
        Poll::Ready(Ok(res))
    }
}

#[pin_project]
pub struct DecompressionBody<B>(#[pin] GzipDecoder<StreamReader<BodyIntoStream<B>, Bytes>>);

impl<B> Body for DecompressionBody<B>
where
    B: Body<Data = Bytes>,
{
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_data(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        let mut this = self.project();
        let mut buf = BytesMut::new();

        let read = match ready!(poll_read_buf(this.0.as_mut(), cx, &mut buf)) {
            Ok(read) => read,
            Err(err) => {
                return Poll::Ready(Some(Err(err)));
            }
        };

        if read == 0 {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(buf.freeze())))
        }
    }

    fn poll_trailers(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<http::HeaderMap>, Self::Error>> {
        let trailers = ready!(self
            .project()
            .0
            .get_pin_mut()
            .get_pin_mut()
            .get_pin_mut()
            .poll_trailers(cx))
        .unwrap_or_else(|_| panic!());

        Poll::Ready(Ok(trailers))
    }
}
