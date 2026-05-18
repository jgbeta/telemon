use super::model::{NvidiaDeviceInfo, NvidiaMemory, NvidiaUtilization};
use super::provider::{NvidiaError, NvidiaProvider};

#[derive(Debug, Clone)]
pub struct FakeNvidiaDevice {
    pub info: NvidiaDeviceInfo,
    pub temperature_celsius: Option<f64>,
    pub utilization: Option<NvidiaUtilization>,
    pub memory: Option<NvidiaMemory>,
    pub fan_speed_ratio: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct FakeNvidiaProvider {
    supported: bool,
    devices: Vec<FakeNvidiaDevice>,
    device_count_error: Option<NvidiaError>,
}

impl FakeNvidiaProvider {
    pub fn new(devices: Vec<FakeNvidiaDevice>) -> Self {
        Self {
            supported: true,
            devices,
            device_count_error: None,
        }
    }

    pub fn unsupported() -> Self {
        Self {
            supported: false,
            devices: Vec::new(),
            device_count_error: None,
        }
    }

    pub fn with_device_count_error(error: NvidiaError) -> Self {
        Self {
            supported: true,
            devices: Vec::new(),
            device_count_error: Some(error),
        }
    }

    pub fn one_gpu() -> Self {
        Self::new(vec![FakeNvidiaDevice {
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
        }])
    }

    fn device(&self, index: u32) -> Result<&FakeNvidiaDevice, NvidiaError> {
        self.devices
            .get(index as usize)
            .ok_or(NvidiaError::DeviceIndexOutOfRange { index })
    }
}

impl NvidiaProvider for FakeNvidiaProvider {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_provider_returns_one_gpu() {
        let mut provider = FakeNvidiaProvider::one_gpu();

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
    }
}
