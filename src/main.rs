#![feature(string_from_utf8_lossy_owned)]
pub mod db;
mod formats;
mod io;
pub mod queue;
use db::audio_file::AudioFile;
use formats::opus_ogg::remove_comments;
use io::{
    ogg::OggPageReader,
    reader::{load_data_from_paths, walk_dir, ThrottleConfig, UringBufReader},
};
use sqlx::SqlitePool;
use std::{env, error::Error};
use tokio_uring::fs::OpenOptions;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    sysinfo::set_open_files_limit(10000);
    if env::args().last().unwrap() == "rehash" {
        let crazy_path = "./x/wheeler.opus".to_owned();
        tokio_uring::start(async {
            let file = OpenOptions::new()
                .write(true)
                .read(true)
                .open(&crazy_path)
                .await
                .unwrap();
            let mut reader = UringBufReader::new(file, crazy_path.into());
            let _bytes_read = reader.read_next(8196).await.unwrap();
            let mut reader = OggPageReader::new(&mut reader).await.unwrap();
            reader.parse_till_end().await.unwrap();
            reader.recalculate_last_crc().await.unwrap();
            reader.parse_header().await.unwrap();
            reader.rehash_headers().await.unwrap();
            reader.reader.file.sync_all().await.unwrap();
        });
        return Ok(());
    }
    if env::args().last().unwrap() == "write" {
        let crazy_path = "./x/wheeler.opus".to_owned();

        tokio_uring::start(async {
            let pool = SqlitePool::connect("sqlite://dev.db").await.unwrap();
            let file = AudioFile::from_path(crazy_path.clone(), &pool)
                .await
                .unwrap();

            let file = file.fetch_meta(&pool).await.unwrap();
            remove_comments(
                file,
                vec!["metadata_block_picture".to_owned(), "author".to_owned()],
            )
            .await
            .unwrap();
        });
        return Ok(());
    }

    let paths = walk_dir("./tmp");
    let conf = ThrottleConfig::new(8);
    let _ = tokio_uring::builder()
        .entries(1024)
        .start(async { load_data_from_paths(paths, conf).await });

    Ok(())
}
