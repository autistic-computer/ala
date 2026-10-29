use core::ops::Range;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
    Cr,
    LfCr,
    None,
}

impl LineEnding {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
            Self::LfCr => "\n\r",
            Self::None => "",
        }
    }
}

#[derive(Debug)]
pub struct LineIter<'a> {
    string: &'a str,
    start: usize,
    end: usize,
}

impl<'a> LineIter<'a> {
    pub const fn new(string: &'a str) -> Self {
        Self {
            string,
            start: 0,
            end: string.len(),
        }
    }
}

impl Iterator for LineIter<'_> {
    type Item = (Range<usize>, LineEnding);
    fn next(&mut self) -> Option<Self::Item> {
        let start = self.start;
        match self.string[start..self.end].find(['\r', '\n']) {
            Some(i) => {
                let end = start + i;
                self.start = end;
                let after = &self.string[end..];
                let ending = if after.starts_with("\r\n") {
                    LineEnding::CrLf
                } else if after.starts_with("\n\r") {
                    LineEnding::LfCr
                } else if after.starts_with('\n') {
                    LineEnding::Lf
                } else if after.starts_with('\r') {
                    LineEnding::Cr
                } else {
                    LineEnding::None
                };
                self.start += ending.as_str().len();
                Some((start..end, ending))
            }
            None => {
                if self.start < self.end {
                    self.start = self.end;
                    Some((start..self.end, LineEnding::None))
                } else {
                    None
                }
            }
        }
    }
}

#[test]
fn test_line_iter() {
    let string = "LF\nCRLF\r\nCR\rLFCR\n\rNONE";
    let mut iter = LineIter::new(string);
    assert_eq!(iter.next(), Some((0..2, LineEnding::Lf)));
    assert_eq!(iter.next(), Some((3..7, LineEnding::CrLf)));
    assert_eq!(iter.next(), Some((9..11, LineEnding::Cr)));
    assert_eq!(iter.next(), Some((12..16, LineEnding::LfCr)));
    assert_eq!(iter.next(), Some((18..22, LineEnding::None)));
}
