//! `cut`'s list of positions: `N`, `N-M`, `N-` (from N to the end), `-M` (from the start to M), separated by commas, counted
//! from 1. Parsed into sorted, merged ranges. Pure `no_std` + `alloc`, so it is tested on the host.

use alloc::vec::Vec;

/// An inclusive range of positions from 1; `end` `None` runs to the end of the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    pub end: Option<usize>,
}

/// Why a list was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListError {
    /// Empty, an empty item, or not a number.
    Invalid,
    /// A position of 0: positions count from 1.
    Zero,
    /// `N-M` with N greater than M.
    Decreasing,
}

fn number(text: &str) -> Result<usize, ListError> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ListError::Invalid);
    }
    match text.parse::<usize>() {
        Ok(0) => Err(ListError::Zero),
        Ok(n) => Ok(n),
        Err(_) => Ok(usize::MAX), // too large to be a position on any line
    }
}

/// The ranges `list` names, sorted, with overlapping and adjacent ones merged.
pub fn parse(list: &str) -> Result<Vec<Range>, ListError> {
    let mut ranges = Vec::new();
    for item in list.split(',') {
        let range = match item.split_once('-') {
            None => {
                let n = number(item)?;
                Range {
                    start: n,
                    end: Some(n),
                }
            }
            Some(("", "")) => return Err(ListError::Invalid),
            Some(("", to)) => Range {
                start: 1,
                end: Some(number(to)?),
            },
            Some((from, "")) => Range {
                start: number(from)?,
                end: None,
            },
            Some((from, to)) => {
                let (from, to) = (number(from)?, number(to)?);
                if from > to {
                    return Err(ListError::Decreasing);
                }
                Range {
                    start: from,
                    end: Some(to),
                }
            }
        };
        ranges.push(range);
    }
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match merged.last_mut() {
            Some(last) if last.end.is_none_or(|e| r.start <= e.saturating_add(1)) => {
                last.end = match (last.end, r.end) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => None,
                };
            }
            _ => merged.push(r),
        }
    }
    Ok(merged)
}

/// Whether position `n` (from 1) is in `ranges`.
pub fn contains(ranges: &[Range], n: usize) -> bool {
    ranges
        .iter()
        .any(|r| n >= r.start && r.end.is_none_or(|e| n <= e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: usize, end: Option<usize>) -> Range {
        Range { start, end }
    }

    #[test]
    fn single_positions_and_ranges() {
        assert_eq!(parse("3"), Ok(vec![r(3, Some(3))]));
        assert_eq!(parse("2-5"), Ok(vec![r(2, Some(5))]));
        assert_eq!(parse("4-"), Ok(vec![r(4, None)]));
        assert_eq!(parse("-3"), Ok(vec![r(1, Some(3))]));
    }

    #[test]
    fn several_items_are_sorted_and_merged() {
        assert_eq!(parse("5,1"), Ok(vec![r(1, Some(1)), r(5, Some(5))]));
        assert_eq!(parse("1-3,2-5"), Ok(vec![r(1, Some(5))]));
        assert_eq!(parse("1-2,3-4"), Ok(vec![r(1, Some(4))])); // adjacent
        assert_eq!(parse("1,3"), Ok(vec![r(1, Some(1)), r(3, Some(3))]));
        assert_eq!(parse("2,2"), Ok(vec![r(2, Some(2))]));
        assert_eq!(parse("3-,1-2"), Ok(vec![r(1, None)]));
        assert_eq!(parse("1-,5"), Ok(vec![r(1, None)]));
    }

    #[test]
    fn refusals() {
        assert_eq!(parse(""), Err(ListError::Invalid));
        assert_eq!(parse("1,,2"), Err(ListError::Invalid));
        assert_eq!(parse("a"), Err(ListError::Invalid));
        assert_eq!(parse("1-a"), Err(ListError::Invalid));
        assert_eq!(parse("-"), Err(ListError::Invalid));
        assert_eq!(parse("0"), Err(ListError::Zero));
        assert_eq!(parse("0-3"), Err(ListError::Zero));
        assert_eq!(parse("5-2"), Err(ListError::Decreasing));
        assert_eq!(parse("1-2-3"), Err(ListError::Invalid));
    }

    #[test]
    fn membership() {
        let ranges = parse("2-3,6-").unwrap();
        let picked: Vec<usize> = (1..=8).filter(|&n| contains(&ranges, n)).collect();
        assert_eq!(picked, [2, 3, 6, 7, 8]);
        assert!(contains(&parse("-2").unwrap(), 1));
        assert!(!contains(&parse("-2").unwrap(), 3));
    }

    #[test]
    fn a_huge_number_is_a_position_no_line_reaches() {
        let ranges = parse("99999999999999999999999").unwrap();
        assert!(!contains(&ranges, 1000));
    }
}
