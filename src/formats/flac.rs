use crate::{
    db::{
        audio_file::{AudioFile, AudioFileMeta},
        padding::Padding,
        picture::Picture,
        vorbis::VorbisComment,
    },
    io::reader::{Corruption, UringBufReader},
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

#[allow(non_camel_case_types)]
struct PADDING_MARKER;
impl PADDING_MARKER {
    const END_OF_BLOCK: u8 = 0b10000001;
    const MARKER: u8 = 0b00000001;
}

pub async fn parse_flac(reader: &mut UringBufReader) -> Result<AudioFileMeta, Corruption> {
    let audio_file = AudioFile {
        id: None,
        path: reader.path.to_string_lossy().to_string(),
        name: reader
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string(),
        format: Some("flac".to_owned()),
    };
    let mut vorbis_sections = Vec::new();
    let mut pictures = Vec::new();
    let mut paddings = Vec::new();
    loop {
        let header = reader.get_bytes(4).await?;
        let block_length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;

        match header[0] {
            VORBIS_COMMENT_MARKER::MARKER => {
                let vorbis_ptr = (reader.file_ptr + reader.cursor) as i64;
                let vorbis_block = reader.get_bytes(block_length).await?;
                if vorbis_block.len() < block_length {
                    return Err(Corruption {
                        message: format!(
                            "Not enough bytes for vorbis block. Length: {block_length}"
                        ),
                        file_cursor: reader.current_offset(),
                        path: reader.path.to_owned(),
                    });
                }

                vorbis_sections.push(
                    VorbisComment::parse_block(vorbis_block, vorbis_ptr)
                        .await
                        .map_err(|mut err| {
                            err.path = reader.path.to_owned();
                            err
                        })?,
                );
            }
            VORBIS_COMMENT_MARKER::END_OF_BLOCK => {
                let vorbis_ptr = reader.current_offset() as i64;
                let vorbis_block = reader.get_bytes(block_length).await?;

                if vorbis_block.len() < block_length {
                    return Err(Corruption {
                        message: format!(
                            "Not enough bytes for vorbis block. Length: {block_length}"
                        ),
                        file_cursor: reader.current_offset(),
                        path: reader.path.to_owned(),
                    });
                }

                vorbis_sections.push(
                    VorbisComment::parse_block(vorbis_block, vorbis_ptr)
                        .await
                        .map_err(|mut err| {
                            err.path = reader.path.to_owned();
                            err
                        })?,
                );
                break;
            }
            PICTURE_MARKER::MARKER => {
                pictures.push(parse_picture(reader).await?);
            }
            PICTURE_MARKER::END_OF_BLOCK => {
                pictures.push(parse_picture(reader).await?);
                break;
            }
            PADDING_MARKER::MARKER => {
                paddings.push(Padding {
                    id: None,
                    file_id: None,
                    file_ptr: Some(reader.current_offset() as i64),
                    byte_size: Some(block_length as i64),
                });
                reader.skip(block_length as u64).await?;
            }
            PADDING_MARKER::END_OF_BLOCK => {
                paddings.push(Padding {
                    id: None,
                    file_id: None,
                    file_ptr: Some(reader.current_offset() as i64),
                    byte_size: Some(block_length as i64),
                });

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
    Ok(AudioFileMeta {
        audio_file,
        comments: vorbis_sections,
        pictures,
        paddings,
    })
}
async fn parse_picture(reader: &mut UringBufReader) -> Result<Picture, Corruption> {
    let file_ptr = reader.current_offset() as i64;
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
        id: None,
        file_id: None,
        file_ptr,
        picture_type: picture_type as i64,
        size: picture_len as i64,
        mime,
        description,
        width: width as i64,
        height: height as i64,
        color_depth: color_depth as i64,
        indexed_color_number: indexed_color_number as i64,
        vorbis_comment: false,
    })
}
