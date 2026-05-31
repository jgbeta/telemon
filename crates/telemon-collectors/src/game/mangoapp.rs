use std::ffi::CString;
use std::io;
use std::mem::{offset_of, size_of, MaybeUninit};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use libc::c_long;
use telemon_core::config::GamescopeMangoappConfig;

const MANGOAPP_FRAME_MESSAGE_TYPE: c_long = 1;
const MANGOAPP_MESSAGE_VERSION: u32 = 1;
const INVALID_FRAME_TIME_NS: u64 = u64::MAX;

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
    pub dropped_invalid_sentinel: u64,
    pub dropped_unsupported_version: u64,
    pub dropped_too_short: u64,
}

impl MangoAppReadResult {
    fn record_drop(&mut self, reason: MangoAppDropReason) {
        match reason {
            MangoAppDropReason::Zero => {
                self.dropped_zero = self.dropped_zero.saturating_add(1);
            }
            MangoAppDropReason::TooLarge => {
                self.dropped_too_large = self.dropped_too_large.saturating_add(1);
            }
            MangoAppDropReason::InvalidSentinel => {
                self.dropped_invalid_sentinel = self.dropped_invalid_sentinel.saturating_add(1);
            }
            MangoAppDropReason::UnsupportedVersion => {
                self.dropped_unsupported_version =
                    self.dropped_unsupported_version.saturating_add(1);
            }
            MangoAppDropReason::TooShort => {
                self.dropped_too_short = self.dropped_too_short.saturating_add(1);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MangoAppDropReason {
    Zero,
    TooLarge,
    InvalidSentinel,
    UnsupportedVersion,
    TooShort,
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
            match self.read_one()? {
                Some(MangoAppReadOne::Sample(sample)) => {
                    samples.push(sample);
                    result.samples_read += 1;
                }
                Some(MangoAppReadOne::Dropped(reason)) => result.record_drop(reason),
                None => break,
            }
        }
        Ok(result)
    }

    fn read_one(&self) -> io::Result<Option<MangoAppReadOne>> {
        let mut message = MaybeUninit::<MangoAppMsgV1>::zeroed();
        let message_size = size_of::<MangoAppMsgV1>() - size_of::<c_long>();
        let result = unsafe {
            libc::msgrcv(
                self.queue_id,
                message.as_mut_ptr().cast(),
                message_size,
                MANGOAPP_FRAME_MESSAGE_TYPE,
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
        Ok(Some(decode_message(
            &message,
            result as usize,
            self.max_frame_time_ns,
        )))
    }
}

pub fn mangoapp_queue_id(path: &Path, project_id: i32) -> io::Result<libc::c_int> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    let key = unsafe { libc::ftok(path.as_ptr(), project_id) };
    if key == -1 {
        return Err(io::Error::last_os_error());
    }
    let queue_id = unsafe { libc::msgget(key, 0o666 | libc::IPC_CREAT) };
    if queue_id == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(queue_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MangoAppReadOne {
    Sample(MangoAppFrameSample),
    Dropped(MangoAppDropReason),
}

fn decode_message(
    message: &MangoAppMsgV1,
    payload_len: usize,
    max_frame_time_ns: u64,
) -> MangoAppReadOne {
    if payload_len < size_of::<u32>() {
        return MangoAppReadOne::Dropped(MangoAppDropReason::TooShort);
    }

    let version = unsafe { std::ptr::addr_of!(message.header.version).read_unaligned() };
    if version != MANGOAPP_MESSAGE_VERSION {
        return MangoAppReadOne::Dropped(MangoAppDropReason::UnsupportedVersion);
    }

    if payload_len < payload_bytes_through::<u64>(offset_of!(MangoAppMsgV1, visible_frametime_ns)) {
        return MangoAppReadOne::Dropped(MangoAppDropReason::TooShort);
    }

    let visible_frametime_ns =
        unsafe { std::ptr::addr_of!(message.visible_frametime_ns).read_unaligned() };
    if visible_frametime_ns == INVALID_FRAME_TIME_NS {
        return MangoAppReadOne::Dropped(MangoAppDropReason::InvalidSentinel);
    }
    if visible_frametime_ns == 0 {
        return MangoAppReadOne::Dropped(MangoAppDropReason::Zero);
    }
    if visible_frametime_ns > max_frame_time_ns {
        return MangoAppReadOne::Dropped(MangoAppDropReason::TooLarge);
    }

    MangoAppReadOne::Sample(MangoAppFrameSample {
        pid: read_optional_field(message, payload_len, offset_of!(MangoAppMsgV1, pid))
            .unwrap_or_default(),
        app_frametime_ns: read_optional_field(
            message,
            payload_len,
            offset_of!(MangoAppMsgV1, app_frametime_ns),
        )
        .unwrap_or_default(),
        visible_frametime_ns,
        latency_ns: read_optional_field(
            message,
            payload_len,
            offset_of!(MangoAppMsgV1, latency_ns),
        )
        .unwrap_or_default(),
    })
}

fn read_optional_field<T: Copy>(
    message: &MangoAppMsgV1,
    payload_len: usize,
    field_offset: usize,
) -> Option<T> {
    if payload_len < payload_bytes_through::<T>(field_offset) {
        return None;
    }
    let base = std::ptr::from_ref(message).cast::<u8>();
    Some(unsafe { base.add(field_offset).cast::<T>().read_unaligned() })
}

fn payload_bytes_through<T>(field_offset: usize) -> usize {
    field_offset + size_of::<T>() - size_of::<c_long>()
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
    visible_frametime_ns: u64,
    fsr_upscale: u8,
    fsr_sharpness: u8,
    app_frametime_ns: u64,
    latency_ns: u64,
    output_width: u32,
    output_height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message_with_visible_frametime(version: u32, visible_frametime_ns: u64) -> MangoAppMsgV1 {
        MangoAppMsgV1 {
            header: MangoAppMsgHeader {
                msg_type: MANGOAPP_FRAME_MESSAGE_TYPE,
                version,
            },
            pid: 4242,
            visible_frametime_ns,
            fsr_upscale: 0,
            fsr_sharpness: 0,
            app_frametime_ns: 17_000_000,
            latency_ns: 5_000_000,
            output_width: 1280,
            output_height: 800,
        }
    }

    #[test]
    fn message_size_and_offsets_match_mangohud_protocol() {
        assert_eq!(size_of::<MangoAppMsgHeader>(), size_of::<c_long>() + 4);
        assert_eq!(offset_of!(MangoAppMsgV1, pid), size_of::<c_long>() + 4);
        assert_eq!(
            offset_of!(MangoAppMsgV1, visible_frametime_ns),
            size_of::<c_long>() + 8
        );
        assert_eq!(
            offset_of!(MangoAppMsgV1, app_frametime_ns),
            size_of::<c_long>() + 18
        );
        assert_eq!(size_of::<MangoAppMsgV1>(), size_of::<c_long>() + 42);
    }

    #[test]
    fn frame_message_type_is_exactly_one() {
        assert_eq!(MANGOAPP_FRAME_MESSAGE_TYPE, 1);
    }

    #[test]
    fn decodes_version_one_message() {
        let message = message_with_visible_frametime(1, 16_666_667);
        let payload_len = size_of::<MangoAppMsgV1>() - size_of::<c_long>();
        assert_eq!(
            decode_message(&message, payload_len, 1_000_000_000),
            MangoAppReadOne::Sample(MangoAppFrameSample {
                pid: 4242,
                app_frametime_ns: 17_000_000,
                visible_frametime_ns: 16_666_667,
                latency_ns: 5_000_000,
            })
        );
    }

    #[test]
    fn rejects_invalid_version() {
        let message = message_with_visible_frametime(2, 16_666_667);
        let payload_len = size_of::<MangoAppMsgV1>() - size_of::<c_long>();
        assert_eq!(
            decode_message(&message, payload_len, 1_000_000_000),
            MangoAppReadOne::Dropped(MangoAppDropReason::UnsupportedVersion)
        );
    }

    #[test]
    fn rejects_short_payload_before_visible_frame_time() {
        let message = message_with_visible_frametime(1, 16_666_667);
        let payload_len = payload_bytes_through::<u32>(offset_of!(MangoAppMsgV1, pid));
        assert_eq!(
            decode_message(&message, payload_len, 1_000_000_000),
            MangoAppReadOne::Dropped(MangoAppDropReason::TooShort)
        );
    }

    #[test]
    fn rejects_invalid_frame_times() {
        for (value, reason) in [
            (0, MangoAppDropReason::Zero),
            (u64::MAX, MangoAppDropReason::InvalidSentinel),
            (2_000_000_000, MangoAppDropReason::TooLarge),
        ] {
            let message = message_with_visible_frametime(1, value);
            let payload_len = size_of::<MangoAppMsgV1>() - size_of::<c_long>();
            assert_eq!(
                decode_message(&message, payload_len, 1_000_000_000),
                MangoAppReadOne::Dropped(reason)
            );
        }
    }
}
