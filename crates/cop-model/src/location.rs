use std::{
    error::Error,
    fmt,
    ops::Range,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocationError {
    ZeroLine,
    ZeroColumn,
    DifferentFiles,
    ReversedOffsets,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceLocation {
    path: PathBuf,
    byte_offset: usize,
    line: u32,
    column: u32,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceSpan {
    start: SourceLocation,
    end: SourceLocation,
}

impl LocationError {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ZeroLine => "line must be greater than zero",
            Self::ZeroColumn => "column must be greater than zero",
            Self::DifferentFiles => "span endpoints must refer to the same file",
            Self::ReversedOffsets => "span start must not follow its end",
        }
    }
}

impl fmt::Display for LocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Error for LocationError {}

impl SourceLocation {
    pub fn new(
        path: impl Into<PathBuf>,
        byte_offset: usize,
        line: u32,
        column: u32,
    ) -> Result<Self, LocationError> {
        if line == 0 {
            return Err(LocationError::ZeroLine);
        }

        if column == 0 {
            return Err(LocationError::ZeroColumn);
        }

        Ok(Self {
            path: path.into(),
            byte_offset,
            line,
            column,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn byte_offset(&self) -> usize {
        self.byte_offset
    }

    pub fn line(&self) -> u32 {
        self.line
    }

    pub fn column(&self) -> u32 {
        self.column
    }
}

impl SourceSpan {
    pub fn new(start: SourceLocation, end: SourceLocation) -> Result<Self, LocationError> {
        if start.path != end.path {
            return Err(LocationError::DifferentFiles);
        }

        if start.byte_offset > end.byte_offset {
            return Err(LocationError::ReversedOffsets);
        }

        Ok(Self { start, end })
    }

    pub fn start(&self) -> &SourceLocation {
        &self.start
    }

    pub fn end(&self) -> &SourceLocation {
        &self.end
    }

    pub fn path(&self) -> &Path {
        self.start.path()
    }

    pub fn byte_range(&self) -> Range<usize> {
        self.start.byte_offset()..self.end.byte_offset()
    }

    pub fn start_line(&self) -> u32 {
        self.start.line()
    }

    pub fn start_column(&self) -> u32 {
        self.start.column()
    }

    pub fn end_line(&self) -> u32 {
        self.end.line()
    }

    pub fn end_column(&self) -> u32 {
        self.end.column()
    }
}

#[cfg(test)]
mod tests {
    use super::{LocationError, SourceLocation, SourceSpan};

    #[test]
    fn rejects_zero_coordinates_and_cross_file_spans() {
        assert_eq!(
            SourceLocation::new("src/lib.rs", 0, 0, 1),
            Err(LocationError::ZeroLine)
        );
        assert_eq!(
            SourceLocation::new("src/lib.rs", 0, 1, 0),
            Err(LocationError::ZeroColumn)
        );

        let first = SourceLocation::new("src/lib.rs", 0, 1, 1).expect("起始位置");
        let second = SourceLocation::new("src/main.rs", 1, 1, 2).expect("结束位置");
        assert_eq!(
            SourceSpan::new(first, second),
            Err(LocationError::DifferentFiles)
        );
    }

    #[test]
    fn rejects_reversed_byte_ranges_and_keeps_valid_ranges_half_open() {
        let start = SourceLocation::new("src/lib.rs", 4, 2, 1).expect("起始位置");
        let end = SourceLocation::new("src/lib.rs", 3, 1, 4).expect("结束位置");
        assert_eq!(
            SourceSpan::new(start, end),
            Err(LocationError::ReversedOffsets)
        );

        let start = SourceLocation::new("src/lib.rs", 3, 1, 2).expect("有效起始位置");
        let end = SourceLocation::new("src/lib.rs", 7, 2, 1).expect("有效结束位置");
        let span = SourceSpan::new(start, end).expect("有效源码范围");
        assert_eq!(span.byte_range(), 3..7);
    }
}
