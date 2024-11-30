use std::os::unix::net::SocketAddr;
use std::{os::linux::net::SocketAddrExt, sync::Arc};
use tokio::net::UnixListener;
use tokio::sync::{oneshot, Mutex};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{
    transport::{server::UdsConnectInfo, Server},
    Request, Response, Status,
};

use crate::cli::SearchService;
use crate::oni::oni::{
    oni_control_server::{OniControl, OniControlServer},
    SearchRequest, SearchResponse,
};

#[derive(Default)]
pub struct MyOniControl {}

#[tonic::async_trait]
impl OniControl for MyOniControl {
    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        // let conn_info = request.extensions().get::<UdsConnectInfo>().unwrap();
        // println!("{conn_info:?}");
        println!("{request:?}");

        let search_service: SearchService = request.get_ref().search_service.into();

        match search_service {
            SearchService::Local => todo!(),
            SearchService::LocalMusicbrainz => {
                let mut client = musicbrainz_db_client::create_client()
                    .await
                    .expect("Hurr durr");
                musicbrainz_db_client::search(&mut client, request.get_ref().query.clone()).await;
            }
        }

        Ok(Response::new(SearchResponse {}))
    }
}

pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let uds_addr = SocketAddr::from_abstract_name(b"musicmaid_oni")?;
    let uds_std_listener = std::os::unix::net::UnixListener::bind_addr(&uds_addr)?;
    let uds_listener = UnixListener::from_std(uds_std_listener)?;
    let uds_stream = UnixListenerStream::new(uds_listener);

    let oni_control_service = MyOniControl {
        ..MyOniControl::default()
    };

    Server::builder()
        .add_service(OniControlServer::new(oni_control_service))
        .serve_with_incoming(uds_stream)
        .await?;

    Ok(())
}
