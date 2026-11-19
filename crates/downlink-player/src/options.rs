//! Every mpv option string Downlink uses (PLAN §4.3). The single source of truth: no mpv options
//! live in JS or in `downlink-core`.

/// Playback profile applied per file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Profile {
    /// Live TV.
    Live,
    /// Catch-up, VOD, recordings.
    Seekable,
}

/// How video reaches the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    /// libmpv render API into a surface we own (macOS, Linux).
    RenderApi,
    /// mpv's own `--wid` child window (Windows).
    WidWindow,
}

impl RenderBackend {
    pub fn platform_default() -> Self {
        if cfg!(target_os = "windows") {
            Self::WidWindow
        } else {
            Self::RenderApi
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Deinterlace {
    #[default]
    Auto,
    On,
    Off,
}

impl Deinterlace {
    pub fn mpv_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::On => "yes",
            Self::Off => "no",
        }
    }
}

/// Audio output mode (PLAN §4.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum AudioMode {
    Stereo,
    #[default]
    Surround,
    Passthrough,
}

/// mpv options for an audio mode. Passthrough only applies when the device reports support;
/// use [`effective_audio_mode`] first.
pub fn audio_mode_options(mode: AudioMode) -> Opts {
    match mode {
        AudioMode::Stereo => vec![kv("audio-channels", "stereo"), kv("audio-spdif", "")],
        AudioMode::Surround => vec![kv("audio-channels", "auto-safe"), kv("audio-spdif", "")],
        AudioMode::Passthrough => vec![
            kv("audio-channels", "auto"),
            kv("audio-spdif", "ac3,eac3,dts"),
        ],
    }
}

/// Heuristic from the mpv device name (`alsa/hdmi:...`, `coreaudio/...`, `wasapi/...`): digital
/// outputs that can carry AC-3/E-AC-3/DTS bitstreams.
pub fn device_supports_passthrough(device_name: &str) -> bool {
    let n = device_name.to_ascii_lowercase();
    [
        "hdmi",
        "iec958",
        "spdif",
        "digital",
        "displayport",
        "optical",
    ]
    .iter()
    .any(|k| n.contains(k))
}

/// Passthrough is only honoured when the chosen device supports it; otherwise Surround.
pub fn effective_audio_mode(requested: AudioMode, device_name: Option<&str>) -> AudioMode {
    match (requested, device_name) {
        (AudioMode::Passthrough, Some(d)) if device_supports_passthrough(d) => {
            AudioMode::Passthrough
        }
        (AudioMode::Passthrough, _) => AudioMode::Surround,
        (m, _) => m,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleBackground {
    None,
    #[default]
    Dim,
    Solid,
}

/// Subtitle styling is limited to size, colour and background (PLAN §4.8).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SubtitleStyle {
    pub size: SubtitleSize,
    /// `#RRGGBB`; anything else falls back to white.
    pub color: String,
    pub background: SubtitleBackground,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            size: SubtitleSize::default(),
            color: "#FFFFFF".to_owned(),
            background: SubtitleBackground::default(),
        }
    }
}

fn valid_hex_rgb(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|h| h.is_ascii_hexdigit())
}

pub fn subtitle_options(style: &SubtitleStyle) -> Opts {
    let size = match style.size {
        SubtitleSize::Small => "36",
        SubtitleSize::Medium => "46",
        SubtitleSize::Large => "62",
    };
    let color = if valid_hex_rgb(&style.color) {
        style.color.as_str()
    } else {
        "#FFFFFF"
    };
    let (border, back) = match style.background {
        SubtitleBackground::None => ("outline-and-shadow", "#00000000"),
        SubtitleBackground::Dim => ("background-box", "#99000000"),
        SubtitleBackground::Solid => ("background-box", "#FF000000"),
    };
    vec![
        kv("sub-font-size", size),
        kv("sub-color", color),
        kv("sub-border-style", border),
        kv("sub-back-color", back),
    ]
}

/// Values accepted for the per-user `hwdec` override (PLAN §4.7).
pub const HWDEC_ALLOWED: &[&str] = &[
    "auto-safe",
    "auto",
    "d3d11va",
    "videotoolbox",
    "vaapi",
    "nvdec",
    "vulkan",
    "no",
];

pub fn valid_hwdec(v: &str) -> bool {
    HWDEC_ALLOWED.contains(&v)
}

/// User-tunable playback preferences.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub alang: String,
    pub slang: String,
    /// "Live buffer" setting, seconds. Clamped to 1..=10.
    pub live_buffer_secs: u32,
    /// `hwdec` value, default `auto-safe`; invalid values fall back to it.
    pub hwdec: String,
    pub deinterlace: Deinterlace,
    pub audio_mode: AudioMode,
    /// mpv audio device name; `None` = auto.
    pub audio_device: Option<String>,
    pub subtitle: SubtitleStyle,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            alang: String::new(),
            slang: String::new(),
            live_buffer_secs: 2,
            hwdec: "auto-safe".to_owned(),
            deinterlace: Deinterlace::Auto,
            audio_mode: AudioMode::Surround,
            audio_device: None,
            subtitle: SubtitleStyle::default(),
        }
    }
}

impl Preferences {
    pub fn live_buffer_secs_clamped(&self) -> u32 {
        self.live_buffer_secs.clamp(1, 10)
    }

    pub fn hwdec_effective(&self) -> &str {
        if valid_hwdec(&self.hwdec) {
            &self.hwdec
        } else {
            "auto-safe"
        }
    }

    pub fn audio_mode_effective(&self) -> AudioMode {
        effective_audio_mode(self.audio_mode, self.audio_device.as_deref())
    }

    /// The options that can be changed on a running instance (everything but `vo` etc.).
    pub fn live_options(&self) -> Opts {
        let mut o = vec![
            kv("hwdec", self.hwdec_effective()),
            kv("deinterlace", self.deinterlace.mpv_value()),
            kv("alang", self.alang.clone()),
            kv("slang", self.slang.clone()),
            kv(
                "audio-device",
                self.audio_device
                    .clone()
                    .unwrap_or_else(|| "auto".to_owned()),
            ),
        ];
        o.extend(audio_mode_options(self.audio_mode_effective()));
        o.extend(subtitle_options(&self.subtitle));
        o
    }
}

type Opts = Vec<(String, String)>;

fn kv(k: &str, v: impl Into<String>) -> (String, String) {
    (k.to_owned(), v.into())
}

/// Base options, applied once before `mpv_initialize` (all sessions).
pub fn base_options(prefs: &Preferences, backend: RenderBackend) -> Opts {
    let mut o: Opts = Vec::new();
    match backend {
        RenderBackend::RenderApi => o.push(kv("vo", "libmpv")),
        RenderBackend::WidWindow => {
            o.push(kv("vo", "gpu-next"));
            o.push(kv(
                "gpu-api",
                if cfg!(target_os = "windows") {
                    "d3d11"
                } else {
                    "auto"
                },
            ));
        }
    }
    o.push(kv("hwdec", prefs.hwdec_effective()));
    o.push(kv("video-sync", "audio"));
    o.push(kv("keep-open", "no"));
    o.push(kv("idle", "yes"));
    o.push(kv("osc", "no"));
    o.push(kv("input-default-bindings", "no"));
    o.push(kv("input-vo-keyboard", "no"));
    o.push(kv("terminal", "no"));
    // Never let the ytdl hook re-request a stream URL after a failure (CLAUDE.md rule 4), and
    // never run user scripts.
    o.push(kv("ytdl", "no"));
    o.push(kv("load-scripts", "no"));
    o.push(kv(
        "stream-lavf-o",
        "reconnect=1,reconnect_streamed=1,reconnect_delay_max=2",
    ));
    o.push(kv("deinterlace", prefs.deinterlace.mpv_value()));
    o.extend(audio_mode_options(prefs.audio_mode_effective()));
    o.extend(subtitle_options(&prefs.subtitle));
    if let Some(d) = &prefs.audio_device {
        o.push(kv("audio-device", d.clone()));
    }
    if !prefs.alang.is_empty() {
        o.push(kv("alang", prefs.alang.clone()));
    }
    if !prefs.slang.is_empty() {
        o.push(kv("slang", prefs.slang.clone()));
    }
    o
}

/// Profile options, applied per file via `loadfile`.
pub fn profile_options(profile: Profile, prefs: &Preferences) -> Opts {
    match profile {
        Profile::Live => vec![
            kv("cache", "yes"),
            kv("cache-secs", prefs.live_buffer_secs_clamped().to_string()),
            kv("demuxer-readahead-secs", "2"),
            kv("demuxer-max-bytes", "32MiB"),
            kv("demuxer-max-back-bytes", "16MiB"),
        ],
        Profile::Seekable => vec![
            kv("cache", "yes"),
            kv("demuxer-readahead-secs", "20"),
            kv("demuxer-max-bytes", "128MiB"),
            kv("demuxer-max-back-bytes", "64MiB"),
        ],
    }
}

/// mpv's `%n%` length-prefixed quoting, safe for any byte content.
pub fn quote(value: &str) -> String {
    format!("%{}%{}", value.len(), value)
}

/// Build the `options` argument of `loadfile <url> replace -1 <options>`.
///
/// Auth headers are attached **per file only** (CLAUDE.md rule 5), never as a global option.
pub fn loadfile_options(
    profile: Profile,
    prefs: &Preferences,
    headers: &[(String, String)],
    start_secs: Option<f64>,
) -> String {
    let mut parts: Vec<String> = profile_options(profile, prefs)
        .into_iter()
        .map(|(k, v)| format!("{k}={}", quote(&v)))
        .collect();
    if !headers.is_empty() {
        let inner = headers
            .iter()
            .map(|(k, v)| quote(&format!("{k}: {v}")))
            .collect::<Vec<_>>()
            .join(",");
        parts.push(format!("http-header-fields={}", quote(&inner)));
    }
    if let Some(s) = start_secs.filter(|s| s.is_finite() && *s > 0.0) {
        parts.push(format!("start={}", quote(&format!("{s:.3}"))));
    }
    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(o: &Opts, k: &str, v: &str) -> bool {
        o.iter().any(|(a, b)| a == k && b == v)
    }

    #[test]
    fn base_render_api() {
        let o = base_options(&Preferences::default(), RenderBackend::RenderApi);
        assert!(has(&o, "vo", "libmpv"));
        assert!(has(&o, "hwdec", "auto-safe"));
        assert!(has(&o, "video-sync", "audio"));
        assert!(has(&o, "idle", "yes"));
        assert!(has(&o, "keep-open", "no"));
        assert!(has(&o, "terminal", "no"));
        assert!(has(&o, "ytdl", "no"));
        assert!(has(
            &o,
            "stream-lavf-o",
            "reconnect=1,reconnect_streamed=1,reconnect_delay_max=2"
        ));
        assert!(!o.iter().any(|(k, _)| k == "alang"));
    }

    #[test]
    fn base_wid() {
        let o = base_options(&Preferences::default(), RenderBackend::WidWindow);
        assert!(has(&o, "vo", "gpu-next"));
        assert!(o.iter().any(|(k, _)| k == "gpu-api"));
    }

    #[test]
    fn language_preferences_apply() {
        let p = Preferences {
            alang: "eng".into(),
            slang: "eng,fra".into(),
            ..Preferences::default()
        };
        let o = base_options(&p, RenderBackend::RenderApi);
        assert!(has(&o, "alang", "eng"));
        assert!(has(&o, "slang", "eng,fra"));
    }

    #[test]
    fn live_buffer_is_clamped() {
        for (input, want) in [(0u32, "1"), (2, "2"), (10, "10"), (99, "10")] {
            let p = Preferences {
                live_buffer_secs: input,
                ..Preferences::default()
            };
            assert!(has(&profile_options(Profile::Live, &p), "cache-secs", want));
        }
    }

    #[test]
    fn live_and_seekable_profiles() {
        let p = Preferences::default();
        let live = profile_options(Profile::Live, &p);
        assert!(has(&live, "demuxer-readahead-secs", "2"));
        assert!(has(&live, "demuxer-max-bytes", "32MiB"));
        assert!(has(&live, "demuxer-max-back-bytes", "16MiB"));
        let seek = profile_options(Profile::Seekable, &p);
        assert!(has(&seek, "demuxer-readahead-secs", "20"));
        assert!(has(&seek, "demuxer-max-bytes", "128MiB"));
        assert!(has(&seek, "demuxer-max-back-bytes", "64MiB"));
        assert!(!seek.iter().any(|(k, _)| k == "cache-secs"));
    }

    #[test]
    fn quoting_is_length_prefixed() {
        assert_eq!(quote("a,b"), "%3%a,b");
        assert_eq!(quote("é"), "%2%é");
    }

    #[test]
    fn headers_are_per_file_and_quoted() {
        let s = loadfile_options(
            Profile::Live,
            &Preferences::default(),
            &[("X-API-Key".into(), "k,1".into())],
            None,
        );
        // inner item: "X-API-Key: k,1" is 14 bytes -> "%14%X-API-Key: k,1" (18 bytes)
        assert!(
            s.contains("http-header-fields=%18%%14%X-API-Key: k,1"),
            "{s}"
        );
        assert!(s.contains("cache-secs=%1%2"));
    }

    #[test]
    fn no_headers_no_header_option() {
        let s = loadfile_options(Profile::Seekable, &Preferences::default(), &[], Some(12.5));
        assert!(!s.contains("http-header-fields"));
        assert!(s.contains("start=%6%12.500"));
    }

    #[test]
    fn passthrough_only_when_device_supports_it() {
        assert_eq!(
            effective_audio_mode(AudioMode::Passthrough, Some("alsa/hdmi:CARD=PCH,DEV=0")),
            AudioMode::Passthrough
        );
        assert_eq!(
            effective_audio_mode(AudioMode::Passthrough, Some("alsa/pipewire")),
            AudioMode::Surround
        );
        assert_eq!(
            effective_audio_mode(AudioMode::Passthrough, None),
            AudioMode::Surround
        );
        assert_eq!(
            effective_audio_mode(AudioMode::Stereo, None),
            AudioMode::Stereo
        );
        let o = audio_mode_options(AudioMode::Passthrough);
        assert!(has(&o, "audio-spdif", "ac3,eac3,dts"));
        assert!(has(
            &audio_mode_options(AudioMode::Stereo),
            "audio-channels",
            "stereo"
        ));
    }

    #[test]
    fn deinterlace_mapping_and_base() {
        assert_eq!(Deinterlace::On.mpv_value(), "yes");
        let p = Preferences {
            deinterlace: Deinterlace::Off,
            ..Preferences::default()
        };
        assert!(has(
            &base_options(&p, RenderBackend::RenderApi),
            "deinterlace",
            "no"
        ));
    }

    #[test]
    fn subtitle_style_maps_and_validates_colour() {
        let st = SubtitleStyle {
            size: SubtitleSize::Large,
            color: "javascript".into(),
            background: SubtitleBackground::Solid,
        };
        let o = subtitle_options(&st);
        assert!(has(&o, "sub-font-size", "62"));
        assert!(has(&o, "sub-color", "#FFFFFF"));
        assert!(has(&o, "sub-border-style", "background-box"));
        let none = subtitle_options(&SubtitleStyle {
            background: SubtitleBackground::None,
            ..SubtitleStyle::default()
        });
        assert!(has(&none, "sub-border-style", "outline-and-shadow"));
    }

    #[test]
    fn hwdec_override_is_validated() {
        let p = Preferences {
            hwdec: "vaapi".into(),
            ..Preferences::default()
        };
        assert_eq!(p.hwdec_effective(), "vaapi");
        let bad = Preferences {
            hwdec: "rm -rf".into(),
            ..Preferences::default()
        };
        assert_eq!(bad.hwdec_effective(), "auto-safe");
    }

    #[test]
    fn base_options_never_carry_headers() {
        let o = base_options(&Preferences::default(), RenderBackend::RenderApi);
        assert!(!o.iter().any(|(k, _)| k.contains("header")));
    }
}
