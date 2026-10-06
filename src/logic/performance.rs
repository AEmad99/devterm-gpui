//! Performance presets. Balanced, Low memory, and Full fidelity change
//! scrollback, the search index size, and the hibernate delay.
//!
//! Standalone: hibernate clamps are duplicated here so this file does not
//! reference another module.

#![allow(dead_code)]

const DEFAULT_HIBERNATE_AFTER_MS: u64 = 30_000;
const MIN_HIBERNATE_AFTER_MS: u64 = 1_000;
const MAX_HIBERNATE_AFTER_MS: u64 = 24 * 60 * 60 * 1000;
const DEFAULT_OUTPUT_RING_LINES: u32 = 10_000;
const MIN_OUTPUT_RING_LINES: u32 = 100;
const MAX_OUTPUT_RING_LINES: u32 = 100_000;

pub const DEFAULT_SEARCH_INDEX_LINES: u32 = 2000;
pub const MIN_SEARCH_INDEX_LINES: u32 = 200;
pub const MAX_SEARCH_INDEX_LINES: u32 = 10_000;

fn normalize_hibernate_after_ms(value: f64) -> u64 {
    if !value.is_finite() {
        return DEFAULT_HIBERNATE_AFTER_MS;
    }
    let n = value.floor() as i64;
    n.clamp(MIN_HIBERNATE_AFTER_MS as i64, MAX_HIBERNATE_AFTER_MS as i64) as u64
}

fn normalize_output_ring_lines(value: u32) -> u32 {
    value.clamp(MIN_OUTPUT_RING_LINES, MAX_OUTPUT_RING_LINES)
}

/// `None` is a non-number (the TypeScript `unknown` fallback path).
pub fn normalize_search_index_lines(value: Option<f64>) -> u32 {
    normalize_search_index_lines_with_fallback(value, DEFAULT_SEARCH_INDEX_LINES)
}

pub fn normalize_search_index_lines_with_fallback(value: Option<f64>, fallback: u32) -> u32 {
    let Some(value) = value else {
        return fallback;
    };
    if !value.is_finite() {
        return fallback;
    }
    let n = value.floor() as i64;
    n.clamp(MIN_SEARCH_INDEX_LINES as i64, MAX_SEARCH_INDEX_LINES as i64) as u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerformancePresetId {
    Balanced,
    LowMemory,
    FullFidelity,
}

impl PerformancePresetId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::LowMemory => "low-memory",
            Self::FullFidelity => "full-fidelity",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteConnectMode {
    Focus,
    Stagger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerformancePresetValues {
    pub hibernate_enabled: bool,
    pub hibernate_after_ms: u64,
    pub scrollback: u32,
    pub output_ring_lines: u32,
    pub search_index_lines: u32,
    pub remote_connect_mode: RemoteConnectMode,
}

#[derive(Clone, Copy, Debug)]
pub struct PerformancePreset {
    pub id: PerformancePresetId,
    pub label: &'static str,
    pub hint: &'static str,
    pub values: PerformancePresetValues,
}

pub const PERFORMANCE_PRESETS: [PerformancePreset; 3] = [
    PerformancePreset {
        id: PerformancePresetId::Balanced,
        label: "Balanced",
        hint: "Hibernate at 30s, 10k scrollback, 2k search, remotes on focus.",
        values: PerformancePresetValues {
            hibernate_enabled: true,
            hibernate_after_ms: DEFAULT_HIBERNATE_AFTER_MS,
            scrollback: 10_000,
            output_ring_lines: DEFAULT_OUTPUT_RING_LINES,
            search_index_lines: DEFAULT_SEARCH_INDEX_LINES,
            remote_connect_mode: RemoteConnectMode::Focus,
        },
    },
    PerformancePreset {
        id: PerformancePresetId::LowMemory,
        label: "Low memory",
        hint: "Hibernate at 10s, 2k scrollback, 1k search, remotes on focus.",
        values: PerformancePresetValues {
            hibernate_enabled: true,
            hibernate_after_ms: 10_000,
            scrollback: 2000,
            output_ring_lines: 2000,
            search_index_lines: 1000,
            remote_connect_mode: RemoteConnectMode::Focus,
        },
    },
    PerformancePreset {
        id: PerformancePresetId::FullFidelity,
        label: "Full fidelity",
        hint: "Keep every renderer terminal live; scrollback at the 100k clamp.",
        values: PerformancePresetValues {
            hibernate_enabled: false,
            hibernate_after_ms: DEFAULT_HIBERNATE_AFTER_MS,
            scrollback: MAX_OUTPUT_RING_LINES,
            output_ring_lines: MAX_OUTPUT_RING_LINES,
            search_index_lines: DEFAULT_SEARCH_INDEX_LINES,
            remote_connect_mode: RemoteConnectMode::Stagger,
        },
    },
];

pub fn performance_preset(id: PerformancePresetId) -> &'static PerformancePreset {
    PERFORMANCE_PRESETS
        .iter()
        .find(|p| p.id == id)
        .expect("preset")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerformanceMatch {
    Preset(PerformancePresetId),
    Custom,
}

pub fn match_performance_preset(snapshot: &PerformancePresetValues) -> PerformanceMatch {
    for preset in &PERFORMANCE_PRESETS {
        let v = &preset.values;
        if snapshot.hibernate_enabled == v.hibernate_enabled
            && normalize_hibernate_after_ms(snapshot.hibernate_after_ms as f64) == v.hibernate_after_ms
            && snapshot.scrollback == v.scrollback
            && normalize_output_ring_lines(snapshot.output_ring_lines) == v.output_ring_lines
            && normalize_search_index_lines(Some(snapshot.search_index_lines as f64))
                == v.search_index_lines
            && snapshot.remote_connect_mode == v.remote_connect_mode
        {
            return PerformanceMatch::Preset(preset.id);
        }
    }
    PerformanceMatch::Custom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_balanced_default() {
        assert_eq!(
            match_performance_preset(&performance_preset(PerformancePresetId::Balanced).values),
            PerformanceMatch::Preset(PerformancePresetId::Balanced)
        );
    }

    #[test]
    fn recognizes_low_memory_and_full_fidelity() {
        assert_eq!(
            match_performance_preset(&performance_preset(PerformancePresetId::LowMemory).values),
            PerformanceMatch::Preset(PerformancePresetId::LowMemory)
        );
        assert_eq!(
            match_performance_preset(&performance_preset(PerformancePresetId::FullFidelity).values),
            PerformanceMatch::Preset(PerformancePresetId::FullFidelity)
        );
    }

    #[test]
    fn returns_custom_when_a_knob_diverges() {
        let mut values = performance_preset(PerformancePresetId::Balanced).values;
        values.hibernate_after_ms = 12_000;
        assert_eq!(
            match_performance_preset(&values),
            PerformanceMatch::Custom
        );
    }

    #[test]
    fn clamps_search_index_lines() {
        assert_eq!(normalize_search_index_lines(Some(50.0)), 200);
        assert_eq!(normalize_search_index_lines(Some(50_000.0)), 10_000);
        assert_eq!(normalize_search_index_lines(None), 2000);
    }

    #[test]
    fn labels_match_the_presets() {
        assert_eq!(performance_preset(PerformancePresetId::Balanced).label, "Balanced");
        assert_eq!(
            performance_preset(PerformancePresetId::LowMemory).label,
            "Low memory"
        );
        assert_eq!(
            performance_preset(PerformancePresetId::FullFidelity).label,
            "Full fidelity"
        );
        assert_eq!(
            performance_preset(PerformancePresetId::Balanced).hint,
            "Hibernate at 30s, 10k scrollback, 2k search, remotes on focus."
        );
        assert_eq!(
            performance_preset(PerformancePresetId::LowMemory).hint,
            "Hibernate at 10s, 2k scrollback, 1k search, remotes on focus."
        );
        assert_eq!(
            performance_preset(PerformancePresetId::FullFidelity).hint,
            "Keep every renderer terminal live; scrollback at the 100k clamp."
        );
    }
}
