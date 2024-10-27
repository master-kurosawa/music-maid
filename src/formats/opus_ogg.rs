use std::io;

use tokio_uring::fs::File;

use crate::{
    reader::UringBufReader,
    shared::{parse_vorbis, Picture, VorbisComment, OGG_MARKER},
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

async fn parse_ogg_vorbis(reader: &mut UringBufReader) -> Result<Vec<u8>, io::Error> {
    let mut vorbis_comments_bytes: Vec<u8> = Vec::new();
    let mut padding_ptr = 0;
    let mut padding_size = 0;

    loop {
        let header_cursor = reader.file_ptr;
        // loops through pages and exracts vorbis until it finds vorbis embedded image
        // if it does it skips as much as possible, then finds padding and its length
        //
        let header_prefix = reader.get_bytes(27).await?;
        let header: usize = header_prefix[5].into();
        let segment_len: usize = header_prefix[26].into();

        let segment_total = reader
            .get_bytes(segment_len)
            .await?
            .iter()
            .fold(0, |acc, x| acc + *x as usize);
        let segment = reader.get_bytes(segment_total).await?;
        let mut segment_cursor = 0;
        if segment[0..8] == OPUS_TAGS_MARKER {
            segment_cursor += 8; // opus tags appears only once inside second page
        }
        // mpv doesnt handle anything else than fully UPPER or LOWER keys
        // so we wont aswell
        let find_vorbis_picture = segment[segment_cursor..].windows(22).position(|window| {
            window == OPUS_PICTURE_VORBIS_UPPER || window == OPUS_PICTURE_VORBIS
        });
        if let Some(pos) = find_vorbis_picture {
            let start_picture_ptr = segment_cursor + pos - 4;
            // extract tags behind image
            vorbis_comments_bytes.extend_from_slice(&segment[segment_cursor..start_picture_ptr]);

            let picture_len = u32::from_le_bytes(
                segment[start_picture_ptr..start_picture_ptr + 4]
                    .try_into()
                    .unwrap(),
            ) as usize;
            if picture_len > MAX_OGG_PAGE_SIZE {
                let picture_offset = picture_len - reader.cursor as usize - segment_cursor - pos;
                reader
                    .skip_read(picture_offset as u64, picture_len + 8196)
                    .await?;

                let _offset_buffer = reader.read_next(picture_len + 8196).await?;

                // Since actual header size still remains unknown we read
                // the whole picture length assuming headers < picture size

                // bigger windows = more accuracy = more time
                let padding_ptr = reader.buf.windows(4).position(|window| window == [0; 4]);

                let (prev_header, padding_ptr) = if let Some(padding_ptr) = padding_ptr {
                    let prev_ogg_header = if let Some(header) = reader.buf[..padding_ptr]
                        .windows(4)
                        .rposition(|window| window == OGG_MARKER)
                    {
                        header
                    } else {
                        let old_file_cursor =
                            reader.cursor as usize - picture_offset - picture_len - 8196;
                        reader
                            .read_at_offset(
                                MAX_OGG_PAGE_SIZE,
                                (old_file_cursor - MAX_OGG_PAGE_SIZE) as u64,
                            )
                            .await?;

                        reader
                            .buf
                            .windows(4)
                            .rposition(|window| window == OGG_MARKER)
                            .unwrap()
                    };
                    (prev_ogg_header, padding_ptr)
                } else {
                    // if we assume img > 64kb we loaded enough for there to be atleast 1 page

                    reader.cursor = reader
                        .buf
                        .windows(4)
                        .rposition(|window| window == OGG_MARKER)
                        .unwrap() as u64;

                    if let Some((header_pos, pos)) = position_ogg_page(reader, vec![0; 4]).await? {
                        (header_pos, pos)
                    } else {
                        break;
                    }
                };

                reader.cursor = prev_header as u64;
            } else {
                reader.read_at_offset(8196, header_cursor).await?;
                let (prev_header, padding_pos) =
                    position_ogg_page(reader, vec![0; 4]).await?.unwrap();
                reader.read_at_offset(8196, prev_header as u64).await?;
            }
            let padding = parse_ogg_page(reader).await?;
            // wild guess that there wont be sequences of 0's longer than 3 outside of padding
            let pad_pos = padding.windows(4).position(|x| *x == [0, 0, 0, 0]).unwrap();
            let padding_len = padding[pad_pos..].len();
            break;
        } else {
            vorbis_comments_bytes
                .extend_from_slice(&segment[segment_cursor..segment_cursor + segment_total - 8]);
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
    Ok(vorbis_comments_bytes)
}

async fn position_ogg_page(
    reader: &mut UringBufReader,
    item: Vec<u8>,
) -> Result<Option<(usize, usize)>, io::Error> {
    loop {
        let header_cursor = reader.cursor;
        let header_prefix = reader.get_bytes(27).await?;
        let header: usize = header_prefix[5].into();
        let segment_len: usize = header_prefix[26].into();

        let segment_total = reader
            .get_bytes(segment_len)
            .await?
            .iter()
            .fold(0, |acc, x| acc + *x as usize);

        if segment_total % 255 > 0 || header == 4 {
            reader
                .get_bytes(segment_total)
                .await?
                .windows(item.len())
                .position(|x| x == item)
                .map(|pos| {
                    (
                        header_cursor,
                        reader.file_ptr as usize + reader.cursor as usize + pos - segment_total,
                    )
                })
        } else {
            if let Some(pos) = reader
                .get_bytes(segment_total)
                .await?
                .windows(item.len())
                .position(|x| x == item)
            {
                Some((
                    header_cursor,
                    reader.file_ptr as usize + reader.cursor as usize + pos - segment_total,
                ))
            } else {
                continue;
            }
        };
    }
}

async fn parse_ogg_page(reader: &mut UringBufReader) -> Result<Vec<u8>, io::Error> {
    let header_prefix = reader.get_bytes(27).await?;
    let header: usize = header_prefix[5].into();
    let segment_len: usize = header_prefix[26].into();

    let segment_total = reader
        .get_bytes(segment_len)
        .await?
        .iter()
        .fold(0, |acc, x| acc + *x as usize);

    if segment_total % 255 > 0 || header == 4 {
        Ok(reader.get_bytes(segment_total).await?.to_vec())
    } else {
        let mut content = reader.get_bytes(segment_total).await?.to_vec();
        let next_content = Box::pin(parse_ogg_page(reader)).await?;
        content.extend_from_slice(next_content.as_slice());
        Ok(content)
    }
}

pub async fn parse_ogg_pages(
    reader: &mut UringBufReader,
    vorbis_comments: &mut Vec<VorbisComment>,
    pictures_metadata: &mut Vec<Picture>,
) -> anyhow::Result<()> {
    reader.cursor -= 4;
    let first_page = parse_ogg_page(reader).await?;

    if first_page[0..8] == OPUS_MARKER {
        let vorbis_comment = parse_ogg_vorbis(reader).await?;
        //let z = String::from_utf8_lossy(&vorbis_comment);
        //println!("{z:?}");
        let comments = parse_vorbis(&vorbis_comment).await?;
        vorbis_comments.push(comments);
    } else {
        // println!("probably ogg")
    }
    Ok(())
}
