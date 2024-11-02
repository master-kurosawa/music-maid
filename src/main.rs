pub mod db;
mod formats;
pub mod queue;
pub mod reader;

use anyhow::{anyhow, Context};
use db::{
    audio_file::AudioFile,
    padding::Padding,
    picture::Picture,
    vorbis::{VorbisComment, FLAC_MARKER},
};
use formats::opus_ogg::parse_ogg_pages;
use formats::{flac::parse_flac, opus_ogg::OGG_MARKER};
use ignore::{WalkBuilder, WalkState};
use queue::TaskQueue;
use reader::UringBufReader;
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_uring::fs::File;

async fn read_with_uring(
    path: &Path,
    queue: Arc<tokio::sync::Mutex<TaskQueue>>,
) -> anyhow::Result<()> {
    let file = File::open(path).await?;

    let mut vorbis_comments: Vec<VorbisComment> = Vec::new();
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
            parse_flac(
                &mut reader,
                &mut vorbis_comments,
                &mut pictures_metadata,
                &mut paddings,
            )
            .await?;
        }
        OGG_MARKER => {
            if bytes_read < 42 {
                return Err(anyhow!(
                    "Not enough bytes for proper flac STREAMINFO, got {}",
                    bytes_read
                ));
            }
            format = Some(
                parse_ogg_pages(
                    &mut reader,
                    &mut vorbis_comments,
                    &mut pictures_metadata,
                    &mut paddings,
                )
                .await?,
            );
        }
        _ => {}
    }

    queue
        .lock()
        .await
        .push(AudioFile {
            path: path.to_string_lossy().to_string(),
            name: path.file_name().unwrap().to_string_lossy().to_string(),
            format,
            comments: vorbis_comments,
            pictures: pictures_metadata,
            paddings,
        })
        .await;

    Ok(())
}

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    sysinfo::set_open_files_limit(10000);
    let paths: Arc<Mutex<Vec<Arc<PathBuf>>>> = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = Vec::new();
    let builder = WalkBuilder::new("./tmp");
    builder.build_parallel().run(|| {
        Box::new(|path| {
            match path {
                Ok(entry) => {
                    if entry.file_type().unwrap().is_dir() {
                        return WalkState::Continue;
                    }
                    let path = Arc::new(entry.path().to_path_buf());
                    let paths = Arc::clone(&paths);
                    paths.lock().unwrap().push(path);
                }
                Err(_) => panic!(),
            }
            WalkState::Continue
        })
    });
    tokio_uring::start(async {
        let queue = Arc::new(tokio::sync::Mutex::new(TaskQueue::new()));
        for entry in paths.lock().into_iter() {
            entry.clone().into_iter().for_each(|path| {
                let queue = Arc::clone(&queue);
                let spawn =
                    tokio_uring::spawn(async move { read_with_uring(&path, queue).await.unwrap() });

                tasks.push(spawn);
            });
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
