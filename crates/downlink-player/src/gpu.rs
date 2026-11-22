//! Graphics adapter detection for NVIDIA RTX Video (Super Resolution and RTX HDR).
//!
//! RTX Video is a Windows driver feature of RTX cards, reached through mpv's `d3d11vpp` filter
//! (see `options::rtx_video_filter`). There is no Linux or macOS driver support.

use std::sync::OnceLock;

/// What the UI needs to show or grey out the RTX Video settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCaps {
    /// An NVIDIA RTX adapter is present and the platform can use RTX Video.
    pub rtx_video: bool,
    /// Name of that adapter, e.g. "NVIDIA GeForce RTX 4070".
    pub rtx_adapter: Option<String>,
}

/// One display adapter: its marketing name and its PnP hardware id (`PCI\VEN_10DE&DEV_...`).
#[derive(Debug, Clone)]
pub struct Adapter {
    pub name: String,
    pub hardware_id: String,
}

const NVIDIA_VENDOR: &str = "VEN_10DE";

/// The first NVIDIA RTX adapter. GTX and older cards have no RTX Video support.
pub fn find_rtx_adapter(adapters: &[Adapter]) -> Option<&Adapter> {
    adapters.iter().find(|a| {
        a.hardware_id.to_ascii_uppercase().contains(NVIDIA_VENDOR)
            && a.name.to_ascii_uppercase().contains("RTX")
    })
}

/// Detected once per process (adapters do not change while the app runs in practice).
pub fn video_caps() -> VideoCaps {
    static CAPS: OnceLock<VideoCaps> = OnceLock::new();
    CAPS.get_or_init(|| {
        let adapters = enumerate_adapters();
        let caps = match find_rtx_adapter(&adapters) {
            Some(a) => VideoCaps {
                rtx_video: true,
                rtx_adapter: Some(a.name.clone()),
            },
            None => VideoCaps::default(),
        };
        tracing::info!(
            adapters = ?adapters.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            rtx_video = caps.rtx_video,
            "graphics adapters"
        );
        caps
    })
    .clone()
}

#[cfg(target_os = "windows")]
fn enumerate_adapters() -> Vec<Adapter> {
    use windows_sys::Win32::Graphics::Gdi::{DISPLAY_DEVICEW, EnumDisplayDevicesW};

    fn wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    let mut out: Vec<Adapter> = Vec::new();
    for i in 0..32u32 {
        // SAFETY: DISPLAY_DEVICEW is plain data; zeroed is a valid initial state once `cb` is set.
        let mut dd: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
        dd.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        // SAFETY: a null device name enumerates adapters; `dd` is a valid, sized out-pointer.
        let ok = unsafe { EnumDisplayDevicesW(std::ptr::null(), i, &mut dd, 0) };
        if ok == 0 {
            break;
        }
        let a = Adapter {
            name: wide(&dd.DeviceString),
            hardware_id: wide(&dd.DeviceID),
        };
        // One entry per output: the same adapter repeats for each display it drives.
        if !out
            .iter()
            .any(|b| b.name == a.name && b.hardware_id == a.hardware_id)
        {
            out.push(a);
        }
    }
    out
}

#[cfg(not(target_os = "windows"))]
fn enumerate_adapters() -> Vec<Adapter> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(name: &str, id: &str) -> Adapter {
        Adapter {
            name: name.into(),
            hardware_id: id.into(),
        }
    }

    #[test]
    fn finds_only_nvidia_rtx() {
        let list = [
            a("Intel(R) UHD Graphics 770", r"PCI\VEN_8086&DEV_4680"),
            a("NVIDIA GeForce GTX 1080", r"PCI\VEN_10DE&DEV_1B80"),
            a("NVIDIA GeForce RTX 4070", r"PCI\VEN_10DE&DEV_2786"),
        ];
        assert_eq!(
            find_rtx_adapter(&list).map(|a| a.name.as_str()),
            Some("NVIDIA GeForce RTX 4070")
        );
        assert!(find_rtx_adapter(&list[..2]).is_none());
        // A non-NVIDIA name containing "RTX" does not count.
        assert!(find_rtx_adapter(&[a("Some RTX thing", r"PCI\VEN_1002&DEV_0001")]).is_none());
        assert!(find_rtx_adapter(&[a("NVIDIA RTX A2000", r"pci\ven_10de&dev_2531")]).is_some());
    }
}
