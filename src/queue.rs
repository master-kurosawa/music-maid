use futures::{SinkExt, StreamExt};
use sqlx::{Pool, Sqlite, SqlitePool};
use tokio::task::JoinHandle;

use futures::channel::{mpsc, mpsc::Sender};

use crate::db::{audio_file::AudioFileMeta, vorbis::VorbisComment};

const QUEUE_LIMIT: usize = 25;

#[derive(Debug)]
pub struct TaskQueue {
    queue: Vec<AudioFileMeta>,
    executor: JoinHandle<()>,
    sender: Sender<Option<Vec<AudioFileMeta>>>,
}
impl Default for TaskQueue {
    fn default() -> Self {
        TaskQueue::new()
    }
}
impl TaskQueue {
    pub fn new() -> Self {
        let (sender, mut receiver) = mpsc::channel::<Option<Vec<AudioFileMeta>>>(100);
        let executor = tokio::spawn(async move {
            let pool = SqlitePool::connect("sqlite://dev.db").await.unwrap();
            while let Some(queue) = receiver.next().await {
                match queue {
                    Some(queue) => TaskQueue::insert(queue, &pool).await,
                    None => break,
                }
            }
        });
        TaskQueue {
            queue: Vec::with_capacity(QUEUE_LIMIT),
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
    pub async fn insert(queue: Vec<AudioFileMeta>, pool: &Pool<Sqlite>) {
        let mut transaction = pool.begin().await.unwrap();
        for item in queue {
            let file_id = item.audio_file.insert(&mut *transaction).await.unwrap();
            for (mut vorbis_meta, vorbis) in item.comments {
                vorbis_meta.file_id = Some(file_id);
                let meta_id = vorbis_meta.insert(&mut *transaction).await.unwrap();
                if vorbis.is_empty() {
                    continue;
                }
                VorbisComment::insert_many(meta_id, vorbis, &mut *transaction)
                    .await
                    .unwrap();
            }
            for picture in item.pictures {
                picture.insert(file_id, &mut *transaction).await.unwrap();
            }
            for padding in item.paddings {
                padding.insert(file_id, &mut *transaction).await.unwrap();
            }
        }
        transaction.commit().await.unwrap();
    }
    pub async fn push(&mut self, item: AudioFileMeta) {
        self.queue.push(item);
        if self.queue.len() >= QUEUE_LIMIT {
            let _ = self.sender.send(Some(self.queue.clone())).await;
            self.queue.clear();
        }
    }
}
