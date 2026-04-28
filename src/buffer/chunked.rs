use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::PathBuf;
use super::{Buffer, LineBytes};
use crate::index::SparseIndex;

const CHUNK_SIZE: usize = 8 * 1024 * 1024; // 8MB

/// File-backed buffer for large files and tail mode.
/// The index is built externally (by IndexBuilder) and injected via `update_index`.
pub struct ChunkedBuffer {
    path: PathBuf,
    index: SparseIndex,
    file_len: u64,
}

impl ChunkedBuffer {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        let meta = std::fs::metadata(&path)?;
        let mut buf = Self {
            path,
            index: SparseIndex::new(),
            file_len: meta.len(),
        };
        buf.bootstrap_index()?;
        Ok(buf)
    }

    /// Sync-scan the first 8MB to build an initial index so the viewport
    /// renders immediately while the background indexer continues.
    fn bootstrap_index(&mut self) -> anyhow::Result<()> {
        use std::io::Read;
        let file = std::fs::File::open(&self.path)?;
        let mut reader = std::io::BufReader::with_capacity(CHUNK_SIZE, file);
        let mut buf = vec![0u8; CHUNK_SIZE];
        let n = reader.read(&mut buf)?;
        let chunk = &buf[..n];

        self.index.push(0, 0);
        let mut line_no: u64 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            if b == b'\n' {
                line_no += 1;
                self.index.push(line_no, i as u64 + 1);
            }
        }
        Ok(())
    }

    pub fn update_index(&mut self, index: SparseIndex) {
        self.index = index;
    }

    pub fn append_to_index(&mut self, base_line: u64, all_offsets: &[u64]) {
        self.index.push_batch(base_line, all_offsets);
    }

    pub fn set_file_len(&mut self, len: u64) {
        self.file_len = len;
    }

    /// Read raw bytes at a specific byte range from the file.
    fn read_at(&self, offset: u64, max_bytes: usize) -> anyhow::Result<Vec<u8>> {
        let file = File::open(&self.path)?;
        let mut reader = BufReader::with_capacity(CHUNK_SIZE.min(max_bytes + 1), file);
        reader.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; max_bytes];
        let n = reader.read(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Seek to the start of a line via the index, then read until newline.
    fn read_line_at_offset(&self, offset: u64) -> Option<LineBytes> {
        let buf = self.read_at(offset, 65536).ok()?;
        let end = buf.iter().position(|&b| b == b'\n').unwrap_or(buf.len());
        Some(buf[..end].to_vec())
    }
}

impl Buffer for ChunkedBuffer {
    fn read_line(&self, line_no: u64) -> Option<LineBytes> {
        if self.index.total_lines == 0 && line_no > 0 {
            return None;
        }
        let (block_offset, skip) = self.index.block_for_line(line_no);

        if skip == 0 {
            return self.read_line_at_offset(block_offset);
        }

        // Need to scan past `skip` newlines starting from block_offset
        let scan_buf = self.read_at(block_offset, 65536 * (skip as usize + 1)).ok()?;
        let mut pos = 0usize;
        let mut skipped = 0u64;
        while skipped < skip {
            let nl = scan_buf[pos..].iter().position(|&b| b == b'\n')?;
            pos += nl + 1;
            skipped += 1;
        }
        let end = scan_buf[pos..].iter().position(|&b| b == b'\n')
            .map(|i| pos + i)
            .unwrap_or(scan_buf.len());
        Some(scan_buf[pos..end].to_vec())
    }

    fn line_count(&self) -> u64 {
        self.index.total_lines
    }

    fn byte_offset(&self, line_no: u64) -> u64 {
        let (offset, _) = self.index.block_for_line(line_no);
        offset
    }
}
