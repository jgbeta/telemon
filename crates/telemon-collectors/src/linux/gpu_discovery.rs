use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LinuxGpuDevice {
    pub node_name: String,
    pub render_node: Option<String>,
    pub card: Option<String>,
    pub drm_path: String,
    pub device_path: PathBuf,
    pub pci_bdf: Option<String>,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub driver: Option<String>,
}

impl LinuxGpuDevice {
    pub fn backend_driver(&self) -> &str {
        self.driver.as_deref().unwrap_or("unknown")
    }

    pub fn stable_id(&self) -> String {
        self.pci_bdf
            .clone()
            .or_else(|| self.card.clone())
            .or_else(|| self.render_node.clone())
            .unwrap_or_else(|| self.node_name.clone())
    }
}

pub fn discover_drm_gpus(drm_root: &Path) -> Result<Vec<LinuxGpuDevice>> {
    if !drm_root.exists() {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(drm_root)
        .with_context(|| format!("failed to read DRM root {}", drm_root.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());

    let mut devices = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in entries.iter().filter(|entry| {
        entry
            .file_name()
            .to_str()
            .map(is_render_node_name)
            .unwrap_or(false)
    }) {
        if let Some(device) = device_from_drm_entry(drm_root, &entry.path(), true)? {
            let key = device.device_path.display().to_string();
            if seen.insert(key) {
                devices.push(device);
            }
        }
    }

    for entry in entries.iter().filter(|entry| {
        entry
            .file_name()
            .to_str()
            .map(is_card_name)
            .unwrap_or(false)
    }) {
        if let Some(device) = device_from_drm_entry(drm_root, &entry.path(), false)? {
            let key = device.device_path.display().to_string();
            if seen.insert(key) {
                devices.push(device);
            }
        }
    }

    Ok(devices)
}

fn device_from_drm_entry(
    drm_root: &Path,
    drm_path: &Path,
    is_render_node: bool,
) -> Result<Option<LinuxGpuDevice>> {
    let Some(node_name) = drm_path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    let device_link = drm_path.join("device");
    if !device_link.exists() {
        return Ok(None);
    }
    let device_path = fs::canonicalize(&device_link).unwrap_or(device_link);
    let card = if is_render_node {
        find_card_for_device(drm_root, &device_path)?
    } else {
        Some(node_name.to_string())
    };

    Ok(Some(LinuxGpuDevice {
        node_name: node_name.to_string(),
        render_node: is_render_node.then(|| node_name.to_string()),
        card,
        drm_path: drm_path.display().to_string(),
        pci_bdf: device_path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| name.contains(':'))
            .map(ToString::to_string),
        vendor_id: read_trimmed(device_path.join("vendor")),
        device_id: read_trimmed(device_path.join("device")),
        driver: driver_name(&device_path),
        device_path,
    }))
}

fn find_card_for_device(drm_root: &Path, target_device_path: &Path) -> Result<Option<String>> {
    let mut entries = fs::read_dir(drm_root)
        .with_context(|| format!("failed to read DRM root {}", drm_root.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(ToString::to_string) else {
            continue;
        };
        if !is_card_name(&name) {
            continue;
        }
        let card_device = entry.path().join("device");
        let card_device = fs::canonicalize(&card_device).unwrap_or(card_device);
        if card_device == target_device_path {
            return Ok(Some(name));
        }
    }

    Ok(None)
}

fn driver_name(device_path: &Path) -> Option<String> {
    fs::read_link(device_path.join("driver"))
        .ok()
        .and_then(|path| path.file_name().map(|value| value.to_os_string()))
        .and_then(|value| value.to_str().map(ToString::to_string))
}

fn read_trimmed(path: PathBuf) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

fn is_render_node_name(value: &str) -> bool {
    value
        .strip_prefix("renderD")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
}

fn is_card_name(value: &str) -> bool {
    value
        .strip_prefix("card")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "telemon-gpu-discovery-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn discovers_render_node_and_matching_card() {
        let root = temp_dir("render");
        let pci = root.join("pci/0000:04:00.0");
        fs::create_dir_all(&pci).unwrap();
        fs::write(pci.join("vendor"), "0x8086\n").unwrap();
        fs::write(pci.join("device"), "0x1234\n").unwrap();
        fs::create_dir_all(root.join("class/card0")).unwrap();
        fs::create_dir_all(root.join("class/renderD128")).unwrap();
        symlink(&pci, root.join("class/card0/device")).unwrap();
        symlink(&pci, root.join("class/renderD128/device")).unwrap();

        let devices = discover_drm_gpus(&root.join("class")).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].render_node.as_deref(), Some("renderD128"));
        assert_eq!(devices[0].card.as_deref(), Some("card0"));
        assert_eq!(devices[0].vendor_id.as_deref(), Some("0x8086"));

        fs::remove_dir_all(root).unwrap();
    }
}
