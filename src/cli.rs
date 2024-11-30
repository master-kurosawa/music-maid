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

impl From<SearchService> for i32 {
    fn from(value: SearchService) -> Self {
        match value {
            SearchService::Local => 0,
            SearchService::LocalMusicbrainz => 1,
        }
    }
}

impl From<i32> for SearchService {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::Local,
            1 => Self::LocalMusicbrainz,
            _ => Self::Local,
        }
    }
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
