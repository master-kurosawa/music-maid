use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(clap::ValueEnum, Clone, Default)]
pub enum SearchService {
    #[default]
    Local,
    LocalMusicbrainz,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Daemonize musicmaid opening an abstract unix socket for communication
    Oni,
    /// Search for releases, albums, artists using a query
    Search {
        query: String,

        #[arg(short, long, default_value_t, value_enum)]
        service: SearchService,
    },
}
