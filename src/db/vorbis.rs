use sqlx::{prelude::FromRow, Acquire, Executor, Sqlite};

pub const FLAC_MARKER: [u8; 4] = [0x66, 0x4C, 0x61, 0x43];
// Used for checking if 4 byte list length is present in vorbis.
// 0x20 is space ' ' symbol. Smallest utf-8 printable one
pub const SMALLEST_VORBIS_4BYTE_POSSIBLE: u32 = u32::from_le_bytes([0x20, 0x20, 0x20, 0x20]);

pub const VORBIS_FIELDS_LOWER: [&str; 15] = [
    "title",
    "version",
    "album",
    "tracknumber",
    "artist",
    "performer",
    "copyright",
    "license",
    "organization",
    "description",
    "genre",
    "date",
    "location",
    "contact",
    "isrc",
];

#[derive(Debug, Clone, FromRow)]
pub struct VorbisMeta {
    pub id: Option<i64>,
    pub file_id: Option<i64>,
    pub file_ptr: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct VorbisComment {
    pub id: Option<i64>,
    pub meta_id: Option<i64>,
    pub key: String,
    pub file_ptr: i64,
    pub last_ogg_header_ptr: Option<i64>,
    pub size: i64,
    pub value: Option<String>,
}

impl VorbisComment {
    pub async fn from_meta_id<'a, E>(meta_id: i64, pool: E) -> Result<Vec<Self>, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        sqlx::query_as!(
            Self,
            "SELECT * FROM vorbis_comments WHERE meta_id = ?",
            meta_id
        )
        .fetch_all(pool)
        .await
    }
    pub async fn insert_many<'a, E>(
        meta_id: i64,
        comments: Vec<Self>,
        pool: E,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        let mut query = "INSERT INTO vorbis_comments(
                meta_id,
                key,
                value,
                file_ptr,
                last_ogg_header_ptr,
                size) VALUES"
            .to_owned();

        for i in 0..comments.len() {
            if i > 0 {
                query.push(',');
            }
            query.push_str("(?, ?, ?, ?, ?, ?)");
        }
        query.push(';');

        let mut query: sqlx::query::Query<'_, Sqlite, _> = sqlx::query(&query);
        for c in comments {
            query = query
                .bind(meta_id)
                .bind(c.key)
                .bind(c.value)
                .bind(c.file_ptr)
                .bind(c.last_ogg_header_ptr)
                .bind(c.size);
        }
        query.execute(pool).await?;
        Ok(())
    }
    /// vorbis comment string into key,val pair
    pub fn into_key_val(comment: &[u8]) -> Option<(String, String)> {
        comment.iter().position(|&b| b == b'=').map(|index| {
            (
                String::from_utf8_lossy(&comment[..index]).to_lowercase(),
                String::from_utf8_lossy(&comment[index + 1..]).to_lowercase(),
            )
        })
    }

    pub async fn parse_block(vorbis_block: &[u8], block_ptr: i64) -> anyhow::Result<Vec<Self>> {
        let mut comments = Vec::new();
        let block_length = vorbis_block.len();

        let vendor_len = u32::from_le_bytes(vorbis_block[0..4].try_into()?) as usize;
        comments.push(Self {
            key: "vendor".to_owned(),
            meta_id: None,
            file_ptr: block_ptr,
            size: vendor_len as i64 + 4,
            last_ogg_header_ptr: None,
            value: Some(String::from_utf8_lossy(&vorbis_block[4..vendor_len + 4]).to_string()),
            id: None,
        });
        let mut comment_cursor = vendor_len + 4;
        let comment_amount: usize =
            u32::from_le_bytes(vorbis_block[comment_cursor..comment_cursor + 4].try_into()?)
                as usize;
        let mut comment_len =
            u32::from_le_bytes(vorbis_block[comment_cursor + 4..comment_cursor + 8].try_into()?)
                as usize;

        comment_cursor += 8;
        while comment_cursor + comment_len <= block_length {
            if let Some((key, val)) =
                Self::into_key_val(&vorbis_block[comment_cursor..comment_cursor + comment_len])
            {
                comments.push(Self {
                    id: None,
                    meta_id: None,
                    value: Some(val),
                    size: comment_len as i64 + 4,
                    last_ogg_header_ptr: None,
                    key,
                    file_ptr: block_ptr as i64 + comment_cursor as i64 - 4,
                })
            } else {
                println!(
                    "corrupted comment {:?}",
                    String::from_utf8_lossy(
                        &vorbis_block[comment_cursor..comment_cursor + comment_len]
                    )
                );
                //return Err(anyhow!("Corrupted comment: {comment}"));
                // skip the corrupted comments for now
            }

            comment_cursor += comment_len + 4;

            if comment_cursor >= block_length {
                break;
            }
            comment_len =
                u32::from_le_bytes(vorbis_block[comment_cursor - 4..comment_cursor].try_into()?)
                    as usize;
        }

        assert_eq!(comments.len(), comment_amount);
        Ok(comments)
    }
}

impl VorbisMeta {
    pub async fn from_file_id<'a, E>(file_id: i64, pool: E) -> Result<Vec<Self>, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        sqlx::query_as!(Self, "SELECT * FROM vorbis_meta WHERE file_id = ?", file_id)
            .fetch_all(pool)
            .await
    }

    pub async fn insert<'a, E>(&self, pool: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        let id = sqlx::query!(
            "INSERT INTO vorbis_meta(file_id, file_ptr) VALUES (?, ?)",
            self.file_id,
            self.file_ptr
        )
        .execute(pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }
}
