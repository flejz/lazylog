pub mod builder;

/// Sparse line-offset index at stride 64.
/// Entry at index `i` stores the byte offset of line `i * STRIDE`.
pub const STRIDE: u64 = 64;

#[derive(Default, Clone)]
pub struct SparseIndex {
    /// offsets[i] = byte offset of line (i * STRIDE)
    offsets: Vec<u64>,
    /// Total lines indexed so far
    pub total_lines: u64,
}

impl SparseIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the byte offset of line `line_no` if it falls on a stride boundary.
    pub fn push(&mut self, line_no: u64, byte_offset: u64) {
        if line_no.is_multiple_of(STRIDE) {
            let expected = (line_no / STRIDE) as usize;
            if self.offsets.len() == expected {
                self.offsets.push(byte_offset);
            }
        }
        if line_no + 1 > self.total_lines {
            self.total_lines = line_no + 1;
        }
    }

    /// Append a batch of (line_no, byte_offset) pairs efficiently.
    pub fn push_batch(&mut self, base_line: u64, offsets: &[u64]) {
        for (i, &off) in offsets.iter().enumerate() {
            self.push(base_line + i as u64, off);
        }
    }

    /// Returns the byte offset of the stride-block containing `line_no`,
    /// plus the number of lines to skip within that block.
    pub fn block_for_line(&self, line_no: u64) -> (u64, u64) {
        if self.offsets.is_empty() {
            return (0, line_no);
        }
        let block = line_no / STRIDE;
        let idx = block.min(self.offsets.len() as u64 - 1) as usize;
        let block_start_line = idx as u64 * STRIDE;
        let skip = line_no.saturating_sub(block_start_line);
        (self.offsets[idx], skip)
    }

    /// Approximate line number for a given byte offset (binary search).
    pub fn line_for_offset(&self, target_offset: u64) -> u64 {
        if self.offsets.is_empty() {
            return 0;
        }
        match self.offsets.binary_search(&target_offset) {
            Ok(i)  => i as u64 * STRIDE,
            Err(i) => {
                let idx = i.saturating_sub(1);
                idx as u64 * STRIDE
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }
}
