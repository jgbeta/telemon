use std::ffi::CString;
use std::io;
use std::mem::{offset_of, size_of};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use libc::{c_int, c_long, key_t};
use telemon_core::config::GamescopeMangoappConfig;

const MANGOAPP_FRAME_MESSAGE_TYPE: c_long = 1;
const MANGOAPP_MESSAGE_VERSION: u32 = 1;
const INVALID_FRAME_TIME_NS: u64 = u64::MAX;
const MANGOAPP_PAYLOAD_CAPACITY: usize = 4096;
const MANGOAPP_MESSAGE_BUFFER_SIZE: usize = size_of::<c_long>() + MANGOAPP_PAYLOAD_CAPACITY;

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
        let mut message = [0_u8; MANGOAPP_MESSAGE_BUFFER_SIZE];
        let result = unsafe {
            libc::msgrcv(
                queue_id,
                message.as_mut_ptr().cast(),
                MANGOAPP_PAYLOAD_CAPACITY,
                MANGOAPP_FRAME_MESSAGE_TYPE,
                mangoapp_msgrcv_flags(),
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOMSG) {
                return Ok(None);
            }
            return Err(error);
        }

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

fn mangoapp_msgrcv_flags() -> c_int {
    libc::IPC_NOWAIT
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MangoAppReadOne {
    Sample(MangoAppFrameSample),
    Dropped(MangoAppDropReason),
}

fn decode_message(message: &[u8], payload_len: usize, max_frame_time_ns: u64) -> MangoAppReadOne {
    let Some(version) = read_field::<u32>(message, payload_len, mangoapp_version_offset()) else {
        return MangoAppReadOne::Dropped(MangoAppDropReason::TooShort);
    };
    if version != MANGOAPP_MESSAGE_VERSION {
        return MangoAppReadOne::Dropped(MangoAppDropReason::UnsupportedVersion);
    }

    let Some(visible_frametime_ns) = read_field::<u64>(
        message,
        payload_len,
        offset_of!(MangoAppMsgV1, visible_frametime_ns),
    ) else {
        return MangoAppReadOne::Dropped(MangoAppDropReason::TooShort);
    };
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
        pid: read_field(message, payload_len, offset_of!(MangoAppMsgV1, pid)).unwrap_or_default(),
        app_frametime_ns: read_field(
            message,
            payload_len,
            offset_of!(MangoAppMsgV1, app_frametime_ns),
        )
        .unwrap_or_default(),
        visible_frametime_ns,
        latency_ns: read_field(message, payload_len, offset_of!(MangoAppMsgV1, latency_ns))
            .unwrap_or_default(),
    })
}

fn read_field<T: Copy>(message: &[u8], payload_len: usize, field_offset: usize) -> Option<T> {
    if payload_len < payload_bytes_through::<T>(field_offset) {
        return None;
    }
    let field_end = field_offset.checked_add(size_of::<T>())?;
    let field = message.get(field_offset..field_end)?;
    Some(unsafe { field.as_ptr().cast::<T>().read_unaligned() })
}

fn payload_bytes_through<T>(field_offset: usize) -> usize {
    field_offset + size_of::<T>() - size_of::<c_long>()
}

fn mangoapp_version_offset() -> usize {
    offset_of!(MangoAppMsgV1, header) + offset_of!(MangoAppMsgHeader, version)
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

    fn message_bytes(message: &MangoAppMsgV1) -> Vec<u8> {
        let ptr = std::ptr::from_ref(message).cast::<u8>();
        unsafe { std::slice::from_raw_parts(ptr, size_of::<MangoAppMsgV1>()).to_vec() }
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
    fn receive_flags_do_not_block_or_truncate_protocol_changes() {
        let flags = mangoapp_msgrcv_flags();
        let payload_capacity = std::hint::black_box(MANGOAPP_PAYLOAD_CAPACITY);
        assert_eq!(flags & libc::IPC_NOWAIT, libc::IPC_NOWAIT);
        assert_eq!(flags & libc::MSG_NOERROR, 0);
        assert!(payload_capacity >= 1024);
        assert_eq!(
            MANGOAPP_MESSAGE_BUFFER_SIZE,
            size_of::<c_long>() + payload_capacity
        );
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
            decode_message(&message_bytes(&message), payload_len, 1_000_000_000),
            MangoAppReadOne::Sample(MangoAppFrameSample {
                pid: 4242,
                app_frametime_ns: 17_000_000,
                visible_frametime_ns: 16_666_667,
                latency_ns: 5_000_000,
            })
        );
    }

    #[test]
    fn decodes_known_prefix_when_payload_has_future_fields() {
        let message = message_with_visible_frametime(1, 16_666_667);
        let mut message = message_bytes(&message);
        message.extend([0xa5; 128]);
        let payload_len = size_of::<MangoAppMsgV1>() - size_of::<c_long>() + 128;
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
    fn optional_fields_require_complete_payloads() {
        let message = message_with_visible_frametime(1, 16_666_667);
        let payload_len =
            payload_bytes_through::<u64>(offset_of!(MangoAppMsgV1, visible_frametime_ns));
        assert_eq!(
            decode_message(&message_bytes(&message), payload_len, 1_000_000_000),
            MangoAppReadOne::Sample(MangoAppFrameSample {
                pid: 4242,
                app_frametime_ns: 0,
                visible_frametime_ns: 16_666_667,
                latency_ns: 0,
            })
        );
    }

    #[test]
    fn rejects_invalid_version() {
        let message = message_with_visible_frametime(2, 16_666_667);
        let payload_len = size_of::<MangoAppMsgV1>() - size_of::<c_long>();
        assert_eq!(
            decode_message(&message_bytes(&message), payload_len, 1_000_000_000),
            MangoAppReadOne::Dropped(MangoAppDropReason::UnsupportedVersion)
        );
    }

    #[test]
    fn rejects_short_payload_before_visible_frame_time() {
        let message = message_with_visible_frametime(1, 16_666_667);
        let payload_len = payload_bytes_through::<u32>(offset_of!(MangoAppMsgV1, pid));
        assert_eq!(
            decode_message(&message_bytes(&message), payload_len, 1_000_000_000),
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
                decode_message(&message_bytes(&message), payload_len, 1_000_000_000),
                MangoAppReadOne::Dropped(reason)
            );
        }
    }
}
