use base64::{engine::general_purpose, Engine as _};
use std::cmp::Ordering;

use anyhow::anyhow;

pub const OGG_MARKER: [u8; 4] = [0x4F, 0x67, 0x67, 0x53];
use crate::{
    db::{
        padding::Padding,
        picture::Picture,
        vorbis::{VorbisComment, VorbisMeta, SMALLEST_VORBIS_4BYTE_POSSIBLE, VORBIS_FIELDS_LOWER},
    },
    io::{ogg::OggPageReader, reader::UringBufReader},
};

const MAX_OGG_PAGE_SIZE: u32 = 65_307;
const VORBIS_SIZE_LIMIT: u32 = MAX_OGG_PAGE_SIZE; // skips any comments > this size

const OPUS_MARKER: [u8; 8] = [0x4F, 0x70, 0x75, 0x73, 0x48, 0x65, 0x61, 0x64];
const OPUS_TAGS_MARKER: [u8; 8] = [0x4F, 0x70, 0x75, 0x73, 0x54, 0x61, 0x67, 0x73];
const VORBIS_PICTURE_MARKER: [u8; 22] = [
    0x6D, 0x65, 0x74, 0x61, 0x64, 0x61, 0x74, 0x61, 0x5F, 0x62, 0x6C, 0x6F, 0x63, 0x6B, 0x5F, 0x70,
    0x69, 0x63, 0x74, 0x75, 0x72, 0x65,
];
const VORBIS_PICTURE_MARKER_UPPER: [u8; 22] = [
    0x4D, 0x45, 0x54, 0x41, 0x44, 0x41, 0x54, 0x41, 0x5F, 0x42, 0x4C, 0x4F, 0x43, 0x4B, 0x5F, 0x50,
    0x49, 0x43, 0x54, 0x55, 0x52, 0x45,
];

async fn parse_opus_vorbis<'a>(
    ogg_reader: &mut OggPageReader<'a>,
    pictures_metadata: &mut Vec<Picture>,
) -> anyhow::Result<(Vec<VorbisComment>, Option<Padding>)> {
    let mut comments = Vec::new();
    let mut padding: Option<Padding> = None;

    let vendor_file_ptr = ogg_reader.reader.file_ptr + ogg_reader.reader.cursor;
    let vendor_bytes: [u8; 4] = ogg_reader.get_bytes(4).await?.try_into().unwrap();
    let vendor_len = u32::from_le_bytes(vendor_bytes);
    let vendor = ogg_reader.get_bytes(vendor_len as usize).await?;
    comments.push(VorbisComment {
        meta_id: None,
        file_ptr: vendor_file_ptr as i64,
        value: Some(String::from_utf8_lossy(&vendor).to_string()),
        size: vendor_len as i64,
        last_ogg_header_ptr: Some(ogg_reader.last_header_ptr as i64),
        key: "vendor".to_owned(),
        id: None,
    });

    let comment_amount_bytes: [u8; 4] = ogg_reader.get_bytes(4).await?.try_into().unwrap();
    let comment_len_bytes: [u8; 4] = ogg_reader.get_bytes(4).await?.try_into().unwrap();

    let mut comment_amount: Option<u32> = Some(u32::from_le_bytes(comment_amount_bytes));
    let mut comment_len = u32::from_le_bytes(comment_len_bytes);

    if comment_len >= SMALLEST_VORBIS_4BYTE_POSSIBLE {
        comment_len = comment_amount.unwrap();
        comment_amount = None;
        ogg_reader.cursor -= 4;
    }

    let mut comment_counter = 0;

    loop {
        let comment_ptr = ogg_reader.reader.file_ptr + ogg_reader.reader.cursor - 4;
        comment_counter += 1;
        if comment_len == 0 {
            // padding found
            ogg_reader.reader.cursor -= 4;
            ogg_reader.cursor -= 4;
            let file_ptr = ogg_reader.reader.file_ptr + ogg_reader.reader.cursor;
            let padding_len = ogg_reader.parse_till_end().await?.len();
            if padding_len > 0 {
                padding = Some(Padding {
                    id: None,
                    file_id: None,
                    byte_size: Some(padding_len as i64),
                    file_ptr: Some(file_ptr as i64),
                });
            }

            break;
        }
        if comment_len > VORBIS_SIZE_LIMIT {
            let mut comment_key = Vec::with_capacity(VORBIS_PICTURE_MARKER.len());
            // glowing
            loop {
                let k = ogg_reader.get_bytes(1).await?[0];
                if k == b'=' {
                    break;
                }
                comment_key.push(k);
            }

            comments.push(VorbisComment {
                id: None,
                meta_id: None,
                key: String::from_utf8_lossy(&comment_key).to_string(),
                size: comment_len as i64 + 4,
                last_ogg_header_ptr: Some(ogg_reader.last_header_ptr as i64),
                value: None,
                file_ptr: comment_ptr as i64,
            });

            let skipped = if comment_key == VORBIS_PICTURE_MARKER
                || comment_key == VORBIS_PICTURE_MARKER_UPPER
            {
                let (skipped, picture) = parse_picture_meta(ogg_reader, comment_ptr as i64).await?;
                pictures_metadata.push(picture);
                skipped
            } else {
                0
            };
            // if huge comment is at the end we don't waste time skipping it if its last
            if comment_amount.is_some() && comment_amount.unwrap() == comment_counter {
                break;
            }
            ogg_reader
                .skip(comment_len as usize - comment_key.len() - skipped as usize - 1)
                .await?;
        } else {
            let comment = ogg_reader.get_bytes(comment_len as usize).await?;
            if let Some(picture_check) = comment.get(0..VORBIS_PICTURE_MARKER.len()) {
                if picture_check == VORBIS_PICTURE_MARKER
                    || picture_check == VORBIS_PICTURE_MARKER_UPPER
                {
                    pictures_metadata.push(Picture::from_picture_block(
                        &general_purpose::STANDARD
                            .decode(&comment[VORBIS_PICTURE_MARKER.len() + 1..])
                            .unwrap(),
                        comment_ptr as i64,
                        true,
                    ));
                }
            }
            if let Some((key, val)) = VorbisComment::into_key_val(&comment) {
                comments.push(VorbisComment {
                    meta_id: None,
                    id: None,
                    key,
                    size: comment_len as i64 + 4,
                    file_ptr: comment_ptr as i64,
                    last_ogg_header_ptr: Some(ogg_reader.last_header_ptr as i64),
                    value: Some(val),
                });
            } else {
                println!("corrupted comment {:?}", String::from_utf8_lossy(&comment));
                //return Err(anyhow!("Corrupted comment: {comment}"));
                // skip the corrupted comments for now
            }
        }
        if ogg_reader.ends_stream && ogg_reader.segment_size - ogg_reader.cursor == 0 {
            break;
        }

        let comment_len_bytes: [u8; 4] =
            if let Ok(comment_len_bytes) = ogg_reader.get_bytes(4).await?.try_into() {
                comment_len_bytes
            } else {
                break;
            };

        comment_len = u32::from_le_bytes(comment_len_bytes);
    }

    Ok((comments, padding))
}

async fn parse_picture_meta<'a>(
    ogg_reader: &mut OggPageReader<'a>,
    file_ptr: i64,
) -> anyhow::Result<(u32, Picture)> {
    let mut size_read = 0;
    let mut final_bytes = Vec::new();
    let to_base64_bytes = |bytes: usize| -> usize {
        let base64_chars = bytes / 3 * 4;
        let padding_chars = if bytes % 3 > 0 { 4 } else { 0 };
        base64_chars + padding_chars
    };
    let get_u32 = |bytes: &[u8]| -> u32 { u32::from_be_bytes(bytes.try_into().unwrap()) };

    let base_len = to_base64_bytes(32);
    final_bytes.extend(
        general_purpose::STANDARD
            .decode(ogg_reader.get_bytes(base_len).await?)
            .unwrap(),
    );
    size_read += base_len;

    let mime_len_bytes = get_u32(&final_bytes[4..8]);
    let mime_len = to_base64_bytes(mime_len_bytes as usize);
    let b64_cursor = 8 + mime_len_bytes as usize;
    final_bytes.extend(
        general_purpose::STANDARD
            .decode(ogg_reader.get_bytes(mime_len).await?)
            .unwrap(),
    );

    size_read += mime_len;
    let description_len_bytes = get_u32(&final_bytes[b64_cursor..b64_cursor + 4]) as usize;
    let description_len = to_base64_bytes(description_len_bytes);
    final_bytes.extend(
        general_purpose::STANDARD
            .decode(ogg_reader.get_bytes(description_len).await?)
            .unwrap(),
    );
    size_read += description_len;

    let suffix_len = to_base64_bytes(20);
    final_bytes.extend(
        general_purpose::STANDARD
            .decode(ogg_reader.get_bytes(suffix_len).await?)
            .unwrap(),
    );
    size_read += suffix_len;

    Ok((
        size_read as u32,
        Picture::from_picture_block(&final_bytes, file_ptr, true),
    ))
}

pub async fn parse_ogg_pages(
    reader: &mut UringBufReader,
) -> anyhow::Result<(
    Option<String>,
    Vec<(Vec<VorbisComment>, i64)>,
    Vec<Picture>,
    Vec<Padding>,
)> {
    reader.cursor -= 4;
    let mut paddings = Vec::new();
    let mut vorbis_comments = Vec::new();
    let mut pictures = Vec::new();
    let mut ogg_reader = OggPageReader::new(reader).await?;

    let first_page = ogg_reader.parse_till_end().await?;

    if first_page[0..8] == OPUS_MARKER {
        ogg_reader.parse_header().await?;
        if ogg_reader.get_bytes(8).await? == OPUS_TAGS_MARKER {
            let vorbis_ptr = ogg_reader.reader.cursor + ogg_reader.reader.file_ptr;
            let (comment, padding) = parse_opus_vorbis(&mut ogg_reader, &mut pictures).await?;
            if let Some(padding) = padding {
                paddings.push(padding);
            }
            vorbis_comments.push((comment, vorbis_ptr as i64));
        }

        Ok((Some("opus".to_owned()), vorbis_comments, pictures, paddings))
    } else {
        // TODO
        Ok((Some("ogg".to_owned()), vorbis_comments, pictures, paddings))
    }
}
