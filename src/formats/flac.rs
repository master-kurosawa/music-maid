use anyhow::anyhow;
use std::mem;
use tokio_uring::fs::File;

use crate::{
    shared::{parse_vorbis, Picture, VorbisComment},
    utils::{read_ahead_offset, read_u32},
};

#[allow(non_camel_case_types)]
struct VORBIS_COMMENT_MARKER;
impl VORBIS_COMMENT_MARKER {
    const END_OF_BLOCK: u8 = 0b10000100;
    const MARKER: u8 = 0b00000100;
}

#[allow(non_camel_case_types)]
struct PICTURE_MARKER;
impl PICTURE_MARKER {
    const END_OF_BLOCK: u8 = 0b10000110;
    const MARKER: u8 = 0b00000110;
}

pub async fn parse_flac(
    buf: Vec<u8>,
    file: File,
    vorbis_comments: &mut Vec<VorbisComment>,
    pictures_metadata: &mut Vec<Picture>,
) -> anyhow::Result<()> {
    let mut cursor = 4;
    let mut buf = buf;

    loop {
        let header: Box<[u8]> = buf[cursor..cursor + 4].to_vec().into_boxed_slice();
        let block_length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
        let buf_len = buf.len();
        cursor += 4;

        match header[0] {
            VORBIS_COMMENT_MARKER::MARKER => {
                if buf_len <= cursor + block_length {
                    mem::drop(buf);
                    buf = read_ahead_offset(&file, block_length, cursor as u64).await?;
                    cursor = 0;
                }
                vorbis_comments.push(parse_vorbis(&cursor, &buf, block_length)?);
                cursor += block_length;
            }
            VORBIS_COMMENT_MARKER::END_OF_BLOCK => {
                if buf_len <= cursor + block_length {
                    mem::drop(buf);
                    buf = read_ahead_offset(&file, block_length - 8196, cursor as u64).await?;
                    cursor = 0;
                }

                vorbis_comments.push(parse_vorbis(&cursor, &buf, block_length)?);
                break;
            }
            PICTURE_MARKER::MARKER => {
                // mime and description can be up to 2^32 bytes each for some reason
                // Im assigning max 8196 bytes for the whole meta and i dont care
                if buf_len <= cursor + 8196 {
                    mem::drop(buf);
                    buf = read_ahead_offset(&file, 4, cursor as u64).await?;
                    cursor = 0;
                }
                pictures_metadata.push(parse_picture(&mut cursor, &buf)?);
            }
            PICTURE_MARKER::END_OF_BLOCK => {
                // mime and description can be up to 2^32 bytes each for some reason
                // Im assigning max 8196 bytes for the whole meta and i dont care
                if buf_len <= cursor + 8196 {
                    mem::drop(buf);
                    buf = read_ahead_offset(&file, 0, cursor as u64).await?;
                    cursor = 0;
                }
                pictures_metadata.push(parse_picture(&mut cursor, &buf)?);
                break;
            }
            n if n >= 128 => {
                // reached end marker
                break;
            }
            _ => {
                // ignored block
                cursor += block_length;
                if buf_len <= cursor + 4 {
                    mem::drop(buf);
                    buf = read_ahead_offset(&file, 0, cursor as u64).await?;
                    cursor = 0;
                }
            }
        }
    }
    Ok(())
}
fn parse_picture(cursor: &mut usize, buf: &[u8]) -> anyhow::Result<Picture> {
    let picture_type = read_u32(cursor, buf)?;
    let mime_len = read_u32(cursor, buf)? as usize;
    let mime = String::from_utf8_lossy(
        buf.get(*cursor..*cursor + mime_len)
            .ok_or(anyhow!("Buffer too small"))?,
    );
    *cursor += mime_len;
    let description_len = read_u32(cursor, buf)? as usize;
    let description = String::from_utf8_lossy(
        buf.get(*cursor..*cursor + description_len)
            .ok_or(anyhow!("Buffer too small"))?,
    );
    *cursor += description_len;
    let width = read_u32(cursor, buf)?;
    let height = read_u32(cursor, buf)?;
    let color_depth = read_u32(cursor, buf)?;
    let indexed_color_number = read_u32(cursor, buf)?;
    let picture_len = read_u32(cursor, buf)?;
    //let picture_data = buf
    //    .get(*cursor..*cursor + picture_len)
    //    .ok_or(anyhow!("Buffer too small"))?
    //    .to_vec();

    Ok(Picture {
        picture_type,
        size: picture_len,
        mime: mime.to_string(),
        description: description.to_string(),
        width,
        height,
        color_depth,
        indexed_color_number,
    })
}
