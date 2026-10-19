//! Word error rate: (substitutions + deletions + insertions) / reference words, after a
//! simple English normalisation (lower case, punctuation removed, hyphens split,
//! a few spelled-out forms unified). The normaliser is ours and simpler than Whisper's
//! EnglishTextNormalizer, so numbers can differ slightly from published WERs.

pub fn normalize(s: &str) -> Vec<String> {
    let mut t = String::with_capacity(s.len());
    for c in s.chars() {
        let c = match c {
            '\u{2019}' | '\u{2018}' => '\'',
            _ => c,
        };
        if c.is_alphanumeric() || c == '\'' {
            t.extend(c.to_lowercase());
        } else {
            t.push(' ');
        }
    }
    t.split_whitespace()
        .map(|w| w.trim_matches('\'').to_string())
        .filter(|w| !w.is_empty())
        .map(|w| match w.as_str() {
            "mr" => "mister".into(),
            "mrs" => "missus".into(),
            "dr" => "doctor".into(),
            "st" => "saint".into(),
            _ => w,
        })
        .collect()
}

pub struct Wer {
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
    pub ref_words: usize,
}

impl Wer {
    pub fn rate(&self) -> f64 {
        (self.substitutions + self.deletions + self.insertions) as f64 / self.ref_words.max(1) as f64
    }
}

pub fn wer(reference: &str, hypothesis: &str) -> Wer {
    let r = normalize(reference);
    let h = normalize(hypothesis);
    let (n, m) = (r.len(), h.len());
    // dp[i][j] = (cost, subs, dels, ins)
    let mut dp = vec![vec![(0usize, 0usize, 0usize, 0usize); m + 1]; n + 1];
    for i in 1..=n {
        dp[i][0] = (i, 0, i, 0);
    }
    for j in 1..=m {
        dp[0][j] = (j, 0, 0, j);
    }
    for i in 1..=n {
        for j in 1..=m {
            let same = r[i - 1] == h[j - 1];
            let d = dp[i - 1][j - 1];
            let mut best = if same { d } else { (d.0 + 1, d.1 + 1, d.2, d.3) };
            let del = dp[i - 1][j];
            if del.0 + 1 < best.0 {
                best = (del.0 + 1, del.1, del.2 + 1, del.3);
            }
            let ins = dp[i][j - 1];
            if ins.0 + 1 < best.0 {
                best = (ins.0 + 1, ins.1, ins.2, ins.3 + 1);
            }
            dp[i][j] = best;
        }
    }
    let (_, s, d, i) = dp[n][m];
    Wer { substitutions: s, deletions: d, insertions: i, ref_words: n }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_edits() {
        let w = wer("the cat sat on the mat", "The cat, sat on a mat!");
        assert_eq!((w.substitutions, w.deletions, w.insertions, w.ref_words), (1, 0, 0, 6));
        let w = wer("a b c", "a c d e");
        assert_eq!(w.substitutions + w.deletions + w.insertions, 3);
        assert_eq!(wer("Hello, world.", "hello world").rate(), 0.0);
    }
}
