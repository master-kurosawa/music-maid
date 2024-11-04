pub mod db;
mod formats;
pub mod queue;
pub mod reader;

use db::audio_file::AudioFile;
use reader::{load_data_from_paths, walk_dir};
use sqlx::SqlitePool;
use std::{env, error::Error};

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
    tokio_uring::start(async { load_data_from_paths(paths).await });

    Ok(())
}
