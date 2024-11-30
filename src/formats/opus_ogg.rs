use crate::{
    db::{
        audio_file::{AudioFile, AudioFileMeta},
        padding::Padding,
        picture::Picture,
        vorbis::{VorbisComment, VorbisMeta},
    },
    io::{ogg::OggPageReader, reader::UringBufReader},
};
use base64::{engine::general_purpose, Engine as _};
use std::os::fd::AsRawFd;
use tokio_uring::fs::OpenOptions;

pub const OGG_MARKER: [u8; 4] = [0x4F, 0x67, 0x67, 0x53];
const MAX_OGG_PAGE_SIZE: u32 = 65_307;
const VORBIS_SIZE_LIMIT: u32 = MAX_OGG_PAGE_SIZE; // skips values of any comments > this size
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
) -> anyhow::Result<AudioFileMeta> {
    let mut comments = Vec::new();
    let mut pictures = Vec::new();
    let mut padding = Vec::new();

    let vorbis_ptr = ogg_reader.reader.current_offset() as i64;

    let vendor_bytes: [u8; 4] = ogg_reader.get_bytes(4).await?.try_into().unwrap();
    let vendor = String::from_utf8_lossy(
        &ogg_reader
            .get_bytes(u32::from_le_bytes(vendor_bytes) as usize)
            .await?,
    )
    .to_string();

    let comment_amount_ptr = ogg_reader.reader.current_offset() as i64;

    let comment_amount_bytes: [u8; 4] = ogg_reader.get_bytes(4).await?.try_into().unwrap();

    let mut vorbis_end_ptr = ogg_reader.reader.current_offset();

    if comment_amount_bytes != [0; 4] {
        let comment_amount = u32::from_le_bytes(comment_amount_bytes);

        let comment_len_bytes: [u8; 4] = ogg_reader.get_bytes(4).await?.try_into().unwrap();
        let mut comment_len = u32::from_le_bytes(comment_len_bytes);

        let mut comment_counter = 0;

        loop {
            let comment_ptr = ogg_reader.reader.current_offset() - 4;
            comment_counter += 1;
            if comment_len > VORBIS_SIZE_LIMIT {
                let mut comment_key = Vec::with_capacity(VORBIS_PICTURE_MARKER.len());
                // glowing ( extracts comment key without worrying about page headers )
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
                    let (skipped, picture) =
                        parse_picture_meta(ogg_reader, comment_ptr as i64).await?;

                    pictures.push(picture);
                    skipped
                } else {
                    0
                };

                ogg_reader.reader.extend_buf(comment_len as usize).await?;
                ogg_reader
                    .safe_skip(comment_len as usize - comment_key.len() - skipped as usize - 1)
                    .await?;
            } else {
                let comment = ogg_reader.get_bytes(comment_len as usize).await?;
                if let Some(picture_check) = comment.get(0..VORBIS_PICTURE_MARKER.len()) {
                    if picture_check == VORBIS_PICTURE_MARKER
                        || picture_check == VORBIS_PICTURE_MARKER_UPPER
                    {
                        pictures.push(Picture::from_picture_block(
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
                    // skip the corrupted comments for now
                }
            }
            if comment_counter == comment_amount
                || (ogg_reader.ends_stream && ogg_reader.page_left() == 0)
            {
                vorbis_end_ptr = ogg_reader.reader.current_offset();
                if ogg_reader.get_bytes(4).await? == [0; 4] {
                    // padding found
                    ogg_reader.reader.cursor -= 4;
                    ogg_reader.cursor -= 4;
                    let file_ptr = ogg_reader.reader.current_offset();
                    let padding_len = ogg_reader.parse_till_end().await?.len();
                    vorbis_end_ptr = ogg_reader.reader.current_offset();

                    if padding_len > 0 {
                        padding.push(Padding {
                            id: None,
                            file_id: None,
                            byte_size: Some(padding_len as i64),
                            file_ptr: Some(file_ptr as i64),
                        });
                    }
                }
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
    }

    let meta = VorbisMeta {
        vendor,
        comment_amount_ptr,
        file_ptr: vorbis_ptr,
        end_ptr: vorbis_end_ptr as i64,
        id: None,
        file_id: None,
    };
    Ok(AudioFileMeta {
        audio_file: AudioFile {
            id: None,
            path: ogg_reader.reader.path.to_string_lossy().to_string(),
            name: ogg_reader
                .reader
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            format: Some("opus".to_owned()),
        },
        pictures,
        comments: vec![(meta, comments)],
        paddings: padding,
    })
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

pub async fn parse_ogg_pages(reader: &mut UringBufReader) -> anyhow::Result<AudioFileMeta> {
    reader.cursor -= 4; // Go back to OGGs
    let mut ogg_reader = OggPageReader::new(reader).await?;

    let first_page = ogg_reader.parse_till_end().await?;

    if first_page[0..8] == OPUS_MARKER {
        ogg_reader.parse_header().await?;
        if ogg_reader.get_bytes(8).await? == OPUS_TAGS_MARKER {
            return parse_opus_vorbis(&mut ogg_reader).await;
        }
        Ok(AudioFileMeta {
            audio_file: AudioFile {
                path: ogg_reader.reader.path.to_string_lossy().to_string(),
                format: Some("opus".to_owned()),
                name: ogg_reader
                    .reader
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                id: None,
            },
            paddings: vec![],
            comments: vec![],
            pictures: vec![],
        })
    } else {
        // TODO
        Ok(AudioFileMeta {
            audio_file: AudioFile {
                path: ogg_reader.reader.path.to_string_lossy().to_string(),
                format: Some("ogg".to_owned()),
                name: ogg_reader
                    .reader
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                id: None,
            },
            paddings: vec![],
            comments: vec![],
            pictures: vec![],
        })
    }
}

pub async fn remove_comments(meta: AudioFileMeta, names: Vec<String>) -> anyhow::Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .read(true)
        .open(meta.audio_file.path.clone())
        .await
        .unwrap();
    let mut reader = UringBufReader::new(file, meta.audio_file.path.into());
    let mut ogg_reader = OggPageReader::new(&mut reader).await.unwrap();
    ogg_reader.parse_till_end().await.unwrap();
    ogg_reader.parse_header().await.unwrap();
    let (vorbis_meta, comments) = &meta.comments[0]; // oggs can contain only 1 meta field
    let mut comment_bytes = Vec::new();
    let mut _removed_comment_size = 0;
    let mut kept_comments: u32 = 0;
    for comment in comments.iter() {
        if names.contains(&comment.key) {
            _removed_comment_size += comment.size;
        } else {
            let comment = comment
                .to_owned()
                .into_bytes_ogg(&mut ogg_reader)
                .await
                .unwrap();
            kept_comments += 1;
            comment_bytes.extend(comment);
        }
    }
    ogg_reader.reader.end_of_file = false;

    ogg_reader.reader.read_at_offset(8196, 0).await?;
    ogg_reader.cursor = ogg_reader.segment_size;
    ogg_reader.parse_header().await?;
    ogg_reader.parse_till_end().await?;
    ogg_reader.parse_header().await?;
    ogg_reader.safe_skip(12 + vorbis_meta.vendor.len()).await?;
    ogg_reader
        .write_stream(&kept_comments.to_le_bytes())
        .await
        .unwrap();
    ogg_reader.write_stream(&comment_bytes).await.unwrap();
    ogg_reader
        .reader
        .write_at_current_offset(vec![0; ogg_reader.segment_size - ogg_reader.cursor])
        .await
        .unwrap();

    ogg_reader.recalculate_last_crc().await.unwrap();

    let mut offset = vorbis_meta.end_ptr;

    let mut total_size = ogg_reader.reader.file_ptr + ogg_reader.reader.cursor;

    loop {
        let buf = vec![0; MAX_OGG_PAGE_SIZE as usize];
        let (res, mut buf) = ogg_reader.reader.file.read_at(buf, offset as u64).await;
        match res {
            Ok(bytes_read) if bytes_read < MAX_OGG_PAGE_SIZE as usize => {
                buf.resize(bytes_read, 0);
                ogg_reader
                    .reader
                    .write_at_current_offset(buf)
                    .await
                    .unwrap();
                total_size += bytes_read as u64;
                break;
            }
            Ok(bytes_read) => {
                ogg_reader
                    .reader
                    .write_at_current_offset(buf)
                    .await
                    .unwrap();
                offset += bytes_read as i64;
                total_size += bytes_read as u64;
            }
            Err(e) => return Err(e.into()),
        }
    }

    unsafe {
        let fd = ogg_reader.reader.file.as_raw_fd();
        libc::ftruncate64(fd, total_size.try_into().unwrap());
    }
    ogg_reader.reader.file.sync_data().await.unwrap();

    Ok(())
}
