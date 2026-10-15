use std::{
    ops::Range,
    path::{Path, PathBuf},
};

use cop_model::{LocationError, SourceLocation, SourceSpan};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFile {
    path: PathBuf,
    text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(path: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        let text = text.into();
        let mut line_starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }
        Self {
            path: path.into(),
            text,
            line_starts,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn line_range(&self, line: u32) -> Option<Range<usize>> {
        let line_index = usize::try_from(line).ok()?.checked_sub(1)?;
        let start = *self.line_starts.get(line_index)?;
        let mut end = self
            .line_starts
            .get(line_index + 1)
            .copied()
            .unwrap_or(self.text.len());
        if end > start && self.text.as_bytes().get(end - 1) == Some(&b'\n') {
            end -= 1;
        }
        if end > start && self.text.as_bytes().get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
        Some(start..end)
    }

    pub fn line_text(&self, line: u32) -> Option<&str> {
        let range = self.line_range(line)?;
        self.text.get(range)
    }

    pub fn slice(&self, start: usize, end: usize) -> Option<&str> {
        self.text.get(start..end)
    }

    pub fn line_column(&self, offset: usize) -> Option<(u32, u32)> {
        if offset > self.text.len() || !self.text.is_char_boundary(offset) {
            return None;
        }
        let line_index = self
            .line_starts
            .partition_point(|line_start| *line_start <= offset)
            - 1;
        let line_start = self.line_starts[line_index];
        let line = u32::try_from(line_index + 1).ok()?;
        let column = u32::try_from(self.text[line_start..offset].chars().count() + 1).ok()?;
        Some((line, column))
    }

    pub fn span(&self, start: usize, end: usize) -> Result<SourceSpan, LocationError> {
        let (start_line, start_column) = self
            .line_column(start)
            .ok_or(LocationError::ReversedOffsets)?;
        let (end_line, end_column) = self
            .line_column(end)
            .ok_or(LocationError::ReversedOffsets)?;
        let start = SourceLocation::new(self.path.clone(), start, start_line, start_column)?;
        let end = SourceLocation::new(self.path.clone(), end, end_line, end_column)?;
        SourceSpan::new(start, end)
    }
}
