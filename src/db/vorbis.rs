use sqlx::{prelude::FromRow, Executor, Sqlite};

use crate::io::{ogg::OggPageReader, reader::Corruption};

pub const FLAC_MARKER: [u8; 4] = [0x66, 0x4C, 0x61, 0x43];
/// #TODO incomplete list
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
    pub end_ptr: i64,
    pub comment_amount_ptr: i64,
    pub vendor: String,
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
    pub async fn into_bytes_ogg<'a>(
        self,
        reader: &mut OggPageReader<'a>,
    ) -> Result<Vec<u8>, Corruption> {
        if let Some(val) = self.value {
            let mut comment = Vec::with_capacity(self.size as usize);
            comment.extend((self.size as u32 - 4).to_le_bytes());
            comment.extend(self.key.into_bytes());
            comment.push(b'=');
            comment.extend(val.into_bytes());
            Ok(comment)
        } else {
            reader
                .reader
                .read_at_offset(
                    self.size as usize + 8196,
                    self.last_ogg_header_ptr.unwrap() as u64,
                )
                .await?;
            reader.cursor = reader.segment_size;
            reader.parse_header().await?;
            reader
                .safe_skip(
                    (self.file_ptr as u64 - reader.reader.file_ptr - reader.reader.cursor) as usize,
                )
                .await?;
            return reader.get_bytes(self.size as usize).await;
        }
    }
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
    pub async fn from_meta_exclude_desc<'a, E>(
        meta_id: i64,
        exclude: Vec<i64>,
        pool: E,
    ) -> Result<Vec<Self>, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        let query = format!(
            "SELECT * FROM vorbis_comments WHERE meta_id = ? AND id NOT IN ({}) ORDER BY size;",
            "?,".repeat(exclude.len() - 1) + "?"
        );
        let mut query = sqlx::query_as::<Sqlite, Self>(&query).bind(meta_id);
        for id in exclude {
            query = query.bind(id);
        }
        query.fetch_all(pool).await
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

    pub async fn parse_block(
        vorbis_block: Vec<u8>,
        block_ptr: i64,
    ) -> Result<(VorbisMeta, Vec<Self>), Corruption> {
        let mut comments = Vec::new();
        let block_length = vorbis_block.len();

        let vendor_len =
            u32::from_le_bytes(vorbis_block[0..4].try_into().map_err(|_| Corruption {
                path: "".into(),
                file_cursor: block_ptr as u64,
                message: "Corrupted VorbisBlock".to_owned(),
            })?) as usize;
        let vendor = String::from_utf8_lossy(&vorbis_block[4..vendor_len + 4]).to_string();
        let mut comment_cursor = vendor_len + 4;
        let comment_amount_ptr = comment_cursor as i64 + block_ptr;
        let comment_amount: usize = u32::from_le_bytes(
            vorbis_block[comment_cursor..comment_cursor + 4]
                .try_into()
                .map_err(|_| Corruption {
                    path: "".into(),
                    file_cursor: block_ptr as u64,
                    message: "Corrupted VorbisBlock".to_owned(),
                })?,
        ) as usize;
        let mut comment_len = u32::from_le_bytes(
            vorbis_block[comment_cursor + 4..comment_cursor + 8]
                .try_into()
                .map_err(|_| Corruption {
                    path: "".into(),
                    file_cursor: block_ptr as u64,
                    message: "Corrupted VorbisBlock".to_owned(),
                })?,
        ) as usize;

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
                    file_ptr: block_ptr + comment_cursor as i64 - 4,
                })
            } else {
                println!(
                    "corrupted comment {:?}",
                    String::from_utf8_lossy(
                        &vorbis_block[comment_cursor..comment_cursor + comment_len]
                    )
                );
            }

            comment_cursor += comment_len + 4;

            if comment_cursor >= block_length {
                break;
            }
            comment_len = u32::from_le_bytes(
                vorbis_block[comment_cursor - 4..comment_cursor]
                    .try_into()
                    .map_err(|_| Corruption {
                        path: "".into(),
                        file_cursor: block_ptr as u64,
                        message: "Corrupted VorbisBlock".to_owned(),
                    })?,
            ) as usize;
        }

        if comments.len() != comment_amount {
            return Err(Corruption {
                file_cursor: block_ptr as u64,
                path: "".into(),
                message: "Comment amount does not match vorbis comment list length".to_owned(),
            });
        }
        let vorbis_meta = VorbisMeta {
            id: None,
            file_ptr: block_ptr,
            end_ptr: block_ptr + block_length as i64,
            comment_amount_ptr,
            file_id: None,
            vendor,
        };
        Ok((vorbis_meta, comments))
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
            "INSERT INTO vorbis_meta(file_id, file_ptr, end_ptr, vendor, comment_amount_ptr) VALUES (?, ?, ?, ?, ?)",
            self.file_id,
            self.file_ptr,
            self.end_ptr,
            self.vendor,
            self.comment_amount_ptr
        )
        .execute(pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }
}
