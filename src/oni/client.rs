use std::os::unix::net::SocketAddr;
use std::{borrow::BorrowMut, os::linux::net::SocketAddrExt};

use crate::{
    cli::SearchService, oni::oni::oni_control_client::OniControlClient, oni::oni::SearchRequest,
};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

pub struct Client {
    client: OniControlClient<Channel>,
}

impl Client {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let channel = Endpoint::try_from("http://[::]:50051")?
            .connect_with_connector(service_fn(|_: Uri| async {
                let uds_addr = SocketAddr::from_abstract_name(b"musicmaid_oni")?;
                let uds_std_stream = std::os::unix::net::UnixStream::connect_addr(&uds_addr)?;
                let uds_stream = UnixStream::from_std(uds_std_stream)?;
                Ok::<_, std::io::Error>(TokioIo::new(uds_stream))
            }))
            .await?;

        let mut client = OniControlClient::new(channel);

        Ok(Self { client })
    }

    pub async fn search(
        mut self,
        query: String,
        service: SearchService,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let request = tonic::Request::new(SearchRequest {
            query,
            search_service: service.into(),
        });
        let response = self.client.search(request).await?;
        println!("{response:?}");

        Ok(())
    }
}
