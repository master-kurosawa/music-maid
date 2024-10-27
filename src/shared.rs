use std::collections::HashMap;

use anyhow::anyhow;

use crate::reader::UringBufReader;
//use anyhow::anyhow;

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

pub const FLAC_MARKER: [u8; 4] = [0x66, 0x4C, 0x61, 0x43];
pub const OGG_MARKER: [u8; 4] = [0x4F, 0x67, 0x67, 0x53];

#[derive(Debug, Clone)]
pub struct MusicFile {
    pub path: String,
    pub comments: Vec<VorbisComment>,
    pub pictures: Vec<Picture>,
}

#[derive(Debug, Clone)]
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
        let vendor = map.get("vendor").map_or(String::new(), |v| v.to_string());
        let contact = map.get("contact").map_or(String::new(), |v| v.to_string());
        let location = map.get("location").map_or(String::new(), |v| v.to_string());
        let date = map.get("date").map_or(String::new(), |v| v.to_string());
        let genre = map.get("genre").map_or(String::new(), |v| v.to_string());
        let isrc = map.get("isrc").map_or(String::new(), |v| v.to_string());
        let album = map.get("album").map_or(String::new(), |v| v.to_string());
        let version = map.get("version").map_or(String::new(), |v| v.to_string());
        let title = map.get("title").map_or(String::new(), |v| v.to_string());
        let description = map
            .get("description")
            .map_or(String::new(), |v| v.to_string());
        let organization = map
            .get("organization")
            .map_or(String::new(), |v| v.to_string());
        let license = map.get("license").map_or(String::new(), |v| v.to_string());
        let copyright = map
            .get("copyright")
            .map_or(String::new(), |v| v.to_string());
        let performer = map
            .get("performer")
            .map_or(String::new(), |v| v.to_string());
        let artist = map.get("artist").map_or(String::new(), |v| v.to_string());
        let tracknumber = map
            .get("tracknumber")
            .map_or(String::new(), |v| v.to_string());

        VorbisComment {
            title,
            vendor,
            description,
            version,
            album,
            date,
            isrc,
            genre,
            artist,
            license,
            contact,
            location,
            performer,
            copyright,
            tracknumber,
            organization,
            outcast,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Picture {
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

pub async fn parse_vorbis(vorbis_block: &[u8]) -> anyhow::Result<VorbisComment> {
    let mut comments = HashMap::new();
    let mut outcasts = Vec::new();
    let block_length = vorbis_block.len();
    let vendor_end = 4 + u32::from_le_bytes(vorbis_block[0..4].try_into()?) as usize;
    comments.insert(
        "vendor".to_string(),
        String::from_utf8_lossy(&vorbis_block[4..vendor_end]).to_string(),
    );
    let comment_list_len =
        u32::from_le_bytes(vorbis_block[vendor_end..vendor_end + 4].try_into()?) as usize;
    let first_comment_len =
        u32::from_le_bytes(vorbis_block[vendor_end + 4..vendor_end + 8].try_into()?) as usize;
    if comment_list_len > block_length {
        return Err(anyhow!("Comment list len > block length"));
    } else if first_comment_len > block_length {
        let mut comment_cursor = vendor_end;
        while comment_cursor < block_length {
            let comment_len =
                u32::from_le_bytes(vorbis_block[comment_cursor..comment_cursor + 4].try_into()?)
                    as usize;
            comment_cursor += 4;
            if comment_cursor + comment_len >= block_length {
                break;
            }
            let comment = String::from_utf8_lossy(
                &vorbis_block[comment_cursor..comment_cursor + comment_len],
            )
            .to_lowercase();
            match &comment.split_once('=') {
                Some((key, val)) => {
                    if VORBIS_FIELDS_LOWER.contains(key) {
                        comments.insert(key.to_lowercase(), val.to_string());
                        comment_cursor += comment_len;
                    } else {
                        outcasts.push(comment);
                        comment_cursor += comment_len;
                        continue;
                    }
                }
                None => {
                    println!("corrupted comment {comment:?}");
                    continue;
                    //return Err(anyhow!("Corrupted comment: {comment}"));
                    // skip the corrupted comments for now
                }
            };
        }
    } else {
        let mut comment_cursor = vendor_end + 4;
        for _ in 1..=comment_list_len {
            if comment_cursor + 4 >= block_length {
                break;
            }
            let comment_len =
                u32::from_le_bytes(vorbis_block[comment_cursor..4 + comment_cursor].try_into()?)
                    as usize;

            //let z = String::from_utf8_lossy(&vorbis_block[comment_cursor..]);
            //println!("{z:?}");
            if comment_len + comment_cursor >= block_length {
                //println!("{comment_len} + {comment_cursor} > {block_length}");
                continue;
                // skip any corrupted comment lengths
                //return Err(anyhow!(
                //    "Corrupted comment length: {comment_len} > {block_length}"
                //));
            }
            comment_cursor += 4;
            let comment = String::from_utf8_lossy(
                &vorbis_block[comment_cursor..comment_cursor + comment_len],
            )
            .to_lowercase();
            match &comment.split_once('=') {
                Some((key, val)) => {
                    if VORBIS_FIELDS_LOWER.contains(key) {
                        comments.insert(key.to_lowercase(), val.to_string());
                    } else {
                        outcasts.push(comment);
                        comment_cursor += comment_len;
                        continue;
                    }
                }
                None => {
                    println!("corrupted comment {comment:?}");
                    continue;
                    //return Err(anyhow!("Corrupted comment: {comment}"));
                    // skip the corrupted comments for now
                }
            };

            comment_cursor += comment_len;
        }
    }
    Ok(VorbisComment::init(comments, outcasts))
}
