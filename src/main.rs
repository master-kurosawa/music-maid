use ignore::{WalkBuilder, WalkState};
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_uring::fs::File;

async fn read_with_uring(path: &Path) -> Result<(), Box<dyn Error + Send + Sync>> {
    let file = File::open(path).await?;
    let buf = vec![0; 4096];
    let (_res, _buf) = file.read_at(buf, 0).await;
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
