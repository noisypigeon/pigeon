//! A generated `CHANGELOG.md` tier's text, parsed and rendered.

const TITLE: &str = "# Changelog";

/// A changelog: `# Changelog`, then `## <heading>` sections of `- ` lines.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changelog {
    /// Sections in file order.
    pub sections: Vec<(String, Vec<String>)>,
}

impl Changelog {
    /// Parses `text`; anything but section headings and bullets is dropped.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut log = Self::default();
        for line in text.lines() {
            if let Some(heading) = line.strip_prefix("## ") {
                log.sections.push((heading.trim().to_string(), Vec::new()));
            } else if line.starts_with("- ")
                && let Some((_, lines)) = log.sections.last_mut()
            {
                lines.push(line.to_string());
            }
        }
        log
    }

    /// Whether any line contains `needle`.
    #[must_use]
    pub fn mentions(&self, needle: &str) -> bool {
        self.sections
            .iter()
            .any(|(_, lines)| lines.iter().any(|l| l.contains(needle)))
    }

    /// The log with `line` first under the `date` section, dates kept newest first.
    #[must_use]
    pub fn add_dated(mut self, date: &str, line: String) -> Self {
        if let Some((_, lines)) = self.sections.iter_mut().find(|(h, _)| h == date) {
            lines.insert(0, line);
            return self;
        }
        let at = self
            .sections
            .iter()
            .position(|(h, _)| h.as_str() < date)
            .unwrap_or(self.sections.len());
        self.sections.insert(at, (date.to_string(), vec![line]));
        self
    }

    /// The log with a section prepended.
    #[must_use]
    pub fn prepend(mut self, heading: String, lines: Vec<String>) -> Self {
        self.sections.insert(0, (heading, lines));
        self
    }

    /// Renders the file.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::from(TITLE);
        out.push('\n');
        for (heading, lines) in &self.sections {
            out.push_str("\n## ");
            out.push_str(heading);
            out.push_str("\n\n");
            for line in lines {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Changelog;

    #[test]
    fn round_trips_and_orders_dates() {
        let text = "# Changelog\n\n## 2026-01-03\n\n- b\n- a\n\n## 2026-01-01\n\n- z\n";
        let log = Changelog::parse(text);
        assert_eq!(log.render(), text);
        let log = log
            .add_dated("2026-01-03", "- c".into())
            .add_dated("2026-01-02", "- y".into())
            .add_dated("2026-01-04", "- d".into());
        assert_eq!(
            log.render(),
            "# Changelog\n\n## 2026-01-04\n\n- d\n\n## 2026-01-03\n\n- c\n- b\n- a\n\n## 2026-01-02\n\n- y\n\n## 2026-01-01\n\n- z\n"
        );
        assert!(log.mentions("- y"));
    }
}
