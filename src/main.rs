use anyhow::anyhow;
use futures::channel::{mpsc, mpsc::Sender};
use futures::{SinkExt, StreamExt};
use ignore::{WalkBuilder, WalkState};
use sqlx::migrate::MigrateDatabase;
use sqlx::{Pool, Sqlite, SqlitePool};
use std::collections::HashMap;
use std::mem;
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::task::JoinHandle;
use tokio_uring::fs::File;

const FLAC_MARKER: [u8; 4] = [0x66, 0x4C, 0x61, 0x43];
const QUEUE_LIMIT: usize = 50;
const VORBIS_FIELDS_LOWER: [&str; 15] = [
    "title",
    "version",
    "album",
    "tracknumber",
    "artist",
    "performer",
    "copyright",
    "license",
    "organization",
    "description",
    "genre",
    "date",
    "location",
    "contact",
    "isrc",
];

#[allow(non_camel_case_types)]
struct VORBIS_COMMENT_MARKER;
impl VORBIS_COMMENT_MARKER {
    const END_OF_BLOCK: u8 = 0b10000100;
    const MARKER: u8 = 0b00000100;
}

#[allow(non_camel_case_types)]
struct PICTURE_MARKER;
impl PICTURE_MARKER {
    const END_OF_BLOCK: u8 = 0b10000110;
    const MARKER: u8 = 0b00000110;
}

#[derive(Debug, Clone)]
struct MusicFile {
    path: String,
    comments: Vec<VorbisComment>,
}

#[derive(Debug, Clone)]
struct VorbisComment {
    vendor: String,
    title: String,
    version: String,
    album: String,
    tracknumber: String,
    artist: String,
    performer: String,
    copyright: String,
    license: String,
    organization: String,
    description: String,
    genre: String,
    date: String,
    location: String,
    contact: String,
    isrc: String,
}
impl VorbisComment {
    fn init(map: HashMap<String, String>) -> Self {
        let vendor = map.get("vendor").map_or(String::new(), |v| v.to_string());
        let contact = map.get("contact").map_or(String::new(), |v| v.to_string());
        let location = map.get("location").map_or(String::new(), |v| v.to_string());
        let date = map.get("date").map_or(String::new(), |v| v.to_string());
        let genre = map.get("genre").map_or(String::new(), |v| v.to_string());
        let isrc = map.get("isrc").map_or(String::new(), |v| v.to_string());
        let album = map.get("album").map_or(String::new(), |v| v.to_string());
        let version = map.get("version").map_or(String::new(), |v| v.to_string());
        let title = map.get("title").map_or(String::new(), |v| v.to_string());
        let description = map
            .get("description")
            .map_or(String::new(), |v| v.to_string());
        let organization = map
            .get("organization")
            .map_or(String::new(), |v| v.to_string());
        let license = map.get("license").map_or(String::new(), |v| v.to_string());
        let copyright = map
            .get("copyright")
            .map_or(String::new(), |v| v.to_string());
        let performer = map
            .get("performer")
            .map_or(String::new(), |v| v.to_string());
        let artist = map.get("artist").map_or(String::new(), |v| v.to_string());
        let tracknumber = map
            .get("tracknumber")
            .map_or(String::new(), |v| v.to_string());

        VorbisComment {
            title,
            vendor,
            description,
            version,
            album,
            date,
            isrc,
            genre,
            artist,
            license,
            contact,
            location,
            performer,
            copyright,
            tracknumber,
            organization,
        }
    }
}

#[derive(Debug)]
struct Picture {
    picture_type: u32,
    mime: String,
    description: String,
    width: u32,
    height: u32,
    color_depth: u32,
    indexed_color_number: u32,
    // picture_data: Vec<u8>,
}

fn parse_vorbis(
    main_cursor: &usize,
    buf: &[u8],
    block_length: usize,
) -> anyhow::Result<VorbisComment> {
    let cursor = *main_cursor;
    let mut comments = HashMap::new();
    let vorbis_end = cursor + block_length;
    let vorbis_block = &buf[cursor..vorbis_end];
    let vendor_end = 4 + u32::from_le_bytes(vorbis_block[0..4].try_into().unwrap()) as usize;
    comments.insert(
        "vendor".to_string(),
        String::from_utf8_lossy(&vorbis_block[4..vendor_end]).to_string(),
    );
    let comment_list_len =
        u32::from_le_bytes(vorbis_block[vendor_end..vendor_end + 4].try_into().unwrap());
    let mut comment_cursor = vendor_end + 4;
    for _ in 1..=comment_list_len {
        let comment_len = u32::from_le_bytes(
            vorbis_block[comment_cursor..4 + comment_cursor]
                .try_into()
                .unwrap(),
        ) as usize;
        comment_cursor += 4;
        let comment =
            String::from_utf8_lossy(&vorbis_block[comment_cursor..comment_cursor + comment_len])
                .to_lowercase();
        match &comment.split_once('=') {
            Some((key, val)) => {
                if VORBIS_FIELDS_LOWER.contains(key) {
                    comments.insert(key.to_lowercase(), val.to_string());
                } else {
                    comment_cursor += comment_len;
                    continue;
                }
            }
            None => return Err(anyhow!("Corrupted comment")),
        };

        comment_cursor += comment_len;
    }
    /*
         7) [framing_bit] = read a single bit as boolean
         8) if ( [framing_bit] unset or end of packet ) then ERROR
         9) done.
    USE CASE FOR READING FRAMING BIT????
    */
    if (vorbis_block[comment_cursor - 1] & 0x00000001) == 0 {
        return Err(anyhow!(
            "framing bit is 0, lol lmao, everything else works tho"
        ));
    };

    Ok(VorbisComment::init(comments))
}

fn read_u32(cursor: &mut usize, buf: &[u8]) -> anyhow::Result<u32> {
    let bytes = buf
        .get(*cursor..*cursor + 4)
        .ok_or(anyhow!("Buffer too small"))?;
    *cursor += 4;
    Ok(u32::from_be_bytes(
        bytes.try_into().map_err(|_| anyhow!("Invalid slice"))?,
    ))
}

fn parse_picture(cursor: &mut usize, buf: &[u8]) -> anyhow::Result<Picture> {
    let picture_type = read_u32(cursor, buf)?;
    let mime_len = read_u32(cursor, buf)? as usize;
    let mime = String::from_utf8_lossy(
        buf.get(*cursor..*cursor + mime_len)
            .ok_or(anyhow!("Buffer too small"))?,
    );
    *cursor += mime_len;
    let description_len = read_u32(cursor, buf)? as usize;
    let description = String::from_utf8_lossy(
        buf.get(*cursor..*cursor + description_len)
            .ok_or(anyhow!("Buffer too small"))?,
    );
    *cursor += description_len;
    let width = read_u32(cursor, buf)?;
    let height = read_u32(cursor, buf)?;
    let color_depth = read_u32(cursor, buf)?;
    let indexed_color_number = read_u32(cursor, buf)?;
    //let picture_len = read_u32(&mut cursor, buf)? as usize;
    //let picture_data = buf
    //    .get(*cursor..*cursor + picture_len)
    //    .ok_or(anyhow!("Buffer too small"))?
    //    .to_vec();

    Ok(Picture {
        picture_type,
        mime: mime.to_string(),
        description: description.to_string(),
        width,
        height,
        color_depth,
        indexed_color_number,
    })
}

pub enum ParseError {
    EndOfBufer,
}

async fn read_ahead_offset(file: &File, size: usize, offset: u64) -> anyhow::Result<Vec<u8>> {
    let buf = vec![0; size + 8196];
    let (_res, prefix_buf) = file.read_at(buf, offset).await;
    let bytes_read = _res?;
    if bytes_read < size + 4 {
        return Err(anyhow!("Not enough bytes for next header"));
    }
    Ok(prefix_buf)
}

async fn read_with_uring(
    path: &Path,
    queue: Arc<tokio::sync::Mutex<TaskQueue>>,
) -> anyhow::Result<()> {
    let file = File::open(path).await?;
    let mut vorbis_comments: Vec<VorbisComment> = Vec::new();

    let buf = vec![0; 8196];
    let (_res, mut prefix_buf) = file.read_at(buf, 0).await;
    let bytes_read = _res?;

    if prefix_buf[0..4] == FLAC_MARKER {
        if bytes_read < 42 {
            return Err(anyhow!(
                "Not enough bytes for proper flac STREAMINFO, got {}",
                bytes_read
            ));
        }

        let mut cursor = 4;

        loop {
            let header: Box<[u8]> = prefix_buf[cursor..cursor + 4].to_vec().into_boxed_slice();
            let block_length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
            let buf_len = prefix_buf.len();
            cursor += 4;

            match header[0] {
                VORBIS_COMMENT_MARKER::MARKER => {
                    if buf_len <= cursor + block_length {
                        mem::drop(prefix_buf);
                        prefix_buf = read_ahead_offset(&file, block_length, cursor as u64).await?;
                        cursor = 0;
                    }
                    vorbis_comments.push(parse_vorbis(&cursor, &prefix_buf, block_length)?);
                    cursor += block_length;
                }
                VORBIS_COMMENT_MARKER::END_OF_BLOCK => {
                    if buf_len <= cursor + block_length {
                        mem::drop(prefix_buf);
                        prefix_buf =
                            read_ahead_offset(&file, block_length - 8196, cursor as u64).await?;
                        cursor = 0;
                    }

                    vorbis_comments.push(parse_vorbis(&cursor, &prefix_buf, block_length)?);
                    break;
                }
                PICTURE_MARKER::MARKER => {
                    // mime and description can be up to 2^32 bytes each for some reason
                    // Im assigning max 8196 bytes for the whole meta and i dont care
                    if buf_len <= cursor + 8196 {
                        mem::drop(prefix_buf);
                        prefix_buf = read_ahead_offset(&file, 4, cursor as u64).await?;
                        cursor = 0;
                    }
                    let picture = parse_picture(&mut cursor, &prefix_buf)?;
                }
                PICTURE_MARKER::END_OF_BLOCK => {
                    // mime and description can be up to 2^32 bytes each for some reason
                    // Im assigning max 8196 bytes for the whole meta and i dont care
                    if buf_len <= cursor + 8196 {
                        mem::drop(prefix_buf);
                        prefix_buf = read_ahead_offset(&file, 0, cursor as u64).await?;
                        cursor = 0;
                    }
                    let picture = parse_picture(&mut cursor, &prefix_buf)?;
                    break;
                }
                n if n >= 128 => {
                    // reached end marker
                    break;
                }
                _ => {
                    // ignored block
                    cursor += block_length;
                    if buf_len <= cursor + 4 {
                        mem::drop(prefix_buf);
                        prefix_buf = read_ahead_offset(&file, 0, cursor as u64).await?;
                        cursor = 0;
                    }
                }
            }
        }
    }
    mem::drop(prefix_buf);

    let path = path.to_string_lossy().to_string();
    queue
        .lock()
        .await
        .push(MusicFile {
            path,
            comments: vorbis_comments,
        })
        .await;
    Ok(())
}

#[derive(Debug)]
struct TaskQueue {
    pool: Pool<Sqlite>,
    queue: Vec<MusicFile>,
    executor: JoinHandle<()>,
    sender: Sender<Option<Vec<MusicFile>>>,
}
impl TaskQueue {
    pub fn new(pool: Pool<Sqlite>) -> Self {
        let (sender, mut receiver) = mpsc::channel::<Option<Vec<MusicFile>>>(100);
        let executor = tokio::spawn(async move {
            let pool = SqlitePool::connect("sqlite://music.db").await.unwrap();
            while let Some(queue) = receiver.next().await {
                match queue {
                    Some(queue) => TaskQueue::insert(queue, &pool).await,
                    None => break,
                }
            }
        });
        TaskQueue {
            queue: Vec::new(),
            executor,
            sender,
            pool,
        }
    }
    pub async fn finish(self) {
        let _ = self.sender.to_owned().send(Some(self.queue.clone())).await;
        let _ = self.sender.to_owned().send(None).await;
        self.queue.to_owned().clear();
        self.queue.to_owned().shrink_to_fit();
        let _ = self.executor.await;
    }
    pub async fn insert(queue: Vec<MusicFile>, pool: &Pool<Sqlite>) {
        let mut transaction = pool.begin().await.unwrap();
        for item in queue {
            let file_id = sqlx::query("INSERT INTO files(path) VALUES(?);")
                .bind(&item.path)
                .execute(&mut *transaction)
                .await
                .unwrap()
                .last_insert_rowid();
            for comment in &item.comments {
                sqlx::query(
                    "
                    INSERT INTO 
                    vorbis_comments(
                        file_id,
                        vendor,
                        title,
                        version,
                        album,
                        tracknumber,
                        artist,
                        performer,
                        copyright,
                        license,
                        organization,
                        description,
                        genre,
                        date,
                        location,
                        contact,
                        isrc)
                    VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?);",
                )
                .bind(file_id)
                .bind(&comment.vendor)
                .bind(&comment.title)
                .bind(&comment.version)
                .bind(&comment.album)
                .bind(&comment.tracknumber)
                .bind(&comment.artist)
                .bind(&comment.performer)
                .bind(&comment.copyright)
                .bind(&comment.license)
                .bind(&comment.organization)
                .bind(&comment.description)
                .bind(&comment.genre)
                .bind(&comment.date)
                .bind(&comment.location)
                .bind(&comment.contact)
                .bind(&comment.isrc)
                .execute(&mut *transaction)
                .await
                .unwrap();
            }
        }
        transaction.commit().await.unwrap();
    }
    pub async fn push(&mut self, item: MusicFile) {
        self.queue.push(item);
        if self.queue.len() >= QUEUE_LIMIT {
            let _ = self.sender.send(Some(self.queue.clone())).await;
            self.queue.clear();
            self.queue.shrink_to_fit();
        }
    }
}

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    sysinfo::set_open_files_limit(10000);
    let paths: Arc<Mutex<Vec<Arc<PathBuf>>>> = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = Vec::new();
    let builder = WalkBuilder::new("./tmp");
    builder.build_parallel().run(|| {
        Box::new(|path| {
            match path {
                Ok(entry) => {
                    if entry.file_type().unwrap().is_dir() {
                        return WalkState::Continue;
                    }
                    let path = Arc::new(entry.path().to_path_buf());
                    let clone_xd = Arc::clone(&paths);
                    clone_xd.lock().unwrap().push(path);
                }
                Err(_) => panic!(),
            }
            WalkState::Continue
        })
    });
    tokio_uring::start(async {
        let url = "sqlite://music.db";
        if !Sqlite::database_exists(url).await.unwrap_or(false) {
            Sqlite::create_database(url).await.unwrap();
        }

        let pool = SqlitePool::connect(url).await.unwrap();
        sqlx::query(
            "
    CREATE TABLE IF NOT EXISTS vorbis_comments (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        file_id INTEGER NOT NULL,
        vendor TEXT NOT NULL,
        title TEXT NOT NULL,
        version TEXT NOT NULL,
        album TEXT NOT NULL,
        tracknumber TEXT NOT NULL,
        artist TEXT NOT NULL,
        performer TEXT NOT NULL,
        copyright TEXT NOT NULL,
        license TEXT NOT NULL,
        organization TEXT NOT NULL,
        description TEXT NOT NULL,
        genre TEXT NOT NULL,
        date TEXT NOT NULL,
        location TEXT NOT NULL,
        contact TEXT NOT NULL,
        isrc TEXT NOT NULL,
        FOREIGN KEY (file_id) REFERENCES files(id)
    );
",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL
);
",
        )
        .execute(&pool)
        .await
        .unwrap();
        let queue = Arc::new(tokio::sync::Mutex::new(TaskQueue::new(pool)));
        for entry in paths.lock().into_iter() {
            entry.clone().into_iter().for_each(|path| {
                let queue = Arc::clone(&queue);
                let spawn =
                    tokio_uring::spawn(async move { read_with_uring(&path, queue).await.unwrap() });

                tasks.push(spawn);
            });
        }
        for task in tasks.into_iter().rev() {
            task.await.unwrap();
        }
        let q = Arc::try_unwrap(queue).unwrap().into_inner();
        TaskQueue::finish(q).await;
    });

    Ok(())
}
