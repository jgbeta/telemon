use chrono::{SecondsFormat, Utc};
use serde::Serialize;

use telemon_core::metrics::model::MetricSample;
use telemon_core::metrics::names;

#[derive(Debug, Serialize)]
struct MacmonJsonSnapshot {
    timestamp: String,
    temp: MacmonJsonTemperature,
    memory: MacmonJsonMemory,
    ecpu_usage: Option<[f64; 2]>,
    pcpu_usage: Option<[f64; 2]>,
    cpu_usage_pct: Option<f64>,
    gpu_usage: Option<[f64; 2]>,
    cpu_power: Option<f64>,
    gpu_power: Option<f64>,
    ane_power: Option<f64>,
    all_power: Option<f64>,
    sys_power: Option<f64>,
    ram_power: Option<f64>,
    gpu_ram_power: Option<f64>,
}

#[derive(Debug, Serialize)]
struct MacmonJsonTemperature {
    cpu_temp_avg: Option<f64>,
    gpu_temp_avg: Option<f64>,
}

#[derive(Debug, Serialize)]
struct MacmonJsonMemory {
    ram_total: Option<u64>,
    ram_usage: Option<u64>,
    swap_total: Option<u64>,
    swap_usage: Option<u64>,
}

pub fn encode_snapshot(
    dynamic_samples: &[MetricSample],
    static_samples: &[MetricSample],
) -> String {
    let mut samples = Vec::with_capacity(dynamic_samples.len() + static_samples.len());
    samples.extend_from_slice(dynamic_samples);
    samples.extend_from_slice(static_samples);

    let snapshot = MacmonJsonSnapshot {
        timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true),
        temp: MacmonJsonTemperature {
            cpu_temp_avg: metric_f64(&samples, names::MACMON_CPU_TEMP_CELSIUS),
            gpu_temp_avg: metric_f64(&samples, names::MACMON_GPU_TEMP_CELSIUS),
        },
        memory: MacmonJsonMemory {
            ram_total: metric_u64(&samples, names::MACMON_MEMORY_RAM_TOTAL_BYTES),
            ram_usage: metric_u64(&samples, names::MACMON_MEMORY_RAM_USED_BYTES),
            swap_total: metric_u64(&samples, names::MACMON_MEMORY_SWAP_TOTAL_BYTES),
            swap_usage: metric_u64(&samples, names::MACMON_MEMORY_SWAP_USED_BYTES),
        },
        ecpu_usage: metric_pair(
            &samples,
            names::MACMON_ECPU_FREQUENCY_MHZ,
            names::MACMON_ECPU_USAGE_RATIO,
        ),
        pcpu_usage: metric_pair(
            &samples,
            names::MACMON_PCPU_FREQUENCY_MHZ,
            names::MACMON_PCPU_USAGE_RATIO,
        ),
        cpu_usage_pct: metric_f64(&samples, names::MACMON_CPU_USAGE_RATIO),
        gpu_usage: metric_pair(
            &samples,
            names::MACMON_GPU_FREQUENCY_MHZ,
            names::MACMON_GPU_USAGE_RATIO,
        ),
        cpu_power: metric_f64(&samples, names::MACMON_CPU_POWER_WATTS),
        gpu_power: metric_f64(&samples, names::MACMON_GPU_POWER_WATTS),
        ane_power: metric_f64(&samples, names::MACMON_ANE_POWER_WATTS),
        all_power: metric_f64(&samples, names::MACMON_ALL_POWER_WATTS),
        sys_power: metric_f64(&samples, names::MACMON_SYS_POWER_WATTS),
        ram_power: metric_f64(&samples, names::MACMON_RAM_POWER_WATTS),
        gpu_ram_power: metric_f64(&samples, names::MACMON_GPU_RAM_POWER_WATTS),
    };

    serde_json::to_string(&snapshot).expect("macmon JSON snapshot must serialize")
}

fn metric_pair(
    samples: &[MetricSample],
    frequency_name: &str,
    usage_name: &str,
) -> Option<[f64; 2]> {
    Some([
        metric_f64(samples, frequency_name)?,
        metric_f64(samples, usage_name)?,
    ])
}

fn metric_f64(samples: &[MetricSample], name: &str) -> Option<f64> {
    samples
        .iter()
        .find(|sample| sample.name == name && sample.value.is_finite())
        .map(|sample| sample.value)
}

fn metric_u64(samples: &[MetricSample], name: &str) -> Option<u64> {
    metric_f64(samples, name)
        .filter(|value| *value >= 0.0)
        .map(|value| value.round() as u64)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use telemon_core::metrics::model::{labels, MetricSample};

    use super::*;

    fn gauge(name: &str, value: f64) -> MetricSample {
        MetricSample::gauge(name, "test", labels(&[("chip", "Apple M3 Pro")]), value)
    }

    #[test]
    fn encodes_canonical_macmon_json_snapshot() {
        let body = encode_snapshot(
            &[
                gauge(names::MACMON_CPU_TEMP_CELSIUS, 43.73614),
                gauge(names::MACMON_GPU_TEMP_CELSIUS, 36.95167),
                gauge(names::MACMON_ECPU_FREQUENCY_MHZ, 1181.0),
                gauge(names::MACMON_ECPU_USAGE_RATIO, 0.082656614),
                gauge(names::MACMON_PCPU_FREQUENCY_MHZ, 1974.0),
                gauge(names::MACMON_PCPU_USAGE_RATIO, 0.015181795),
                gauge(names::MACMON_CPU_USAGE_RATIO, 0.036854),
                gauge(names::MACMON_GPU_FREQUENCY_MHZ, 461.0),
                gauge(names::MACMON_GPU_USAGE_RATIO, 0.021497859),
                gauge(names::MACMON_MEMORY_RAM_USED_BYTES, 20_985_479_168.0),
                gauge(names::MACMON_MEMORY_RAM_TOTAL_BYTES, 25_769_803_776.0),
                gauge(names::MACMON_MEMORY_SWAP_USED_BYTES, 2_602_434_560.0),
                gauge(names::MACMON_MEMORY_SWAP_TOTAL_BYTES, 4_294_967_296.0),
                gauge(names::MACMON_CPU_POWER_WATTS, 0.20486385),
                gauge(names::MACMON_GPU_POWER_WATTS, 0.017451683),
                gauge(names::MACMON_ANE_POWER_WATTS, 0.0),
                gauge(names::MACMON_ALL_POWER_WATTS, 0.22231553),
                gauge(names::MACMON_SYS_POWER_WATTS, 5.876533),
                gauge(names::MACMON_RAM_POWER_WATTS, 0.11635789),
                gauge(names::MACMON_GPU_RAM_POWER_WATTS, 0.0009615385),
            ],
            &[],
        );

        let value: Value = serde_json::from_str(&body).unwrap();

        assert!(value["timestamp"].as_str().unwrap().ends_with('Z'));
        assert_eq!(value["temp"]["cpu_temp_avg"], 43.73614);
        assert_eq!(value["memory"]["ram_total"], 25_769_803_776_u64);
        assert_eq!(value["ecpu_usage"][0], 1181.0);
        assert_eq!(value["ecpu_usage"][1], 0.082656614);
        assert_eq!(value["cpu_power"], 0.20486385);
    }

    #[test]
    fn empty_snapshot_is_valid_json() {
        let body = encode_snapshot(&[], &[]);
        let value: Value = serde_json::from_str(&body).unwrap();

        assert!(value["timestamp"].is_string());
        assert!(value["temp"]["cpu_temp_avg"].is_null());
        assert!(value["ecpu_usage"].is_null());
    }
}
