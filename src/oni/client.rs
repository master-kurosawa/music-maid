use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::SocketAddr;

use crate::oni::oni::{oni_control_client::OniControlClient, QuitRequest};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Endpoint, Uri};
use tower::service_fn;

pub async fn quit() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let channel = Endpoint::try_from("http://[::]:50051")?
        .connect_with_connector(service_fn(|_: Uri| async {
            let uds_addr = SocketAddr::from_abstract_name(b"musicmaid_oni")?;
            let uds_std_stream = std::os::unix::net::UnixStream::connect_addr(&uds_addr)?;
            let uds_stream = UnixStream::from_std(uds_std_stream)?;
            Ok::<_, std::io::Error>(TokioIo::new(uds_stream))
        }))
        .await?;

    let mut client = OniControlClient::new(channel);
    let request = tonic::Request::new(QuitRequest {});
    let response = client.quit(request).await?;

    println!("{response:?}");

    Ok(())
}
