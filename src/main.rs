use ignore::Walk;
use std::{error::Error, path::Path};
use tokio_uring::fs::File;

async fn read_with_uring(path: &Path) -> Result<(), Box<dyn Error>> {
    let file = File::open(path).await?;
    let buf = vec![0; 4096];
    let (_res, _buf) = file.read_at(buf, 0).await;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    tokio_uring::start(async {
        let mut tasks = Vec::new();

        for entry in Walk::new("../") {
            let path = entry.unwrap().path().to_path_buf();
            let spawn = tokio_uring::spawn(async move { read_with_uring(&path).await });
            tasks.push(spawn);
        }

        for task in tasks {
            let _ = task.await;
        }
    });

    Ok(())
}
