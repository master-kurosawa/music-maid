use sqlx::{prelude::FromRow, Executor, Sqlite};

#[derive(Debug, Clone, FromRow)]
pub struct Picture {
    pub id: Option<i64>,
    pub file_id: Option<i64>,
    pub file_ptr: i64,
    pub picture_type: i64,
    pub mime: String,
    pub description: String,
    pub width: i64,
    pub height: i64,
    pub color_depth: i64,
    pub indexed_color_number: i64,
    pub size: i64,
}

impl Picture {
    pub async fn from_file_id<'a, E>(file_id: i64, pool: E) -> Result<Vec<Self>, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        sqlx::query_as!(
            Self,
            "SELECT * FROM picture_metadata WHERE file_id = ?",
            file_id
        )
        .fetch_all(pool)
        .await
    }

    pub fn from_picture_block(picture: &[u8], file_ptr: i64) -> Self {
        let mut cursor = 0;
        let get_u32 =
            |bytes: &[u8]| -> i64 { u32::from_be_bytes(bytes.try_into().unwrap()) as i64 };

        let picture_type = get_u32(&picture[cursor..cursor + 4]);
        cursor += 4;

        let mime_len = get_u32(&picture[cursor..cursor + 4]) as usize;
        cursor += 4;
        let mime_bytes = &picture[cursor..mime_len + cursor];
        let mime = String::from_utf8_lossy(mime_bytes).to_string();
        cursor += mime_len;

        let description_len = get_u32(&picture[cursor..cursor + 4]) as usize;
        cursor += 4;
        let description_bytes = &picture[cursor..description_len + cursor];
        let description = String::from_utf8_lossy(description_bytes).to_string();
        cursor += description_len;

        let width = get_u32(&picture[cursor..cursor + 4]);
        cursor += 4;
        let height = get_u32(&picture[cursor..cursor + 4]);
        cursor += 4;
        let color_depth = get_u32(&picture[cursor..cursor + 4]);
        cursor += 4;
        let indexed_color_number = get_u32(&picture[cursor..cursor + 4]);
        cursor += 4;
        let picture_len = get_u32(&picture[cursor..cursor + 4]);

        Picture {
            id: None,
            file_id: None,
            file_ptr,
            picture_type,
            size: picture_len,
            mime,
            description,
            width,
            height,
            color_depth,
            indexed_color_number,
        }
    }

    pub async fn insert<'a, E>(&self, file_id: i64, pool: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        Ok(sqlx::query!(
            "INSERT INTO picture_metadata(
                file_id,
                file_ptr,
                picture_type,
                mime,
                description,
                width,
                height,
                color_depth,
                indexed_color_number,
                size)
            VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?);",
            file_id,
            self.file_ptr,
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
