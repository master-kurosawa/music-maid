mod cli;

// use musicbrainz_db_client;
use std::{path::PathBuf, time::Duration};

use clap::Parser;
use cli::{Cli, Commands, SearchService};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /*
        let mut client = musicbrainz_db_client::create_client().await?;
        let resp = musicbrainz_db_client::search(&mut client, "test".to_string()).await;
        println!("{:?}", resp);
    */
    let progress_bar = ProgressBar::new(100).with_style(get_spinner_style(0, LineStyle::Normal));

    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Search { service, query, .. }) => {
            progress_bar.enable_steady_tick(Duration::from_millis(100));
            progress_bar.set_message(format!("Searching for `{query}`"));
            sleep(Duration::from_secs(2)).await;
            progress_bar.finish();
            perform_search(service, query).await;
        }
        _ => (),
    }

    Ok(())
}

async fn perform_search(service: SearchService, query: String) {
    match service {
        SearchService::Local => todo!(),
        SearchService::LocalMusicbrainz => todo!(),
    }
}

fn get_spinner_style(label_width: usize, style: LineStyle) -> ProgressStyle {
    let template = format!(
        "{{prefix:>{}.bold.dim}} {{spinner}} {{elapsed}} {{wide_msg}}",
        label_width
    );

    match style {
        LineStyle::Normal | LineStyle::Success | LineStyle::SuccessNoop => {
            ProgressStyle::default_spinner()
                .tick_chars("🕛🕐🕑🕒🕓🕔🕕🕖🕗🕘🕙🕚✅")
                .template(&template)
                .unwrap()
        }
        LineStyle::Failure => ProgressStyle::default_spinner()
            .tick_chars("❌❌")
            .template(&template)
            .unwrap(),
    }
}

/// Style of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    Normal,
    Success,
    SuccessNoop,
    Failure,
}
