use crate::{
    reader::UringBufReader,
    shared::{parse_vorbis, Picture, VorbisComment},
};
use anyhow::anyhow;

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

#[allow(non_camel_case_types)]
struct PADDING_MARKER;
impl PADDING_MARKER {
    const END_OF_BLOCK: u8 = 0b10000001;
    const MARKER: u8 = 0b00000001;
}

pub async fn parse_flac(
    reader: &mut UringBufReader,
    vorbis_comments: &mut Vec<VorbisComment>,
    pictures_metadata: &mut Vec<Picture>,
) -> anyhow::Result<()> {
    loop {
        let header = reader.get_bytes(4).await?;
        let block_length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;

        match header[0] {
            VORBIS_COMMENT_MARKER::MARKER => {
                let vorbis_block = reader.get_bytes(block_length).await?;
                if vorbis_block.len() < block_length {
                    return Err(anyhow!(
                        "Not enough bytes for vorbis block. Length: {block_length}"
                    ));
                }
                let comment = parse_vorbis(vorbis_block).await?;
                vorbis_comments.push(comment);
            }
            VORBIS_COMMENT_MARKER::END_OF_BLOCK => {
                let vorbis_block = reader.get_bytes(block_length).await?;
                if vorbis_block.len() < block_length {
                    return Err(anyhow!(
                        "Not enough bytes for vorbis block. Length: {block_length}"
                    ));
                }
                let comment = parse_vorbis(vorbis_block).await?;
                vorbis_comments.push(comment);
                break;
            }
            PICTURE_MARKER::MARKER => {
                pictures_metadata.push(parse_picture(reader).await?);
            }
            PICTURE_MARKER::END_OF_BLOCK => {
                pictures_metadata.push(parse_picture(reader).await?);
                break;
            }
            PADDING_MARKER::MARKER => {
                reader.skip(block_length as u64).await?;
            }
            PADDING_MARKER::END_OF_BLOCK => {
                break;
            }
            n if n >= 128 => {
                // reached end marker
                break;
            }
            _ => {
                // ignored block
                reader.skip(block_length as u64).await?;
            }
        }
    }
    Ok(())
}
async fn parse_picture(reader: &mut UringBufReader) -> anyhow::Result<Picture> {
    let picture_type = reader.read_u32().await?;

    let mime_len = reader.read_u32().await? as usize;
    let mime_bytes = reader.get_bytes(mime_len).await?;
    let mime = String::from_utf8_lossy(mime_bytes).to_string();

    let description_len = reader.read_u32().await? as usize;
    let description_bytes = reader.get_bytes(description_len).await?;
    let description = String::from_utf8_lossy(description_bytes).to_string();

    let width = reader.read_u32().await?;
    let height = reader.read_u32().await?;
    let color_depth = reader.read_u32().await?;
    let indexed_color_number = reader.read_u32().await?;
    let picture_len = reader.read_u32().await?;

    reader.skip(picture_len as u64).await?;

    Ok(Picture {
        picture_type,
        size: picture_len,
        mime,
        description,
        width,
        height,
        color_depth,
        indexed_color_number,
    })
}
