use std::path::PathBuf;

use thiserror::Error;

use super::model::{
    percent_to_ratio, validate_memory, validate_temperature_celsius, NvidiaDeviceInfo,
    NvidiaMemory, NvidiaUtilization,
};
use super::nvml_loader::{NvmlApi, NvmlCallError, NvmlLoadError};

pub trait NvidiaProvider: Send + Sync {
    fn is_supported(&self) -> bool;
    fn device_count(&self) -> Result<u32, NvidiaError>;
    fn device_info(&mut self, index: u32) -> Result<NvidiaDeviceInfo, NvidiaError>;
    fn temperature_celsius(&mut self, index: u32) -> Result<Option<f64>, NvidiaError>;
    fn utilization(&mut self, index: u32) -> Result<Option<NvidiaUtilization>, NvidiaError>;
    fn memory(&mut self, index: u32) -> Result<Option<NvidiaMemory>, NvidiaError>;
    fn fan_speed_ratio(&mut self, index: u32) -> Result<Option<f64>, NvidiaError>;
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum NvidiaError {
    #[error("NVML library not found; tried: {candidates:?}")]
    LibraryNotFound {
        candidates: Vec<String>,
        errors: Vec<String>,
    },
    #[error("required NVML symbol {symbol} not found in {library}: {message}")]
    SymbolNotFound {
        library: String,
        symbol: &'static str,
        message: String,
    },
    #[error("NVML initialization failed with code {code}: {message}")]
    InitFailed { code: u32, message: String },
    #[error("NVML call {operation} failed with code {code}: {message}")]
    NvmlCall {
        operation: &'static str,
        code: u32,
        message: String,
    },
    #[error("NVIDIA device index {index} is out of range")]
    DeviceIndexOutOfRange { index: u32 },
}

impl NvidiaError {
    pub fn status(&self) -> &'static str {
        match self {
            Self::LibraryNotFound { .. } => "library_missing",
            Self::SymbolNotFound { .. } | Self::InitFailed { .. } => "unsupported",
            Self::NvmlCall { .. } | Self::DeviceIndexOutOfRange { .. } => "error",
        }
    }

    pub fn library_loaded(&self) -> bool {
        !matches!(self, Self::LibraryNotFound { .. })
    }
}

impl From<NvmlLoadError> for NvidiaError {
    fn from(error: NvmlLoadError) -> Self {
        match error {
            NvmlLoadError::LibraryNotFound { candidates, errors } => {
                Self::LibraryNotFound { candidates, errors }
            }
            NvmlLoadError::SymbolNotFound {
                library,
                symbol,
                message,
            } => Self::SymbolNotFound {
                library,
                symbol,
                message,
            },
        }
    }
}

impl From<NvmlCallError> for NvidiaError {
    fn from(error: NvmlCallError) -> Self {
        Self::NvmlCall {
            operation: error.operation,
            code: error.code,
            message: error.message,
        }
    }
}

pub struct DynamicNvmlProvider {
    api: NvmlApi,
}

impl DynamicNvmlProvider {
    pub fn load(library_paths: &[PathBuf]) -> Result<Self, NvidiaError> {
        let api = NvmlApi::load(library_paths).map_err(NvidiaError::from)?;
        api.init().map_err(|error| NvidiaError::InitFailed {
            code: error.code,
            message: error.message,
        })?;
        Ok(Self { api })
    }
}

impl Drop for DynamicNvmlProvider {
    fn drop(&mut self) {
        if let Err(error) = self.api.shutdown() {
            tracing::debug!(%error, "NVML shutdown failed");
        }
    }
}

impl NvidiaProvider for DynamicNvmlProvider {
    fn is_supported(&self) -> bool {
        true
    }

    fn device_count(&self) -> Result<u32, NvidiaError> {
        self.api.device_count().map_err(NvidiaError::from)
    }

    fn device_info(&mut self, index: u32) -> Result<NvidiaDeviceInfo, NvidiaError> {
        Ok(NvidiaDeviceInfo {
            index,
            name: self.api.device_name(index).map_err(NvidiaError::from)?,
            uuid: self.api.device_uuid(index).map_err(NvidiaError::from)?,
        })
    }

    fn temperature_celsius(&mut self, index: u32) -> Result<Option<f64>, NvidiaError> {
        Ok(self
            .api
            .device_temperature_celsius(index)
            .map_err(NvidiaError::from)?
            .and_then(validate_temperature_celsius))
    }

    fn utilization(&mut self, index: u32) -> Result<Option<NvidiaUtilization>, NvidiaError> {
        let Some(raw) = self
            .api
            .device_utilization_rates(index)
            .map_err(NvidiaError::from)?
        else {
            return Ok(None);
        };

        let Some(graphics_ratio) = percent_to_ratio(raw.gpu) else {
            return Ok(None);
        };
        let Some(memory_ratio) = percent_to_ratio(raw.memory) else {
            return Ok(None);
        };

        Ok(Some(NvidiaUtilization {
            graphics_ratio,
            memory_ratio,
        }))
    }

    fn memory(&mut self, index: u32) -> Result<Option<NvidiaMemory>, NvidiaError> {
        let Some(raw) = self
            .api
            .device_memory_info(index)
            .map_err(NvidiaError::from)?
        else {
            return Ok(None);
        };

        Ok(validate_memory(raw.total, raw.used, raw.free))
    }

    fn fan_speed_ratio(&mut self, index: u32) -> Result<Option<f64>, NvidiaError> {
        Ok(self
            .api
            .device_fan_speed_percent(index)
            .map_err(NvidiaError::from)?
            .and_then(percent_to_ratio))
    }
}
