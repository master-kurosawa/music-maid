use std::{cmp::Ordering, io};

use anyhow::anyhow;

use crate::formats::opus_ogg::OGG_MARKER;

use super::{checksum::crc32, reader::UringBufReader};

pub struct OggPageReader<'a> {
    pub reader: &'a mut UringBufReader,
    pub cursor: usize,
    pub ends_stream: bool,
    pub segment_size: usize,
    pub last_header_ptr: usize,
    last_header: Vec<u8>,
}

impl<'a> OggPageReader<'a> {
    /// Creates a new OggPageReader and immediately parses first header
    /// returns Err if reader isn't positioned on header
    pub async fn new(reader: &'a mut UringBufReader) -> anyhow::Result<Self> {
        let mut ogg_reader = OggPageReader {
            reader,
            last_header_ptr: 0,
            last_header: Vec::with_capacity(64),
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
        self.last_header.clear();
        self.last_header.extend(&header_prefix[0..22]);
        self.last_header.extend([0; 4]); // 0s out CRC
        self.last_header.push(header_prefix[26]);

        assert_eq!(
            header_prefix[0..4],
            OGG_MARKER,
            "OGG marker doesn't match. Possibly corrupted file: {path}"
        );
        let header: usize = header_prefix[5].into();
        let segment_len: usize = header_prefix[26].into();
        let segments = self.reader.get_bytes(segment_len).await?;
        let segment_total = segments.iter().fold(0, |acc, x| acc + *x as usize);
        self.last_header.extend(segments);
        self.segment_size = segment_total;
        self.ends_stream = header == 4 || segment_total % 255 > 0;
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
    pub async fn write_stream(&mut self, buf: &[u8]) -> anyhow::Result<()> {
        self.check_cursor().await?;
        let segment = self.segment_size - self.cursor;
        if buf.len() > segment {
            let data = &buf[0..segment];
            self.reader.write_at_current_offset(data.to_vec()).await?;
            if segment != self.segment_size {
                self.recalculate_last_crc().await?;
            } else {
                let mut header = self.last_header.clone();
                header.extend(data);
                self.write_last_crc(&header).await?;
            }
            self.cursor = self.segment_size;
            Box::pin(self.write_stream(&buf[segment..])).await?
        } else {
            self.reader.write_at_current_offset(buf.to_vec()).await?;
            self.cursor += buf.len();
            if buf.len() == self.segment_size {
                let mut header = self.last_header.clone();
                header.extend(buf);
                self.write_last_crc(&header).await?;
                self.check_cursor().await?;
            } else if self.cursor == self.segment_size {
                self.recalculate_last_crc().await?;
                self.check_cursor().await?;
            }
        }
        Ok(())
    }

    /// Writes \0 bytes to segments until end of stream, starting from current cursor
    pub async fn pad_till_end(&mut self) -> anyhow::Result<()> {
        while !self.ends_stream {
            println!("lool");
            let segment_len = self.segment_size - self.cursor;
            self.write_stream(&vec![0; segment_len]).await?;
        }
        self.write_stream(&vec![0; self.segment_size - self.cursor])
            .await?;
        Ok(())
    }
}
