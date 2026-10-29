// Adapted for ALA performance experiments from Mesa Hills Research GPUI.
// Source: mesa-hills-research/gpui@30d883237d5f913f2d11cfcb938bd7697da88aa6
// Apache-2.0; retains the GPUI and Mesa Hills Research attribution.
//! Shaping a long line in segments.
//!
//! This prototype caches pieces cut after spaces, keyed by text, size and font runs.
//! Native verification finds that joining their widths and glyph positions changes
//! floating-point accumulation and can change wrapping. Glyph identities alone do
//! not establish layout equivalence.
//!
//! Where to cut depends on the text around the cut alone, so that a change moves the cuts near it
//! and leaves the others where they were.

use ala::{FontRun, LineLayout, ShapedGlyph, ShapedRun};
use ala::{Pixels, point};
use smallvec::SmallVec;
use std::{ops::Range, sync::Arc};

/// Lines shorter than this are shaped whole.
pub(super) const MIN_LINE_LEN: usize = 1024;

/// A segment is at least this long, unless it is the last.
const MIN_SEGMENT_LEN: usize = 128;

/// A segment ends at the first place to cut after it reaches this length.
const MAX_SEGMENT_LEN: usize = 2048;

/// A place to cut is used when the hash of the two words before it begins with this many zero
/// bits, which is one place in 32 on average.
const CUT_BITS: u32 = 5;

/// Where to cut `text` into segments, as byte offsets inside the text in increasing order.
///
/// Returns `None` when the text is short, has no place to cut, or has characters whose layout
/// depends on the text around them in more than the word before them. Those are right-to-left
/// text and bidirectional controls, which reorder across words, and tabs and other control
/// characters, whose width depends on where in the line they fall.
///
/// Cuts fall after spaces at accepted word starts. The filter does not prove
/// font-selection or shaping independence for every accepted script or backend.
pub(super) fn cut_points(text: &str) -> Option<Vec<usize>> {
    if text.len() < MIN_LINE_LEN {
        return None;
    }

    let mut cuts = Vec::new();
    let mut last_cut = 0;
    let mut previous_was_space = false;
    let mut word_hash = WORD_HASH_SEED;
    let mut previous_word_hash = WORD_HASH_SEED;
    let mut prior_word_hash = WORD_HASH_SEED;
    for (ix, character) in text.char_indices() {
        if character == ' ' {
            if !previous_was_space {
                prior_word_hash = previous_word_hash;
                previous_word_hash = word_hash;
                word_hash = WORD_HASH_SEED;
            }
            previous_was_space = true;
            continue;
        }

        if character.is_ascii() {
            if character.is_ascii_control() {
                return None;
            }
        } else if character.is_control() || is_bidirectional(character) {
            return None;
        }

        if previous_was_space {
            let len = ix - last_cut;
            if starts_word(character)
                && len >= MIN_SEGMENT_LEN
                && (len >= MAX_SEGMENT_LEN || is_cut(previous_word_hash, prior_word_hash))
            {
                cuts.push(ix);
                last_cut = ix;
            }
            previous_was_space = false;
        }
        word_hash = hash_character(word_hash, character);
    }

    (!cuts.is_empty()).then_some(cuts)
}

const WORD_HASH_SEED: u32 = 0x811c_9dc5;

/// Whether the words before a place to cut, as hashes, make it one to cut at.
fn is_cut(previous_word_hash: u32, prior_word_hash: u32) -> bool {
    let hash = (previous_word_hash ^ prior_word_hash.rotate_left(13)).wrapping_mul(0x9e37_79b1);
    hash >> (u32::BITS - CUT_BITS) == 0
}

fn hash_character(hash: u32, character: char) -> u32 {
    (hash ^ character as u32).wrapping_mul(0x0100_0193)
}

/// Accepted word starts for this experimental cut heuristic. Their inclusion
/// does not establish an exact native shaping contract across the cut.
fn starts_word(character: char) -> bool {
    match character {
        'a'..='z' | 'A'..='Z' | '0'..='9' => true,
        '(' | '[' | '{' | '"' | '\'' | '`' | '*' | '_' | '#' | '@' | '$' => true,
        // Latin, Greek and Cyrillic letters, without the combining marks among them.
        '\u{c0}'..='\u{24f}'
        | '\u{370}'..='\u{3ff}'
        | '\u{400}'..='\u{481}'
        | '\u{48a}'..='\u{52f}' => character.is_alphabetic(),
        _ => false,
    }
}

/// Whether `character` is written right to left, or controls how text is ordered.
fn is_bidirectional(character: char) -> bool {
    matches!(
        character,
        '\u{590}'..='\u{8ff}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{fb1d}'..='\u{fdff}'
            | '\u{fe70}'..='\u{feff}'
            | '\u{10800}'..='\u{10fff}'
            | '\u{1e800}'..='\u{1efff}'
    )
}

/// Clips the font runs of a line to its segments, one after the other.
pub(super) struct RunClipper<'a> {
    runs: &'a [FontRun],
    /// The first run that ends after the end of the last segment clipped, and where it starts.
    next_run: usize,
    next_run_start: usize,
}

impl<'a> RunClipper<'a> {
    pub(super) fn new(runs: &'a [FontRun]) -> Self {
        Self {
            runs,
            next_run: 0,
            next_run_start: 0,
        }
    }

    /// The font runs that fall inside `range`, with the start of the range taken off. The ranges
    /// must come one after the other from the start of the line.
    pub(super) fn clip(&mut self, range: Range<usize>) -> SmallVec<[FontRun; 1]> {
        let mut clipped = SmallVec::new();
        let mut run_start = self.next_run_start;
        for (ix, run) in self.runs.iter().enumerate().skip(self.next_run) {
            let run_end = run_start + run.len;
            if run_start >= range.end {
                break;
            }
            let len = run_end
                .min(range.end)
                .saturating_sub(run_start.max(range.start));
            if len > 0 {
                clipped.push(FontRun {
                    len,
                    font_id: run.font_id,
                });
            }
            if run_end > range.end {
                // The next segment starts inside this run.
                self.next_run = ix;
                self.next_run_start = run_start;
                return clipped;
            }
            run_start = run_end;
            self.next_run = ix + 1;
            self.next_run_start = run_end;
        }
        clipped
    }
}

/// Puts the layouts of segments side by side, as the layout of the whole line. Each part is a
/// segment's offset in the text and its layout.
pub(super) fn join(
    parts: &[(usize, Arc<LineLayout>)],
    len: usize,
    font_size: Pixels,
) -> LineLayout {
    let mut layout = LineLayout {
        font_size,
        len,
        ascent: parts
            .first()
            .map_or_else(Default::default, |(_, part)| part.ascent),
        descent: parts
            .first()
            .map_or_else(Default::default, |(_, part)| part.descent),
        ..Default::default()
    };
    let glyph_count = parts
        .iter()
        .flat_map(|(_, part)| &part.runs)
        .map(|run| run.glyphs.len())
        .sum::<usize>();
    for (start, part) in parts {
        let x_offset = layout.width;
        for run in &part.runs {
            let glyphs = run.glyphs.iter().map(|glyph| ShapedGlyph {
                id: glyph.id,
                position: point(glyph.position.x + x_offset, glyph.position.y),
                index: glyph.index + start,
                is_emoji: glyph.is_emoji,
            });
            match layout
                .runs
                .last_mut()
                .filter(|last| last.font_id == run.font_id)
            {
                Some(last) => last.glyphs.extend(glyphs),
                None => {
                    let mut run_glyphs = Vec::with_capacity(if layout.runs.is_empty() {
                        glyph_count
                    } else {
                        run.glyphs.len()
                    });
                    run_glyphs.extend(glyphs);
                    layout.runs.push(ShapedRun {
                        font_id: run.font_id,
                        glyphs: run_glyphs,
                    });
                }
            }
        }
        layout.width += part.width;
        layout.ascent = layout.ascent.max(part.ascent);
        layout.descent = layout.descent.max(part.descent);
    }
    layout
}
