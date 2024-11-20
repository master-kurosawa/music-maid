use super::{checksum::crc32, reader::UringBufReader};
use crate::formats::opus_ogg::OGG_MARKER;
use anyhow::{anyhow, Context};
use std::{cmp::Ordering, io};

pub struct OggPageReader<'a> {
    pub reader: &'a mut UringBufReader,
    pub cursor: usize,
    pub ends_stream: bool,
    pub segment_size: usize,
    pub last_header_ptr: usize,
    pub page_number: u32,
    last_header: Vec<u8>,
}

impl<'a> OggPageReader<'a> {
    pub fn header_length(&self) -> usize {
        self.last_header.len()
    }
    /// Creates a new OggPageReader and immediately parses first header
    /// returns Err if reader isn't positioned on header
    pub async fn new(reader: &'a mut UringBufReader) -> anyhow::Result<Self> {
        let mut ogg_reader = OggPageReader {
            reader,
            last_header_ptr: 0,
            last_header: Vec::with_capacity(64),
            segment_size: 0,
            ends_stream: true,
            page_number: 0,
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
                "Attempted to read header while cursor is in the middle of segment. file: {}",
                path.to_str().unwrap()
            ));
        }
        self.last_header_ptr = (self.reader.file_ptr + self.reader.cursor) as usize;
        let header_prefix = self.reader.get_bytes(27).await?;
        self.last_header.clear();
        self.last_header.extend(&header_prefix[0..22]);
        self.last_header.extend([0; 4]); // 0s out CRC
        self.last_header.push(header_prefix[26]);

        assert_eq!(
            header_prefix[0..4],
            OGG_MARKER,
            "OGG marker doesn't match. Possibly corrupted file: {}",
            path.to_str().unwrap(),
        );
        let header: usize = header_prefix[5].into();
        let page_number = u32::from_be_bytes(header_prefix[18..22].try_into()?);
        let segment_len: usize = header_prefix[26].into();
        let segments = self.reader.get_bytes(segment_len).await?;
        let segment_total = segments.iter().fold(0, |acc, x| acc + *x as usize);
        self.last_header.extend(segments);
        self.segment_size = segment_total;
        self.page_number = page_number;
        self.ends_stream = header > 4 || segment_total % 255 > 0;
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
        match self.cursor.cmp(&self.segment_size) {
            Ordering::Equal => self.parse_header().await,
            Ordering::Greater => Err(anyhow!(
                "Attempted to read bytes from header segment {}",
                self.reader.path.to_str().unwrap(),
            )),
            _ => Ok(()),
        }
    }
    /// parses current stream till end.
    pub async fn parse_till_end(&mut self) -> anyhow::Result<Vec<u8>> {
        let mut result = Vec::with_capacity(self.segment_size - self.cursor);

        self.check_cursor().await?;
        while !self.ends_stream {
            result.extend(self.get_bytes(self.segment_size - self.cursor).await?);

            self.check_cursor().await?;
        }
        let funny = self.get_bytes(self.segment_size - self.cursor).await?;
        result.extend(funny);
        Ok(result)
    }

    /// skips bytes from stream's content
    /// calculates how many header bytes need to be skipped depending on previous header size
    /// generally should be ok as all encoders divide pages in a stream equally
    /// probably will explode if it skips beyond stream, into different one
    pub async fn skip(&mut self, size: usize) -> anyhow::Result<()> {
        let current_page_skip = self.segment_size - self.cursor;
        match current_page_skip.cmp(&size) {
            Ordering::Less => {}
            Ordering::Greater => {
                self.reader.skip(size as u64).await?;
                self.cursor += size;
                return Ok(());
            }
            Ordering::Equal => {
                self.reader.skip(size as u64).await?;
                self.cursor = self.segment_size;
                self.parse_header().await?;
                return Ok(());
            }
        }

        // crazy ceiled integer division
        let segments_per_page = (self.segment_size + 254) / 255;

        let page_header_size = 27 + segments_per_page;

        // Works 100% times whenever it does work.
        let lol = ((size - current_page_skip) % self.segment_size < self.segment_size / 2) as usize;
        let mut filled_pages_amount = ((size - current_page_skip) / 255) / segments_per_page;
        if filled_pages_amount <= 1 {
            self.reader.skip(current_page_skip as u64).await?;
            self.cursor = self.segment_size;
            self.parse_header().await?;
            self.reader.skip((size - current_page_skip) as u64).await?;
            self.cursor += size - current_page_skip;
            return Ok(());
        }
        filled_pages_amount -= lol;
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

        let x = &self.reader.buf[0..];
        self.parse_header().await?;

        self.reader.skip(leftover as u64).await?;
        self.cursor += leftover;
        self.check_cursor().await?;

        Ok(())
    }
}

impl<'a> OggPageReader<'a> {
    async fn write_last_crc(&mut self, segment_bytes: &[u8]) -> Result<(), io::Error> {
        let (res, _buf) = self
            .reader
            .file
            .write_all_at(
                crc32(segment_bytes).to_le_bytes().to_vec(),
                self.last_header_ptr as u64 + 22, // crc offset
            )
            .await;
        res
    }
    /// reads entire page (from last header) including header and recalculates its checksum
    pub async fn recalculate_last_crc(&mut self) -> Result<(), io::Error> {
        let buf = Vec::with_capacity(self.segment_size + self.last_header.len());
        let (res, mut buf) = self
            .reader
            .file
            .read_exact_at(buf, self.last_header_ptr as u64)
            .await;
        res?;

        // writes 0's at CRC offset
        unsafe {
            let ptr = buf.as_mut_ptr();
            std::ptr::copy_nonoverlapping([0; 4].as_ptr(), ptr.add(22), 4);
        }
        self.write_last_crc(&buf).await
    }

    /// Writes buffer into segment part of stream at current cursor
    /// recalculates checksum
    pub async fn write_stream(&mut self, buf: &[u8]) -> anyhow::Result<()> {
        self.check_cursor().await?;

        let remaining_in_segment = self.segment_size - self.cursor;
        let (current_chunk, remaining_data) = if buf.len() > remaining_in_segment {
            buf.split_at(remaining_in_segment)
        } else {
            (buf, &[][..])
        };

        self.reader
            .write_at_current_offset(current_chunk.to_vec())
            .await?;
        self.cursor += current_chunk.len();

        if self.cursor == self.segment_size {
            if current_chunk.len() == self.segment_size {
                let mut header = self.last_header.clone();
                header.extend(current_chunk);
                self.write_last_crc(&header).await?;
            } else {
                self.recalculate_last_crc().await?;
            }

            self.parse_header().await?;
        }

        if !remaining_data.is_empty() {
            Box::pin(self.write_stream(remaining_data)).await?;
        }

        Ok(())
    }

    /// Writes \0 bytes to segments until end of stream, starting from current cursor
    pub async fn pad_till_end(&mut self) -> anyhow::Result<()> {
        while !self.ends_stream {
            let remaining_segment = self.segment_size - self.cursor;
            self.write_stream(&vec![0; remaining_segment]).await?;
        }
        self.write_stream(&vec![0; self.segment_size - self.cursor])
            .await?;
        Ok(())
    }
    pub async fn rehash_headers(&mut self) -> anyhow::Result<()> {
        while !self.ends_stream {
            self.skip(self.segment_size - self.cursor).await?;
            self.recalculate_last_crc().await?;
            self.check_cursor().await?;
        }
        Ok(())
    }
}
