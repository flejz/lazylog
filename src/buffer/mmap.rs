use std::path::Path;
use memmap2::Mmap;
use super::{Buffer, LineBytes, MMAP_THRESHOLD};
use crate::index::SparseIndex;

pub struct MmapBuffer {
    mmap: Mmap,
    index: SparseIndex,
}

impl MmapBuffer {
    /// Returns Err if file is too large for mmap strategy.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)?;
        let meta = file.metadata()?;
        if meta.len() > MMAP_THRESHOLD {
            anyhow::bail!("file too large for mmap strategy ({} bytes)", meta.len());
        }
        let mmap = unsafe { Mmap::map(&file)? };
        let index = build_index(&mmap);
        Ok(Self { mmap, index })
    }
}

fn build_index(data: &[u8]) -> SparseIndex {
    let mut idx = SparseIndex::new();
    idx.push(0, 0);
    let mut line_no: u64 = 0;
    for (i, &b) in data.iter().enumerate() {
        if b == b'\n' {
            line_no += 1;
            idx.push(line_no, i as u64 + 1);
        }
    }
    idx
}

impl Buffer for MmapBuffer {
    fn read_line(&self, line_no: u64) -> Option<LineBytes> {
        let (block_offset, skip) = self.index.block_for_line(line_no);
        let start_search = block_offset as usize;
        if start_search >= self.mmap.len() {
            return None;
        }

        // Skip `skip` newlines from block_offset to find the actual line start
        let mut pos = start_search;
        let data = &self.mmap[..];
        for _ in 0..skip {
            while pos < data.len() && data[pos] != b'\n' {
                pos += 1;
            }
            pos += 1; // skip past the newline
        }
        if pos >= data.len() {
            return None;
        }

        let end = data[pos..].iter().position(|&b| b == b'\n')
            .map(|i| pos + i)
            .unwrap_or(data.len());

        Some(data[pos..end].to_vec())
    }

    fn line_count(&self) -> u64 {
        self.index.total_lines
    }

    fn byte_offset(&self, line_no: u64) -> u64 {
        let (offset, _) = self.index.block_for_line(line_no);
        offset
    }
}
