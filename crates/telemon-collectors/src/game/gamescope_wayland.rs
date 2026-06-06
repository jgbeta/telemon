use std::env;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use telemon_core::config::GamescopeWaylandConfig;

const WL_DISPLAY_ID: u32 = 1;
const WL_REGISTRY_ID: u32 = 2;
const WL_INIT_SYNC_ID: u32 = 3;
const GAMESCOPE_CONTROL_ID: u32 = 4;

const WL_DISPLAY_ERROR: u16 = 0;
const WL_DISPLAY_GET_REGISTRY: u16 = 1;
const WL_DISPLAY_SYNC: u16 = 0;
const WL_REGISTRY_GLOBAL: u16 = 0;
const WL_REGISTRY_BIND: u16 = 0;
const WL_CALLBACK_DONE: u16 = 0;
const GAMESCOPE_CONTROL_REQUEST_APP_PERFORMANCE_STATS: u16 = 6;
const GAMESCOPE_CONTROL_APP_PERFORMANCE_STATS: u16 = 3;

const GAMESCOPE_CONTROL_INTERFACE: &str = "gamescope_control";
const GAMESCOPE_CONTROL_MIN_VERSION: u32 = 6;
const INIT_ROUNDTRIP_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GamescopeWaylandFrameSample {
    pub app_id: u32,
    pub visible_frametime_ns: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GamescopeWaylandReadResult {
    pub samples_read: usize,
    pub dropped_zero: u64,
    pub dropped_too_large: u64,
    pub dropped_wrong_session: u64,
}

pub struct GamescopeWaylandFrameReader {
    stream: UnixStream,
    read_buffer: Vec<u8>,
    pending_app_id: Option<u32>,
}

impl GamescopeWaylandFrameReader {
    pub fn open(config: &GamescopeWaylandConfig) -> io::Result<Self> {
        let socket_path = gamescope_socket_path(config)?;
        let mut stream = UnixStream::connect(&socket_path)?;
        stream.set_read_timeout(Some(INIT_ROUNDTRIP_TIMEOUT))?;

        write_message(
            &mut stream,
            WL_DISPLAY_ID,
            WL_DISPLAY_GET_REGISTRY,
            &WL_REGISTRY_ID.to_ne_bytes(),
        )?;
        write_message(
            &mut stream,
            WL_DISPLAY_ID,
            WL_DISPLAY_SYNC,
            &WL_INIT_SYNC_ID.to_ne_bytes(),
        )?;
        stream.flush()?;

        let mut read_buffer = Vec::new();
        let mut sync_done = false;
        let mut control_bound = false;
        while !sync_done {
            read_into_buffer_blocking(&mut stream, &mut read_buffer)?;
            while let Some(message) = pop_message(&mut read_buffer)? {
                match (message.object_id, message.opcode) {
                    (WL_DISPLAY_ID, WL_DISPLAY_ERROR) => {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!(
                                "Wayland display error during Gamescope init: {:?}",
                                message.payload
                            ),
                        ));
                    }
                    (WL_REGISTRY_ID, WL_REGISTRY_GLOBAL) => {
                        if let Some(global) = parse_registry_global(&message.payload) {
                            if global.interface == GAMESCOPE_CONTROL_INTERFACE
                                && global.version >= GAMESCOPE_CONTROL_MIN_VERSION
                            {
                                bind_gamescope_control(&mut stream, global.name)?;
                                stream.flush()?;
                                control_bound = true;
                            }
                        }
                    }
                    (WL_INIT_SYNC_ID, WL_CALLBACK_DONE) => {
                        sync_done = true;
                    }
                    _ => {}
                }
            }
        }

        if !control_bound {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "gamescope_control version 6 global not advertised",
            ));
        }

        stream.set_read_timeout(None)?;
        stream.set_nonblocking(true)?;

        Ok(Self {
            stream,
            read_buffer,
            pending_app_id: None,
        })
    }

    pub fn read_available(
        &mut self,
        app_id: u32,
        max_frame_time_ns: u64,
        max_messages: usize,
        samples: &mut Vec<GamescopeWaylandFrameSample>,
    ) -> io::Result<GamescopeWaylandReadResult> {
        self.ensure_request(app_id)?;

        let mut result = GamescopeWaylandReadResult::default();
        let mut processed = 0_usize;
        loop {
            while let Some(message) = pop_message(&mut self.read_buffer)? {
                processed = processed.saturating_add(1);
                self.handle_message(app_id, max_frame_time_ns, message, samples, &mut result)?;
                if processed >= max_messages {
                    self.ensure_request(app_id)?;
                    return Ok(result);
                }
            }

            if !read_into_buffer_nonblocking(&mut self.stream, &mut self.read_buffer)? {
                break;
            }
        }

        self.ensure_request(app_id)?;
        Ok(result)
    }

    fn ensure_request(&mut self, app_id: u32) -> io::Result<()> {
        if self.pending_app_id != Some(app_id) {
            self.request_app_performance_stats(app_id)?;
        }
        Ok(())
    }

    fn request_app_performance_stats(&mut self, app_id: u32) -> io::Result<()> {
        write_message(
            &mut self.stream,
            GAMESCOPE_CONTROL_ID,
            GAMESCOPE_CONTROL_REQUEST_APP_PERFORMANCE_STATS,
            &app_id.to_ne_bytes(),
        )?;
        self.stream.flush()?;
        self.pending_app_id = Some(app_id);
        Ok(())
    }

    fn handle_message(
        &mut self,
        current_app_id: u32,
        max_frame_time_ns: u64,
        message: WaylandMessage,
        samples: &mut Vec<GamescopeWaylandFrameSample>,
        result: &mut GamescopeWaylandReadResult,
    ) -> io::Result<()> {
        if message.object_id != GAMESCOPE_CONTROL_ID
            || message.opcode != GAMESCOPE_CONTROL_APP_PERFORMANCE_STATS
        {
            return Ok(());
        }

        let Some((app_id, frametime_ns)) = parse_app_performance_stats(&message.payload) else {
            return Ok(());
        };

        if self.pending_app_id == Some(app_id) {
            self.pending_app_id = None;
        }

        if app_id != current_app_id {
            result.dropped_wrong_session = result.dropped_wrong_session.saturating_add(1);
            return Ok(());
        }

        if frametime_ns == 0 {
            result.dropped_zero = result.dropped_zero.saturating_add(1);
        } else if frametime_ns > max_frame_time_ns {
            result.dropped_too_large = result.dropped_too_large.saturating_add(1);
        } else {
            samples.push(GamescopeWaylandFrameSample {
                app_id,
                visible_frametime_ns: frametime_ns,
            });
            result.samples_read = result.samples_read.saturating_add(1);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistryGlobal {
    name: u32,
    interface: String,
    version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WaylandMessage {
    object_id: u32,
    opcode: u16,
    payload: Vec<u8>,
}

pub fn frametime_ns_from_parts(lo: u32, hi: u32) -> u64 {
    ((hi as u64) << 32) | lo as u64
}

fn gamescope_socket_path(config: &GamescopeWaylandConfig) -> io::Result<PathBuf> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() })));
    let display = if config.display.trim().is_empty() {
        env::var("GAMESCOPE_WAYLAND_DISPLAY").unwrap_or_else(|_| "gamescope-0".to_string())
    } else {
        config.display.clone()
    };

    if display.contains("/") {
        Ok(PathBuf::from(display))
    } else {
        Ok(runtime_dir.join(display))
    }
}

fn bind_gamescope_control(stream: &mut UnixStream, global_name: u32) -> io::Result<()> {
    let mut body = Vec::new();
    body.extend_from_slice(&global_name.to_ne_bytes());
    push_wayland_string(&mut body, GAMESCOPE_CONTROL_INTERFACE);
    body.extend_from_slice(&GAMESCOPE_CONTROL_MIN_VERSION.to_ne_bytes());
    body.extend_from_slice(&GAMESCOPE_CONTROL_ID.to_ne_bytes());
    write_message(stream, WL_REGISTRY_ID, WL_REGISTRY_BIND, &body)
}

fn write_message(
    stream: &mut UnixStream,
    object_id: u32,
    opcode: u16,
    body: &[u8],
) -> io::Result<()> {
    let size = 8_usize.saturating_add(body.len());
    if size > u16::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Wayland message too large",
        ));
    }
    let size_opcode = ((size as u32) << 16) | opcode as u32;
    stream.write_all(&object_id.to_ne_bytes())?;
    stream.write_all(&size_opcode.to_ne_bytes())?;
    stream.write_all(body)
}

fn push_wayland_string(body: &mut Vec<u8>, value: &str) {
    let len = value.len().saturating_add(1) as u32;
    body.extend_from_slice(&len.to_ne_bytes());
    body.extend_from_slice(value.as_bytes());
    body.push(0);
    while body.len() % 4 != 0 {
        body.push(0);
    }
}

fn read_into_buffer_blocking(stream: &mut UnixStream, buffer: &mut Vec<u8>) -> io::Result<()> {
    let mut scratch = [0_u8; 8192];
    let read = stream.read(&mut scratch)?;
    if read == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Wayland socket closed",
        ));
    }
    buffer.extend_from_slice(&scratch[..read]);
    Ok(())
}

fn read_into_buffer_nonblocking(stream: &mut UnixStream, buffer: &mut Vec<u8>) -> io::Result<bool> {
    let mut scratch = [0_u8; 8192];
    match stream.read(&mut scratch) {
        Ok(0) => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Wayland socket closed",
        )),
        Ok(read) => {
            buffer.extend_from_slice(&scratch[..read]);
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(true),
        Err(error) => Err(error),
    }
}

fn pop_message(buffer: &mut Vec<u8>) -> io::Result<Option<WaylandMessage>> {
    if buffer.len() < 8 {
        return Ok(None);
    }

    let object_id = u32::from_ne_bytes(buffer[0..4].try_into().unwrap());
    let size_opcode = u32::from_ne_bytes(buffer[4..8].try_into().unwrap());
    let opcode = (size_opcode & 0xffff) as u16;
    let size = (size_opcode >> 16) as usize;
    if size < 8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Wayland message size",
        ));
    }
    if buffer.len() < size {
        return Ok(None);
    }

    let payload = buffer[8..size].to_vec();
    buffer.drain(..size);
    Ok(Some(WaylandMessage {
        object_id,
        opcode,
        payload,
    }))
}

fn parse_registry_global(payload: &[u8]) -> Option<RegistryGlobal> {
    let mut offset = 0_usize;
    let name = read_u32(payload, &mut offset)?;
    let interface = read_string(payload, &mut offset)?;
    let version = read_u32(payload, &mut offset)?;
    Some(RegistryGlobal {
        name,
        interface,
        version,
    })
}

fn parse_app_performance_stats(payload: &[u8]) -> Option<(u32, u64)> {
    let mut offset = 0_usize;
    let app_id = read_u32(payload, &mut offset)?;
    let lo = read_u32(payload, &mut offset)?;
    let hi = read_u32(payload, &mut offset)?;
    Some((app_id, frametime_ns_from_parts(lo, hi)))
}

fn read_u32(payload: &[u8], offset: &mut usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let bytes = payload.get(*offset..end)?;
    *offset = end;
    Some(u32::from_ne_bytes(bytes.try_into().ok()?))
}

fn read_string(payload: &[u8], offset: &mut usize) -> Option<String> {
    let len = read_u32(payload, offset)? as usize;
    if len == 0 {
        return None;
    }
    let end = offset.checked_add(len)?;
    let bytes = payload.get(*offset..end)?;
    let string_bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    *offset = align_to_4(end)?;
    String::from_utf8(string_bytes.to_vec()).ok()
}

fn align_to_4(value: usize) -> Option<usize> {
    value.checked_add(3).map(|value| value & !3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frametime_reconstructs_from_lo_hi_parts() {
        assert_eq!(
            frametime_ns_from_parts(0x89ab_cdef, 0x0123_4567),
            0x0123_4567_89ab_cdef
        );
    }

    #[test]
    fn registry_global_parser_reads_wayland_string() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&7_u32.to_ne_bytes());
        push_wayland_string(&mut payload, GAMESCOPE_CONTROL_INTERFACE);
        payload.extend_from_slice(&6_u32.to_ne_bytes());

        assert_eq!(
            parse_registry_global(&payload),
            Some(RegistryGlobal {
                name: 7,
                interface: GAMESCOPE_CONTROL_INTERFACE.to_string(),
                version: 6,
            })
        );
    }

    #[test]
    fn app_performance_parser_reads_split_frametime() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&996_580_u32.to_ne_bytes());
        payload.extend_from_slice(&0x89ab_cdef_u32.to_ne_bytes());
        payload.extend_from_slice(&0x0123_4567_u32.to_ne_bytes());

        assert_eq!(
            parse_app_performance_stats(&payload),
            Some((996_580, 0x0123_4567_89ab_cdef))
        );
    }
}
