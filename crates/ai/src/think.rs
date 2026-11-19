//! Removes `<think>...</think>` reasoning blocks that some models put in the visible text.
//! Works on a stream: a tag may be split across any two pieces.

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

/// Incremental filter. Feed pieces, collect visible text, call [`ThinkFilter::finish`] at the end.
#[derive(Debug, Default)]
pub struct ThinkFilter {
    hold: String,
    inside: bool,
}

impl ThinkFilter {
    /// A fresh filter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a piece and returns the visible text that is now certain.
    pub fn push(&mut self, piece: &str) -> String {
        self.hold.push_str(piece);
        let mut out = String::new();
        loop {
            if self.inside {
                if let Some(i) = self.hold.find(CLOSE) {
                    self.hold.drain(..i + CLOSE.len());
                    self.inside = false;
                    continue;
                }
                // Keep only what could still be the start of the closing tag.
                let keep = partial_suffix(&self.hold, CLOSE);
                let cut = self.hold.len() - keep;
                self.hold.drain(..cut);
                return out;
            }
            if let Some(i) = self.hold.find(OPEN) {
                out.push_str(&self.hold[..i]);
                self.hold.drain(..i + OPEN.len());
                self.inside = true;
                continue;
            }
            let keep = partial_suffix(&self.hold, OPEN);
            let cut = self.hold.len() - keep;
            out.push_str(&self.hold[..cut]);
            self.hold.drain(..cut);
            return out;
        }
    }

    /// Ends the text. A held partial tag that never completed is real text; an unclosed block
    /// stays hidden.
    pub fn finish(&mut self) -> String {
        if self.inside {
            String::new()
        } else {
            std::mem::take(&mut self.hold)
        }
    }
}

/// Length of the longest suffix of `text` that is a proper prefix of `tag` (ASCII tags, so the
/// cut is always on a character boundary).
fn partial_suffix(text: &str, tag: &str) -> usize {
    (1..tag.len())
        .rev()
        .find(|&n| {
            text.len() >= n && text.is_char_boundary(text.len() - n) && text.ends_with(&tag[..n])
        })
        .unwrap_or(0)
}

/// Convenience for whole strings.
#[must_use]
pub fn strip_think(text: &str) -> String {
    let mut f = ThinkFilter::new();
    let mut out = f.push(text);
    out.push_str(&f.finish());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pieces: &[&str]) -> String {
        let mut f = ThinkFilter::new();
        let mut out = String::new();
        for p in pieces {
            out.push_str(&f.push(p));
        }
        out.push_str(&f.finish());
        out
    }

    #[test]
    fn removes_blocks_and_keeps_the_rest() {
        assert_eq!(run(&["a<think>secret</think>b"]), "ab");
        assert_eq!(run(&["<think>x</think>hi"]), "hi");
        assert_eq!(run(&["no tags here"]), "no tags here");
        assert_eq!(run(&["a<think>1</think>b<think>2</think>c"]), "abc");
    }

    #[test]
    fn every_split_point_gives_the_same_answer() {
        let text = "pre <think>hidden é😀 thoughts</think> post <b>bold</b> a<th";
        let expected = run(&[text]);
        assert_eq!(expected, "pre  post <b>bold</b> a<th");
        for cut in (1..text.len()).filter(|c| text.is_char_boundary(*c)) {
            assert_eq!(run(&[&text[..cut], &text[cut..]]), expected, "cut {cut}");
        }
        let singles: Vec<String> = text.chars().map(String::from).collect();
        let refs: Vec<&str> = singles.iter().map(String::as_str).collect();
        assert_eq!(run(&refs), expected);
    }

    #[test]
    fn an_unclosed_block_stays_hidden_and_a_lone_prefix_is_text() {
        assert_eq!(run(&["a<think>never closed"]), "a");
        assert_eq!(run(&["a<thin"]), "a<thin");
    }
}
