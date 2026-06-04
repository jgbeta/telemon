use std::ffi::CString;
use std::io;
use std::mem::{offset_of, size_of, MaybeUninit};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use libc::{c_int, c_long, key_t};
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

    fn has_activity(&self) -> bool {
        self.samples_read > 0
            || self.dropped_zero > 0
            || self.dropped_too_large > 0
            || self.dropped_invalid_sentinel > 0
            || self.dropped_unsupported_version > 0
            || self.dropped_too_short > 0
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MangoAppQueueSource {
    ConfiguredFtok,
    LegacyFailedFtok,
}

impl MangoAppQueueSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::ConfiguredFtok => "configured_ftok",
            Self::LegacyFailedFtok => "legacy_failed_ftok",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct MangoAppQueueReader {
    source: MangoAppQueueSource,
    queue_id: libc::c_int,
}

pub struct MangoAppFrameReader {
    sources: Vec<MangoAppQueueReader>,
    active_source: Option<MangoAppQueueSource>,
    max_frame_time_ns: u64,
}

impl MangoAppFrameReader {
    pub fn open(config: &GamescopeMangoappConfig, max_frame_time_ns: u64) -> io::Result<Self> {
        let mut sources = Vec::new();
        let mut first_error = None;

        match configured_mangoapp_queue_id(&config.ftok_path, config.project_id) {
            Ok(queue_id) => sources.push(MangoAppQueueReader {
                source: MangoAppQueueSource::ConfiguredFtok,
                queue_id,
            }),
            Err(error) => first_error = Some(error),
        }

        if config.legacy_failed_ftok_fallback_enabled {
            match legacy_failed_ftok_queue_id() {
                Ok(queue_id) => sources.push(MangoAppQueueReader {
                    source: MangoAppQueueSource::LegacyFailedFtok,
                    queue_id,
                }),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }

        if sources.is_empty() {
            return Err(first_error.unwrap_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "no MangoApp queues available")
            }));
        }

        Ok(Self {
            sources,
            active_source: None,
            max_frame_time_ns,
        })
    }

    pub fn queue_label(&self) -> &'static str {
        self.active_source
            .or_else(|| self.sources.first().map(|source| source.source))
            .map(MangoAppQueueSource::label)
            .unwrap_or("unavailable")
    }

    pub fn refresh_sources(&mut self, config: &GamescopeMangoappConfig) {
        if !config.legacy_failed_ftok_fallback_enabled
            || self.has_source(MangoAppQueueSource::LegacyFailedFtok)
        {
            return;
        }
        if let Ok(queue_id) = legacy_failed_ftok_queue_id() {
            self.sources.push(MangoAppQueueReader {
                source: MangoAppQueueSource::LegacyFailedFtok,
                queue_id,
            });
        }
    }

    fn has_source(&self, source: MangoAppQueueSource) -> bool {
        self.sources
            .iter()
            .any(|candidate| candidate.source == source)
    }

    pub fn read_available(
        &mut self,
        max_messages: usize,
        samples: &mut Vec<MangoAppFrameSample>,
    ) -> io::Result<MangoAppReadResult> {
        let mut first_activity = None;
        for index in read_order_for_sources(self.sources.len(), self.active_source_index()) {
            let mut source_samples = Vec::new();
            let result = self.read_available_from_source(
                self.sources[index].queue_id,
                max_messages,
                &mut source_samples,
            )?;

            if result.samples_read > 0 {
                self.active_source = Some(self.sources[index].source);
                samples.extend(source_samples);
                return Ok(result);
            }

            if first_activity.is_none() && result.has_activity() {
                first_activity = Some((self.sources[index].source, result));
            }
        }

        if let Some((source, result)) = first_activity {
            self.active_source = Some(source);
            return Ok(result);
        }

        Ok(MangoAppReadResult::default())
    }

    fn active_source_index(&self) -> Option<usize> {
        self.active_source.and_then(|active| {
            self.sources
                .iter()
                .position(|source| source.source == active)
        })
    }

    fn read_available_from_source(
        &self,
        queue_id: libc::c_int,
        max_messages: usize,
        samples: &mut Vec<MangoAppFrameSample>,
    ) -> io::Result<MangoAppReadResult> {
        let mut result = MangoAppReadResult::default();
        for _ in 0..max_messages {
            match self.read_one(queue_id)? {
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

    fn read_one(&self, queue_id: libc::c_int) -> io::Result<Option<MangoAppReadOne>> {
        let mut message = MaybeUninit::<MangoAppMsgV1>::zeroed();
        let message_size = size_of::<MangoAppMsgV1>() - size_of::<c_long>();
        let result = unsafe {
            libc::msgrcv(
                queue_id,
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

fn read_order_for_sources(source_count: usize, active_index: Option<usize>) -> Vec<usize> {
    let mut order = Vec::with_capacity(source_count);
    if let Some(active_index) = active_index.filter(|index| *index < source_count) {
        order.push(active_index);
    }
    for index in 0..source_count {
        if Some(index) != active_index {
            order.push(index);
        }
    }
    order
}

pub fn mangoapp_queue_id(path: &Path, project_id: i32) -> io::Result<libc::c_int> {
    configured_mangoapp_queue_id(path, project_id)
}

fn configured_mangoapp_queue_id(path: &Path, project_id: i32) -> io::Result<libc::c_int> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    let key = unsafe { libc::ftok(path.as_ptr(), project_id) };
    if key == -1 {
        return Err(io::Error::last_os_error());
    }
    msgget_queue_id(key, configured_msgget_flags())
}

fn legacy_failed_ftok_queue_id() -> io::Result<libc::c_int> {
    msgget_queue_id(legacy_failed_ftok_key(), legacy_msgget_flags())
}

fn msgget_queue_id(key: key_t, flags: c_int) -> io::Result<libc::c_int> {
    let queue_id = unsafe { libc::msgget(key, flags) };
    if queue_id == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(queue_id)
}

fn legacy_failed_ftok_key() -> key_t {
    -1 as key_t
}

fn configured_msgget_flags() -> c_int {
    0o666 | libc::IPC_CREAT
}

fn legacy_msgget_flags() -> c_int {
    0
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
    fn legacy_failed_ftok_source_uses_key_minus_one_without_create() {
        assert_eq!(legacy_failed_ftok_key(), -1 as key_t);
        assert_eq!(legacy_msgget_flags() & libc::IPC_CREAT, 0);
    }

    #[test]
    fn configured_source_uses_create_flag() {
        assert_eq!(configured_msgget_flags() & libc::IPC_CREAT, libc::IPC_CREAT);
    }

    #[test]
    fn active_source_is_checked_first() {
        assert_eq!(read_order_for_sources(2, Some(1)), vec![1, 0]);
        assert_eq!(read_order_for_sources(2, None), vec![0, 1]);
        assert_eq!(read_order_for_sources(2, Some(9)), vec![0, 1]);
    }

    #[test]
    fn source_presence_is_tracked_by_source_kind() {
        let reader = MangoAppFrameReader {
            sources: vec![MangoAppQueueReader {
                source: MangoAppQueueSource::ConfiguredFtok,
                queue_id: 7,
            }],
            active_source: None,
            max_frame_time_ns: 1_000_000_000,
        };
        assert!(reader.has_source(MangoAppQueueSource::ConfiguredFtok));
        assert!(!reader.has_source(MangoAppQueueSource::LegacyFailedFtok));
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
