use alloc::vec::Vec;
use unicode_bidi::{bidi_class, BidiClass, BidiInfo, ParagraphInfo};

#[derive(Debug)]
pub struct BidiParagraphs<'text> {
    text: &'text str,
    info: alloc::vec::IntoIter<ParagraphInfo>,
}

impl<'text> BidiParagraphs<'text> {
    pub fn new(text: &'text str) -> Self {
        if text.is_ascii()
            && !text
                .chars()
                .any(|c| c.is_ascii_control() && c != '\n' && c != '\r' && c != '\t')
        {
            let mut paragraphs = Vec::new();
            let mut start = 0;

            for (i, c) in text.char_indices() {
                if c == '\n' {
                    paragraphs.push(ParagraphInfo {
                        range: start..i,
                        level: unicode_bidi::Level::ltr(),
                    });
                    start = i + 1;
                }
            }

            if start < text.len() {
                paragraphs.push(ParagraphInfo {
                    range: start..text.len(),
                    level: unicode_bidi::Level::ltr(),
                });
            }

            let info = paragraphs.into_iter();
            Self { text, info }
        } else {
            let info = BidiInfo::new(text, None);
            let info = info.paragraphs.into_iter();
            Self { text, info }
        }
    }
}

impl<'text> Iterator for BidiParagraphs<'text> {
    type Item = &'text str;

    fn next(&mut self) -> Option<Self::Item> {
        let para = self.info.next()?;
        let paragraph = &self.text[para.range];
        let mut char_indices = paragraph.char_indices();
        char_indices
            .next_back()
            .and_then(|(i, c)| (bidi_class(c) == BidiClass::B).then_some(i))
            .map_or(Some(paragraph), |i| Some(&paragraph[0..i]))
    }
}
