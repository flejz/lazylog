pub mod mmap;
pub mod chunked;
pub mod ring;
pub mod multi;

/// Line content returned by buffers. Avoids copying when possible.
pub type LineBytes = Vec<u8>;

pub trait Buffer: Send {
    /// Returns raw bytes for line `line_no` (0-based). None if out of range.
    fn read_line(&self, line_no: u64) -> Option<LineBytes>;

    /// Number of lines currently known (may grow in tail mode).
    fn line_count(&self) -> u64;

    /// Byte offset of line `line_no`, or 0 if unknown.
    fn byte_offset(&self, line_no: u64) -> u64;
}

pub const MMAP_THRESHOLD: u64 = 256 * 1024 * 1024; // 256MB
