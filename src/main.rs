use clap::Parser;
use cli::Cli;
use libc::printf;
use oni::client;

mod cli;
mod oni;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    // let uds_addr = SocketAddr::from_abstract_name(b"musicmaid_oni")?;
    // let uds_std_listener = std::os::unix::net::UnixStream::bind_addr(&uds_addr)?;

    match cli.command {
        None => return Ok(()),
        Some(cli::Commands::Oni) => {
            oni::server::run().await?;
        }
        Some(cli::Commands::Search { query, service }) => {
            println!("Searching!");
            let result = client::quit().await;
            println!("{result:?}");
        }
    }

    Ok(())
}
