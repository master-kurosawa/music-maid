use crate::shared::MusicFile;

use anyhow::anyhow;
use futures::{SinkExt, StreamExt};
use sqlx::{Pool, Sqlite, SqlitePool};
use tokio::task::JoinHandle;
use tokio_uring::fs::File;

use futures::channel::{mpsc, mpsc::Sender};
const QUEUE_LIMIT: usize = 50;

#[derive(Debug)]
pub struct TaskQueue {
    queue: Vec<MusicFile>,
    executor: JoinHandle<()>,
    sender: Sender<Option<Vec<MusicFile>>>,
}
impl Default for TaskQueue {
    fn default() -> Self {
        TaskQueue::new()
    }
}
impl TaskQueue {
    pub fn new() -> Self {
        let (sender, mut receiver) = mpsc::channel::<Option<Vec<MusicFile>>>(100);
        let executor = tokio::spawn(async move {
            let pool = SqlitePool::connect("sqlite://music.db").await.unwrap();
            while let Some(queue) = receiver.next().await {
                match queue {
                    Some(queue) => TaskQueue::insert(queue, &pool).await,
                    None => break,
                }
            }
        });
        TaskQueue {
            queue: Vec::new(),
            executor,
            sender,
        }
    }
    pub async fn finish(self) {
        let _ = self.sender.to_owned().send(Some(self.queue.clone())).await;
        let _ = self.sender.to_owned().send(None).await;
        self.queue.to_owned().clear();
        self.queue.to_owned().shrink_to_fit();
        let _ = self.executor.await;
    }
    pub async fn insert(queue: Vec<MusicFile>, pool: &Pool<Sqlite>) {
        let mut transaction = pool.begin().await.unwrap();
        for item in queue {
            let file_id = sqlx::query("INSERT INTO files(path) VALUES(?);")
                .bind(&item.path)
                .execute(&mut *transaction)
                .await
                .unwrap()
                .last_insert_rowid();
            for comment in &item.comments {
                sqlx::query(
                    "
                    INSERT INTO 
                    vorbis_comments(
                        file_id,
                        vendor,
                        title,
                        version,
                        album,
                        tracknumber,
                        artist,
                        performer,
                        copyright,
                        license,
                        organization,
                        description,
                        genre,
                        date,
                        location,
                        contact,
                        isrc,
                        outcast)
                    VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?);",
                )
                .bind(file_id)
                .bind(&comment.vendor)
                .bind(&comment.title)
                .bind(&comment.version)
                .bind(&comment.album)
                .bind(&comment.tracknumber)
                .bind(&comment.artist)
                .bind(&comment.performer)
                .bind(&comment.copyright)
                .bind(&comment.license)
                .bind(&comment.organization)
                .bind(&comment.description)
                .bind(&comment.genre)
                .bind(&comment.date)
                .bind(&comment.location)
                .bind(&comment.contact)
                .bind(&comment.isrc)
                .bind(&comment.outcast)
                .execute(&mut *transaction)
                .await
                .unwrap();
            }
            for picture in &item.pictures {
                sqlx::query(
                    "
                    INSERT INTO 
                    picture_metadata(
                        file_id,
                        picture_type,
                        mime,
                        description,
                        width,
                        height,
                        color_depth,
                        indexed_color_number,
                        size
                        )
                    VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?);",
                )
                .bind(file_id)
                .bind(picture.picture_type)
                .bind(&picture.mime)
                .bind(&picture.description)
                .bind(picture.width)
                .bind(picture.height)
                .bind(picture.color_depth)
                .bind(picture.indexed_color_number)
                .bind(picture.size)
                .execute(&mut *transaction)
                .await
                .unwrap();
            }
        }
        transaction.commit().await.unwrap();
    }
    pub async fn push(&mut self, item: MusicFile) {
        self.queue.push(item);
        if self.queue.len() >= QUEUE_LIMIT {
            let _ = self.sender.send(Some(self.queue.clone())).await;
            self.queue.clear();
            self.queue.shrink_to_fit();
        }
    }
}

pub async fn read_ahead_offset(file: &File, size: usize, offset: u64) -> anyhow::Result<Vec<u8>> {
    let buf = vec![0; size + 8196];
    let (_res, prefix_buf) = file.read_at(buf, offset).await;
    let bytes_read = _res?;
    if bytes_read < size + 4 {
        return Err(anyhow!("Not enough bytes for next header"));
    }
    Ok(prefix_buf)
}

pub fn read_u32(cursor: &mut usize, buf: &[u8]) -> anyhow::Result<u32> {
    let bytes = buf
        .get(*cursor..*cursor + 4)
        .ok_or(anyhow!("Buffer too small"))?;
    *cursor += 4;
    Ok(u32::from_be_bytes(
        bytes.try_into().map_err(|_| anyhow!("Invalid slice"))?,
    ))
}
