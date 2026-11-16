//! Line-level Markdown structure: headings, with fenced code blocks ignored.

/// One line of a document.
#[derive(Debug, Clone, Copy)]
pub struct Line<'a> {
    /// 1-based line number.
    pub number: usize,
    /// The line, without its line ending.
    pub text: &'a str,
    /// The heading level and text, for a heading outside a fence.
    pub heading: Option<(u8, &'a str)>,
}

impl Line<'_> {
    /// Whether the line has no visible content.
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// Splits `content` into lines.
pub fn parse(content: &str) -> Vec<Line<'_>> {
    let mut in_fence = false;
    content
        .lines()
        .enumerate()
        .map(|(i, raw)| {
            let text = raw.strip_suffix('\r').unwrap_or(raw);
            let trimmed = text.trim_start();
            let delimiter = trimmed.starts_with("```") || trimmed.starts_with("~~~");
            let fenced = in_fence || delimiter;
            if delimiter {
                in_fence = !in_fence;
            }
            let heading = if fenced { None } else { heading(text) };
            Line {
                number: i + 1,
                text,
                heading,
            }
        })
        .collect()
}

fn heading(text: &str) -> Option<(u8, &str)> {
    let hashes = text.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = text[hashes..].strip_prefix(' ')?;
    Some((u8::try_from(hashes).ok()?, rest.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn headings_outside_fences_only() {
        let lines = parse("# One\n```\n## Not\n```\n### Three \n####no\n");
        let headings: Vec<_> = lines.iter().filter_map(|l| l.heading).collect();
        assert_eq!(headings, vec![(1, "One"), (3, "Three")]);
        assert!(lines[2].heading.is_none());
    }

    #[test]
    fn tilde_fences_and_crlf() {
        let lines = parse("~~~\n# x\n~~~\r\n## y\r\n");
        assert_eq!(lines[3].heading, Some((2, "y")));
        assert!(lines[1].heading.is_none());
    }
}
