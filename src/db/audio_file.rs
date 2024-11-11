use sqlx::{prelude::FromRow, Executor, Sqlite};

use super::{
    padding::Padding,
    picture::Picture,
    vorbis::{VorbisComment, VorbisMeta},
};

#[derive(Debug, Clone)]
pub struct AudioFileMeta {
    pub audio_file: AudioFile,
    pub comments: Vec<(VorbisMeta, Vec<VorbisComment>)>,
    pub pictures: Vec<Picture>,
    pub paddings: Vec<Padding>,
}

#[derive(Debug, Clone, FromRow)]
pub struct AudioFile {
    pub id: Option<i64>,
    pub path: String,
    pub name: String,
    pub format: Option<String>,
}

impl AudioFile {
    pub async fn from_path<'a, E>(path: String, pool: E) -> Result<Self, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        sqlx::query_as!(AudioFile, "SELECT * FROM files WHERE path = ?", path)
            .fetch_one(pool)
            .await
    }

    pub async fn fetch_meta<'a, E>(self, pool: E) -> Result<AudioFileMeta, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite> + std::marker::Copy,
    {
        let id = self.id.unwrap();
        let metas = VorbisMeta::from_file_id(id, pool).await?;
        let mut comments = Vec::with_capacity(metas.len());
        for meta in metas {
            let meta_id = meta.id.unwrap();
            let vorbis_comments = VorbisComment::from_meta_id(meta_id, pool).await?;
            comments.push((meta, vorbis_comments));
        }
        let pictures = Picture::from_file_id(id, pool).await?;
        let paddings = Padding::from_file_id(id, pool).await?;

        Ok(AudioFileMeta {
            audio_file: self,
            pictures,
            paddings,
            comments,
        })
    }

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
