use crate::{
    db::vorbis::FLAC_MARKER,
    formats::{
        flac::parse_flac,
        opus_ogg::{parse_ogg_pages, OGG_MARKER},
    },
    queue::TaskQueue,
};
use ignore::{WalkBuilder, WalkState};
use std::{cmp::min, sync::Mutex};
use std::{
    io::{self},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::Semaphore;
use tokio_uring::fs::File;

const BASE_SIZE: usize = 8196;

pub struct ThrottleConfig {
    max_concurrent_tasks: usize,
}

impl ThrottleConfig {
    pub fn new(max_concurrent_tasks: usize) -> Self {
        Self {
            max_concurrent_tasks,
        }
    }
}

#[derive(Debug)]
pub struct Corruption {
    pub path: PathBuf,
    pub message: String,
    pub file_cursor: u64,
}

impl Corruption {
    pub fn io(path: PathBuf, file_cursor: u64, io_error: io::Error) -> Self {
        Corruption {
            file_cursor,
            path,
            message: format!("IO Error: {io_error:?}"),
        }
    }
}

pub struct UringBufReader {
    pub buf: Vec<u8>,
    pub path: PathBuf,
    pub cursor: u64,
    pub file_ptr: u64,
    pub end_of_file: bool,
    pub file: File,
}

impl UringBufReader {
    /// writes buf at the current offset + cursor then increments cursor.
    pub async fn write_at_current_offset(&mut self, buf: Vec<u8>) -> Result<(), Corruption> {
        let buf_len = buf.len() as u64;
        let (res, buf) = self.file.write_all_at(buf, self.current_offset()).await;
        drop(buf);
        self.skip_read(buf_len, 0).await?;
        res.map_err(|err| Corruption::io(self.path.to_owned(), self.current_offset(), err))
    }
}

impl UringBufReader {
    #[inline]
    pub const fn current_offset(&self) -> u64 {
        self.file_ptr + self.cursor
    }
    pub fn new(file: File, path: PathBuf) -> Self {
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
    /// if cursor is at EOF, returns io::Error instead of allocating
    pub async fn skip_read(&mut self, skip: u64, size: usize) -> Result<(), Corruption> {
        self.cursor += skip;
        if self.cursor as usize >= self.buf.len() {
            if self.end_of_file {
                return Err(Corruption {
                    message: "Reached end of file".to_owned(),
                    path: self.path.to_owned(),
                    file_cursor: self.current_offset(),
                });
            }
            if size > 0 {
                self.read_next(size).await?;
            } else {
                self.buf.drain(0..size);
            }
        }

        Ok(())
    }
    /// skips u64 bytes, then allocates 8196 bytes if needed
    /// if cursor is at EOF, returns io::Error instead of allocating
    pub async fn skip(&mut self, size: u64) -> Result<(), Corruption> {
        self.skip_read(size, BASE_SIZE).await
    }
    /// reads usize bytes at u64 offset.
    /// self.buf gets replaced by new buffer, use self.extend()
    /// in case you don't want to replace the current buf
    /// sets cursor to 0 and file_ptr to offset
    pub async fn read_at_offset(&mut self, size: usize, offset: u64) -> Result<usize, Corruption> {
        self.buf.clear();
        let buf = vec![0; size];
        self.cursor = 0;
        self.file_ptr = offset;
        let (res, mut _buf) = self.file.read_at(buf, offset).await;
        if let Ok(res) = res {
            if res < size {
                self.end_of_file = true;
                _buf.truncate(res);
            }
            self.buf = _buf;
        }
        res.map_err(|err| Corruption::io(self.path.to_owned(), offset, err))
    }
    /// extends the current buffer by usize, reads from file_ptr + buf.len() offset
    pub async fn extend_buf(&mut self, size: usize) -> Result<usize, Corruption> {
        if self.end_of_file {
            return Err(Corruption {
                path: self.path.to_owned(),
                message: "Reached end of file".to_owned(),
                file_cursor: self.current_offset(),
            });
        }
        let buf = vec![0; size];
        let (res, mut _buf) = self
            .file
            .read_at(buf, self.file_ptr + self.buf.len() as u64)
            .await;
        if let Ok(res) = res {
            if res < size {
                self.end_of_file = true;
                _buf.shrink_to(res);
            }
            self.buf.extend(_buf);
        }
        res.map_err(|res| Corruption::io(self.path.to_owned(), self.current_offset(), res))
    }
    /// reads size from current file_ptr + cursor
    /// doesn't read from END OF BUFFER unless cursor is there
    pub async fn read_next(&mut self, size: usize) -> Result<usize, Corruption> {
        self.read_at_offset(size, self.current_offset()).await
    }

    /// gets usize bytes from the current buffer, extending it if needed
    /// extends by missing amount + additional 8196 bytes
    /// returns rest of the buffer if it reaches EOF
    pub async fn get_bytes(&mut self, amount: usize) -> Result<&[u8], Corruption> {
        let buf_len = self.buf.len();
        if buf_len <= amount + self.cursor as usize {
            self.extend_buf(amount + self.cursor as usize - buf_len + BASE_SIZE)
                .await?;
            if self.end_of_file {
                return Err(Corruption {
                    file_cursor: self.current_offset(),
                    message: format!("File ended before {amount} bytes could be read"),
                    path: self.path.to_owned(),
                });
            }
        }
        let slice = self
            .buf
            .get(self.cursor as usize..self.cursor as usize + amount)
            .unwrap();
        self.cursor += amount as u64;
        Ok(slice)
    }

    /// gets usize bytes from the current buffer, extending it if needed
    /// extends by missing amount + additional 8196 bytes
    /// returns rest of the buffer if it reaches EOF
    /// returns part of the buf if EOF is reached before reading full amount
    pub async fn get_bytes_unchecked(&mut self, amount: usize) -> Result<&[u8], Corruption> {
        let buf_len = self.buf.len();
        if buf_len <= amount + self.cursor as usize {
            self.extend_buf(amount + self.cursor as usize - buf_len + BASE_SIZE)
                .await?;
            if self.end_of_file {
                return Ok(self
                    .buf
                    .get(self.cursor as usize..min(self.buf.len(), amount) + self.cursor as usize)
                    .unwrap());
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
    pub async fn read_u32(&mut self) -> Result<u32, Corruption> {
        let bytes = self.get_bytes(4).await?;
        if bytes.len() != 4 {
            return Err(Corruption {
                path: self.path.to_owned(),
                message: "File ended".to_owned(),
                file_cursor: self.current_offset(),
            });
        }
        Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
    }
}

/// Walks directory recursevly in parallel and returns file paths
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

/// requires io_uring runtime
pub async fn load_data_from_paths(paths: Vec<PathBuf>, config: ThrottleConfig) {
    let mut tasks = Vec::new();
    let semaphore = Arc::new(Semaphore::new(config.max_concurrent_tasks));
    let queue = Arc::new(tokio::sync::Mutex::new(TaskQueue::new()));
    for path in paths {
        let semaphore = Arc::clone(&semaphore);
        let queue = Arc::clone(&queue);
        let spawn = tokio_uring::spawn(async move {
            // just dont close semaphore and it will be all alright. right?
            let _permit = semaphore.acquire().await.unwrap();
            read_with_uring(path, queue).await
        });
        tasks.push(spawn);
    }
    for task in tasks {
        let t = task.await.unwrap();
        if let Err(t) = t {
            println!("{t:?}")
        }
    }
    let q = Arc::try_unwrap(queue).unwrap().into_inner();
    TaskQueue::finish(q).await;
}

async fn read_with_uring(
    path: PathBuf,
    queue: Arc<tokio::sync::Mutex<TaskQueue>>,
) -> Result<(), Corruption> {
    let file = File::open(&path)
        .await
        .map_err(|err| Corruption::io(path.to_owned(), 0, err))?;
    let mut reader = UringBufReader::new(file, path);
    let bytes_read = reader.read_next(8196).await?;

    let marker: [u8; 4] = reader.get_bytes(4).await?.try_into().unwrap();

    let file_meta = match marker {
        FLAC_MARKER => {
            if bytes_read < 42 {
                return Err(Corruption {
                    path: reader.path.to_owned(),
                    message: "Not enough bytes for proper flac STREAMINFO.".to_owned(),
                    file_cursor: reader.current_offset(),
                });
            }
            parse_flac(&mut reader).await?
        }
        OGG_MARKER => {
            if bytes_read < 42 {
                return Err(Corruption {
                    path: reader.path.to_owned(),
                    message: "Placeholder (figure out how much minima bytes ogg needs)".to_owned(),
                    file_cursor: reader.current_offset(),
                });
            }
            parse_ogg_pages(&mut reader).await?
        }
        _ => return Ok(()),
    };
    reader.buf.clear();
    queue.lock().await.push(file_meta).await;
    Ok(())
}
