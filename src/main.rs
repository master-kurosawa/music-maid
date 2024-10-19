use ::futures::channel::{mpsc, mpsc::Sender};
use ::futures::stream::FuturesUnordered;
use ::futures::{SinkExt, StreamExt};
use anyhow::anyhow;
use ignore::{Walk, WalkBuilder, WalkState};
use nom::{bits, bytes};
use nom::{
    bytes::{complete, streaming},
    IResult,
};
use sqlx::migrate::MigrateDatabase;
use sqlx::{Pool, Sqlite, SqlitePool};
use tokio::stream;
use tokio::sync::futures;

use std::mem;
use std::thread::JoinHandle;
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_uring::fs::File;

const FLAC_MARKER: [u8; 4] = [0x66, 0x4C, 0x61, 0x43];
const QUEUE_LIMIT: usize = 50;

#[allow(non_camel_case_types)]
struct VORBIS_COMMENT_MARKER;
impl VORBIS_COMMENT_MARKER {
    const END_OF_BLOCK: u8 = 0b10000100;
    const MARKER: u8 = 0b00000100;
}

#[allow(non_camel_case_types)]
struct PICTURE_MARKER;
impl PICTURE_MARKER {
    const END_OF_BLOCK: u8 = 0b10000110;
    const MARKER: u8 = 0b00000110;
}

#[derive(Debug)]
struct Picture {
    picture_type: u32,
    mime: String,
    description: String,
    width: u32,
    height: u32,
    color_depth: u32,
    indexed_color_number: u32,
    picture_data: Vec<u8>,
}

fn parse_vorbis(main_cursor: &mut usize, buf: &[u8], header: &[u8]) -> String {
    let cursor = *main_cursor;
    let vorbis_end = cursor + 4 + u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
    let vorbis_block = &buf[cursor + 4..vorbis_end];
    let vendor_end = 4 + u32::from_le_bytes(vorbis_block[0..4].try_into().unwrap()) as usize;
    let vendor_string = String::from_utf8_lossy(&vorbis_block[4..vendor_end]);
    let comment_list_len =
        u32::from_le_bytes(vorbis_block[vendor_end..vendor_end + 4].try_into().unwrap());
    let mut comment_cursor = vendor_end + 4;
    let mut comment_total = String::new();
    for _ in 1..=comment_list_len {
        let comment_len = u32::from_le_bytes(
            vorbis_block[comment_cursor..4 + comment_cursor]
                .try_into()
                .unwrap(),
        ) as usize;
        comment_cursor += 4;
        let comment =
            String::from_utf8_lossy(&vorbis_block[comment_cursor..comment_cursor + comment_len]);
        comment_total.push_str(comment.as_ref());
        comment_cursor += comment_len;
    }
    /*
         7) [framing_bit] = read a single bit as boolean
         8) if ( [framing_bit] unset or end of packet ) then ERROR
         9) done.
    USE CASE FOR READING FRAMING BIT????
    */
    let framing_bit = vorbis_block[comment_cursor - 1] & 0x00000001;
    *main_cursor = vorbis_end;
    comment_total.to_string()
}

fn read_u32(cursor: &mut usize, buf: &[u8]) -> anyhow::Result<u32> {
    let bytes = buf
        .get(*cursor..*cursor + 4)
        .ok_or(anyhow!("Buffer too small"))?;
    *cursor += 4;
    Ok(u32::from_be_bytes(
        bytes.try_into().map_err(|_| anyhow!("Invalid slice"))?,
    ))
}

fn parse_picture(cursor: &mut usize, buf: &[u8]) -> anyhow::Result<Picture> {
    let picture_type = read_u32(cursor, buf)?;
    let mime_len = read_u32(cursor, buf)? as usize;
    let mime = String::from_utf8_lossy(
        buf.get(*cursor..*cursor + mime_len)
            .ok_or(anyhow!("Buffer too small"))?,
    );
    *cursor += mime_len;
    let description_len = read_u32(cursor, buf)? as usize;
    let description = String::from_utf8_lossy(
        buf.get(*cursor..*cursor + description_len)
            .ok_or(anyhow!("Buffer too small"))?,
    );
    *cursor += description_len;
    let width = read_u32(cursor, buf)?;
    let height = read_u32(cursor, buf)?;
    let color_depth = read_u32(cursor, buf)?;
    let indexed_color_number = read_u32(cursor, buf)?;
    let picture_len = read_u32(cursor, buf)? as usize;
    let picture_data = buf
        .get(*cursor..*cursor + picture_len)
        .ok_or(anyhow!("Buffer too small"))?
        .to_vec();

    *cursor += picture_len;
    Ok(Picture {
        picture_type,
        mime: mime.to_string(),
        description: description.to_string(),
        width,
        height,
        color_depth,
        indexed_color_number,
        picture_data,
    })
}

pub enum ParseError {
    EndOfBufer,
}

async fn read_ahead_next_header(file: &File, size: usize, offset: u64) -> anyhow::Result<Vec<u8>> {
    let buf = vec![0; size + 8196];
    let (_res, prefix_buf) = file.read_at(buf, offset).await;
    let bytes_read = _res?;
    if bytes_read < size + 4 {
        return Err(anyhow!("Not enough bytes for next header"));
    }
    Ok(prefix_buf)
}

async fn read_with_uring(
    path: &Path,
    queue: Arc<tokio::sync::Mutex<TaskQueue>>,
) -> anyhow::Result<String> {
    let file = File::open(path).await?;
    let mut comment = "".to_string();

    let buf = vec![0; 8196];
    let (_res, mut prefix_buf) = file.read_at(buf, 0).await;
    let bytes_read = _res?;

    if prefix_buf[0..4] == FLAC_MARKER {
        if bytes_read < 42 {
            return Err(anyhow!(
                "Not enough bytes for proper flac STREAMINFO, got {}",
                bytes_read
            ));
        }

        let mut cursor = 4;
        loop {
            let header: Box<[u8]> = prefix_buf[cursor..cursor + 4].to_vec().into_boxed_slice();
            let block_length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
            if prefix_buf.len() < cursor + block_length + 4 {
                mem::drop(prefix_buf);
                prefix_buf =
                    read_ahead_next_header(&file, 0, (cursor + block_length) as u64).await?;
                cursor = 0;
            };

            match header[0] {
                VORBIS_COMMENT_MARKER::MARKER => {
                    comment = parse_vorbis(&mut cursor, &prefix_buf, &header);
                }
                VORBIS_COMMENT_MARKER::END_OF_BLOCK => {
                    comment = parse_vorbis(&mut cursor, &prefix_buf, &header);
                    break;
                }
                PICTURE_MARKER::MARKER => {
                    cursor += 4;
                    //let picture = parse_picture(&mut cursor, &prefix_buf)?;
                }
                PICTURE_MARKER::END_OF_BLOCK => {
                    cursor += 4;
                    break;
                    let picture = parse_picture(&mut cursor, &prefix_buf)?;
                    break;
                }
                // end marker
                n if n >= 128 => {
                    break;
                }
                _ => {
                    cursor += block_length;
                    cursor += 4;
                }
            }
        }
    }
    mem::drop(prefix_buf);
    queue.lock().await.push(comment).await;
    Ok("ok".to_string())
}

struct TaskQueue {
    pool: Pool<Sqlite>,
    queue: Vec<String>,
    sender: Sender<Vec<String>>,
}
impl TaskQueue {
    pub fn new(pool: Pool<Sqlite>) -> Self {
        let (sender, mut receiver) = mpsc::channel::<Vec<String>>(100);
        tokio::spawn(async move {
            let pool = SqlitePool::connect("sqlite://music.db").await.unwrap();
            while let Some(queue) = receiver.next().await {
                TaskQueue::insert(queue, &pool).await;
            }
        });
        TaskQueue {
            queue: Vec::new(),
            sender,
            pool,
        }
    }
    pub async fn insert(queue: Vec<String>, pool: &Pool<Sqlite>) {
        let mut transaction = pool.begin().await.unwrap();
        for item in queue {
            sqlx::query("INSERT INTO music(comment) VALUES(?);")
                .bind(item)
                .execute(&mut *transaction)
                .await
                .unwrap();
        }
        transaction.commit().await.unwrap();
    }
    pub async fn push(&mut self, item: String) {
        self.queue.push(item);
        if self.queue.len() >= QUEUE_LIMIT {
            let x = self.sender.send(self.queue.clone()).await;
            println!("{x:?}");
            self.queue.clear();
            self.queue.shrink_to_fit();
        }
    }
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
        if !Sqlite::database_exists(&url).await.unwrap_or(false) {
            Sqlite::create_database(&url).await.unwrap();
        }

        let pool = SqlitePool::connect(url).await.unwrap();
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS music (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    comment TEXT NOT NULL
);
",
        )
        .execute(&pool)
        .await
        .unwrap();
        let queue = Arc::new(tokio::sync::Mutex::new(TaskQueue::new(pool)));
        for entry in paths.lock().into_iter() {
            entry.clone().into_iter().for_each(|path| {
                let queue = Arc::clone(&queue);
                let spawn =
                    tokio_uring::spawn(async move { read_with_uring(&path, queue).await.unwrap() });

                tasks.push(spawn);
            });
        }
        for task in tasks.into_iter().rev() {
            let _ = task.await.unwrap();
        }
    });

    Ok(())
}
