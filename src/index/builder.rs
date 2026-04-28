use std::fs::File;
use std::io::{BufReader, Read};
use std::path::PathBuf;
use crossbeam_channel::Sender;

const CHUNK_SIZE: usize = 8 * 1024 * 1024; // 8MB

#[derive(Debug)]
pub enum IndexMsg {
    /// A batch of (base_line, stride_offsets) — one offset per STRIDE lines
    Chunk {
        base_line: u64,
        /// byte offsets for lines base_line, base_line+STRIDE, base_line+2*STRIDE, ...
        stride_offsets: Vec<u64>,
        /// all line byte offsets in this chunk (for the sparse push_batch)
        all_offsets: Vec<u64>,
    },
    /// Sent when the full file has been indexed
    Done { total_lines: u64 },
    /// New tail data: offsets for newly appended lines
    TailChunk { base_line: u64, all_offsets: Vec<u64> },
}

pub struct IndexBuilder;

impl IndexBuilder {
    pub fn spawn(path: PathBuf, start_offset: u64, start_line: u64, tx: Sender<IndexMsg>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            if let Err(e) = run(path, start_offset, start_line, tx) {
                eprintln!("IndexBuilder error: {e}");
            }
        })
    }
}

fn run(path: PathBuf, start_offset: u64, start_line: u64, tx: Sender<IndexMsg>) -> anyhow::Result<()> {
    use std::io::Seek;
    let file = File::open(&path)?;
    let mut reader = BufReader::with_capacity(CHUNK_SIZE, file);
    if start_offset > 0 {
        reader.seek(std::io::SeekFrom::Start(start_offset))?;
    }

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut current_offset = start_offset;
    let mut current_line = start_line;
    let mut all_offsets: Vec<u64> = Vec::new();

    // Always record offset of first line in chunk
    if current_line == 0 || start_offset > 0 {
        all_offsets.push(current_offset);
    }

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];

        for (i, &b) in chunk.iter().enumerate() {
            if b == b'\n' {
                current_line += 1;
                let next_offset = current_offset + i as u64 + 1;
                all_offsets.push(next_offset);

                // Flush batch every 64k lines to keep message size bounded
                if all_offsets.len() >= 65536 {
                    let base = current_line - all_offsets.len() as u64 + 1;
                    let _ = tx.send(IndexMsg::Chunk {
                        base_line: base,
                        stride_offsets: vec![],
                        all_offsets: std::mem::take(&mut all_offsets),
                    });
                }
            }
        }
        current_offset += n as u64;
    }

    // Send remaining
    if !all_offsets.is_empty() {
        let base = current_line.saturating_sub(all_offsets.len() as u64 - 1);
        let _ = tx.send(IndexMsg::Chunk {
            base_line: base,
            stride_offsets: vec![],
            all_offsets,
        });
    }

    let _ = tx.send(IndexMsg::Done { total_lines: current_line });
    Ok(())
}
