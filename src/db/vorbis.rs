use std::collections::HashMap;

use sqlx::{prelude::FromRow, Executor, Sqlite};

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
pub struct VorbisComment {
    pub vendor: String,
    pub title: String,
    pub version: String,
    pub album: String,
    pub tracknumber: String,
    pub artist: String,
    pub performer: String,
    pub copyright: String,
    pub license: String,
    pub organization: String,
    pub description: String,
    pub genre: String,
    pub date: String,
    pub location: String,
    pub contact: String,
    pub isrc: String,
    pub outcast: String,
}

impl VorbisComment {
    pub fn init(map: HashMap<String, String>, outcasts: Vec<String>) -> Self {
        let outcast = outcasts.join("|||");

        let get_value = |key: &str| map.get(key).unwrap_or(&String::new()).clone();

        VorbisComment {
            vendor: get_value("vendor"),
            title: get_value("title"),
            version: get_value("version"),
            album: get_value("album"),
            tracknumber: get_value("tracknumber"),
            artist: get_value("artist"),
            performer: get_value("performer"),
            copyright: get_value("copyright"),
            license: get_value("license"),
            organization: get_value("organization"),
            description: get_value("description"),
            genre: get_value("genre"),
            date: get_value("date"),
            location: get_value("location"),
            contact: get_value("contact"),
            isrc: get_value("isrc"),
            outcast,
        }
    }
    pub async fn insert<'a, E>(&self, file_id: i64, pool: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'a, Database = Sqlite>,
    {
        Ok(sqlx::query!(
            "INSERT INTO vorbis_comments(
                file_id,
                vendor,
                title,
                version,
                album,
                tracknumber,
                artist,
                performer,
                copyright,
                license,
                organization,
                description,
                genre,
                date,
                location,
                contact,
                isrc,
                outcast)
            VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?);",
            file_id,
            self.vendor,
            self.title,
            self.version,
            self.album,
            self.tracknumber,
            self.artist,
            self.performer,
            self.copyright,
            self.license,
            self.organization,
            self.description,
            self.genre,
            self.date,
            self.location,
            self.contact,
            self.isrc,
            self.outcast,
        )
        .execute(pool)
        .await?
        .last_insert_rowid())
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

    pub async fn parse_block(vorbis_block: &[u8]) -> anyhow::Result<Self> {
        let mut comments = HashMap::new();
        let mut outcasts = Vec::new();
        let block_length = vorbis_block.len();

        let vendor_len = u32::from_le_bytes(vorbis_block[0..4].try_into()?) as usize;
        comments.insert(
            "vendor".to_string(),
            String::from_utf8_lossy(&vorbis_block[4..vendor_len + 4]).to_string(),
        );
        let mut comment_cursor = vendor_len + 4;
        let comment_amount: usize =
            u32::from_le_bytes(vorbis_block[comment_cursor..comment_cursor + 4].try_into()?)
                as usize;
        let mut comment_len =
            u32::from_le_bytes(vorbis_block[comment_cursor + 4..comment_cursor + 8].try_into()?)
                as usize;

        comment_cursor += 8;
        if comment_len >= SMALLEST_VORBIS_4BYTE_POSSIBLE as usize {
            comment_len = comment_amount;
            comment_cursor -= 4;
        }
        while comment_cursor + comment_len <= block_length {
            if let Some((key, val)) =
                Self::into_key_val(&vorbis_block[comment_cursor..comment_cursor + comment_len])
            {
                if VORBIS_FIELDS_LOWER.contains(&key.as_str()) {
                    comments.insert(key, val);
                } else {
                    outcasts.push(format!("{key}={val}"));
                }
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

        Ok(VorbisComment::init(comments, outcasts))
    }
}
