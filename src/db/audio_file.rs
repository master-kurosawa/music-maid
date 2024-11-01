use sqlx::{Executor, Sqlite};

use super::{picture::Picture, vorbis::VorbisComment};

#[derive(Debug, Clone)]
pub struct AudioFile {
    pub path: String,
    pub name: String,
    pub format: Option<String>,
    pub comments: Vec<VorbisComment>,
    pub pictures: Vec<Picture>,
}

impl AudioFile {
    pub async fn insert<'a, E>(&self, pool: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        Ok(sqlx::query!(
            "INSERT INTO files(path, name, format) VALUES(?, ?, ?);",
            self.path,
            self.name,
            self.format
        )
        .execute(pool)
        .await?
        .last_insert_rowid())
    }
}
