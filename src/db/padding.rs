use sqlx::{prelude::FromRow, Executor, Sqlite};

#[derive(Debug, Clone, FromRow)]
pub struct Padding {
    pub file_id: Option<i64>,
    pub file_ptr: Option<i64>,
    pub byte_size: Option<i64>,
}

impl Padding {
    pub async fn insert<'a, E>(&self, file_id: i64, pool: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        Ok(sqlx::query!(
            "INSERT INTO padding(
                file_id,
                file_ptr,
                byte_size
                )
            VALUES(?, ?, ?);",
            file_id,
            self.file_ptr,
            self.byte_size
        )
        .execute(pool)
        .await?
        .last_insert_rowid())
    }
}
