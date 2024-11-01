use futures::{SinkExt, StreamExt};
use sqlx::{Pool, Sqlite, SqlitePool};
use tokio::task::JoinHandle;

use futures::channel::{mpsc, mpsc::Sender};

use crate::db::audio_file::AudioFile;

const QUEUE_LIMIT: usize = 50;

#[derive(Debug)]
pub struct TaskQueue {
    queue: Vec<AudioFile>,
    executor: JoinHandle<()>,
    sender: Sender<Option<Vec<AudioFile>>>,
}
impl Default for TaskQueue {
    fn default() -> Self {
        TaskQueue::new()
    }
}
impl TaskQueue {
    pub fn new() -> Self {
        let (sender, mut receiver) = mpsc::channel::<Option<Vec<AudioFile>>>(100);
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
    pub async fn insert(queue: Vec<AudioFile>, pool: &Pool<Sqlite>) {
        let mut transaction = pool.begin().await.unwrap();
        for item in queue {
            let file_id = item.insert(&mut *transaction).await.unwrap();
            for comment in item.comments {
                comment.insert(file_id, &mut *transaction).await.unwrap();
            }
            for picture in item.pictures {
                picture.insert(file_id, &mut *transaction).await.unwrap();
            }
        }
        transaction.commit().await.unwrap();
    }
    pub async fn push(&mut self, item: AudioFile) {
        self.queue.push(item);
        if self.queue.len() >= QUEUE_LIMIT {
            let _ = self.sender.send(Some(self.queue.clone())).await;
            self.queue.clear();
            self.queue.shrink_to_fit();
        }
    }
}
