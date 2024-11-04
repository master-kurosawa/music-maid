pub mod db;
mod formats;
pub mod queue;
pub mod reader;

use anyhow::{anyhow, Context};
use db::{
    audio_file::{AudioFile, AudioFileMeta},
    padding::Padding,
    picture::Picture,
    vorbis::{VorbisComment, FLAC_MARKER},
};
use formats::opus_ogg::parse_ogg_pages;
use formats::{flac::parse_flac, opus_ogg::OGG_MARKER};
use ignore::{WalkBuilder, WalkState};
use queue::TaskQueue;
use reader::{walk_dir, UringBufReader};
use sqlx::SqlitePool;
use std::{
    env,
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_uring::fs::File;

async fn read_with_uring(
    path: PathBuf,
    queue: Arc<tokio::sync::Mutex<TaskQueue>>,
) -> anyhow::Result<()> {
    let file = File::open(&path).await?;

    let mut vorbis_comments: Vec<(Vec<VorbisComment>, i64)> = Vec::new();
    let mut pictures_metadata: Vec<Picture> = Vec::new();
    let mut paddings: Vec<Padding> = Vec::new();

    let mut format: Option<String> = None;

    let mut reader = UringBufReader::new(file, path.to_string_lossy().to_string());
    let bytes_read = reader.read_next(8196).await?;

    let marker: [u8; 4] = reader
        .get_bytes(4)
        .await?
        .try_into()
        .with_context(|| anyhow!("Empty file"))?;

    match marker {
        FLAC_MARKER => {
            if bytes_read < 42 {
                return Err(anyhow!(
                    "Not enough bytes for proper flac STREAMINFO, got {}",
                    bytes_read
                ));
            }
            format = Some("flac".to_owned());
            (vorbis_comments, pictures_metadata, paddings) = parse_flac(&mut reader).await?;
        }
        OGG_MARKER => {
            if bytes_read < 42 {
                return Err(anyhow!(
                    "Not enough bytes for proper flac STREAMINFO, got {}",
                    bytes_read
                ));
            }
            (format, vorbis_comments, pictures_metadata, paddings) =
                parse_ogg_pages(&mut reader).await?;
        }
        _ => {}
    }

    let audio_file = AudioFile {
        id: None,
        path: path.to_string_lossy().to_string(),
        name: path.file_name().unwrap().to_string_lossy().to_string(),
        format,
    };
    queue
        .lock()
        .await
        .push(AudioFileMeta {
            audio_file,
            comments: vorbis_comments,
            pictures: pictures_metadata,
            paddings,
        })
        .await;

    Ok(())
}

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    sysinfo::set_open_files_limit(10000);
    if env::args().last().unwrap() == "write" {
        let crazy_path = "./tmp/dir1/dir1/dir2/dir3/dir4/dir5/dir6/dir7/dir8/dir9/dir10/dir11/dir12/dir13/dir14/dir15/dir16/dir17/dir18/dir19/dir20/dir21/dir22/dir23/dir24/dir25/dir26/dir27/dir28/dir29/dir30/seq6/output.opus".to_owned();

        tokio_uring::start(async {
            let pool = SqlitePool::connect("sqlite://dev.db").await.unwrap();
            let file = AudioFile::from_path(crazy_path, &pool).await.unwrap();
            let file = file.fetch_meta(&pool).await.unwrap();
            println!("{file:?}");
        });
        return Ok(());
    }

    let paths = walk_dir("./tmp");
    let mut tasks = Vec::new();
    tokio_uring::start(async {
        let queue = Arc::new(tokio::sync::Mutex::new(TaskQueue::new()));
        for path in paths {
            let queue = Arc::clone(&queue);
            let spawn =
                tokio_uring::spawn(async move { read_with_uring(path, queue).await.unwrap() });
            tasks.push(spawn);
        }
        for task in tasks {
            let t = task.await;
            if let Err(t) = t {
                println!("{t:?}");
            }
        }
        let q = Arc::try_unwrap(queue).unwrap().into_inner();
        TaskQueue::finish(q).await;
    });

    Ok(())
}
