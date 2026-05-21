use super::model::{NvidiaDeviceInfo, NvidiaMemory, NvidiaUtilization};
use super::provider::{NvidiaError, NvidiaProvider};

#[derive(Debug, Clone)]
pub struct TestNvidiaDevice {
    pub info: NvidiaDeviceInfo,
    pub temperature_celsius: Option<f64>,
    pub utilization: Option<NvidiaUtilization>,
    pub memory: Option<NvidiaMemory>,
    pub fan_speed_ratio: Option<f64>,
    pub power_usage_milliwatts: Option<u32>,
    pub power_limit_milliwatts: Option<u32>,
    pub graphics_clock_mhz: Option<u32>,
    pub memory_clock_mhz: Option<u32>,
    pub performance_state: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct TestNvidiaProvider {
    supported: bool,
    devices: Vec<TestNvidiaDevice>,
    device_count_error: Option<NvidiaError>,
}

impl TestNvidiaProvider {
    pub fn new(devices: Vec<TestNvidiaDevice>) -> Self {
        Self {
            supported: true,
            devices,
            device_count_error: None,
        }
    }

    pub fn one_gpu() -> Self {
        Self::new(vec![TestNvidiaDevice {
            info: NvidiaDeviceInfo {
                index: 0,
                name: Some("Test NVIDIA GPU".to_string()),
                uuid: Some("GPU-test-uuid".to_string()),
            },
            temperature_celsius: Some(58.0),
            utilization: Some(NvidiaUtilization {
                graphics_ratio: 0.31,
                memory_ratio: 0.12,
            }),
            memory: Some(NvidiaMemory {
                total_bytes: 16 * 1024 * 1024 * 1024,
                used_bytes: 2 * 1024 * 1024 * 1024,
                free_bytes: 14 * 1024 * 1024 * 1024,
            }),
            fan_speed_ratio: Some(0.42),
            power_usage_milliwatts: Some(57_622),
            power_limit_milliwatts: Some(450_000),
            graphics_clock_mhz: Some(2_520),
            memory_clock_mhz: Some(10_501),
            performance_state: Some(0),
        }])
    }

    fn device(&self, index: u32) -> Result<&TestNvidiaDevice, NvidiaError> {
        self.devices
            .get(index as usize)
            .ok_or(NvidiaError::DeviceIndexOutOfRange { index })
    }
}

impl NvidiaProvider for TestNvidiaProvider {
    fn is_supported(&self) -> bool {
        self.supported
    }

    fn device_count(&self) -> Result<u32, NvidiaError> {
        if let Some(error) = &self.device_count_error {
            return Err(error.clone());
        }
        Ok(self.devices.len() as u32)
    }

    fn device_info(&mut self, index: u32) -> Result<NvidiaDeviceInfo, NvidiaError> {
        Ok(self.device(index)?.info.clone())
    }

    fn temperature_celsius(&mut self, index: u32) -> Result<Option<f64>, NvidiaError> {
        Ok(self.device(index)?.temperature_celsius)
    }

    fn utilization(&mut self, index: u32) -> Result<Option<NvidiaUtilization>, NvidiaError> {
        Ok(self.device(index)?.utilization)
    }

    fn memory(&mut self, index: u32) -> Result<Option<NvidiaMemory>, NvidiaError> {
        Ok(self.device(index)?.memory)
    }

    fn fan_speed_ratio(&mut self, index: u32) -> Result<Option<f64>, NvidiaError> {
        Ok(self.device(index)?.fan_speed_ratio)
    }

    fn power_usage_milliwatts(&mut self, index: u32) -> Result<Option<u32>, NvidiaError> {
        Ok(self.device(index)?.power_usage_milliwatts)
    }

    fn power_limit_milliwatts(&mut self, index: u32) -> Result<Option<u32>, NvidiaError> {
        Ok(self.device(index)?.power_limit_milliwatts)
    }

    fn graphics_clock_mhz(&mut self, index: u32) -> Result<Option<u32>, NvidiaError> {
        Ok(self.device(index)?.graphics_clock_mhz)
    }

    fn memory_clock_mhz(&mut self, index: u32) -> Result<Option<u32>, NvidiaError> {
        Ok(self.device(index)?.memory_clock_mhz)
    }

    fn performance_state(&mut self, index: u32) -> Result<Option<u32>, NvidiaError> {
        Ok(self.device(index)?.performance_state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_returns_one_gpu() {
        let mut provider = TestNvidiaProvider::one_gpu();

        assert!(provider.is_supported());
        assert_eq!(provider.device_count().unwrap(), 1);
        assert_eq!(
            provider.device_info(0).unwrap().name.as_deref(),
            Some("Test NVIDIA GPU")
        );
        assert_eq!(provider.temperature_celsius(0).unwrap(), Some(58.0));
        assert_eq!(
            provider.utilization(0).unwrap(),
            Some(NvidiaUtilization {
                graphics_ratio: 0.31,
                memory_ratio: 0.12,
            })
        );
        assert_eq!(provider.power_usage_milliwatts(0).unwrap(), Some(57_622));
        assert_eq!(provider.power_limit_milliwatts(0).unwrap(), Some(450_000));
        assert_eq!(provider.graphics_clock_mhz(0).unwrap(), Some(2_520));
        assert_eq!(provider.memory_clock_mhz(0).unwrap(), Some(10_501));
        assert_eq!(provider.performance_state(0).unwrap(), Some(0));
    }
}
