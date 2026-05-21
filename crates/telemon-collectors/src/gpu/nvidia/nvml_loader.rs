use std::ffi::CStr;
use std::os::raw::{c_char, c_uint};
use std::path::PathBuf;
use std::ptr;

use libloading::{Library, Symbol};
use thiserror::Error;

use super::nvml_ffi;

const NAME_BUFFER_LEN: usize = 96;
const UUID_BUFFER_LEN: usize = 96;
const SERIAL_BUFFER_LEN: usize = 96;
const VBIOS_BUFFER_LEN: usize = 96;

#[derive(Debug, Error)]
pub enum NvmlLoadError {
    #[error("NVML library not found; tried: {candidates:?}")]
    LibraryNotFound {
        candidates: Vec<String>,
        errors: Vec<String>,
    },
    #[error("NVML symbol {symbol} not found in {library}: {message}")]
    SymbolNotFound {
        library: String,
        symbol: &'static str,
        message: String,
    },
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("{operation} failed with NVML code {code}: {message}")]
pub struct NvmlCallError {
    pub operation: &'static str,
    pub code: nvml_ffi::NvmlReturn,
    pub message: String,
}

#[derive(Debug, Clone)]
struct LibraryCandidate {
    path: PathBuf,
    display: String,
}

pub struct NvmlApi {
    _library: Library,
    nvml_init_v2: nvml_ffi::NvmlInitV2,
    nvml_shutdown: nvml_ffi::NvmlShutdown,
    nvml_device_get_count_v2: nvml_ffi::NvmlDeviceGetCountV2,
    nvml_device_get_handle_by_index_v2: nvml_ffi::NvmlDeviceGetHandleByIndexV2,
    nvml_device_get_name: nvml_ffi::NvmlDeviceGetName,
    nvml_device_get_uuid: Option<nvml_ffi::NvmlDeviceGetUuid>,
    nvml_device_get_temperature: nvml_ffi::NvmlDeviceGetTemperature,
    nvml_device_get_utilization_rates: nvml_ffi::NvmlDeviceGetUtilizationRates,
    nvml_device_get_memory_info: nvml_ffi::NvmlDeviceGetMemoryInfo,
    nvml_device_get_fan_speed: nvml_ffi::NvmlDeviceGetFanSpeed,
    nvml_device_get_serial: Option<nvml_ffi::NvmlDeviceGetSerial>,
    nvml_device_get_vbios_version: Option<nvml_ffi::NvmlDeviceGetVbiosVersion>,
    nvml_device_get_power_usage: Option<nvml_ffi::NvmlDeviceGetPowerUsage>,
    nvml_device_get_enforced_power_limit: Option<nvml_ffi::NvmlDeviceGetEnforcedPowerLimit>,
    nvml_device_get_clock_info: Option<nvml_ffi::NvmlDeviceGetClockInfo>,
    nvml_device_get_performance_state: Option<nvml_ffi::NvmlDeviceGetPerformanceState>,
    nvml_error_string: nvml_ffi::NvmlErrorString,
}

impl NvmlApi {
    pub fn load(configured_paths: &[PathBuf]) -> Result<Self, NvmlLoadError> {
        let candidates = library_candidates(configured_paths);
        let mut errors = Vec::new();

        for candidate in &candidates {
            // Loading a dynamic library is unsafe because constructors may run and the
            // symbols are trusted to match the declared ABI.
            match unsafe { Library::new(&candidate.path) } {
                Ok(library) => {
                    return unsafe { Self::from_library(library, candidate.display.as_str()) };
                }
                Err(error) => {
                    errors.push(format!("{}: {error}", candidate.display));
                }
            }
        }

        Err(NvmlLoadError::LibraryNotFound {
            candidates: candidates
                .into_iter()
                .map(|candidate| candidate.display)
                .collect(),
            errors,
        })
    }

    unsafe fn from_library(library: Library, library_name: &str) -> Result<Self, NvmlLoadError> {
        let nvml_init_v2 = load_symbol(&library, library_name, "nvmlInit_v2", b"nvmlInit_v2\0")?;
        let nvml_shutdown = load_symbol(&library, library_name, "nvmlShutdown", b"nvmlShutdown\0")?;
        let nvml_device_get_count_v2 = load_symbol(
            &library,
            library_name,
            "nvmlDeviceGetCount_v2",
            b"nvmlDeviceGetCount_v2\0",
        )?;
        let nvml_device_get_handle_by_index_v2 = load_symbol(
            &library,
            library_name,
            "nvmlDeviceGetHandleByIndex_v2",
            b"nvmlDeviceGetHandleByIndex_v2\0",
        )?;
        let nvml_device_get_name = load_symbol(
            &library,
            library_name,
            "nvmlDeviceGetName",
            b"nvmlDeviceGetName\0",
        )?;
        let nvml_device_get_uuid = load_optional_symbol(&library, b"nvmlDeviceGetUUID\0");
        let nvml_device_get_temperature = load_symbol(
            &library,
            library_name,
            "nvmlDeviceGetTemperature",
            b"nvmlDeviceGetTemperature\0",
        )?;
        let nvml_device_get_utilization_rates = load_symbol(
            &library,
            library_name,
            "nvmlDeviceGetUtilizationRates",
            b"nvmlDeviceGetUtilizationRates\0",
        )?;
        let nvml_device_get_memory_info = load_symbol(
            &library,
            library_name,
            "nvmlDeviceGetMemoryInfo",
            b"nvmlDeviceGetMemoryInfo\0",
        )?;
        let nvml_device_get_fan_speed = load_symbol(
            &library,
            library_name,
            "nvmlDeviceGetFanSpeed",
            b"nvmlDeviceGetFanSpeed\0",
        )?;
        let nvml_device_get_serial = load_optional_symbol(&library, b"nvmlDeviceGetSerial\0");
        let nvml_device_get_vbios_version =
            load_optional_symbol(&library, b"nvmlDeviceGetVbiosVersion\0");
        let nvml_device_get_power_usage =
            load_optional_symbol(&library, b"nvmlDeviceGetPowerUsage\0");
        let nvml_device_get_enforced_power_limit =
            load_optional_symbol(&library, b"nvmlDeviceGetEnforcedPowerLimit\0");
        let nvml_device_get_clock_info =
            load_optional_symbol(&library, b"nvmlDeviceGetClockInfo\0");
        let nvml_device_get_performance_state =
            load_optional_symbol(&library, b"nvmlDeviceGetPerformanceState\0");
        let nvml_error_string = load_symbol(
            &library,
            library_name,
            "nvmlErrorString",
            b"nvmlErrorString\0",
        )?;

        Ok(Self {
            _library: library,
            nvml_init_v2,
            nvml_shutdown,
            nvml_device_get_count_v2,
            nvml_device_get_handle_by_index_v2,
            nvml_device_get_name,
            nvml_device_get_uuid,
            nvml_device_get_temperature,
            nvml_device_get_utilization_rates,
            nvml_device_get_memory_info,
            nvml_device_get_fan_speed,
            nvml_device_get_serial,
            nvml_device_get_vbios_version,
            nvml_device_get_power_usage,
            nvml_device_get_enforced_power_limit,
            nvml_device_get_clock_info,
            nvml_device_get_performance_state,
            nvml_error_string,
        })
    }

    pub fn init(&self) -> Result<(), NvmlCallError> {
        // The function pointer was loaded from NVML and has the declared C ABI.
        self.check_return("nvmlInit_v2", unsafe { (self.nvml_init_v2)() })
    }

    pub fn shutdown(&self) -> Result<(), NvmlCallError> {
        // The function pointer was loaded from NVML and has the declared C ABI.
        self.check_return("nvmlShutdown", unsafe { (self.nvml_shutdown)() })
    }

    pub fn device_count(&self) -> Result<u32, NvmlCallError> {
        let mut count: c_uint = 0;
        // NVML writes a single unsigned count to the provided valid pointer.
        let result = unsafe { (self.nvml_device_get_count_v2)(&mut count) };
        self.check_return("nvmlDeviceGetCount_v2", result)?;
        Ok(count as u32)
    }

    pub fn device_name(&self, index: u32) -> Result<Option<String>, NvmlCallError> {
        let device = self.device_handle(index)?;
        let mut buffer = [0 as c_char; NAME_BUFFER_LEN];
        // NVML writes at most NAME_BUFFER_LEN bytes and NUL-terminates on success.
        let result = unsafe {
            (self.nvml_device_get_name)(device, buffer.as_mut_ptr(), NAME_BUFFER_LEN as c_uint)
        };
        if !self.check_optional_return("nvmlDeviceGetName", result)? {
            return Ok(None);
        }
        buffer[NAME_BUFFER_LEN - 1] = 0 as c_char;
        Ok(c_string_from_buffer(&buffer))
    }

    pub fn device_uuid(&self, index: u32) -> Result<Option<String>, NvmlCallError> {
        let Some(get_uuid) = self.nvml_device_get_uuid else {
            return Ok(None);
        };

        let device = self.device_handle(index)?;
        let mut buffer = [0 as c_char; UUID_BUFFER_LEN];
        // NVML writes at most UUID_BUFFER_LEN bytes and NUL-terminates on success.
        let result = unsafe { get_uuid(device, buffer.as_mut_ptr(), UUID_BUFFER_LEN as c_uint) };
        if !self.check_optional_return("nvmlDeviceGetUUID", result)? {
            return Ok(None);
        }
        buffer[UUID_BUFFER_LEN - 1] = 0 as c_char;
        Ok(c_string_from_buffer(&buffer))
    }

    pub fn device_temperature_celsius(&self, index: u32) -> Result<Option<f64>, NvmlCallError> {
        let device = self.device_handle(index)?;
        let mut temperature: c_uint = 0;
        // NVML writes the GPU temperature in whole Celsius degrees.
        let result = unsafe {
            (self.nvml_device_get_temperature)(
                device,
                nvml_ffi::NVML_TEMPERATURE_GPU,
                &mut temperature,
            )
        };
        if !self.check_optional_return("nvmlDeviceGetTemperature", result)? {
            return Ok(None);
        }
        Ok(Some(temperature as f64))
    }

    pub fn device_utilization_rates(
        &self,
        index: u32,
    ) -> Result<Option<nvml_ffi::NvmlUtilization>, NvmlCallError> {
        let device = self.device_handle(index)?;
        let mut utilization = nvml_ffi::NvmlUtilization::default();
        // NVML writes a plain C struct containing graphics and memory percentages.
        let result = unsafe { (self.nvml_device_get_utilization_rates)(device, &mut utilization) };
        if !self.check_optional_return("nvmlDeviceGetUtilizationRates", result)? {
            return Ok(None);
        }
        Ok(Some(utilization))
    }

    pub fn device_memory_info(
        &self,
        index: u32,
    ) -> Result<Option<nvml_ffi::NvmlMemory>, NvmlCallError> {
        let device = self.device_handle(index)?;
        let mut memory = nvml_ffi::NvmlMemory::default();
        // NVML writes a plain C struct containing byte counters.
        let result = unsafe { (self.nvml_device_get_memory_info)(device, &mut memory) };
        if !self.check_optional_return("nvmlDeviceGetMemoryInfo", result)? {
            return Ok(None);
        }
        Ok(Some(memory))
    }

    pub fn device_fan_speed_percent(&self, index: u32) -> Result<Option<u32>, NvmlCallError> {
        let device = self.device_handle(index)?;
        let mut percent: c_uint = 0;
        // NVML writes a whole-number fan speed percentage.
        let result = unsafe { (self.nvml_device_get_fan_speed)(device, &mut percent) };
        if !self.check_optional_return("nvmlDeviceGetFanSpeed", result)? {
            return Ok(None);
        }
        Ok(Some(percent as u32))
    }

    pub fn device_serial(&self, index: u32) -> Result<Option<String>, NvmlCallError> {
        let Some(get_serial) = self.nvml_device_get_serial else {
            return Ok(None);
        };

        let device = self.device_handle(index)?;
        let mut buffer = [0 as c_char; SERIAL_BUFFER_LEN];
        // NVML writes at most SERIAL_BUFFER_LEN bytes and NUL-terminates on success.
        let result =
            unsafe { get_serial(device, buffer.as_mut_ptr(), SERIAL_BUFFER_LEN as c_uint) };
        if !self.check_optional_return("nvmlDeviceGetSerial", result)? {
            return Ok(None);
        }
        buffer[SERIAL_BUFFER_LEN - 1] = 0 as c_char;
        Ok(c_string_from_buffer(&buffer))
    }

    pub fn device_vbios_version(&self, index: u32) -> Result<Option<String>, NvmlCallError> {
        let Some(get_vbios_version) = self.nvml_device_get_vbios_version else {
            return Ok(None);
        };

        let device = self.device_handle(index)?;
        let mut buffer = [0 as c_char; VBIOS_BUFFER_LEN];
        // NVML writes at most VBIOS_BUFFER_LEN bytes and NUL-terminates on success.
        let result =
            unsafe { get_vbios_version(device, buffer.as_mut_ptr(), VBIOS_BUFFER_LEN as c_uint) };
        if !self.check_optional_return("nvmlDeviceGetVbiosVersion", result)? {
            return Ok(None);
        }
        buffer[VBIOS_BUFFER_LEN - 1] = 0 as c_char;
        Ok(c_string_from_buffer(&buffer))
    }

    pub fn device_power_usage_milliwatts(&self, index: u32) -> Result<Option<u32>, NvmlCallError> {
        let Some(get_power_usage) = self.nvml_device_get_power_usage else {
            return Ok(None);
        };

        let device = self.device_handle(index)?;
        let mut milliwatts: c_uint = 0;
        // NVML writes current board power usage in milliwatts.
        let result = unsafe { get_power_usage(device, &mut milliwatts) };
        if !self.check_optional_return("nvmlDeviceGetPowerUsage", result)? {
            return Ok(None);
        }
        Ok(Some(milliwatts as u32))
    }

    pub fn device_power_limit_milliwatts(&self, index: u32) -> Result<Option<u32>, NvmlCallError> {
        let Some(get_enforced_power_limit) = self.nvml_device_get_enforced_power_limit else {
            return Ok(None);
        };

        let device = self.device_handle(index)?;
        let mut milliwatts: c_uint = 0;
        // NVML writes current enforced power limit in milliwatts.
        let result = unsafe { get_enforced_power_limit(device, &mut milliwatts) };
        if !self.check_optional_return("nvmlDeviceGetEnforcedPowerLimit", result)? {
            return Ok(None);
        }
        Ok(Some(milliwatts as u32))
    }

    pub fn device_clock_mhz(
        &self,
        index: u32,
        clock_type: c_uint,
    ) -> Result<Option<u32>, NvmlCallError> {
        let Some(get_clock_info) = self.nvml_device_get_clock_info else {
            return Ok(None);
        };

        let device = self.device_handle(index)?;
        let mut mhz: c_uint = 0;
        // NVML writes the requested clock value in MHz.
        let result = unsafe { get_clock_info(device, clock_type, &mut mhz) };
        if !self.check_optional_return("nvmlDeviceGetClockInfo", result)? {
            return Ok(None);
        }
        Ok(Some(mhz as u32))
    }

    pub fn device_performance_state(&self, index: u32) -> Result<Option<u32>, NvmlCallError> {
        let Some(get_performance_state) = self.nvml_device_get_performance_state else {
            return Ok(None);
        };

        let device = self.device_handle(index)?;
        let mut state: c_uint = 0;
        // NVML writes a P-state number where lower values usually mean higher performance.
        let result = unsafe { get_performance_state(device, &mut state) };
        if !self.check_optional_return("nvmlDeviceGetPerformanceState", result)? {
            return Ok(None);
        }
        Ok(Some(state as u32))
    }

    fn device_handle(&self, index: u32) -> Result<nvml_ffi::NvmlDevice, NvmlCallError> {
        let mut device: nvml_ffi::NvmlDevice = ptr::null_mut();
        // NVML writes an opaque device handle for the requested index.
        let result =
            unsafe { (self.nvml_device_get_handle_by_index_v2)(index as c_uint, &mut device) };
        self.check_return("nvmlDeviceGetHandleByIndex_v2", result)?;
        if device.is_null() {
            return Err(NvmlCallError {
                operation: "nvmlDeviceGetHandleByIndex_v2",
                code: nvml_ffi::NVML_SUCCESS,
                message: "NVML returned a null device handle".to_string(),
            });
        }
        Ok(device)
    }

    fn check_return(
        &self,
        operation: &'static str,
        code: nvml_ffi::NvmlReturn,
    ) -> Result<(), NvmlCallError> {
        if code == nvml_ffi::NVML_SUCCESS {
            Ok(())
        } else {
            Err(self.call_error(operation, code))
        }
    }

    fn check_optional_return(
        &self,
        operation: &'static str,
        code: nvml_ffi::NvmlReturn,
    ) -> Result<bool, NvmlCallError> {
        match code {
            nvml_ffi::NVML_SUCCESS => Ok(true),
            nvml_ffi::NVML_ERROR_NOT_SUPPORTED | nvml_ffi::NVML_ERROR_NOT_FOUND => Ok(false),
            other => Err(self.call_error(operation, other)),
        }
    }

    fn call_error(&self, operation: &'static str, code: nvml_ffi::NvmlReturn) -> NvmlCallError {
        NvmlCallError {
            operation,
            code,
            message: self.error_message(code),
        }
    }

    fn error_message(&self, code: nvml_ffi::NvmlReturn) -> String {
        // nvmlErrorString returns a static NUL-terminated string for known error codes.
        let pointer = unsafe { (self.nvml_error_string)(code) };
        if pointer.is_null() {
            return format!("unknown NVML error {code}");
        }
        // The pointer is owned by NVML and valid for read-only string conversion.
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }
}

unsafe fn load_symbol<T: Copy>(
    library: &Library,
    library_name: &str,
    symbol_name: &'static str,
    symbol: &'static [u8],
) -> Result<T, NvmlLoadError> {
    let loaded: Symbol<'_, T> =
        library
            .get(symbol)
            .map_err(|error| NvmlLoadError::SymbolNotFound {
                library: library_name.to_string(),
                symbol: symbol_name,
                message: error.to_string(),
            })?;
    Ok(*loaded)
}

unsafe fn load_optional_symbol<T: Copy>(library: &Library, symbol: &'static [u8]) -> Option<T> {
    let loaded: Symbol<'_, T> = library.get(symbol).ok()?;
    Some(*loaded)
}

fn c_string_from_buffer(buffer: &[c_char]) -> Option<String> {
    let value = unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .trim()
        .to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn library_candidates(configured_paths: &[PathBuf]) -> Vec<LibraryCandidate> {
    let mut candidates = configured_paths
        .iter()
        .map(|path| LibraryCandidate {
            path: path.clone(),
            display: path.display().to_string(),
        })
        .collect::<Vec<_>>();

    for name in default_library_names() {
        candidates.push(LibraryCandidate {
            path: PathBuf::from(name),
            display: name.to_string(),
        });
    }

    candidates
}

fn default_library_names() -> &'static [&'static str] {
    if cfg!(target_os = "linux") {
        &["libnvidia-ml.so.1", "libnvidia-ml.so"]
    } else if cfg!(target_os = "windows") {
        &["nvml.dll"]
    } else {
        &[]
    }
}

#[cfg(test)]
pub(super) fn library_candidate_names(configured_paths: &[PathBuf]) -> Vec<String> {
    library_candidates(configured_paths)
        .into_iter()
        .map(|candidate| candidate.display)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_library_paths_precede_platform_defaults() {
        let candidates = library_candidate_names(&[PathBuf::from("/opt/nvidia/libnvidia-ml.so")]);

        assert_eq!(candidates.first().unwrap(), "/opt/nvidia/libnvidia-ml.so");
        if cfg!(target_os = "linux") {
            assert_eq!(candidates[1], "libnvidia-ml.so.1");
            assert_eq!(candidates[2], "libnvidia-ml.so");
        }
        if cfg!(target_os = "windows") {
            assert_eq!(candidates[1], "nvml.dll");
        }
    }
}
