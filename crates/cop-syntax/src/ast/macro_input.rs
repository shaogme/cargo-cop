use std::ops::Range;

use crate::{Delimiter, Token, TokenKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacroInputStatus {
    Parsed,
    Partial,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MacroTokenTree {
    Leaf {
        kind: TokenKind,
        bounds: Range<usize>,
    },
    Group {
        delimiter: Delimiter,
        bounds: Range<usize>,
        children: Vec<MacroTokenTree>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroInput {
    delimiter: Delimiter,
    bounds: Range<usize>,
    trees: Vec<MacroTokenTree>,
    status: MacroInputStatus,
    balanced: bool,
}

struct MacroTreeReader<'a> {
    tokens: &'a [Token],
    delimiter_pairs: &'a [Option<usize>],
    range_start: usize,
}

impl MacroTreeReader<'_> {
    fn read_trees(
        &self,
        index: &mut usize,
        end: usize,
        balanced: &mut bool,
    ) -> Vec<MacroTokenTree> {
        let mut trees = Vec::new();
        while *index < end {
            let Some(token) = self.tokens.get(*index) else {
                *balanced = false;
                break;
            };
            if let TokenKind::OpenDelimiter(delimiter) = token.kind() {
                if let Some(close) = self
                    .delimiter_pairs
                    .get(*index - self.range_start)
                    .copied()
                    .flatten()
                    .filter(|close| *close < end)
                {
                    let open_index = *index;
                    let mut child_index = open_index + 1;
                    let children = self.read_trees(&mut child_index, close, balanced);
                    let close_token = &self.tokens[close];
                    trees.push(MacroTokenTree::Group {
                        delimiter: *delimiter,
                        bounds: token.start()..close_token.end(),
                        children,
                    });
                    *index = close + 1;
                    continue;
                }
                *balanced = false;
            }
            trees.push(MacroTokenTree::Leaf {
                kind: token.kind().clone(),
                bounds: token.start()..token.end(),
            });
            *index += 1;
        }
        trees
    }
}

impl MacroInput {
    pub(crate) fn from_tokens(
        tokens: &[Token],
        bounds: Range<usize>,
        status: MacroInputStatus,
    ) -> Self {
        let token_range = Self::token_range(tokens, &bounds);
        let Some(range) = token_range else {
            return Self {
                delimiter: Delimiter::Parenthesis,
                bounds,
                trees: Vec::new(),
                status: MacroInputStatus::Partial,
                balanced: false,
            };
        };
        let Some(TokenKind::OpenDelimiter(delimiter)) = tokens.get(range.start).map(Token::kind)
        else {
            return Self {
                delimiter: Delimiter::Parenthesis,
                bounds,
                trees: Vec::new(),
                status: MacroInputStatus::Partial,
                balanced: false,
            };
        };
        let Some(close_index) = range.end.checked_sub(1) else {
            return Self {
                delimiter: *delimiter,
                bounds,
                trees: Vec::new(),
                status: MacroInputStatus::Partial,
                balanced: false,
            };
        };
        let (delimiter_pairs, well_formed) = Self::delimiter_pairs(tokens, range.start, range.end);
        let balanced =
            delimiter_pairs.first().copied().flatten() == Some(close_index) && well_formed;
        let mut index = range.start + 1;
        let mut tree_balanced = balanced;
        let reader = MacroTreeReader {
            tokens,
            delimiter_pairs: &delimiter_pairs,
            range_start: range.start,
        };
        let trees = reader.read_trees(&mut index, close_index, &mut tree_balanced);
        let balanced = balanced && tree_balanced;
        Self {
            delimiter: *delimiter,
            bounds,
            trees,
            status: if balanced {
                status
            } else {
                MacroInputStatus::Partial
            },
            balanced,
        }
    }

    fn token_range(tokens: &[Token], bounds: &Range<usize>) -> Option<Range<usize>> {
        let start = tokens
            .iter()
            .position(|token| token.start() == bounds.start)?;
        let end = tokens
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, token)| token.end() == bounds.end)
            .map(|(index, _)| index + 1)?;
        (start < end).then_some(start..end)
    }

    fn delimiter_pairs(tokens: &[Token], start: usize, end: usize) -> (Vec<Option<usize>>, bool) {
        let mut pairs = vec![None; end - start];
        let mut stack = Vec::new();
        let mut balanced = true;
        for (index, token) in tokens.iter().enumerate().take(end).skip(start) {
            match token.kind() {
                TokenKind::OpenDelimiter(delimiter) => stack.push((index, *delimiter)),
                TokenKind::CloseDelimiter(delimiter)
                    if stack.last().is_some_and(|(_, open)| open == delimiter) =>
                {
                    if let Some((open_index, _)) = stack.pop() {
                        pairs[open_index - start] = Some(index);
                    }
                }
                TokenKind::CloseDelimiter(delimiter) => {
                    balanced = false;
                    if let Some(position) = stack.iter().rposition(|(_, open)| open == delimiter) {
                        let (open_index, _) = stack[position];
                        stack.truncate(position);
                        pairs[open_index - start] = Some(index);
                    }
                }
                _ => {}
            }
        }
        balanced &= stack.is_empty();
        (pairs, balanced)
    }

    pub fn delimiter(&self) -> Delimiter {
        self.delimiter
    }

    pub fn bounds(&self) -> Range<usize> {
        self.bounds.clone()
    }

    pub fn trees(&self) -> &[MacroTokenTree] {
        &self.trees
    }

    pub fn status(&self) -> MacroInputStatus {
        self.status
    }

    pub fn is_balanced(&self) -> bool {
        self.balanced
    }
}

impl MacroTokenTree {
    pub fn bounds(&self) -> Range<usize> {
        match self {
            Self::Leaf { bounds, .. } | Self::Group { bounds, .. } => bounds.clone(),
        }
    }

    pub fn delimiter(&self) -> Option<Delimiter> {
        match self {
            Self::Group { delimiter, .. } => Some(*delimiter),
            Self::Leaf { .. } => None,
        }
    }

    pub fn kind(&self) -> Option<&TokenKind> {
        match self {
            Self::Leaf { kind, .. } => Some(kind),
            Self::Group { .. } => None,
        }
    }

    pub fn children(&self) -> &[MacroTokenTree] {
        match self {
            Self::Group { children, .. } => children,
            Self::Leaf { .. } => &[],
        }
    }
}
