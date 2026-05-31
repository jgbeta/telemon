use std::ffi::CString;
use std::io;
use std::mem::{size_of, MaybeUninit};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use libc::{c_char, c_long};
use telemon_core::config::GamescopeMangoappConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MangoAppFrameSample {
    pub pid: u32,
    pub app_frametime_ns: u64,
    pub visible_frametime_ns: u64,
    pub latency_ns: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MangoAppReadResult {
    pub samples_read: usize,
    pub dropped_zero: u64,
    pub dropped_too_large: u64,
}

pub struct MangoAppFrameReader {
    queue_id: libc::c_int,
    max_frame_time_ns: u64,
}

impl MangoAppFrameReader {
    pub fn open(config: &GamescopeMangoappConfig, max_frame_time_ns: u64) -> io::Result<Self> {
        let queue_id = mangoapp_queue_id(&config.ftok_path, config.project_id)?;
        Ok(Self {
            queue_id,
            max_frame_time_ns,
        })
    }

    pub fn read_available(
        &self,
        max_messages: usize,
        samples: &mut Vec<MangoAppFrameSample>,
    ) -> io::Result<MangoAppReadResult> {
        let mut result = MangoAppReadResult::default();
        for _ in 0..max_messages {
            let Some(sample) = self.read_one()? else {
                break;
            };
            if sample.visible_frametime_ns == 0 {
                result.dropped_zero = result.dropped_zero.saturating_add(1);
                continue;
            }
            if sample.visible_frametime_ns > self.max_frame_time_ns {
                result.dropped_too_large = result.dropped_too_large.saturating_add(1);
                continue;
            }
            samples.push(sample);
            result.samples_read += 1;
        }
        Ok(result)
    }

    fn read_one(&self) -> io::Result<Option<MangoAppFrameSample>> {
        let mut message = MaybeUninit::<MangoAppMsgV1>::zeroed();
        let message_size = size_of::<MangoAppMsgV1>() - size_of::<c_long>();
        let result = unsafe {
            libc::msgrcv(
                self.queue_id,
                message.as_mut_ptr().cast(),
                message_size,
                0,
                libc::IPC_NOWAIT,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOMSG) {
                return Ok(None);
            }
            return Err(error);
        }

        let message = unsafe { message.assume_init() };
        Ok(Some(MangoAppFrameSample {
            pid: unsafe { std::ptr::addr_of!(message.pid).read_unaligned() },
            app_frametime_ns: unsafe {
                std::ptr::addr_of!(message.app_frametime_ns).read_unaligned()
            },
            visible_frametime_ns: unsafe {
                std::ptr::addr_of!(message.visible_frametime_ns).read_unaligned()
            },
            latency_ns: unsafe { std::ptr::addr_of!(message.latency_ns).read_unaligned() },
        }))
    }
}

pub fn mangoapp_queue_id(path: &Path, project_id: i32) -> io::Result<libc::c_int> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    let key = unsafe { libc::ftok(path.as_ptr(), project_id) };
    if key == -1 {
        return Err(io::Error::last_os_error());
    }
    let queue_id = unsafe { libc::msgget(key, 0) };
    if queue_id == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(queue_id)
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct MangoAppMsgHeader {
    msg_type: c_long,
    version: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct MangoAppMsgV1 {
    header: MangoAppMsgHeader,
    pid: u32,
    app_frametime_ns: u64,
    fsr_upscale: u8,
    fsr_sharpness: u8,
    visible_frametime_ns: u64,
    latency_ns: u64,
    output_width: u32,
    output_height: u32,
    display_refresh: u16,
    flags: u8,
    engine_name: [c_char; 40],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_size_excludes_sysv_type_long() {
        assert!(size_of::<MangoAppMsgV1>() > size_of::<c_long>());
        assert_eq!(size_of::<MangoAppMsgHeader>(), size_of::<c_long>() + 4);
    }
}
