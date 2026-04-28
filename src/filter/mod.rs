pub mod view;

use crate::parser::LogLevel;

/// Max number of dynamically-discovered log levels that get key bindings (1-9).
pub const MAX_KEYED_LEVELS: usize = 9;

/// Ordered list of known log levels for display/filter, up to MAX_KEYED_LEVELS.
/// Levels beyond this index are always visible.
#[derive(Debug, Clone)]
pub struct LevelRegistry {
    /// Ordered by severity (most severe first): ERROR, WARN, INFO, DEBUG, TRACE, then custom in discovery order
    pub levels: Vec<LogLevel>,
}

impl LevelRegistry {
    pub fn new() -> Self {
        Self {
            levels: vec![
                LogLevel::Error,
                LogLevel::Warn,
                LogLevel::Info,
                LogLevel::Debug,
                LogLevel::Trace,
            ],
        }
    }

    /// Register a newly discovered level. No-op if already present or if it's a standard level.
    pub fn discover(&mut self, level: LogLevel) {
        if !self.levels.contains(&level) {
            self.levels.push(level);
        }
    }

    /// Returns the 1-based key index for a level (1..=9), or None if > MAX_KEYED_LEVELS.
    pub fn key_for(&self, level: &LogLevel) -> Option<usize> {
        self.levels.iter().position(|l| l == level).map(|i| i + 1).filter(|&k| k <= MAX_KEYED_LEVELS)
    }

    pub fn level_at_key(&self, key: usize) -> Option<&LogLevel> {
        if key == 0 || key > MAX_KEYED_LEVELS {
            return None;
        }
        self.levels.get(key - 1)
    }
}

impl Default for LevelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Active filter state. Bitmask: bit N set = level at registry index N is VISIBLE.
#[derive(Debug, Clone)]
pub struct FilterState {
    /// Bitmask of visible levels. Bit 0 = registry.levels[0] (ERROR), etc.
    /// Bits beyond registry.levels.len() or beyond 16 are ignored.
    pub level_mask: u16,
    /// All levels visible when true (overrides mask).
    pub show_all_levels: bool,
    /// Selected target prefixes (OR logic). Empty = show all.
    pub crate_prefixes: Vec<String>,
}

impl Default for FilterState {
    fn default() -> Self {
        Self {
            level_mask: 0xFFFF,
            show_all_levels: true,
            crate_prefixes: Vec::new(),
        }
    }
}

impl FilterState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle visibility of the level at key index (1-based, 1..=9).
    pub fn toggle_key(&mut self, key: usize, registry: &LevelRegistry) {
        if key == 0 || key > MAX_KEYED_LEVELS {
            return;
        }
        let idx = key - 1;
        if idx >= registry.levels.len() {
            return;
        }
        self.show_all_levels = false;
        self.level_mask ^= 1u16 << idx;
    }

    /// Returns true if `level` should be shown given current mask.
    pub fn level_visible(&self, level: Option<LogLevel>, registry: &LevelRegistry) -> bool {
        if self.show_all_levels {
            return true;
        }
        let Some(lv) = level else {
            // Unknown level: always show
            return true;
        };
        let Some(idx) = registry.levels.iter().position(|l| *l == lv) else {
            // Level not in registry (beyond MAX_KEYED_LEVELS): always show
            return true;
        };
        if idx >= MAX_KEYED_LEVELS {
            return true;
        }
        (self.level_mask >> idx) & 1 == 1
    }

    pub fn crate_visible(&self, target: Option<&str>) -> bool {
        if self.crate_prefixes.is_empty() {
            return true;
        }
        match target {
            Some(t) => self.crate_prefixes.iter().any(|p| t == p || t.starts_with(&format!("{}::", p))),
            None => false,
        }
    }

    pub fn is_active(&self) -> bool {
        !self.show_all_levels || !self.crate_prefixes.is_empty()
    }
}
