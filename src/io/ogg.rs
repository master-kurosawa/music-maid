use super::{
    checksum::crc32,
    reader::{Corruption, UringBufReader},
};
use crate::formats::opus_ogg::OGG_MARKER;
use std::{cmp::Ordering, mem};

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
    pub async fn new(reader: &'a mut UringBufReader) -> Result<Self, Corruption> {
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
    pub async fn parse_header(&mut self) -> Result<(), Corruption> {
        if self.segment_size != self.cursor {
            return Err(Corruption {
                message: "Attempted to read header while cursor is in the middle of the segment"
                    .to_owned(),
                path: self.reader.path.to_owned(),
                file_cursor: self.reader.current_offset(),
            });
        }
        self.last_header_ptr = (self.reader.file_ptr + self.reader.cursor) as usize;
        let header_prefix = self.reader.get_bytes(27).await.map_err(|mut err| {
            err.message = "Not enough bytes for minimal Ogg Header".to_owned();
            err
        })?;

        self.last_header.clear();
        self.last_header.extend(&header_prefix[0..22]);
        self.last_header.extend([0; 4]); // 0s out CRC
        self.last_header.push(header_prefix[26]);
        if header_prefix[0..4] != OGG_MARKER {
            return Err(Corruption {
                message: "OGG Marker was not found in the expected location.".to_owned(),
                path: self.reader.path.to_owned(),
                file_cursor: self.reader.current_offset(),
            });
        }
        let header: usize = header_prefix[5].into();
        let page_number = u32::from_be_bytes(header_prefix[18..22].try_into().unwrap());
        let segment_len: usize = header_prefix[26].into();
        let segments = self
            .reader
            .get_bytes(segment_len)
            .await
            .map_err(|mut err| {
                err.message = "Not enough bytes for header segments".to_owned();
                err
            })?;
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
    pub async fn get_bytes(&mut self, size: usize) -> Result<Vec<u8>, Corruption> {
        let mut result = Vec::with_capacity(size);
        let mut size_left = size;
        loop {
            self.check_cursor().await?;
            let left_in_segment = self.page_left();
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
    async fn check_cursor(&mut self) -> Result<(), Corruption> {
        match self.cursor.cmp(&self.segment_size) {
            Ordering::Equal => self.parse_header().await,
            Ordering::Greater => Err(Corruption {
                message: "Attempted to read data from header bytes (mismatched pages)".to_owned(),
                path: self.reader.path.to_owned(),
                file_cursor: self.reader.current_offset(),
            }),
            _ => Ok(()),
        }
    }
    /// parses current stream till end.
    pub async fn parse_till_end(&mut self) -> Result<Vec<u8>, Corruption> {
        self.check_cursor().await?;
        let mut result = Vec::with_capacity(self.page_left());

        while !self.ends_stream {
            result.extend(self.get_bytes(self.page_left()).await?);
            self.check_cursor().await?;
        }
        let funny = self.get_bytes(self.page_left()).await?;
        result.extend(funny);
        Ok(result)
    }

    #[inline(always)]
    pub const fn page_left(&self) -> usize {
        self.segment_size - self.cursor
    }

    pub async fn safe_skip(&mut self, size: usize) -> Result<(), Corruption> {
        self.check_cursor().await?;
        let mut read = 0;
        while read < size - self.segment_size {
            let read_page = self.page_left();
            self.reader
                .skip_read(read_page as u64, self.segment_size)
                .await?;
            self.cursor += read_page;
            self.parse_header().await?;
            read += read_page;
        }
        self.reader
            .skip_read((size - read) as u64, self.segment_size)
            .await?;
        self.cursor += size - read;
        Ok(())
    }
}

impl<'a> OggPageReader<'a> {
    async fn write_last_crc(&mut self, segment_bytes: &[u8]) -> Result<(), Corruption> {
        let (res, _buf) = self
            .reader
            .file
            .write_all_at(
                crc32(segment_bytes).to_le_bytes().to_vec(),
                self.last_header_ptr as u64 + 22, // crc offset
            )
            .await;
        res.map_err(|err| Corruption {
            path: self.reader.path.to_owned(),
            file_cursor: self.last_header_ptr as u64 + 22,
            message: format!("Failed to write CRC32. IO error: {err:?}"),
        })
    }
    /// reads entire page (from last header) including header and recalculates its checksum
    pub async fn recalculate_last_crc(&mut self) -> Result<(), Corruption> {
        let full_page_size = self.segment_size + self.last_header.len();
        let buf = Vec::with_capacity(full_page_size);
        let (res, mut buf) = self
            .reader
            .file
            .read_exact_at(buf, self.last_header_ptr as u64)
            .await;
        res.map_err(|err| {
            Corruption::io(
                self.reader.path.to_owned(),
                self.last_header_ptr as u64,
                err,
            )
        })?;

        // writes 0's at CRC offset
        unsafe {
            let ptr = buf.as_mut_ptr();
            std::ptr::copy_nonoverlapping([0; 4].as_ptr(), ptr.add(22), 4);
        }
        let res = self.write_last_crc(&buf).await;
        drop(buf);
        res
    }

    /// Writes buffer into segment part of stream at current cursor
    /// recalculates checksum
    pub async fn write_stream(&mut self, buf: &[u8]) -> Result<(), Corruption> {
        self.check_cursor().await?;

        let remaining_in_segment = self.page_left();
        let (current_chunk, remaining_data) = if buf.len() > remaining_in_segment {
            buf.split_at(remaining_in_segment)
        } else {
            (buf, &[][..])
        };

        let chunk_len = current_chunk.len();
        self.reader
            .write_at_current_offset(current_chunk.to_vec())
            .await?;
        self.cursor += chunk_len;

        if self.cursor == self.segment_size {
            if chunk_len == self.segment_size {
                let mut header = mem::take(&mut self.last_header);
                header.extend(current_chunk);
                self.write_last_crc(&header).await?;
                drop(header);
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
    pub async fn pad_till_end(&mut self) -> Result<(), Corruption> {
        while !self.ends_stream {
            let remaining_segment = self.page_left();
            self.write_stream(&vec![0; remaining_segment]).await?;
        }
        self.write_stream(&vec![0; self.page_left()]).await?;
        Ok(())
    }
    pub async fn rehash_headers(&mut self) -> Result<(), Corruption> {
        while !self.ends_stream {
            self.safe_skip(self.page_left()).await?;
            self.recalculate_last_crc().await?;
            self.check_cursor().await?;
        }
        Ok(())
    }
}
