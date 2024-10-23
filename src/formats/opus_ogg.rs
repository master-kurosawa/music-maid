use tokio_uring::fs::File;

use crate::shared::{Picture, VorbisComment};

const OPUS_MARKER: [u8; 8] = [0x4F, 0x70, 0x75, 0x73, 0x48, 0x65, 0x61, 0x64];

pub async fn parse_ogg_page(
    buf: Vec<u8>,
    file: File,
    vorbis_comments: &mut Vec<VorbisComment>,
    pictures_metadata: &mut Vec<Picture>,
) -> anyhow::Result<()> {
    let mut cursor = 4;
    let mut buf = buf;
    let version = &buf[cursor];
    cursor += 1;
    let header = &buf[cursor];
    cursor += 1;
    cursor += 20; // skip granule (8 byte) bitstream serial (4byte) page seq (4byte) CRC32 checksum (4byte)
    let segment_len: usize = buf[cursor].into();
    cursor += segment_len + 1; // skip segment table
    if buf[cursor..cursor + 8] == OPUS_MARKER {
        println!("OPUS");
        parse_opus(buf, file, vorbis_comments, pictures_metadata).await?;
    } else {
        println!("probably ogg")
    }

    Ok(())
}

async fn parse_opus(
    buf: Vec<u8>,
    file: File,
    vorbis_comments: &mut Vec<VorbisComment>,
    pictures_metadata: &mut Vec<Picture>,
) -> anyhow::Result<()> {
    Ok(())
}
