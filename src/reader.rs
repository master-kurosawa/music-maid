pub trait UringReader {}
use std::{io, ops::RangeBounds};

use tokio_uring::{
    buf::{BoundedBuf, Slice},
    fs::File,
};

pub struct UringBufReader {
    pub buf: Vec<u8>,
    pub cursor: u64,
    file_ptr: u64,
    file: File,
    end_of_file: bool,
}

impl UringBufReader {
    pub fn new(file: File) -> Self {
        UringBufReader {
            buf: Vec::new(),
            file,
            end_of_file: false,
            cursor: 0u64,
            file_ptr: 0u64,
        }
    }
    pub async fn read_at_offset(
        &mut self,
        size: usize,
        offset: u64,
    ) -> Result<usize, std::io::Error> {
        let buf = vec![0; size];
        self.cursor = 0;
        self.file_ptr = offset;
        let (res, _buf) = self.file.read_at(buf, offset).await;
        if let Some(res) = res.as_ref().ok() {
            if *res < size {
                self.end_of_file = true;
            }
            self.buf = _buf;
        }
        res
    }
    pub async fn extend_buf(&mut self, size: usize) -> Result<usize, io::Error> {
        let buf = vec![0; size];
        let (res, _buf) = self.file.read_at(buf, self.buf.len() as u64).await;
        if let Some(res) = res.as_ref().ok() {
            if *res < size {
                self.end_of_file = true;
            }
            self.buf.extend(_buf);
        }
        res
    }
    pub async fn read_next(&mut self, size: usize) -> Result<usize, io::Error> {
        self.read_at_offset(size, self.file_ptr + self.cursor).await
    }

    pub async fn get_bytes(&mut self, amount: usize) -> Result<&[u8], io::Error> {
        if self.buf.len() <= amount + self.cursor as usize {
            self.extend_buf(self.buf.len() - amount - self.cursor as usize)
                .await?;
            if self.end_of_file {
                return Ok(self.buf.get(self.cursor as usize..).unwrap());
            }
        }
        let slice = self
            .buf
            .get(self.cursor as usize..self.cursor as usize + amount)
            .unwrap();
        self.cursor = self.cursor + amount as u64;
        Ok(slice)
    }
}

async fn x() {
    let file = File::open("xd.x").await.unwrap();
    let z = UringBufReader::new(file);
}
