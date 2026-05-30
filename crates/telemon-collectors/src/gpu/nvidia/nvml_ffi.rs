use std::os::raw::{c_char, c_uint, c_ulonglong, c_void};

pub type NvmlReturn = c_uint;
pub type NvmlDevice = *mut c_void;

pub const NVML_SUCCESS: NvmlReturn = 0;
pub const NVML_ERROR_NOT_SUPPORTED: NvmlReturn = 3;
pub const NVML_ERROR_NOT_FOUND: NvmlReturn = 6;
pub const NVML_TEMPERATURE_GPU: c_uint = 0;
pub const NVML_CLOCK_GRAPHICS: c_uint = 0;
pub const NVML_CLOCK_MEM: c_uint = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NvmlUtilization {
    pub gpu: c_uint,
    pub memory: c_uint,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NvmlMemory {
    pub total: c_ulonglong,
    pub free: c_ulonglong,
    pub used: c_ulonglong,
}

pub type NvmlInitV2 = unsafe extern "C" fn() -> NvmlReturn;
pub type NvmlShutdown = unsafe extern "C" fn() -> NvmlReturn;
pub type NvmlDeviceGetCountV2 = unsafe extern "C" fn(*mut c_uint) -> NvmlReturn;
pub type NvmlDeviceGetHandleByIndexV2 = unsafe extern "C" fn(c_uint, *mut NvmlDevice) -> NvmlReturn;
pub type NvmlDeviceGetName = unsafe extern "C" fn(NvmlDevice, *mut c_char, c_uint) -> NvmlReturn;
pub type NvmlDeviceGetUuid = unsafe extern "C" fn(NvmlDevice, *mut c_char, c_uint) -> NvmlReturn;
pub type NvmlDeviceGetTemperature =
    unsafe extern "C" fn(NvmlDevice, c_uint, *mut c_uint) -> NvmlReturn;
pub type NvmlDeviceGetUtilizationRates =
    unsafe extern "C" fn(NvmlDevice, *mut NvmlUtilization) -> NvmlReturn;
pub type NvmlDeviceGetMemoryInfo = unsafe extern "C" fn(NvmlDevice, *mut NvmlMemory) -> NvmlReturn;
pub type NvmlDeviceGetFanSpeed = unsafe extern "C" fn(NvmlDevice, *mut c_uint) -> NvmlReturn;
pub type NvmlDeviceGetSerial = unsafe extern "C" fn(NvmlDevice, *mut c_char, c_uint) -> NvmlReturn;
pub type NvmlDeviceGetVbiosVersion =
    unsafe extern "C" fn(NvmlDevice, *mut c_char, c_uint) -> NvmlReturn;
pub type NvmlDeviceGetPowerUsage = unsafe extern "C" fn(NvmlDevice, *mut c_uint) -> NvmlReturn;
pub type NvmlDeviceGetEnforcedPowerLimit =
    unsafe extern "C" fn(NvmlDevice, *mut c_uint) -> NvmlReturn;
pub type NvmlDeviceGetClockInfo =
    unsafe extern "C" fn(NvmlDevice, c_uint, *mut c_uint) -> NvmlReturn;
pub type NvmlDeviceGetPerformanceState =
    unsafe extern "C" fn(NvmlDevice, *mut c_uint) -> NvmlReturn;
pub type NvmlDeviceGetCurrentClocksThrottleReasons =
    unsafe extern "C" fn(NvmlDevice, *mut c_ulonglong) -> NvmlReturn;
pub type NvmlErrorString = unsafe extern "C" fn(NvmlReturn) -> *const c_char;
