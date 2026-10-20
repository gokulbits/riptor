//! Download rules: which new articles get added automatically.

use std::path::PathBuf;

use anyhow::ensure;
use serde::{Deserialize, Serialize};

const MAX_PATTERN: usize = 2000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    /// 0 for a new rule.
    pub id: u32,
    pub name: String,
    pub enabled: bool,
    /// Words that must all appear; `|` separates alternatives.
    pub must_contain: String,
    pub must_not_contain: String,
    pub use_regex: bool,
    /// Feed ids; empty means every feed.
    pub feeds: Vec<u32>,
    pub category: Option<String>,
    pub save_path: Option<PathBuf>,
    pub start_paused: bool,
    pub last_match: Option<u64>,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            enabled: true,
            must_contain: String::new(),
            must_not_contain: String::new(),
            use_regex: false,
            feeds: Vec::new(),
            category: None,
            save_path: None,
            start_paused: false,
            last_match: None,
        }
    }
}

enum Pattern {
    /// Alternatives of words, all lowercase.
    Words(Vec<Vec<String>>),
    Regex(regex::Regex),
}

impl Pattern {
    fn new(src: &str, use_regex: bool) -> anyhow::Result<Option<Self>> {
        let src = src.trim();
        if src.is_empty() {
            return Ok(None);
        }
        if use_regex {
            let re = regex::RegexBuilder::new(src)
                .case_insensitive(true)
                .size_limit(1 << 20)
                .build()?;
            return Ok(Some(Self::Regex(re)));
        }
        let alts = src
            .split('|')
            .map(|alt| {
                alt.split_whitespace()
                    .map(str::to_lowercase)
                    .collect::<Vec<_>>()
            })
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>();
        Ok((!alts.is_empty()).then_some(Self::Words(alts)))
    }

    fn matches(&self, title: &str) -> bool {
        match self {
            Self::Regex(re) => re.is_match(title),
            Self::Words(alts) => {
                let t = title.to_lowercase();
                alts.iter()
                    .any(|words| words.iter().all(|w| t.contains(w.as_str())))
            }
        }
    }
}

/// A rule ready to test titles against.
pub struct Matcher {
    must: Pattern,
    must_not: Option<Pattern>,
}

impl Rule {
    pub fn validate(&self) -> anyhow::Result<()> {
        let name = self.name.trim();
        ensure!(
            !name.is_empty() && name.chars().count() <= 100,
            "give the rule a name"
        );
        ensure!(!name.chars().any(char::is_control), "invalid rule name");
        ensure!(
            self.must_contain.len() <= MAX_PATTERN && self.must_not_contain.len() <= MAX_PATTERN,
            "pattern is too long"
        );
        ensure!(self.feeds.len() <= 1000, "too many feeds");
        if let Some(p) = &self.save_path {
            ensure!(p.is_absolute(), "save folder must be an absolute path");
        }
        self.matcher()?;
        Ok(())
    }

    /// An empty "must contain" would match everything, so it's refused.
    pub fn matcher(&self) -> anyhow::Result<Matcher> {
        let must = Pattern::new(&self.must_contain, self.use_regex)?
            .ok_or_else(|| anyhow::anyhow!("\"must contain\" can't be empty"))?;
        let must_not = Pattern::new(&self.must_not_contain, self.use_regex)?;
        Ok(Matcher { must, must_not })
    }

    pub fn applies_to(&self, feed: u32) -> bool {
        self.feeds.is_empty() || self.feeds.contains(&feed)
    }
}

impl Matcher {
    pub fn matches(&self, title: &str) -> bool {
        self.must.matches(title) && !self.must_not.as_ref().is_some_and(|p| p.matches(title))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(must: &str, not: &str, re: bool) -> Matcher {
        Rule {
            name: "r".into(),
            must_contain: must.into(),
            must_not_contain: not.into(),
            use_regex: re,
            ..Default::default()
        }
        .matcher()
        .unwrap()
    }

    #[test]
    fn words_and_alternatives() {
        let m = rule("show 1080p | show 2160p", "cam", false);
        assert!(m.matches("The.Show.S01E02.1080p.WEB"));
        assert!(m.matches("the show s01e02 2160P"));
        assert!(!m.matches("The Show S01E02 720p"));
        assert!(!m.matches("The Show 1080p CAM"));
    }

    #[test]
    fn regex_rules() {
        let m = rule(r"s\d\de\d\d.*1080p", "", true);
        assert!(m.matches("Show.S01E02.1080p"));
        assert!(!m.matches("Show.S01.1080p"));
    }

    #[test]
    fn bad_rules_are_refused() {
        let base = Rule {
            name: "r".into(),
            must_contain: "x".into(),
            ..Default::default()
        };
        base.validate().unwrap();
        for bad in [
            Rule {
                must_contain: "  ".into(),
                ..base.clone()
            },
            Rule {
                name: " ".into(),
                ..base.clone()
            },
            Rule {
                must_contain: "(".into(),
                use_regex: true,
                ..base.clone()
            },
            Rule {
                must_contain: "a".repeat(MAX_PATTERN + 1),
                ..base.clone()
            },
            Rule {
                save_path: Some("relative".into()),
                ..base.clone()
            },
            // Too big for the regex size limit.
            Rule {
                must_contain: "(a{1000}){1000}".into(),
                use_regex: true,
                ..base.clone()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }
}
