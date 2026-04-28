use std::path::PathBuf;
use std::time::Duration;
use crossbeam_channel::Sender;

#[derive(Debug, Clone)]
pub enum FilePollEvent {
    Grew { new_len: u64 },
    Rotated,
}

pub struct FilePoller;

impl FilePoller {
    pub fn spawn(path: PathBuf, tx: Sender<FilePollEvent>, interval_ms: u64) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            run_poller(path, tx, interval_ms);
        })
    }
}

fn run_poller(path: PathBuf, tx: Sender<FilePollEvent>, interval_ms: u64) {
    use std::fs;

    let interval = Duration::from_millis(interval_ms);
    let mut last_len: u64 = 0;
    let mut last_inode: Option<u64> = None;

    loop {
        std::thread::sleep(interval);

        match fs::metadata(&path) {
            Ok(meta) => {
                let new_len = meta.len();

                #[cfg(unix)]
                let new_inode = {
                    use std::os::unix::fs::MetadataExt;
                    Some(meta.ino())
                };
                #[cfg(not(unix))]
                let new_inode: Option<u64> = None;

                // Detect rotation (inode changed or file shrank)
                let rotated = new_len < last_len
                    || (new_inode.is_some() && last_inode.is_some() && new_inode != last_inode);

                if rotated {
                    last_len = new_len;
                    last_inode = new_inode;
                    if tx.send(FilePollEvent::Rotated).is_err() {
                        return;
                    }
                } else if new_len > last_len {
                    last_len = new_len;
                    last_inode = new_inode;
                    if tx.send(FilePollEvent::Grew { new_len }).is_err() {
                        return;
                    }
                } else {
                    last_inode = new_inode;
                }
            }
            Err(_) => {
                // File temporarily unavailable (e.g., during rotation)
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}
