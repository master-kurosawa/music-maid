use std::os::unix::net::SocketAddr;
use std::{os::linux::net::SocketAddrExt, sync::Arc};
use tokio::net::UnixListener;
use tokio::sync::{oneshot, Mutex};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{
    transport::{server::UdsConnectInfo, Server},
    Request, Response, Status,
};

use crate::oni::oni::{
    oni_control_server::{OniControl, OniControlServer},
    QuitReply, QuitRequest,
};

#[derive(Default)]
pub struct MyOniControl {
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

#[tonic::async_trait]
impl OniControl for MyOniControl {
    async fn quit(&self, request: Request<QuitRequest>) -> Result<Response<QuitReply>, Status> {
        let conn_info = request.extensions().get::<UdsConnectInfo>().unwrap();
        println!("{conn_info:?}");

        let tx = self.shutdown_tx.lock().await.take();

        if let Some(tx) = tx {
            let _ = tx.send(());
            println!("FUCK you!!!!");
        } else {
            println!("Motherfucker {tx:?}");
        }

        Ok(Response::new(QuitReply {}))
    }
}

pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let uds_addr = SocketAddr::from_abstract_name(b"musicmaid_oni")?;
    let uds_std_listener = std::os::unix::net::UnixListener::bind_addr(&uds_addr)?;
    let uds_listener = UnixListener::from_std(uds_std_listener)?;
    let uds_stream = UnixListenerStream::new(uds_listener);

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let oni_control_service = MyOniControl {
        shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
        ..MyOniControl::default()
    };

    Server::builder()
        .add_service(OniControlServer::new(oni_control_service))
        // .serve_with_incoming(uds_stream)
        .serve_with_incoming_shutdown(uds_stream, async {
            shutdown_rx.await.ok();
        })
        .await?;

    Ok(())
}
