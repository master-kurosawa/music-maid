use ignore::{WalkBuilder, WalkState};
use nom::{bits, bytes};
use nom::{
    bytes::{complete, streaming},
    IResult,
};
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_uring::fs::File;

const FLAC_MARKER: [u8; 4] = [0x66, 0x4C, 0x61, 0x43];

fn parse(input: &[u8]) -> IResult<&[u8], &[u8]> {
    bytes::streaming::take(4u8)(input)
}

async fn read_with_uring(path: &Path) -> Result<(), Box<dyn Error + Send + Sync>> {
    let file = File::open(path).await?;
    let buf = vec![0; 8196];
    let (_res, prefix_buf) = file.read_at(buf, 0).await;
    if prefix_buf[0..4] == FLAC_MARKER {
        // 1 byte: last | type.
        // 3 byte: byte length of block
        // comment = 0x10000100 or 0x00000100
        let mut cursor = 4;
        loop {
            let header = &prefix_buf[cursor..cursor + 4];
            if header[0] == 0b10000100 || header[0] == 0b00000100 {
                let vorbis_end =
                    cursor + 4 + u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
                let vorbis_block = &prefix_buf[cursor + 4..vorbis_end];
                let vendor_end =
                    4 + u32::from_le_bytes(vorbis_block[0..4].try_into().unwrap()) as usize;
                let vendor_string = String::from_utf8_lossy(&vorbis_block[4..vendor_end]);
                let comment_list_len = u32::from_le_bytes(
                    vorbis_block[vendor_end..vendor_end + 4].try_into().unwrap(),
                );
                let mut comment_cursor = vendor_end + 4;
                for _ in 1..=comment_list_len {
                    let comment_len = u32::from_le_bytes(
                        vorbis_block[comment_cursor..4 + comment_cursor]
                            .try_into()
                            .unwrap(),
                    ) as usize;
                    comment_cursor += 4;
                    let comment = String::from_utf8_lossy(
                        &vorbis_block[comment_cursor..comment_cursor + comment_len],
                    );
                    println!("{comment}");
                    comment_cursor += comment_len;
                }
                /*
                     7) [framing_bit] = read a single bit as boolean
                     8) if ( [framing_bit] unset or end of packet ) then ERROR
                     9) done.
                USE CASE FOR READING FRAMING BIT????
                */
                let framing_bit = vorbis_block[comment_cursor - 1] & 0x00000001;
                println!("{vendor_string}");
                break;
            } else if header[0] >= 128 {
                // if first bit is pos we hit the end without comment
                break;
            }
            cursor += u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
            cursor += 4;
        }
        //println!("{x:?}");
        //let meta_header: &[u8] = &[prefix_buf[6]];
        //let x = &prefix_buf[8 + 34 + 1 + 3..8 + 34 + 1 + 3 + 168];
        //let y = String::from_utf8_lossy(x);
        //println!("{y:?}");
        //let result: IResult<(&[u8], usize), bool> = bits::streaming::bool((meta_header, 0));
        //println!("{result:?}");
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let paths: Arc<Mutex<Vec<Arc<PathBuf>>>> = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = Vec::new();
    let builder = WalkBuilder::new("./tmp");
    builder.build_parallel().run(|| {
        Box::new(|path| {
            match path {
                Ok(entry) => {
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
        for entry in paths.lock().into_iter() {
            entry.clone().into_iter().for_each(|path| {
                let spawn = tokio_uring::spawn(async move { read_with_uring(&path).await });
                tasks.push(spawn);
            });
        }

        for task in tasks {
            let _ = task.await;
        }
    });

    Ok(())
}
