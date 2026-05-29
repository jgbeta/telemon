use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacosThermalState {
    Unknown,
    Nominal,
    Fair,
    Serious,
    Critical,
}

pub trait MacosThermalProvider: Send + Sync {
    fn thermal_state(&self) -> Result<MacosThermalState>;
}

#[derive(Debug, Default)]
pub struct DefaultMacosThermalProvider;

impl DefaultMacosThermalProvider {
    pub fn new() -> Self {
        Self
    }
}

impl MacosThermalState {
    pub const fn all() -> [Self; 5] {
        [
            Self::Nominal,
            Self::Fair,
            Self::Serious,
            Self::Critical,
            Self::Unknown,
        ]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::Fair => "fair",
            Self::Serious => "serious",
            Self::Critical => "critical",
            Self::Unknown => "unknown",
        }
    }

    pub const fn numeric_value(self) -> f64 {
        match self {
            Self::Unknown => -1.0,
            Self::Nominal => 0.0,
            Self::Fair => 1.0,
            Self::Serious => 2.0,
            Self::Critical => 3.0,
        }
    }

    #[cfg(target_os = "macos")]
    const fn from_process_info_value(value: usize) -> Self {
        match value {
            0 => Self::Nominal,
            1 => Self::Fair,
            2 => Self::Serious,
            3 => Self::Critical,
            _ => Self::Unknown,
        }
    }
}

impl MacosThermalProvider for DefaultMacosThermalProvider {
    fn thermal_state(&self) -> Result<MacosThermalState> {
        platform_thermal_state()
    }
}

#[cfg(target_os = "macos")]
fn platform_thermal_state() -> Result<MacosThermalState> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_void};

    type Id = *mut c_void;
    type Sel = *mut c_void;
    type MsgSendId = unsafe extern "C" fn(Id, Sel) -> Id;
    type MsgSendUsize = unsafe extern "C" fn(Id, Sel) -> usize;

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    let class_name = CString::new("NSProcessInfo")?;
    let process_info_name = CString::new("processInfo")?;
    let thermal_state_name = CString::new("thermalState")?;

    let class = unsafe { objc_getClass(class_name.as_ptr()) };
    if class.is_null() {
        anyhow::bail!("NSProcessInfo class is unavailable");
    }

    let process_info_selector = unsafe { sel_registerName(process_info_name.as_ptr()) };
    let thermal_state_selector = unsafe { sel_registerName(thermal_state_name.as_ptr()) };
    if process_info_selector.is_null() || thermal_state_selector.is_null() {
        anyhow::bail!("NSProcessInfo thermal selectors are unavailable");
    }

    let msg_send_id: MsgSendId = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let msg_send_usize: MsgSendUsize = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let process_info = unsafe { msg_send_id(class, process_info_selector) };
    if process_info.is_null() {
        anyhow::bail!("NSProcessInfo processInfo returned null");
    }

    let value = unsafe { msg_send_usize(process_info, thermal_state_selector) };
    Ok(MacosThermalState::from_process_info_value(value))
}

#[cfg(not(target_os = "macos"))]
fn platform_thermal_state() -> Result<MacosThermalState> {
    anyhow::bail!("macos_thermal_state is unsupported on this OS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_labels_are_stable() {
        assert_eq!(MacosThermalState::Nominal.label(), "nominal");
        assert_eq!(MacosThermalState::Fair.label(), "fair");
        assert_eq!(MacosThermalState::Serious.label(), "serious");
        assert_eq!(MacosThermalState::Critical.label(), "critical");
        assert_eq!(MacosThermalState::Unknown.label(), "unknown");
    }

    #[test]
    fn state_numeric_values_match_contract() {
        assert_eq!(MacosThermalState::Unknown.numeric_value(), -1.0);
        assert_eq!(MacosThermalState::Nominal.numeric_value(), 0.0);
        assert_eq!(MacosThermalState::Fair.numeric_value(), 1.0);
        assert_eq!(MacosThermalState::Serious.numeric_value(), 2.0);
        assert_eq!(MacosThermalState::Critical.numeric_value(), 3.0);
    }
}
