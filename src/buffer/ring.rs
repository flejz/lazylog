use std::collections::VecDeque;
use super::{Buffer, LineBytes};

pub const DEFAULT_RING_CAPACITY: usize = 50_000;

/// Rolling line buffer for stdin. Keeps the last `capacity` lines.
pub struct RingBuffer {
    lines: VecDeque<Vec<u8>>,
    capacity: usize,
    /// Absolute line number of lines[0] in the logical stream
    base_line: u64,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(capacity),
            capacity,
            base_line: 0,
        }
    }

    pub fn push(&mut self, line: Vec<u8>) {
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
            self.base_line += 1;
        }
        self.lines.push_back(line);
    }

    pub fn base_line(&self) -> u64 {
        self.base_line
    }
}

impl Buffer for RingBuffer {
    fn read_line(&self, line_no: u64) -> Option<LineBytes> {
        if line_no < self.base_line {
            return None;
        }
        let idx = (line_no - self.base_line) as usize;
        self.lines.get(idx).cloned()
    }

    fn line_count(&self) -> u64 {
        self.base_line + self.lines.len() as u64
    }

    fn byte_offset(&self, _line_no: u64) -> u64 {
        0 // No byte offsets for ring buffer
    }
}
