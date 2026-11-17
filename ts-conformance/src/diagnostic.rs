//! Diagnostics in the shape tsc's baselines use, and offset → line/column mapping.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A diagnostic located by file, 1-based line, and 1-based UTF-16 column, like
/// tsc's `file(line,col): error TScode: message`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Diagnostic {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub code: u32,
    pub message: String,
}

impl Diagnostic {
    /// The identity used for comparison. Columns and message text are not
    /// compared: zelda often anchors an error on a slightly different node
    /// and words messages differently, while the line and code are what
    /// determine whether it found the same problem.
    pub fn key(&self) -> (&str, u32, u32) {
        (&self.file, self.line, self.code)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}({},{}): error TS{}: {}",
            self.file, self.line, self.col, self.code, self.message
        )
    }
}

/// Maps byte offsets in a source text to tsc-style line and column numbers.
pub struct LineIndex<'a> {
    text: &'a str,
    /// Byte offset of the start of each line.
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    /// Line breaks follow tsc's `computeLineStarts`: `\n`, `\r\n`, lone `\r`,
    /// U+2028, and U+2029.
    pub fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        let mut chars = text.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            match c {
                '\r' => {
                    if let Some(&(_, '\n')) = chars.peek() {
                        chars.next();
                        starts.push(i + 2);
                    } else {
                        starts.push(i + 1);
                    }
                }
                '\n' | '\u{2028}' | '\u{2029}' => starts.push(i + c.len_utf8()),
                _ => {}
            }
        }
        Self { text, starts }
    }

    /// 1-based `(line, column)` for a byte offset, with the column counted in
    /// UTF-16 code units as tsc does.
    pub fn line_col(&self, offset: usize) -> (u32, u32) {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.starts.partition_point(|&start| start <= offset) - 1;
        let col = self.text[self.starts[line]..offset].encode_utf16().count();
        (line as u32 + 1, col as u32 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_ascii() {
        let idx = LineIndex::new("let a;\nlet b;\r\nlet c;\rlet d;");
        assert_eq!(idx.line_col(0), (1, 1));
        assert_eq!(idx.line_col(4), (1, 5));
        assert_eq!(idx.line_col(7), (2, 1));
        assert_eq!(idx.line_col(15), (3, 1));
        assert_eq!(idx.line_col(22), (4, 1));
    }

    #[test]
    fn line_col_counts_utf16_units() {
        // 'é' is 2 UTF-8 bytes / 1 UTF-16 unit; '😀' is 4 bytes / 2 units.
        let text = "é😀x";
        let idx = LineIndex::new(text);
        assert_eq!(idx.line_col(text.find('x').unwrap()), (1, 4));
    }

    #[test]
    fn line_col_unicode_line_separators() {
        let idx = LineIndex::new("a\u{2028}b");
        assert_eq!(idx.line_col(4), (2, 1));
    }

    #[test]
    fn line_col_clamps_out_of_range() {
        let idx = LineIndex::new("ab");
        assert_eq!(idx.line_col(100), (1, 3));
    }
}
