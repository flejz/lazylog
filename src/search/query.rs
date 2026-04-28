use regex::bytes::Regex;
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub pattern: String,
    pub case_sensitive: bool,
    pub regex: Regex,
}

impl SearchQuery {
    pub fn new(pattern: String, case_sensitive: bool) -> Result<Self> {
        let re_pattern = if case_sensitive {
            pattern.clone()
        } else {
            format!("(?i){}", pattern)
        };
        let regex = Regex::new(&re_pattern)?;
        Ok(Self { pattern, case_sensitive, regex })
    }

    pub fn matches(&self, haystack: &[u8]) -> bool {
        self.regex.is_match(haystack)
    }

    pub fn find(&self, haystack: &[u8]) -> Option<(usize, usize)> {
        self.regex.find(haystack).map(|m| (m.start(), m.end()))
    }

    pub fn find_all(&self, haystack: &[u8]) -> Vec<(usize, usize)> {
        self.regex.find_iter(haystack).map(|m| (m.start(), m.end())).collect()
    }
}
