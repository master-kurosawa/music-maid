mod formats;
pub mod shared;
pub mod utils;

use anyhow::anyhow;
use formats::flac::parse_flac;
use formats::opus_ogg::parse_ogg_page;
use ignore::{WalkBuilder, WalkState};
use shared::{MusicFile, Picture, VorbisComment, FLAC_MARKER, OGG_MARKER};
use sqlx::migrate::MigrateDatabase;
use sqlx::{Sqlite, SqlitePool};
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_uring::fs::File;
use utils::TaskQueue;

async fn read_with_uring(
    path: &Path,
    queue: Arc<tokio::sync::Mutex<TaskQueue>>,
) -> anyhow::Result<()> {
    let file = File::open(path).await?;
    let mut vorbis_comments: Vec<VorbisComment> = Vec::new();
    let mut pictures_metadata: Vec<Picture> = Vec::new();

    let buf = vec![0; 8196];
    let (_res, prefix_buf) = file.read_at(buf, 0).await;
    let bytes_read = _res?;

    let marker: [u8; 4] = prefix_buf[0..4].try_into().unwrap();
    match marker {
        FLAC_MARKER => {
            if bytes_read < 42 {
                return Err(anyhow!(
                    "Not enough bytes for proper flac STREAMINFO, got {}",
                    bytes_read
                ));
            }
            parse_flac(
                prefix_buf,
                file,
                &mut vorbis_comments,
                &mut pictures_metadata,
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
            parse_ogg_page(
                prefix_buf,
                file,
                &mut vorbis_comments,
                &mut pictures_metadata,
            )
            .await?;
        }
        _ => {}
    }

    let path = path.to_string_lossy().to_string();
    queue
        .lock()
        .await
        .push(MusicFile {
            path,
            comments: vorbis_comments,
            pictures: pictures_metadata,
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
                    let clone_xd = Arc::clone(&paths);
                    clone_xd.lock().unwrap().push(path);
                }
                Err(_) => panic!(),
            }
            WalkState::Continue
        })
    });
    tokio_uring::start(async {
        let url = "sqlite://music.db";
        if !Sqlite::database_exists(url).await.unwrap_or(false) {
            Sqlite::create_database(url).await.unwrap();
        }

        let pool = SqlitePool::connect(url).await.unwrap();
        sqlx::query(
            "
    CREATE TABLE IF NOT EXISTS vorbis_comments (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        file_id INTEGER NOT NULL,
        vendor TEXT NOT NULL,
        title TEXT NOT NULL,
        version TEXT NOT NULL,
        album TEXT NOT NULL,
        tracknumber TEXT NOT NULL,
        artist TEXT NOT NULL,
        performer TEXT NOT NULL,
        copyright TEXT NOT NULL,
        license TEXT NOT NULL,
        organization TEXT NOT NULL,
        description TEXT NOT NULL,
        genre TEXT NOT NULL,
        date TEXT NOT NULL,
        location TEXT NOT NULL,
        contact TEXT NOT NULL,
        isrc TEXT NOT NULL,
        FOREIGN KEY (file_id) REFERENCES files(id)
    );
",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS picture_metadata(
        file_id INTEGER NOT NULL,
        picture_type INTEGER NOT NULL,
        mime TEXT NOT NULL,
        description TEXT NOT NULL,
        width INTEGER NOT NULL,
        height INTEGER NOT NULL,
        color_depth INTEGER NOT NULL,
        indexed_color_number INTEGER NOT NULL,
        size INTEGER NOT NULL,
        FOREIGN KEY (file_id) REFERENCES files(id)
);
",
        )
        .execute(&pool)
        .await
        .unwrap();
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
            task.await.unwrap();
        }
        let q = Arc::try_unwrap(queue).unwrap().into_inner();
        TaskQueue::finish(q).await;
    });

    Ok(())
}
