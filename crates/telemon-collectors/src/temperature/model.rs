use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Cpu,
    Gpu,
    Storage,
    Motherboard,
    Battery,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemperatureReading {
    pub component: Component,
    pub sensor: String,
    pub source: &'static str,
    pub labels: BTreeMap<String, String>,
    pub temperature_celsius: f64,
    pub critical_celsius: Option<f64>,
    pub warning_celsius: Option<f64>,
    pub raw_label: Option<String>,
}

impl Component {
    pub fn label_value(self) -> &'static str {
        match self {
            Component::Cpu => "cpu",
            Component::Gpu => "gpu",
            Component::Storage => "storage",
            Component::Motherboard => "motherboard",
            Component::Battery => "battery",
            Component::Unknown => "unknown",
        }
    }
}

pub fn milli_celsius_to_celsius(value: i64) -> f64 {
    value as f64 / 1000.0
}

pub fn normalize_sensor_label(raw: &str) -> String {
    let raw = raw.trim();
    let lower = raw.to_ascii_lowercase();
    let normalized = lower
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    let normalized = normalized
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");

    if normalized.starts_with("package") {
        "package".to_string()
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_milli_celsius() {
        assert_eq!(milli_celsius_to_celsius(67_000), 67.0);
    }

    #[test]
    fn component_label_values_are_stable() {
        assert_eq!(Component::Cpu.label_value(), "cpu");
        assert_eq!(Component::Gpu.label_value(), "gpu");
        assert_eq!(Component::Storage.label_value(), "storage");
        assert_eq!(Component::Unknown.label_value(), "unknown");
    }

    #[test]
    fn normalizes_sensor_labels() {
        assert_eq!(normalize_sensor_label("Package id 0"), "package");
        assert_eq!(normalize_sensor_label("Core 0"), "core_0");
        assert_eq!(normalize_sensor_label("Composite"), "composite");
        assert_eq!(normalize_sensor_label("Tctl"), "tctl");
    }
}
