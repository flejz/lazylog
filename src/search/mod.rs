pub mod query;

use crate::buffer::Buffer;
use query::SearchQuery;

pub const OVERLAP: usize = 4096;

pub struct SearchEngine;

impl SearchEngine {
    /// Search forward from `from_line`. Returns (line_no, match_start, match_end).
    pub fn search_forward(
        buf: &dyn Buffer,
        from_line: u64,
        query: &SearchQuery,
    ) -> Option<(u64, usize, usize)> {
        let total = buf.line_count();
        let mut ln = from_line;
        while ln < total {
            if let Some(bytes) = buf.read_line(ln) {
                if let Some((start, end)) = query.find(&bytes) {
                    return Some((ln, start, end));
                }
            }
            ln += 1;
        }
        None
    }

    /// Search backward from `from_line` (inclusive). Returns (line_no, match_start, match_end).
    pub fn search_backward(
        buf: &dyn Buffer,
        from_line: u64,
        query: &SearchQuery,
    ) -> Option<(u64, usize, usize)> {
        let mut ln = from_line;
        loop {
            if let Some(bytes) = buf.read_line(ln) {
                let matches = query.find_all(&bytes);
                if let Some(&(start, end)) = matches.last() {
                    return Some((ln, start, end));
                }
            }
            if ln == 0 { break; }
            ln -= 1;
        }
        None
    }

    /// Collect all matches with byte ranges in [from_line, to_line).
    /// Returns (matches, truncated). Stops at `limit` to avoid blocking on huge files.
    pub fn collect_matches(
        buf: &dyn Buffer,
        query: &SearchQuery,
        from_line: u64,
        to_line: u64,
        limit: usize,
    ) -> (Vec<(u64, usize, usize)>, bool) {
        let mut result = Vec::new();
        let end = to_line.min(buf.line_count());
        for ln in from_line..end {
            if result.len() >= limit {
                return (result, true);
            }
            if let Some(bytes) = buf.read_line(ln) {
                if let Some((start, end_byte)) = query.find(&bytes) {
                    result.push((ln, start, end_byte));
                }
            }
        }
        (result, false)
    }
}

/// Chunk-overlap match scanner for raw byte slices.
/// Returns adjusted byte offsets (relative to the non-overlap start).
pub fn scan_chunk_for_matches(
    data: &[u8],
    overlap_prefix_len: usize,
    query: &SearchQuery,
) -> Vec<usize> {
    query.find_all(data)
        .into_iter()
        .filter(|&(start, _)| start >= overlap_prefix_len)
        .map(|(start, _)| start - overlap_prefix_len)
        .collect()
}
