use super::{Buffer, LineBytes};

pub struct MultiBuffer {
    buffers: Vec<Box<dyn Buffer + Send>>,
    pub file_names: Vec<String>,
    /// merged[i] = (buf_idx, line_no_in_that_buf)
    merged: Vec<(usize, u64)>,
}

impl MultiBuffer {
    pub fn open(paths: &[std::path::PathBuf]) -> anyhow::Result<Self> {
        use super::{mmap::MmapBuffer, chunked::ChunkedBuffer, MMAP_THRESHOLD};
        let mut buffers: Vec<Box<dyn Buffer + Send>> = Vec::new();
        let mut file_names = Vec::new();

        for path in paths {
            let meta = std::fs::metadata(path)?;
            let name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_owned();
            file_names.push(name);
            if meta.len() <= MMAP_THRESHOLD {
                match MmapBuffer::open(path) {
                    Ok(b) => buffers.push(Box::new(b)),
                    Err(_) => buffers.push(Box::new(ChunkedBuffer::open(path.to_path_buf())?)),
                }
            } else {
                buffers.push(Box::new(ChunkedBuffer::open(path.to_path_buf())?));
            }
        }

        // Build sorted merge index
        let mut all: Vec<(String, usize, u64)> = Vec::new(); // (ts_key, buf_idx, line_no)
        for (bi, buf) in buffers.iter().enumerate() {
            for ln in 0..buf.line_count() {
                let raw = buf.read_line(ln).unwrap_or_default();
                let ts = extract_ts_key(&raw);
                all.push((ts, bi, ln));
            }
        }
        all.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let merged = all.into_iter().map(|(_, bi, ln)| (bi, ln)).collect();

        Ok(Self { buffers, file_names, merged })
    }

    pub fn file_idx_for(&self, merged_line: u64) -> usize {
        self.merged.get(merged_line as usize).map(|&(bi, _)| bi).unwrap_or(0)
    }
}

/// Extract a sortable timestamp key from raw line bytes.
/// Returns empty string for lines without a recognizable timestamp.
fn extract_ts_key(raw: &[u8]) -> String {
    let s = std::str::from_utf8(&raw[..raw.len().min(32)]).unwrap_or("");
    let s = s.trim();
    if s.len() >= 19 {
        let prefix = &s[..19];
        let ok = prefix.chars().enumerate().all(|(i, c)| match i {
            0..=3|5..=6|8..=9|11..=12|14..=15|17..=18 => c.is_ascii_digit(),
            4|7 => c == '-',
            10 => c == 'T' || c == ' ',
            13|16 => c == ':',
            _ => false,
        });
        if ok {
            return format!("{}T{}", &prefix[..10], &prefix[11..19]);
        }
    }
    String::new()
}

impl Buffer for MultiBuffer {
    fn read_line(&self, n: u64) -> Option<LineBytes> {
        let &(bi, ln) = self.merged.get(n as usize)?;
        self.buffers[bi].read_line(ln)
    }
    fn line_count(&self) -> u64 { self.merged.len() as u64 }
    fn byte_offset(&self, _: u64) -> u64 { 0 }
}
