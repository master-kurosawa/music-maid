pub mod db;
mod formats;
mod io;
pub mod queue;
use db::{audio_file::AudioFile, vorbis::VorbisComment};
use io::{
    ogg::OggPageReader,
    reader::{load_data_from_paths, walk_dir, UringBufReader},
};
use sqlx::SqlitePool;
use std::{env, error::Error};
use tokio_uring::fs::{File, OpenOptions};

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    sysinfo::set_open_files_limit(10000);
    if env::args().last().unwrap() == "write" {
        let crazy_path = "./lol/output.opus".to_owned();

        tokio_uring::start(async {
            let pool = SqlitePool::connect("sqlite://dev.db").await.unwrap();
            let file = AudioFile::from_path(crazy_path.clone(), &pool)
                .await
                .unwrap();

            let file = file.fetch_meta(&pool).await.unwrap();
            let mut reader = UringBufReader::new(
                OpenOptions::new()
                    .write(true)
                    .read(true)
                    .open(&crazy_path)
                    .await
                    .unwrap(),
                crazy_path.clone(),
            );

            let pics = file.comments[0]
                .0
                .clone()
                .into_iter()
                .filter(|v| v.key == "metadata_block_picture")
                .map(|pv| (pv.file_ptr, pv.size, pv.last_ogg_header_ptr.unwrap()))
                .collect::<Vec<(i64, i64, i64)>>();
            let v_start = file.comments[0].1;
            for (ptr, size, last_ogg) in pics {
                reader.file_ptr = last_ogg as u64;
                let mut r = OggPageReader::new(&mut reader).await.unwrap();
                let s = ptr as u64 - (r.reader.file_ptr + r.reader.cursor);
                r.skip(s as usize).await.unwrap();
                let z = String::from_utf8_lossy(
                    &r.reader.buf[r.reader.cursor as usize..r.reader.cursor as usize + 128],
                );
                //   r.write_stream(&vec![
                //       0x06, 0x00, 0x00, 0x00, b't', b'e', b's', b't', b'=', b'a',
                //   ])
                //   .await
                //   .unwrap();
                let left = r.segment_size - r.cursor;
                r.write_stream(&(3 as u32).to_le_bytes()).await.unwrap();
                r.write_stream(&[b'x', b'=', b'z']).await.unwrap();
                r.pad_till_end().await.unwrap();
            }
            //println!("{file:?}");
        });
        return Ok(());
    }

    let paths = walk_dir("./lol");
    tokio_uring::builder()
        .entries(1024)
        .start(async { load_data_from_paths(paths).await });

    Ok(())
}
