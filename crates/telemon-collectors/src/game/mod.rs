pub mod capture;
#[cfg(target_os = "linux")]
pub mod gamescope_wayland;
pub mod live_window;
#[cfg(target_os = "linux")]
pub mod mangoapp;
pub mod steam;
