use sqlx::{prelude::FromRow, Executor, Sqlite};

#[derive(Debug, Clone, FromRow)]
pub struct Picture {
    pub file_id: Option<i64>,
    pub picture_type: u32,
    pub mime: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub color_depth: u32,
    pub indexed_color_number: u32,
    pub size: u32,
    // picture_data: Vec<u8>,
}

impl Picture {
    pub async fn insert<'a, E>(&self, file_id: i64, pool: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        Ok(sqlx::query!(
            "INSERT INTO picture_metadata(
                file_id,
                picture_type,
                mime,
                description,
                width,
                height,
                color_depth,
                indexed_color_number,
                size)
            VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?);",
            file_id,
            self.picture_type,
            self.mime,
            self.description,
            self.width,
            self.height,
            self.color_depth,
            self.indexed_color_number,
            self.size
        )
        .execute(pool)
        .await?
        .last_insert_rowid())
    }
}
