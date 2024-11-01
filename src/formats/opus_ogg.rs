use std::{cmp::Ordering, collections::HashMap};

use anyhow::anyhow;

pub const OGG_MARKER: [u8; 4] = [0x4F, 0x67, 0x67, 0x53];
use crate::{
    db::{
        picture::Picture,
        vorbis::{VorbisComment, SMALLEST_VORBIS_4BYTE_POSSIBLE, VORBIS_FIELDS_LOWER},
    },
    reader::UringBufReader,
};

const MAX_OGG_PAGE_SIZE: u32 = 65_307;
const VORBIS_SIZE_LIMIT: u32 = MAX_OGG_PAGE_SIZE; // skips any comments > this size

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

struct OggPageReader<'a> {
    pub reader: &'a mut UringBufReader,
    last_header: usize,
    segment_size: usize,
    last_header_ptr: usize,
    cursor: usize,
    ends_stream: bool,
}

impl<'a> OggPageReader<'a> {
    /// Creates a new OggPageReader and immediately parses first header
    /// returns Err if reader isn't positioned on header
    pub async fn new(reader: &'a mut UringBufReader) -> anyhow::Result<Self> {
        let mut ogg_reader = OggPageReader {
            reader,
            last_header_ptr: 0,
            last_header: 0,
            segment_size: 0,
            ends_stream: true,
            cursor: 0,
        };
        ogg_reader.parse_header().await?;
        Ok(ogg_reader)
    }
    /// parses header and mutates self attributes
    /// returns Err if cursor isn't positioned on segment_size (end of segment)
    pub async fn parse_header(&mut self) -> anyhow::Result<()> {
        let path = self.reader.path.clone();
        if self.segment_size != self.cursor {
            return Err(anyhow!(
                "Attempted to read header while cursor is in the middle of segment. file: {path}"
            ));
        }
        self.last_header_ptr = (self.reader.file_ptr + self.reader.cursor) as usize;
        let header_prefix = self.reader.get_bytes(27).await?;

        assert_eq!(header_prefix[0..4], OGG_MARKER, "Corrupted file: {path}");
        let header: usize = header_prefix[5].into();
        let segment_len: usize = header_prefix[26].into();
        let segment_total = self
            .reader
            .get_bytes(segment_len)
            .await?
            .iter()
            .fold(0, |acc, x| acc + *x as usize);
        self.segment_size = segment_total;
        self.ends_stream = header == 4 || segment_total % 255 > 0;
        self.last_header = header;
        self.cursor = 0;
        Ok(())
    }
    /// Gets usize amount of bytes, automatically skipping headers.
    /// Ignores streams, can return bytes from different streams
    pub async fn get_bytes(&mut self, size: usize) -> anyhow::Result<Vec<u8>> {
        let mut result = Vec::with_capacity(size);
        let mut size_left = size;
        loop {
            self.check_cursor().await?;
            let left_in_segment = self.segment_size - self.cursor;
            if left_in_segment == 0 {
                return Ok(result);
            };
            if size_left > left_in_segment {
                size_left -= left_in_segment;
                self.cursor += left_in_segment;
                result.extend(self.reader.get_bytes(left_in_segment).await?);
            } else {
                self.cursor += size_left;
                result.extend(self.reader.get_bytes(size_left).await?);
                break;
            }
        }
        Ok(result)
    }
    /// checks current cursor
    /// cursor = segment_size => parses header
    /// cursor > segment_size => Err
    /// _ => Ok(())
    async fn check_cursor(&mut self) -> anyhow::Result<()> {
        let c = &self.reader.path;
        match self.cursor.cmp(&self.segment_size) {
            Ordering::Equal => self.parse_header().await,
            Ordering::Greater => Err(anyhow!("Attempted to read bytes from header segment {c}")),
            _ => Ok(()),
        }
    }
    /// parses current stream till end.
    pub async fn parse_till_end(&mut self) -> anyhow::Result<Vec<u8>> {
        let mut result = Vec::with_capacity(self.segment_size - self.cursor);
        while !self.ends_stream {
            result.extend(self.get_bytes(self.segment_size - self.cursor).await?);
            self.check_cursor().await?;
        }
        result.extend(self.get_bytes(self.segment_size - self.cursor).await?);
        Ok(result)
    }

    /// skips bytes from stream's content
    /// calculates how many header bytes need to be skipped depending on previous header size
    /// generally should be ok as all encoders divide pages in a stream equally
    /// probably will explode if it skips beyond stream, into different one
    pub async fn skip(&mut self, size: usize) -> anyhow::Result<()> {
        let current_page_skip = self.segment_size - self.cursor;
        if current_page_skip > size {
            self.reader.skip(size as u64).await?;
            self.cursor += size;
            return Ok(());
        }
        // crazy ceiled integer division
        let segments_per_page = (self.segment_size + 254) / 255;
        let page_header_size = 27 + segments_per_page;
        let filled_pages_amount = ((size - current_page_skip) / 255) / segments_per_page;

        let filled_skip_size = filled_pages_amount * page_header_size;
        let skip_with_headers = size + filled_skip_size + page_header_size;

        let leftover = size - (filled_pages_amount * self.segment_size) - current_page_skip;

        self.reader
            .skip_read(
                (skip_with_headers - leftover - page_header_size) as u64,
                self.segment_size + page_header_size + leftover,
            )
            .await?;
        self.cursor = self.segment_size;
        self.parse_header().await?;

        self.reader.skip(leftover as u64).await?;
        self.cursor = leftover;
        self.check_cursor().await?;

        Ok(())
    }
}

async fn parse_opus_vorbis<'a>(
    ogg_reader: &mut OggPageReader<'a>,
) -> anyhow::Result<VorbisComment> {
    let mut comments = HashMap::new();
    let mut outcasts = Vec::new();

    let vendor_bytes: [u8; 4] = ogg_reader.get_bytes(4).await?.try_into().unwrap();
    let vendor_len = u32::from_le_bytes(vendor_bytes);
    let vendor = ogg_reader.get_bytes(vendor_len as usize).await?;
    comments.insert(
        "vendor".to_owned(),
        String::from_utf8_lossy(&vendor).to_string(),
    );

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
        comment_counter += 1;
        if comment_len == 0 {
            // padding found
            ogg_reader.reader.cursor -= 4;
            ogg_reader.cursor -= 4;
            let pad = ogg_reader.parse_till_end().await?.len();

            //let z = String::from_utf8_lossy(&pad);
            //println!("{pad:?}");
            break;
        }
        if comment_len > VORBIS_SIZE_LIMIT {
            // if huge comment is at the end we don't waste time skipping it if its last
            if comment_amount.is_some() && comment_amount.unwrap() == comment_counter {
                break;
            }
            ogg_reader.skip((comment_len) as usize).await?;
        } else {
            let comment = ogg_reader.get_bytes(comment_len as usize).await?;
            if let Some((key, val)) = VorbisComment::into_key_val(&comment) {
                if VORBIS_FIELDS_LOWER.contains(&key.as_str()) {
                    comments.insert(key, val);
                } else {
                    outcasts.push(format!("{key}={val}"));
                }
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

    Ok(VorbisComment::init(comments, outcasts))
}

pub async fn parse_ogg_pages(
    reader: &mut UringBufReader,
    vorbis_comments: &mut Vec<VorbisComment>,
    pictures_metadata: &mut Vec<Picture>,
) -> anyhow::Result<String> {
    reader.cursor -= 4;
    let mut ogg_reader = OggPageReader::new(reader).await?;

    let first_page = ogg_reader.parse_till_end().await?;

    if first_page[0..8] == OPUS_MARKER {
        ogg_reader.parse_header().await?;
        if ogg_reader.get_bytes(8).await? == OPUS_TAGS_MARKER {
            vorbis_comments.push(parse_opus_vorbis(&mut ogg_reader).await?);
        }
        Ok("opus".to_owned())
    } else {
        // TODO
        Ok("ogg".to_owned())
    }
}
