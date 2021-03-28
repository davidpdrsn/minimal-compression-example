#![allow(dead_code)]

use bytes::Bytes;
use futures::StreamExt;
use hyper::body::HttpBody;
use hyper::Server;
use proto::{
    app_client::AppClient, app_server, SendReply, SendRequest, SubscribeReply, SubscribeRequest,
};
use std::{net::SocketAddr, net::TcpListener, pin::Pin};
use tokio::sync::broadcast::{channel, Sender};
use tokio_stream::{wrappers::BroadcastStream, Stream};
use tonic::{async_trait, body::BoxBody, transport::Channel, Request, Response, Status};
use tower::make::Shared;
use tower::{BoxError, Service};
use tower_compression::compression::Compression;
use tower_compression::decompression::Decompression;

mod proto {
    tonic::include_proto!("app");
}

fn main() {}

// We make this a separate function so we're able to call it from tests.
async fn serve_forever(listener: TcpListener) -> Result<(), Box<dyn std::error::Error>> {
    let (tx, _rx) = channel(1024);

    let service = app_server::AppServer::new(ServerImpl { tx });

    // automatically compress response bodies
    let service = Compression::new(service);

    let addr = listener.local_addr()?;

    tracing::info!("Listening on {}", addr);

    Server::from_tcp(listener)?
        .http2_only(true)
        .serve(Shared::new(service))
        .await?;

    Ok(())
}

async fn make_client(
    addr: SocketAddr,
) -> Result<
    AppClient<
        impl Service<
                hyper::Request<BoxBody>,
                Response = hyper::Response<
                    impl HttpBody<Data = Bytes, Error = impl Into<BoxError>>,
                >,
                Error = impl Into<BoxError>,
            > + Clone
            + Send
            + Sync
            + 'static,
    >,
    tonic::transport::Error,
> {
    let uri = format!("http://{}", addr)
        .parse::<tonic::transport::Uri>()
        .unwrap();

    let channel = Channel::builder(uri).connect().await?;

    // automatically decompress response bodies
    let channel = Decompression::new(channel);

    Ok(AppClient::new(channel))
}

#[derive(Debug, Clone)]
struct ServerImpl {
    tx: Sender<SubscribeReply>,
}

#[async_trait]
impl app_server::App for ServerImpl {
    async fn send(&self, request: Request<SendRequest>) -> Result<Response<SendReply>, Status> {
        let SendRequest { value } = request.into_inner();

        self.tx
            .send(SubscribeReply { value })
            .expect("failed to send");

        Ok(Response::new(SendReply {}))
    }

    type SubscribeStream =
        Pin<Box<dyn Stream<Item = Result<SubscribeReply, Status>> + Send + Sync + 'static>>;

    async fn subscribe(
        &self,
        request: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let SubscribeRequest {} = request.into_inner();

        let rx = self.tx.subscribe();
        let stream = BroadcastStream::new(rx)
            .filter_map(|item| async move {
                // ignore receive errors
                item.ok()
            })
            .map(Ok);
        let stream: Self::SubscribeStream = Box::pin(stream);
        let res = Response::new(stream);

        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn get_and_set_value() {
        let addr = run_in_background();

        let mut client = make_client(addr).await.unwrap();

        let mut stream = client
            .subscribe(SubscribeRequest {})
            .await
            .unwrap()
            .into_inner();

        client
            .send(SendRequest {
                value: "foo".to_string(),
            })
            .await
            .unwrap();

        let streamed_value = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("stream hangs")
            .unwrap()
            .unwrap()
            .value;
        assert_eq!(streamed_value, "foo");
    }

    // Run our service in a background task.
    fn run_in_background() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Could not bind ephemeral socket");
        let addr = listener.local_addr().unwrap();

        // just for debugging
        eprintln!("Listening on {}", addr);

        tokio::spawn(async move {
            serve_forever(listener).await.unwrap();
        });

        addr
    }
}
