use crate::db::vorbis::VorbisBlob;
use crate::db::{audio_file::AudioFileMeta, vorbis::VorbisComment};
use futures::channel::{mpsc, mpsc::Sender};
use futures::{SinkExt, StreamExt};
use sqlx::{Pool, Sqlite, SqlitePool};
use std::mem;
use tokio::task::JoinHandle;

const QUEUE_LIMIT: usize = 25;

#[derive(Debug)]
pub struct TaskQueue {
    queue: Vec<AudioFileMeta>,
    executor: JoinHandle<()>,
    sender: Sender<Option<Vec<AudioFileMeta>>>,
}

impl TaskQueue {
    pub async fn new() -> Result<Self, sqlx::Error> {
        let (sender, mut receiver) = mpsc::channel::<Option<Vec<AudioFileMeta>>>(100);
        let pool = SqlitePool::connect("sqlite://dev.db").await?;
        let executor = tokio::spawn(async move {
            while let Some(queue) = receiver.next().await {
                match queue {
                    Some(queue) => {
                        if let Err(e) = TaskQueue::insert(queue, &pool).await {
                            // Log errors somwhere here
                            println!("Temporary log: {e:?}");
                        }
                    }
                    None => break,
                }
            }
        });
        Ok(TaskQueue {
            queue: Vec::with_capacity(QUEUE_LIMIT),
            executor,
            sender,
        })
    }

    pub async fn finish(self) {
        let mut sender = self.sender;
        if !self.queue.is_empty() {
            let _ = sender.send(Some(self.queue)).await;
        }
        let _ = sender.send(None).await;
        let _ = self.executor.await;
    }
    pub async fn insert(queue: Vec<AudioFileMeta>, pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
        let mut transaction = pool.begin().await?;
        for item in queue {
            let file_id = item.audio_file.insert(&mut *transaction).await?;
            for blob in item.blobs {
                if VorbisBlob::hash_exists(blob.hash.clone(), &mut *transaction).await? {
                    continue;
                }
                blob.insert(&mut *transaction).await?;
            }
            for (mut vorbis_meta, vorbis) in item.comments {
                vorbis_meta.file_id = Some(file_id);
                let meta_id = vorbis_meta.insert(&mut *transaction).await?;
                if vorbis.is_empty() {
                    continue;
                }
                VorbisComment::insert_many(meta_id, vorbis, &mut *transaction).await?;
            }
            for picture in item.pictures {
                picture.insert(file_id, &mut *transaction).await?;
            }
            for padding in item.paddings {
                padding.insert(file_id, &mut *transaction).await?;
            }
        }
        transaction.commit().await
    }
    pub async fn push(&mut self, item: AudioFileMeta) {
        self.queue.push(item);
        if self.queue.len() >= QUEUE_LIMIT {
            let _ = self.sender.send(Some(mem::take(&mut self.queue))).await;
        }
    }
}
