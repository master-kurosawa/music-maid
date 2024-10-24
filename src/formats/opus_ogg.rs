use tokio_uring::fs::File;

use crate::{
    shared::{parse_vorbis, Picture, VorbisComment},
    utils::read_ahead_offset,
};

const OPUS_MARKER: [u8; 8] = [0x4F, 0x70, 0x75, 0x73, 0x48, 0x65, 0x61, 0x64];
const OPUS_TAGS_MARKER: [u8; 8] = [0x4F, 0x70, 0x75, 0x73, 0x54, 0x61, 0x67, 0x73];
const OPUS_PICTURE_VORBIS: [u8; 22] = [
    0x6D, 0x65, 0x74, 0x61, 0x64, 0x61, 0x74, 0x61, 0x5F, 0x62, 0x6C, 0x6F, 0x63, 0x6B, 0x5F, 0x70,
    0x69, 0x63, 0x74, 0x75, 0x72, 0x65,
];

async fn parse_ogg_vorbis(file: &File, buf: Vec<u8>, cursor: usize) -> Vec<u8> {
    let mut vorbis_comments_bytes: Vec<u8> = Vec::new();
    let mut buf = buf;
    let mut cursor = cursor;

    loop {
        cursor += 5;
        let header: usize = buf[cursor].into();
        cursor += 21;
        let segment_len: usize = buf[cursor].into();
        let mut segment_total: usize = 0;
        for i in 1..=segment_len {
            segment_total += buf[cursor + i] as usize;
        }
        cursor += segment_len + 1;
        if buf[cursor..cursor + 8] == OPUS_TAGS_MARKER {
            cursor += 8;
        }
        if buf.len() < cursor + segment_total {
            buf = read_ahead_offset(file, segment_total, cursor as u64)
                .await
                .unwrap();
            cursor = 0;
        }
        let find_vorbis_picture = buf[cursor..cursor + segment_total]
            .windows(22)
            .position(|window| window == OPUS_PICTURE_VORBIS);
        if let Some(pos) = find_vorbis_picture {
            let start_picture_ptr = cursor + pos - 4;
            vorbis_comments_bytes.extend_from_slice(&buf[cursor..start_picture_ptr]);
            // skip everything if picture is encountered
            break;
            //let picture_len = u32::from_le_bytes(
            //    buf[start_picture_ptr..start_picture_ptr + 4]
            //        .try_into()
            //        .unwrap(),
            //) as usize;
            //let next_ogg_header = buf
            //    .windows(4)
            //    .position(|window| window == OGG_MARKER)
            //    .unwrap();
            ////let img_section_count = buf[cursor + next_ogg_header + 27] as usize;
            //let end_picture_ptr = cursor + pos + picture_len;
            //// extract pic meta
            //buf = read_ahead_offset(file, 0, end_picture_ptr as u64)
            //    .await
            //    .unwrap();

            //cursor = 0;
            //let end_of_page = buf
            //    .windows(4)
            //    .position(|window| window == OGG_MARKER)
            //    .unwrap();
            //vorbis_comments_bytes.extend_from_slice(&buf[cursor..end_of_page]);
            //cursor += end_of_page;
            //let hd = buf[end_of_page + 5];
            //if hd == 0x01 {
            //    break;
            //}
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

fn parse_ogg_page(buf: &[u8], cursor: &mut usize) -> Vec<u8> {
    let header: usize = buf[*cursor + 5].into();
    let segment_len: usize = buf[*cursor + 26].into();
    *cursor += 27;

    let mut segment_total: usize = 0;
    for i in 0..segment_len {
        segment_total += buf[*cursor + i] as usize;
    }
    *cursor += segment_len + segment_total;
    if segment_total % 255 > 0 || header == 4 {
        buf[*cursor - segment_total..*cursor].to_vec()
    } else {
        [
            &buf[*cursor - segment_total..*cursor],
            &parse_ogg_page(buf, cursor),
        ]
        .concat()
    }
}

pub async fn parse_ogg_pages(
    buf: Vec<u8>,
    file: File,
    vorbis_comments: &mut Vec<VorbisComment>,
    pictures_metadata: &mut Vec<Picture>,
) -> anyhow::Result<()> {
    let mut cursor = 0;
    let first_page = parse_ogg_page(&buf, &mut cursor);

    if first_page[0..8] == OPUS_MARKER {
        let vorbis_comment = parse_ogg_vorbis(&file, buf, cursor).await;
        let comments = parse_vorbis(&0, &vorbis_comment, vorbis_comment.len() - 4).unwrap();
        vorbis_comments.push(comments);
    } else {
        // println!("probably ogg")
    }
    Ok(())
}
