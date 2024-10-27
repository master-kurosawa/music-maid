use tokio_uring::fs::File;

use crate::{
    shared::{parse_vorbis, Picture, VorbisComment, OGG_MARKER},
    utils::read_ahead_offset,
};

const MAX_OGG_PAGE_SIZE: usize = 65_307;
const OPUS_MARKER: [u8; 8] = [0x4F, 0x70, 0x75, 0x73, 0x48, 0x65, 0x61, 0x64];
const OPUS_TAGS_MARKER: [u8; 8] = [0x4F, 0x70, 0x75, 0x73, 0x54, 0x61, 0x67, 0x73];
const OPUS_PICTURE_VORBIS: [u8; 22] = [
    0x6D, 0x65, 0x74, 0x61, 0x64, 0x61, 0x74, 0x61, 0x5F, 0x62, 0x6C, 0x6F, 0x63, 0x6B, 0x5F, 0x70,
    0x69, 0x63, 0x74, 0x75, 0x72, 0x65,
];
const OPUS_PICTURE_VORBIS_UPPER: [u8; 22] = [
    0x4D, 0x45, 0x54, 0x41, 0x44, 0x41, 0x54, 0x41, 0x5F, 0x42, 0x4C, 0x4F, 0x43, 0x4B, 0x5F, 0x50,
    0x49, 0x43, 0x54, 0x55, 0x52, 0x45,
];

async fn parse_ogg_vorbis(file: &File, buf: &mut Vec<u8>, cursor: usize) -> Vec<u8> {
    let mut vorbis_comments_bytes: Vec<u8> = Vec::new();
    let mut cursor = cursor;
    let mut file_cursor = cursor;
    let mut padding_ptr = 0;
    let mut padding_size = 0;

    loop {
        // loops through pages and exracts vorbis until it finds vorbis embedded image
        // if it does it skips as much as possible, then finds padding and its length
        let header_cursor = file_cursor + cursor;
        let header: usize = buf[cursor + 5].into();
        let segment_len: usize = buf[cursor + 26].into();
        cursor += 27;
        let segment_total = buf[cursor..cursor + segment_len]
            .iter()
            .fold(0, |acc, x| acc + *x as usize);
        cursor += segment_len;
        if buf[cursor..cursor + 8] == OPUS_TAGS_MARKER {
            cursor += 8; // opus tags appears only once inside second page
        }
        if buf.len() < cursor + segment_total {
            file_cursor += cursor;
            *buf = read_ahead_offset(file, segment_total, file_cursor as u64 - 8)
                .await
                .unwrap();
            cursor = 0;
        }

        let find_vorbis_picture =
            buf[cursor..cursor + segment_total]
                .windows(22)
                .position(|window| {
                    window == OPUS_PICTURE_VORBIS_UPPER || window == OPUS_PICTURE_VORBIS
                });
        if let Some(pos) = find_vorbis_picture {
            let start_picture_ptr = cursor + pos - 4;
            // extract tags behind image
            vorbis_comments_bytes.extend_from_slice(&buf[cursor..start_picture_ptr]);

            let picture_len = u32::from_le_bytes(
                buf[start_picture_ptr..start_picture_ptr + 4]
                    .try_into()
                    .unwrap(),
            ) as usize;
            if picture_len > MAX_OGG_PAGE_SIZE {
                file_cursor += cursor + pos + picture_len;

                // Since actual header size still remains unknown we read
                // the whole picture length assuming headers < picture size
                *buf = read_ahead_offset(file, picture_len, file_cursor as u64)
                    .await
                    .unwrap();
                // bigger windows = more accuracy = more time
                let padding_ptr = buf.windows(4).position(|window| window == [0; 4]);

                let (prev_header, padding_ptr) = if let Some(padding_ptr) = padding_ptr {
                    let prev_ogg_header = if let Some(header) = buf[..padding_ptr]
                        .windows(4)
                        .rposition(|window| window == OGG_MARKER)
                    {
                        header
                    } else {
                        *buf = read_ahead_offset(
                            file,
                            MAX_OGG_PAGE_SIZE - 8196,
                            (file_cursor - MAX_OGG_PAGE_SIZE) as u64,
                        )
                        .await
                        .unwrap();
                        buf.windows(4)
                            .rposition(|window| window == OGG_MARKER)
                            .unwrap()
                    };
                    (prev_ogg_header, padding_ptr)
                } else {
                    // if we assume img > 64kb we loaded enough for there to be atleast 1 page

                    cursor = buf
                        .windows(4)
                        .rposition(|window| window == OGG_MARKER)
                        .unwrap();

                    if let Some((header_pos, pos)) =
                        position_ogg_page(file, buf, &mut file_cursor, &mut cursor, [0; 4].to_vec())
                            .await
                    {
                        (header_pos, pos)
                    } else {
                        break;
                    }
                };

                cursor = prev_header;
            } else {
                cursor = header_cursor;
                let (prev_header, padding_pos) =
                    position_ogg_page(file, buf, &mut file_cursor, &mut cursor, [0; 4].to_vec())
                        .await
                        .unwrap();
                cursor = prev_header;
            }
            let padding = parse_ogg_page(file, buf, &mut file_cursor, &mut cursor).await;
            // wild guess that there wont be sequences of 0's longer than 3 outside of padding
            let pad_pos = padding.windows(4).position(|x| *x == [0, 0, 0, 0]).unwrap();
            let padding_len = padding[pad_pos..].len();
            break;
        } else {
            vorbis_comments_bytes.extend_from_slice(&buf[cursor..cursor + segment_total - 8]);
            cursor += segment_total;
        }
        if segment_total % 255 > 0 || header == 4 {
            // ends if its the last segment or header = END
            break;
        }
    }
    if let Some(pos) = vorbis_comments_bytes.iter().rposition(|&x| x != 0) {
        vorbis_comments_bytes.truncate(pos + 1); // removes '\0' suffix
    }
    vorbis_comments_bytes.shrink_to_fit();
    vorbis_comments_bytes
}

async fn position_ogg_page(
    file: &File,
    buf: &mut Vec<u8>,
    file_cursor: &mut usize,
    cursor: &mut usize,
    item: Vec<u8>,
) -> Option<(usize, usize)> {
    let mut _cursor = *cursor; // minimize dereferencing by copying
    loop {
        let header_cursor = _cursor + *file_cursor;
        let header: usize = buf[_cursor + 5].into();
        let segment_len: usize = buf[_cursor + 26].into();
        _cursor += 27;

        let segment_total = buf[_cursor.._cursor + segment_len]
            .iter()
            .fold(0, |acc, x| acc + *x as usize);

        _cursor += segment_len;
        if buf.len() <= _cursor + segment_total {
            *file_cursor += _cursor;
            let file_cursor = *file_cursor as u64;
            *buf = read_ahead_offset(file, segment_total, file_cursor)
                .await
                .unwrap();

            *cursor = 0;
            _cursor = 0;
        }
        if segment_total % 255 > 0 || header == 4 {
            _cursor += segment_total;
            *cursor = _cursor;
            buf[_cursor - segment_total.._cursor]
                .windows(item.len())
                .position(|x| x == item)
                .map(|pos| (header_cursor, *file_cursor + pos + _cursor))
        } else {
            _cursor += segment_total;
            *cursor = _cursor;
            if let Some(pos) = buf[_cursor - segment_total.._cursor]
                .windows(item.len())
                .position(|x| x == item)
            {
                Some((header_cursor, pos + _cursor + *file_cursor))
            } else {
                continue;
            }
        };
    }
}

async fn parse_ogg_page(
    file: &File,
    buf: &mut Vec<u8>,
    file_cursor: &mut usize,
    cursor: &mut usize,
) -> Vec<u8> {
    let mut _cursor = *cursor; // minimize dereferencing by copying
    let header: usize = buf[_cursor + 5].into();
    let segment_len: usize = buf[_cursor + 26].into();
    _cursor += 27;

    let segment_total = buf[_cursor.._cursor + segment_len]
        .iter()
        .fold(0, |acc, x| acc + *x as usize);

    _cursor += segment_len;
    if buf.len() <= _cursor + segment_total {
        *file_cursor += _cursor;
        let file_cursor = *file_cursor as u64;
        *buf = read_ahead_offset(file, segment_total, file_cursor)
            .await
            .unwrap();

        *cursor = 0;
        _cursor = 0;
    }
    if segment_total % 255 > 0 || header == 4 {
        _cursor += segment_total;
        *cursor = _cursor;
        buf[_cursor - segment_total.._cursor].to_vec()
    } else {
        _cursor += segment_total;
        *cursor = _cursor;
        let mut content = buf[_cursor - segment_total.._cursor].to_vec();
        let next_content = Box::pin(parse_ogg_page(file, buf, file_cursor, cursor)).await;
        content.extend_from_slice(next_content.as_slice());
        content
    }
}

pub async fn parse_ogg_pages(
    buf: &mut Vec<u8>,
    file: File,
    vorbis_comments: &mut Vec<VorbisComment>,
    pictures_metadata: &mut Vec<Picture>,
) -> anyhow::Result<()> {
    let mut cursor = 0;
    let mut file_cursor = 0;
    let first_page = parse_ogg_page(&file, buf, &mut file_cursor, &mut cursor).await;

    if first_page[0..8] == OPUS_MARKER {
        let vorbis_comment = parse_ogg_vorbis(&file, buf, cursor).await;
        let comments = parse_vorbis(&0, &vorbis_comment, vorbis_comment.len() - 4)?;
        vorbis_comments.push(comments);
    } else {
        // println!("probably ogg")
    }
    Ok(())
}
