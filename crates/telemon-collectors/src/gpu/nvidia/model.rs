#[derive(Debug, Clone, PartialEq)]
pub struct NvidiaDeviceInfo {
    pub index: u32,
    pub name: Option<String>,
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NvidiaUtilization {
    pub graphics_ratio: f64,
    pub memory_ratio: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NvidiaMemory {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
}

pub fn percent_to_ratio(percent: u32) -> Option<f64> {
    if percent <= 100 {
        Some(percent as f64 / 100.0)
    } else {
        None
    }
}

pub fn validate_temperature_celsius(value: f64) -> Option<f64> {
    if (-100.0..=250.0).contains(&value) {
        Some(value)
    } else {
        None
    }
}

pub fn validate_memory(total_bytes: u64, used_bytes: u64, free_bytes: u64) -> Option<NvidiaMemory> {
    if used_bytes > total_bytes || free_bytes > total_bytes {
        return None;
    }
    if used_bytes.saturating_add(free_bytes) > total_bytes {
        return None;
    }

    Some(NvidiaMemory {
        total_bytes,
        used_bytes,
        free_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_percent_to_ratio() {
        assert_eq!(percent_to_ratio(0), Some(0.0));
        assert_eq!(percent_to_ratio(42), Some(0.42));
        assert_eq!(percent_to_ratio(100), Some(1.0));
        assert_eq!(percent_to_ratio(101), None);
    }

    #[test]
    fn filters_implausible_temperatures() {
        assert_eq!(validate_temperature_celsius(-100.0), Some(-100.0));
        assert_eq!(validate_temperature_celsius(250.0), Some(250.0));
        assert_eq!(validate_temperature_celsius(-101.0), None);
        assert_eq!(validate_temperature_celsius(251.0), None);
    }

    #[test]
    fn validates_memory_consistency() {
        assert_eq!(
            validate_memory(16, 4, 12),
            Some(NvidiaMemory {
                total_bytes: 16,
                used_bytes: 4,
                free_bytes: 12,
            })
        );
        assert_eq!(validate_memory(16, 17, 0), None);
        assert_eq!(validate_memory(16, 8, 9), None);
    }
}
