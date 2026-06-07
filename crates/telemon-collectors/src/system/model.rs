#[derive(Debug, Clone, Default, PartialEq)]
pub struct SystemSnapshot {
    pub uptime_seconds: Option<f64>,
    pub memory_total_bytes: Option<u64>,
    pub memory_available_bytes: Option<u64>,
    pub cpu_count: Option<u64>,
    pub cpu_usage_ratio: Option<f64>,
    pub cpu_frequency_samples: Vec<CpuFrequencySample>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CpuFrequencySample {
    pub cpu: u32,
    pub frequency_mhz: f64,
    pub source: &'static str,
}
