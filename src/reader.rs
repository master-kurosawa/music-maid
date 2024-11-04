use std::{
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    sync::Arc,
};

use ignore::{WalkBuilder, WalkState};
use std::sync::Mutex;
use tokio_uring::fs::File;
const BASE_SIZE: usize = 8196;

pub struct UringBufReader {
    pub buf: Vec<u8>,
    pub path: String,
    pub cursor: u64,
    pub file_ptr: u64,
    pub end_of_file: bool,
    file: File,
}

impl UringBufReader {
    pub fn new(file: File, path: String) -> Self {
        UringBufReader {
            buf: Vec::new(),
            file,
            path,
            end_of_file: false,
            cursor: 0u64,
            file_ptr: 0u64,
        }
    }
    /// skips u64 bytes, then allocates usize bytes if needed
    /// if cursor is at EOF, returns io::Error
    pub async fn skip_read(&mut self, skip: u64, size: usize) -> Result<(), io::Error> {
        self.cursor += skip;
        if self.cursor as usize >= self.buf.len() {
            if self.end_of_file {
                return Err(io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "Reached end of file",
                ));
            }
            self.read_next(size).await?;
        }

        Ok(())
    }
    /// skips u64 bytes, then allocates 8196 bytes if needed
    /// if cursor is at EOF, returns io::Error
    pub async fn skip(&mut self, size: u64) -> Result<(), io::Error> {
        self.skip_read(size, BASE_SIZE).await
    }
    /// reads usize bytes at u64 offset.
    /// self.buf gets replaced by new buffer, use self.extend()
    /// in case you don't want to replace the current buf
    /// sets cursor to 0 and file_ptr to offset
    pub async fn read_at_offset(
        &mut self,
        size: usize,
        offset: u64,
    ) -> Result<usize, std::io::Error> {
        let buf = vec![0; size];
        self.cursor = 0;
        self.file_ptr = offset;
        let (res, _buf) = self.file.read_at(buf, offset).await;
        if let Ok(res) = res {
            if res < size {
                self.end_of_file = true;
            }
            self.buf = _buf;
        }
        res
    }
    /// extends the current buffer by usize, reads from file_ptr + buf.len() offset
    pub async fn extend_buf(&mut self, size: usize) -> Result<usize, io::Error> {
        let buf = vec![0; size];
        let (res, _buf) = self
            .file
            .read_at(buf, self.file_ptr + self.buf.len() as u64)
            .await;
        if let Ok(res) = res {
            if res < size {
                self.end_of_file = true;
            }
            self.buf.extend(_buf);
        }
        res
    }
    /// reads size from current file_ptr + cursor
    /// doesn't read from END OF BUFFER unless cursor is there
    pub async fn read_next(&mut self, size: usize) -> Result<usize, io::Error> {
        self.read_at_offset(size, self.file_ptr + self.cursor).await
    }

    /// gets usize bytes from the current buffer, extending it if needed
    /// extends by missing amount + additional 8196 bytes
    /// returns rest of the buffer if it reaches EOF
    pub async fn get_bytes(&mut self, amount: usize) -> Result<&[u8], io::Error> {
        if self.buf.len() <= amount + self.cursor as usize {
            self.extend_buf(amount + self.cursor as usize - self.buf.len() + BASE_SIZE)
                .await?;
            if self.end_of_file {
                return Ok(self.buf.get(self.cursor as usize..).unwrap());
            }
        }
        let slice = self
            .buf
            .get(self.cursor as usize..self.cursor as usize + amount)
            .unwrap();
        self.cursor += amount as u64;
        Ok(slice)
    }
    /// reads next 4 bytes into BE u32
    pub async fn read_u32(&mut self) -> Result<u32, io::Error> {
        let bytes = self.get_bytes(4).await?;
        if bytes.len() != 4 {
            return Err(io::Error::new(ErrorKind::UnexpectedEof, "File ended"));
        }
        Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
    }
}
pub fn walk_dir(path: &str) -> Vec<PathBuf> {
    let paths: Arc<Mutex<Vec<Arc<PathBuf>>>> = Arc::new(Mutex::new(Vec::new()));
    let builder = WalkBuilder::new(path);
    builder.build_parallel().run(|| {
        Box::new(|path| {
            match path {
                Ok(entry) => {
                    if entry.file_type().unwrap().is_dir() {
                        return WalkState::Continue;
                    }
                    let path = Arc::new(entry.path().to_path_buf());
                    let paths = Arc::clone(&paths);
                    paths.lock().unwrap().push(path);
                }
                Err(_) => panic!(),
            }
            WalkState::Continue
        })
    });
    Arc::try_unwrap(paths)
        .unwrap()
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|path| Arc::try_unwrap(path).unwrap().to_owned())
        .collect::<Vec<PathBuf>>()
}
