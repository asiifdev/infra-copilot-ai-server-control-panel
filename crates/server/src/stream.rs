//! Redaction of text that arrives in pieces.
//!
//! The model's answer is stored after being redacted as a whole, so what the browser sees while it
//! streams must be the same text. [`core_domain::redact::redact`] decides by looking at context
//! (a token, the value after `password:`, a quoted value, a private key block that spans lines),
//! and this module does not copy its rules. It uses the redactor itself as the judge:
//!
//! * text is released only up to a cut that comes right after an ASCII value delimiter (the
//!   predicate the redactor uses, [`is_value_delimiter`]) and leaves a non-empty rest that is not
//!   only delimiters;
//! * a cut is accepted only when `redact(prefix) + redact(rest) == redact(everything held)`, so
//!   neither side's meaning depends on the other. Latest candidates are tried first, a few per
//!   attempt; if none passes the text stays held;
//! * the one thing the judge cannot see is text that has not arrived yet turning an earlier
//!   fragment into a secret. The only such case in the redactor is a `-----BEGIN ` marker whose
//!   header is not closed yet, so no cut is made after the start of such a marker;
//! * attempts are scheduled so the total cost stays linear: a long held region is re-examined
//!   only after it has grown by a quarter;
//! * `pending` never grows beyond [`HARD_CAP_BYTES`] (plus one piece). Past it the held text is
//!   redacted whole and all but the last [`TAIL_BYTES`] are released (already redacted); the tail
//!   stays held. An unterminated private key block past the cap is dropped (as the redactor drops
//!   it) until its end marker arrives.
//!
//! Guarantee: the concatenation of everything released equals `redact(whole text)` exactly, as
//! long as no region without a safe cut grows past the hard cap. Past the cap the guarantee is
//! weaker and only this: a secret shape that fits in [`TAIL_BYTES`] and is wholly inside the
//! held text is still redacted. A secret that begins more than [`TAIL_BYTES`] before the forced
//! cut and ends after it is not guaranteed to be hidden; nothing is ever released unredacted.

use core_domain::redact::{
    PEM_BEGIN, PEM_DASHES, REDACTED_PRIVATE_KEY, is_value_delimiter, redact,
};

/// Most text held back. A region with no safe cut that grows past it is redacted and released.
pub(crate) const HARD_CAP_BYTES: usize = 64 * 1024;
/// How much of the end is kept when the cap forces a release.
const TAIL_BYTES: usize = 4096;
/// Below this size every piece triggers an attempt; above it only growth by a quarter does.
const EAGER_BYTES: usize = 4096;
/// Candidate cuts judged per attempt.
const MAX_CANDIDATES: usize = 8;
const END: &str = "-----END ";
/// Possible next characters used to test a cut (see [`survives_probes`]).
const PROBES: [&str; 4] = ["Z", "\"Z", "'Z", " Z"];

/// Collects pieces and releases redacted text when it is safe to.
#[derive(Default)]
pub(crate) struct DeltaRedactor {
    pending: String,
    /// Do not try to cut again before `pending` is this long.
    next_attempt: usize,
    /// Inside a private key block that outgrew the cap: its body is dropped until the end marker.
    swallowing: bool,
}

impl DeltaRedactor {
    /// Adds a piece. Returns redacted text that can be shown now, if any.
    pub(crate) fn push(&mut self, piece: &str) -> Option<String> {
        self.pending.push_str(piece);
        if self.swallowing {
            self.swallow();
            return None;
        }
        let mut out = String::new();
        if self.pending.len() >= self.next_attempt {
            match cut_point(&self.pending) {
                Some(cut) => {
                    out.push_str(&self.take(cut));
                    self.next_attempt = 0;
                }
                None => {
                    self.next_attempt = if self.pending.len() < EAGER_BYTES {
                        0
                    } else {
                        self.pending.len() + self.pending.len() / 4
                    };
                }
            }
        }
        if self.pending.len() > HARD_CAP_BYTES {
            out.push_str(&self.force());
        }
        (!out.is_empty()).then_some(out)
    }

    /// The stream ended: redacts and returns whatever is left.
    pub(crate) fn finish(&mut self) -> Option<String> {
        if self.swallowing {
            self.pending.clear();
            return None;
        }
        let all = self.pending.len();
        let out = self.take(all);
        (!out.is_empty()).then_some(out)
    }

    /// Bytes currently held.
    #[cfg(test)]
    pub(crate) fn held(&self) -> usize {
        self.pending.len()
    }

    fn take(&mut self, cut: usize) -> String {
        let chunk: String = self.pending.drain(..cut).collect();
        redact(&chunk).text
    }

    /// The cap was passed: release all but a redacted tail.
    fn force(&mut self) -> String {
        // An unterminated private key block is dropped to its end, like the redactor does.
        if let Some(start) = open_key_block(&self.pending) {
            let before = self.take(start);
            self.pending.clear();
            self.swallowing = true;
            return format!("{before}{REDACTED_PRIVATE_KEY}");
        }
        let red = redact(&self.pending).text;
        let mut keep = red.len().saturating_sub(TAIL_BYTES);
        while !red.is_char_boundary(keep) {
            keep += 1;
        }
        self.pending = red[keep..].to_owned();
        self.next_attempt = 0;
        red[..keep].to_owned()
    }

    /// Drops text while inside an oversized key block; resumes after its end marker.
    fn swallow(&mut self) {
        if let Some(end) = self.pending.find(END) {
            let tail = end + END.len();
            if let Some(close) = self.pending[tail..].find(PEM_DASHES) {
                let rest = self.pending[tail + close + PEM_DASHES.len()..].to_owned();
                self.pending = rest;
                self.swallowing = false;
                self.next_attempt = 0;
                return;
            }
            self.pending.drain(..end);
            return;
        }
        // Keep only enough to recognise an end marker split between pieces.
        let keep_from = self.pending.len().saturating_sub(END.len() - 1);
        let mut from = keep_from;
        while !self.pending.is_char_boundary(from) {
            from += 1;
        }
        self.pending.drain(..from);
    }
}

/// The start of a private key block that has no end in `text`, if there is one.
fn open_key_block(text: &str) -> Option<usize> {
    let start = text.rfind(PEM_BEGIN)?;
    let block = &text[start..];
    (!block.contains(END) && redact(block).text == REDACTED_PRIVATE_KEY).then_some(start)
}

/// The latest cut of `text` that the redactor agrees is independent of the rest, if any.
fn cut_point(text: &str) -> Option<usize> {
    // After the start of a marker whose header is not closed, later text may reinterpret it.
    let limit = text
        .rfind(PEM_BEGIN)
        .filter(|&m| !text[m + PEM_BEGIN.len()..].contains(PEM_DASHES))
        .unwrap_or(text.len());
    let bytes = text.as_bytes();
    let whole = redact(text).text;
    let mut tried = 0;
    for index in (0..limit.min(bytes.len())).rev() {
        if !is_value_delimiter(bytes[index]) {
            continue;
        }
        let cut = index + 1;
        if cut > limit {
            continue;
        }
        let rest = &bytes[cut..];
        if rest.iter().all(|b| is_value_delimiter(*b)) {
            continue;
        }
        let (head, tail) = text.split_at(cut);
        let mut joined = redact(head).text;
        joined.push_str(&redact(tail).text);
        if joined == whole && survives_probes(head, tail) {
            return Some(cut);
        }
        tried += 1;
        if tried == MAX_CANDIDATES {
            break;
        }
    }
    None
}

/// Text that has not arrived yet can start a value whose first piece is empty so far (a quote
/// right after `password:`, say), which the judge cannot tell from plain text. So the cut must
/// also hold when the rest is continued by a few typical next characters.
fn survives_probes(head: &str, tail: &str) -> bool {
    PROBES.iter().all(|probe| {
        let continued = format!("{tail}{probe}");
        let mut joined = redact(head).text;
        joined.push_str(&redact(&continued).text);
        joined == redact(&format!("{head}{continued}")).text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_pieces<S: AsRef<str>>(pieces: &[S]) -> Vec<String> {
        let mut redactor = DeltaRedactor::default();
        let mut out: Vec<String> = pieces
            .iter()
            .filter_map(|p| redactor.push(p.as_ref()))
            .collect();
        out.extend(redactor.finish());
        out
    }

    fn whole(text: &str) -> String {
        redact(text).text
    }

    fn chunks(text: &str, size: usize) -> Vec<String> {
        let all: Vec<char> = text.chars().collect();
        all.chunks(size)
            .map(|c| c.iter().collect::<String>())
            .collect()
    }

    fn assert_equivalent(text: &str, size: usize) {
        let out = run_pieces(&chunks(text, size)).concat();
        assert_eq!(out, whole(text), "chunk size {size}");
    }

    fn token() -> String {
        format!("{}{}", "sk-", "Zx7Qw2Er9Ty4Ui6Op1As3Df5Gh8Jk0")
    }

    /// PEM markers are built at run time so no key-looking block sits in the repository.
    fn begin(label: &str) -> String {
        format!("{d}BEGIN {label}{d}", d = "-".repeat(5))
    }

    fn end(label: &str) -> String {
        format!("{d}END {label}{d}", d = "-".repeat(5))
    }

    fn pem(label: &str) -> String {
        let body = "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo=";
        format!("{}\n{body}\n{body}\n{}\n", begin(label), end(label))
    }

    #[test]
    fn a_token_is_hidden_at_every_possible_split() {
        let text = format!("My key is {} ok, thanks\nnext line\n", token());
        for (at, _) in text.char_indices() {
            let (a, b) = text.split_at(at);
            let out = run_pieces(&[a, b]).concat();
            assert!(!out.contains(&token()[3..]), "split at {at}: {out}");
            assert_eq!(out, whole(&text), "split at {at}");
        }
        for size in 1..12 {
            assert_equivalent(&text, size);
        }
    }

    #[test]
    fn a_private_key_block_split_across_pieces_and_lines_is_never_shown() {
        let text = format!("Here is the key:\n{}Done.\n", pem("RSA PRIVATE KEY"));
        for size in [1, 5, 17, 40] {
            let out = run_pieces(&chunks(&text, size));
            let all = out.concat();
            assert!(
                !all.contains("QUJDREVG") && !all.contains("BEGIN RSA"),
                "{all}"
            );
            assert_eq!(all, whole(&text), "size {size}");
            assert!(out.iter().all(|piece| !piece.contains("QUJD")));
        }
    }

    #[test]
    fn a_marker_header_cut_between_its_words_is_not_released_early() {
        let text = format!("x\n{}tail\n", pem("OPENSSH PRIVATE KEY"));
        for at in 0..text.len() {
            if !text.is_char_boundary(at) {
                continue;
            }
            let (a, b) = text.split_at(at);
            let out = run_pieces(&[a, b]).concat();
            assert!(!out.contains("QUJD"), "split at {at}: {out}");
            assert_eq!(out, whole(&text), "split at {at}");
        }
    }

    #[test]
    fn an_open_block_holds_everything_and_an_unfinished_one_is_dropped_at_the_end() {
        let mut redactor = DeltaRedactor::default();
        let first = redactor.push(&format!("before\n{}\nAAAA\n", begin("OPENSSH PRIVATE KEY")));
        assert!(first.as_deref().is_none_or(|t| !t.contains("AAAA")));
        assert_eq!(redactor.push("BBBB\nCCCC\n"), None);
        let rest = redactor.finish().unwrap_or_default();
        let all = format!("{}{rest}", first.unwrap_or_default());
        assert_eq!(
            all,
            whole(&format!(
                "before\n{}\nAAAA\nBBBB\nCCCC\n",
                begin("OPENSSH PRIVATE KEY")
            ))
        );
    }

    #[test]
    fn a_begin_marker_after_the_end_of_a_block_on_the_same_line_is_still_held() {
        let text = format!(
            "{} {}\nSECRETBODYLINE\n{}\nok\n",
            pem("RSA PRIVATE KEY").trim_end(),
            begin("EC PRIVATE KEY"),
            end("EC PRIVATE KEY")
        );
        for size in [1, 3, 9, 30] {
            let out = run_pieces(&chunks(&text, size)).concat();
            assert!(
                !out.contains("SECRETBODY") && !out.contains("QUJD"),
                "{out}"
            );
            assert_eq!(out, whole(&text), "size {size}");
        }
    }

    #[test]
    fn a_quote_opened_by_a_non_sensitive_word_does_not_confuse_the_cut() {
        let secret = "hunter2hunter2";
        let text =
            format!("it's a \"nice\" day, don't worry. password={secret} then more words\nok\n");
        for size in [1, 2, 7, 64] {
            let out = run_pieces(&chunks(&text, size)).concat();
            assert!(!out.contains(secret), "size {size}: {out}");
            assert_eq!(out, whole(&text), "size {size}");
        }
        // A backslash-escaped quote inside a sensitive quoted value does not end it.
        let escaped = format!("{{\"password\": \"ab\\\" {secret} cd\"}} tail words\n");
        for size in [1, 4, 11] {
            let out = run_pieces(&chunks(&escaped, size)).concat();
            assert!(!out.contains(secret), "size {size}: {out}");
            assert_eq!(out, whole(&escaped), "size {size}");
        }
    }

    #[test]
    fn unicode_whitespace_is_not_a_cut_and_cannot_hide_a_secret() {
        let secret = "hunter2hunter2";
        for gap in ['\u{a0}', '\u{2003}', '\u{3000}'] {
            let text = format!("see{gap}token={secret}{gap}and{gap}more tokens here\n");
            let t2 = format!("Bearer{gap}{secret}{gap}x y z\n");
            for case in [&text, &t2] {
                for size in [1, 2, 5, 16] {
                    let out = run_pieces(&chunks(case, size)).concat();
                    assert_eq!(out, whole(case), "gap {gap:?} size {size}");
                }
            }
        }
    }

    #[test]
    fn a_certificate_block_is_released_like_any_text() {
        let cert = format!(
            "{}\nMIIB\n{}\nnext words\n",
            begin("CERTIFICATE"),
            end("CERTIFICATE")
        );
        for size in [1, 6, 100] {
            assert_equivalent(&cert, size);
        }
    }

    #[test]
    fn plain_text_flows_word_by_word() {
        let out = run_pieces(&["one\ntw", "o\nthr", "ee"]);
        assert_eq!(out.concat(), "one\ntwo\nthree");
        assert!(out.len() >= 3);
    }

    #[test]
    fn multibyte_text_is_never_cut_inside_a_character() {
        let text = format!("{}\n", "kata \u{00e9}\u{4e2d}\u{1f600} ".repeat(300));
        for size in [1, 3, 10] {
            assert_equivalent(&text, size);
        }
    }

    #[test]
    fn a_long_line_with_spaces_streams_and_stays_equivalent() {
        let line = format!("{} end", "lorem ipsum dolor ".repeat(400));
        let mut redactor = DeltaRedactor::default();
        let mut pieces = Vec::new();
        for piece in chunks(&line, 100) {
            pieces.extend(redactor.push(&piece));
        }
        assert!(!pieces.is_empty());
        pieces.extend(redactor.finish());
        assert_eq!(pieces.concat(), whole(&line));
    }

    #[test]
    fn a_line_without_delimiters_is_held_whole_below_the_cap() {
        let long = format!("{}{}{}", "a".repeat(9000), token(), "b".repeat(9000));
        let mut redactor = DeltaRedactor::default();
        for piece in chunks(&long, 500) {
            assert_eq!(redactor.push(&piece), None);
        }
        assert_eq!(redactor.finish().unwrap_or_default(), whole(&long));
    }

    #[test]
    fn a_one_mebibyte_line_stays_within_the_cap_and_never_leaks() {
        let secret = token();
        let mut text = "a".repeat(500_000);
        text.push_str(&format!(" {secret} "));
        text.push_str(&"b".repeat(300_000));
        text.push_str(&format!(" password=hunter2hunter2 {secret} "));
        text.push_str(&"c".repeat(248_000));
        let mut redactor = DeltaRedactor::default();
        let mut released = String::new();
        for piece in chunks(&text, 1000) {
            released.extend(redactor.push(&piece));
            assert!(redactor.held() <= HARD_CAP_BYTES + 1000);
        }
        released.extend(redactor.finish());
        assert!(!released.contains(&secret[3..]));
        assert!(!released.contains("hunter2"));
        assert!(released.contains("[REDACTED]"));
        // The delimiter-free runs are released (past the cap) but the output is not longer.
        assert!(released.starts_with(&"a".repeat(1000)));
        assert!(released.len() <= text.len());
    }

    #[test]
    fn a_key_block_larger_than_the_cap_is_dropped_and_text_resumes_after_it() {
        let mut text = format!("start\n{}\n", begin("RSA PRIVATE KEY"));
        for _ in 0..4000 {
            text.push_str("QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo=\n");
        }
        text.push_str(&format!("{}\nafter words\n", end("RSA PRIVATE KEY")));
        let mut redactor = DeltaRedactor::default();
        let mut released = String::new();
        for piece in chunks(&text, 700) {
            released.extend(redactor.push(&piece));
            assert!(redactor.held() <= HARD_CAP_BYTES + 700);
        }
        released.extend(redactor.finish());
        assert!(!released.contains("QUJD"));
        assert!(released.starts_with("start\n[REDACTED PRIVATE KEY]"));
        assert!(released.ends_with("\nafter words\n"), "{released:.80}");
    }

    // ---- differential test against the redactor ----------------------------------------------

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next() % n as u64).unwrap_or(0)
        }
    }

    fn fragment(rng: &mut Rng) -> String {
        let secret = format!("s3{}x", "cr3tValue9");
        let t = token();
        match rng.below(24) {
            0 => pem("RSA PRIVATE KEY"),
            1 => pem("CERTIFICATE"),
            2 => format!("API_KEY={secret} "),
            3 => format!("password: {secret}\n"),
            4 => format!("\"token\": \"a \\\" {secret} b\" "),
            5 => format!("pass='{secret} two' "),
            6 => format!("Authorization: Bearer {secret}\n"),
            7 => format!("Bearer {secret} "),
            8 => format!("mysql://app:{secret}@db.example.test/x "),
            9 => format!("{t} "),
            10 => "it's \"quoted\" and don't ".to_owned(),
            11 => format!("{}BEGIN ", "-".repeat(5)),
            12 => format!("{}\nZZZZ\n", begin("EC PRIVATE KEY")),
            13 => end("EC PRIVATE KEY"),
            14 => "\n".to_owned(),
            15 => "\t".to_owned(),
            16 => ["\u{a0}", "\u{2003}", "\u{3000}"][rng.below(3)].to_owned(),
            17 => format!("123456789:{} ", "AbCdEfGhIjKlMnOpQrStUvWxYz0123456"),
            18 => "key: ".to_owned(),
            19 => "= ".to_owned(),
            20 => "caf\u{e9} \u{1f600} ".to_owned(),
            _ => {
                ["alpha ", "beta ", "gamma, ", "delta.\n", "x=1 ", "a:b "][rng.below(6)].to_owned()
            }
        }
    }

    #[test]
    fn random_mixes_split_at_random_places_equal_the_redaction_of_the_whole() {
        for seed in 1..=1500_u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let count = 1 + rng.below(30);
            let text: String = (0..count).map(|_| fragment(&mut rng)).collect();
            let all: Vec<char> = text.chars().collect();
            let mut pieces = Vec::new();
            let mut at = 0;
            while at < all.len() {
                let size = 1 + rng.below(40);
                let end = (at + size).min(all.len());
                pieces.push(all[at..end].iter().collect::<String>());
                at = end;
            }
            let out = run_pieces(&pieces).concat();
            assert_eq!(out, whole(&text), "seed {seed}: {text:?}");
        }
    }
}
