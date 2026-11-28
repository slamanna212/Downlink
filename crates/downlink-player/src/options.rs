//! Every mpv option string Downlink uses (PLAN §4.3). The single source of truth: no mpv options
//! live in JS or in `downlink-core`.

/// Playback profile applied per file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Profile {
    /// Live TV.
    Live,
    /// Live TV with a disk-backed timeshift buffer (pause / rewind / fast-forward live).
    Timeshift,
    /// Catch-up, VOD, recordings.
    Seekable,
    /// A live multiview tile (PLAN §4.11): the live profile, nothing else.
    Multiview,
    /// A live multiview tile when more than 4 are on screen: also skips the loop filter on
    /// non-key frames to keep software decode of many streams affordable.
    MultiviewDense,
}

impl Profile {
    /// Live sessions treat an end of file as a failure (the stream should never end).
    pub fn is_live(self) -> bool {
        matches!(
            self,
            Self::Live | Self::Timeshift | Self::Multiview | Self::MultiviewDense
        )
    }
}

/// How video reaches the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    /// libmpv render API into a surface we own (macOS, Linux).
    RenderApi,
    /// mpv renders into our own `--wid` window (Windows).
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

/// HDR output: `Auto` sends HDR content to the display as HDR when the platform supports it
/// (Windows `target-colorspace-hint`; Linux Wayland colour-management subsurface). `Off` always
/// tone-maps to SDR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum HdrOutput {
    #[default]
    Auto,
    Off,
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
    /// "Live buffer" setting, seconds. Clamped to 3..=10 (the floor we always try to keep buffered).
    pub live_buffer_secs: u32,
    /// `hwdec` value, default `auto-safe`; invalid values fall back to it.
    pub hwdec: String,
    pub deinterlace: Deinterlace,
    pub audio_mode: AudioMode,
    /// mpv audio device name; `None` = auto.
    pub audio_device: Option<String>,
    pub subtitle: SubtitleStyle,
    /// mpv `user-agent` for every request (not a secret, so set globally). Empty = mpv default.
    /// A per-file `User-Agent` header from the play plan still wins.
    pub user_agent: String,
    pub hdr_output: HdrOutput,
    /// Live timeshift buffer length in seconds; 0 turns timeshift off. Clamped to
    /// [`TIMESHIFT_MIN_SECS`]..=[`TIMESHIFT_MAX_SECS`] otherwise.
    pub timeshift_secs: u32,
    /// Directory for the on-disk timeshift cache (`demuxer-cache-dir`). Empty = mpv's default.
    pub timeshift_dir: String,
    /// NVIDIA RTX Video Super Resolution (Windows, RTX cards). Single player only, never tiles.
    pub rtx_super_resolution: bool,
    /// NVIDIA RTX Video HDR: SDR shown as HDR (Windows, RTX cards, needs `hdr_output` Auto).
    pub rtx_hdr: bool,
    /// Performance mode for slow machines: cheapest scaling and rendering, faster software
    /// decoding (see [`PERFORMANCE_OPTIONS`]).
    pub performance_mode: bool,
}

/// Lowest live buffer: never run the cache thinner than this.
pub const LIVE_BUFFER_MIN_SECS: u32 = 3;

/// Shortest and longest timeshift buffer.
pub const TIMESHIFT_MIN_SECS: u32 = 15 * 60;
pub const TIMESHIFT_MAX_SECS: u32 = 3 * 60 * 60;
/// Bytes per second the timeshift cache is sized for (about 12 Mbit/s, a high HD bitrate).
pub const TIMESHIFT_BYTES_PER_SEC: u64 = 1_500_000;

impl Default for Preferences {
    fn default() -> Self {
        Self {
            alang: String::new(),
            slang: String::new(),
            live_buffer_secs: 3,
            hwdec: "auto-safe".to_owned(),
            deinterlace: Deinterlace::Auto,
            audio_mode: AudioMode::Surround,
            audio_device: None,
            subtitle: SubtitleStyle::default(),
            user_agent: String::new(),
            hdr_output: HdrOutput::Auto,
            timeshift_secs: 60 * 60,
            timeshift_dir: String::new(),
            rtx_super_resolution: false,
            rtx_hdr: false,
            performance_mode: false,
        }
    }
}

impl Preferences {
    pub fn live_buffer_secs_clamped(&self) -> u32 {
        self.live_buffer_secs.clamp(LIVE_BUFFER_MIN_SECS, 10)
    }

    /// Timeshift buffer length in seconds, `None` when timeshift is off.
    pub fn timeshift_secs_effective(&self) -> Option<u32> {
        (self.timeshift_secs > 0).then(|| {
            self.timeshift_secs
                .clamp(TIMESHIFT_MIN_SECS, TIMESHIFT_MAX_SECS)
        })
    }

    pub fn hwdec_effective(&self) -> &str {
        if valid_hwdec(&self.hwdec) {
            &self.hwdec
        } else {
            "auto-safe"
        }
    }

    /// The `hwdec` value handed to mpv. The automatic settings also list their copy-back
    /// variants: mpv walks the list and moves on when a zero-copy decoder has no way to hand frames
    /// to the renderer (VAAPI under GLX on X11, for example), so the machine falls back to
    /// copy-back hardware decoding instead of software decoding. Explicit choices are passed as is.
    pub fn hwdec_mpv_value(&self) -> &'static str {
        match self.hwdec_effective() {
            "auto-safe" => "auto-safe,auto-copy-safe",
            "auto" => "auto,auto-copy",
            other => HWDEC_ALLOWED
                .iter()
                .copied()
                .find(|v| *v == other)
                .unwrap_or("auto-safe,auto-copy-safe"),
        }
    }

    pub fn audio_mode_effective(&self) -> AudioMode {
        effective_audio_mode(self.audio_mode, self.audio_device.as_deref())
    }

    /// The options that can be changed on a running instance (everything but `vo` etc.).
    pub fn live_options(&self) -> Opts {
        let mut o = vec![
            kv("hwdec", self.hwdec_mpv_value()),
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
        // Turning the mode off restores mpv's defaults for these keys (the player reads them from
        // mpv, so they are never hard-coded here).
        if self.performance_mode {
            o.extend(performance_options());
        }
        if cfg!(target_os = "windows") {
            o.push(colorspace_hint(self.hdr_output));
        } else {
            // Render-API surfaces (Linux, macOS) never signal HDR through mpv.
            o.push(kv("target-colorspace-hint", "no"));
        }
        o
    }
}

/// Performance mode (mpv's built-in `fast` profile plus faster software decoding), for machines
/// that drop frames:
///
/// - bilinear scaling everywhere, no dithering, no linear/sigmoid light, no correct downscaling:
///   the renderer's shaders are the main GPU cost on weak integrated graphics, more so when
///   scaling up to a large display;
/// - no HDR peak detection when tone-mapping (a compute pass every frame);
/// - software decoding (only used when hardware decoding is unavailable for the stream) takes
///   FFmpeg's non-bit-exact shortcuts and skips the H.264 loop filter on non-key frames, at some
///   cost in picture quality. Both are ignored by hardware decoders.
pub const PERFORMANCE_OPTIONS: &[(&str, &str)] = &[
    ("scale", "bilinear"),
    ("dscale", "bilinear"),
    ("cscale", "bilinear"),
    ("dither-depth", "no"),
    ("correct-downscaling", "no"),
    ("linear-downscaling", "no"),
    ("sigmoid-upscaling", "no"),
    ("hdr-compute-peak", "no"),
    ("vd-lavc-fast", "yes"),
    ("vd-lavc-skiploopfilter", "nonkey"),
];

fn performance_options() -> Opts {
    PERFORMANCE_OPTIONS.iter().map(|(k, v)| kv(k, *v)).collect()
}

/// Label of the RTX Video filter in mpv's `vf` chain.
pub const RTX_FILTER_LABEL: &str = "rtx";

/// Upscale factor for RTX Video Super Resolution: how much the video (`dw`×`dh`, aspect-corrected)
/// grows to fit the output (`ow`×`oh`, pixels). `None` when the video is not upscaled noticeably.
///
/// The factor is rounded up to a quarter step so resizing the window does not rebuild the filter on
/// every pixel; the renderer scales the slightly larger frame down to fit.
pub fn rtx_scale(video: (i64, i64), output: (i64, i64)) -> Option<f64> {
    let ((dw, dh), (ow, oh)) = (video, output);
    if dw <= 0 || dh <= 0 || ow <= 0 || oh <= 0 {
        return None;
    }
    let fit = (ow as f64 / dw as f64).min(oh as f64 / dh as f64);
    if fit < 1.1 {
        return None;
    }
    Some(((fit * 4.0).ceil() / 4.0).min(4.0))
}

/// The `vf` chain for NVIDIA RTX Video (mpv `d3d11vpp`), or an empty string for none.
///
/// Super Resolution only applies to the single player (`profile` not a multiview tile) and only
/// when the video is upscaled (`scale`). RTX HDR needs HDR output on and the window on a monitor
/// with HDR turned on (`display_hdr`); on an SDR monitor mpv would only tone-map the RTX HDR
/// output straight back to SDR. The filter itself skips
/// sources that are already HDR. Output format is left to mpv, which picks the 10-bit RGB format
/// RTX HDR requires. Callers pass `rtx_available` from [`crate::gpu::video_caps`]; the filter is
/// never built on platforms without a D3D11 renderer.
pub fn rtx_video_filter(
    prefs: &Preferences,
    profile: Profile,
    scale: Option<f64>,
    rtx_available: bool,
    display_hdr: bool,
) -> String {
    if !rtx_available || !cfg!(target_os = "windows") {
        return String::new();
    }
    rtx_filter_string(prefs, profile, scale, display_hdr)
}

/// Whether the preferences ask for RTX HDR (before checking the display).
pub fn rtx_hdr_wanted(prefs: &Preferences) -> bool {
    prefs.rtx_hdr && prefs.hdr_output == HdrOutput::Auto
}

fn rtx_filter_string(
    prefs: &Preferences,
    profile: Profile,
    scale: Option<f64>,
    display_hdr: bool,
) -> String {
    let tile = matches!(profile, Profile::Multiview | Profile::MultiviewDense);
    let scale = scale.filter(|_| prefs.rtx_super_resolution && !tile);
    let hdr = rtx_hdr_wanted(prefs) && display_hdr;
    let mut opts: Vec<String> = Vec::new();
    if let Some(s) = scale {
        opts.push(format!("scale={s:.2}"));
        opts.push("scaling-mode=nvidia".to_owned());
    }
    if hdr {
        opts.push("nvidia-true-hdr=yes".to_owned());
    }
    if opts.is_empty() {
        return String::new();
    }
    format!("@{RTX_FILTER_LABEL}:d3d11vpp={}", opts.join(":"))
}

/// What an RTX Video `vf` chain from [`rtx_video_filter`] asks for: the Super Resolution factor
/// (if upscaling) and whether RTX HDR is requested. For the playback info panel.
pub fn rtx_filter_parts(vf: &str) -> (Option<f64>, bool) {
    let Some(opts) = vf.split_once("d3d11vpp=").map(|(_, o)| o) else {
        return (None, false);
    };
    let mut scale = None;
    let mut hdr = false;
    for o in opts.split(':') {
        match o.split_once('=') {
            Some(("scale", v)) => scale = v.parse::<f64>().ok(),
            Some(("nvidia-true-hdr", v)) => hdr = v == "yes",
            _ => {}
        }
    }
    (scale, hdr)
}

/// `--wid` (Windows) renders with mpv's own swapchain, so mpv signals HDR to the OS itself.
fn colorspace_hint(hdr: HdrOutput) -> (String, String) {
    kv(
        "target-colorspace-hint",
        if hdr == HdrOutput::Auto { "yes" } else { "no" },
    )
}

type Opts = Vec<(String, String)>;

fn kv(k: &str, v: impl Into<String>) -> (String, String) {
    (k.to_owned(), v.into())
}

/// Base options, applied once before `mpv_initialize` (all sessions).
/// Audio outputs mpv may try, in order. Never SDL, JACK, sndio or OpenAL: with no usable device mpv
/// otherwise falls through to SDL, whose global audio init crashes when several instances (multiview
/// tiles) start at once. When none of these opens, the player switches to `ao=null` and plays silent.
#[cfg(target_os = "linux")]
pub const AUDIO_OUTPUTS: &str = "pipewire,pulse,alsa";
#[cfg(target_os = "windows")]
pub const AUDIO_OUTPUTS: &str = "wasapi";
#[cfg(target_os = "macos")]
pub const AUDIO_OUTPUTS: &str = "coreaudio";
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub const AUDIO_OUTPUTS: &str = "";

pub fn base_options(prefs: &Preferences, backend: RenderBackend) -> Opts {
    let mut o: Opts = Vec::new();
    match backend {
        RenderBackend::RenderApi => {
            o.push(kv("vo", "libmpv"));
            // The surface (not mpv) owns HDR signalling: mpv must tone-map to SDR unless the
            // Linux HDR surface overrides target-trc/prim/peak at run time. `yes`/`no` are valid
            // on every mpv we support (0.40 takes a flag, 0.41+ a choice).
            o.push(kv("target-colorspace-hint", "no"));
        }
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
            o.push(colorspace_hint(prefs.hdr_output));
        }
    }
    o.push(kv("hwdec", prefs.hwdec_mpv_value()));
    o.push(kv("ao", AUDIO_OUTPUTS));
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
    if !prefs.user_agent.is_empty() {
        o.push(kv("user-agent", prefs.user_agent.clone()));
    }
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
    if prefs.performance_mode {
        o.extend(performance_options());
    }
    o
}

/// Profile options, applied per file via `loadfile`.
///
/// Subtitles and closed captions are off for every play: mpv's default `sid=auto` would otherwise
/// pick a caption track (EIA-608/708 in the video, DVB/teletext) matching `slang` or the forced
/// flag on its own. The player's subtitle menu turns one on after load (`SubtitleTrack`), and
/// because this is a per-file option the choice never carries over to the next channel.
pub fn profile_options(profile: Profile, prefs: &Preferences) -> Opts {
    let mut o = vec![kv("sid", "no")];
    o.extend(profile_cache_options(profile, prefs));
    o
}

fn profile_cache_options(profile: Profile, prefs: &Preferences) -> Opts {
    match profile {
        Profile::Multiview => profile_cache_options(Profile::Live, prefs),
        Profile::MultiviewDense => {
            let mut o = profile_cache_options(Profile::Multiview, prefs);
            o.push(kv("vd-lavc-skiploopfilter", "nonkey"));
            o
        }
        // Keep at least `live_buffer_secs` (>= 3) read ahead: the demuxer reads that far past the
        // playhead (readahead), the cache may hold twice that so a burst is not discarded, and
        // after an underrun playback pauses until the buffer is full again (cache-pause-wait)
        // instead of stuttering along on a near-empty cache.
        Profile::Live => vec![
            kv("cache", "yes"),
            kv(
                "cache-secs",
                (prefs.live_buffer_secs_clamped() * 2).to_string(),
            ),
            kv("cache-pause", "yes"),
            kv(
                "cache-pause-wait",
                prefs.live_buffer_secs_clamped().to_string(),
            ),
            kv(
                "demuxer-readahead-secs",
                prefs.live_buffer_secs_clamped().to_string(),
            ),
            kv("demuxer-max-bytes", "32MiB"),
            kv("demuxer-max-back-bytes", "16MiB"),
        ],
        Profile::Timeshift => {
            let Some(secs) = prefs.timeshift_secs_effective() else {
                return profile_cache_options(Profile::Live, prefs);
            };
            let bytes = u64::from(secs) * TIMESHIFT_BYTES_PER_SEC;
            // Reading continues while paused (cache-secs covers the whole buffer) and seeking
            // stays inside the cache; the forward and back limits are each one buffer long so a
            // pause of the full length still fits. The cache lives on disk, not in RAM.
            let mut o = vec![
                kv("cache", "yes"),
                kv("cache-on-disk", "yes"),
                kv("force-seekable", "yes"),
                kv("demuxer-seekable-cache", "yes"),
                kv("cache-secs", secs.to_string()),
                kv(
                    "cache-pause-wait",
                    prefs.live_buffer_secs_clamped().to_string(),
                ),
                kv("cache-pause", "yes"),
                kv(
                    "demuxer-readahead-secs",
                    prefs.live_buffer_secs_clamped().to_string(),
                ),
                kv("demuxer-max-bytes", bytes.to_string()),
                kv("demuxer-max-back-bytes", bytes.to_string()),
            ];
            if !prefs.timeshift_dir.is_empty() {
                o.push(kv("demuxer-cache-dir", prefs.timeshift_dir.clone()));
            }
            o
        }
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

/// Build the `options` named argument of `loadfile` (see `Engine::command_named`).
///
/// Auth headers are attached **per file only** (CLAUDE.md rule 5), never as a global option.
pub fn loadfile_options(
    profile: Profile,
    prefs: &Preferences,
    headers: &[(String, String)],
    start_secs: Option<f64>,
    start_paused: bool,
) -> String {
    // `pause` is engine state that survives `loadfile`: a session paused (or one that failed while
    // paused) would otherwise start the next file frozen while the UI says it is playing.
    let mut parts: Vec<String> = vec![
        if start_paused {
            "pause=yes"
        } else {
            "pause=no"
        }
        .to_owned(),
    ];
    parts.extend(
        profile_options(profile, prefs)
            .into_iter()
            .map(|(k, v)| format!("{k}={}", quote(&v))),
    );
    // A plan-supplied User-Agent (M3U `http-user-agent`, pipe headers) goes through mpv's own
    // per-file option so it replaces the default instead of being sent twice.
    let (ua, rest): (Vec<_>, Vec<_>) = headers
        .iter()
        .partition(|(k, _)| k.eq_ignore_ascii_case("user-agent"));
    if let Some((_, v)) = ua.last() {
        parts.push(format!("user-agent={}", quote(v)));
    }
    if !rest.is_empty() {
        // `http-header-fields` is a string list: items are split on `,` and only `\,` escapes
        // a comma. `%n%` quoting does not apply inside the list (it would be sent verbatim).
        let inner = rest
            .iter()
            .map(|(k, v)| format!("{k}: {v}").replace(',', "\\,"))
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

    #[test]
    fn rtx_scale_factor() {
        // 1080p on a 4K window: exactly 2x.
        assert_eq!(rtx_scale((1920, 1080), (3840, 2160)), Some(2.0));
        // 720p on 4K: 3x. 576i (aspect-corrected 1024x576) on 1080p: 1.875 -> 2.0.
        assert_eq!(rtx_scale((1280, 720), (3840, 2160)), Some(3.0));
        assert_eq!(rtx_scale((1024, 576), (1920, 1080)), Some(2.0));
        // Letterboxed: the tighter axis wins. 1920x800 in 2560x1440 -> 1.333 -> 1.5.
        assert_eq!(rtx_scale((1920, 800), (2560, 1440)), Some(1.5));
        // No (or barely any) upscaling, and nonsense sizes.
        assert_eq!(rtx_scale((1920, 1080), (1920, 1080)), None);
        assert_eq!(rtx_scale((1920, 1080), (2000, 1125)), None);
        assert_eq!(rtx_scale((1920, 1080), (960, 540)), None);
        assert_eq!(rtx_scale((0, 0), (3840, 2160)), None);
        // Tiny sources are capped at 4x.
        assert_eq!(rtx_scale((320, 180), (3840, 2160)), Some(4.0));
    }

    #[test]
    fn rtx_filter_chain() {
        let both = Preferences {
            rtx_super_resolution: true,
            rtx_hdr: true,
            ..Preferences::default()
        };
        assert_eq!(
            rtx_filter_string(&both, Profile::Live, Some(2.0), true),
            "@rtx:d3d11vpp=scale=2.00:scaling-mode=nvidia:nvidia-true-hdr=yes"
        );
        // No upscaling needed: HDR only.
        assert_eq!(
            rtx_filter_string(&both, Profile::Seekable, None, true),
            "@rtx:d3d11vpp=nvidia-true-hdr=yes"
        );
        // Multiview tiles never get Super Resolution, but keep RTX HDR.
        for p in [Profile::Multiview, Profile::MultiviewDense] {
            assert_eq!(
                rtx_filter_string(&both, p, Some(2.0), true),
                "@rtx:d3d11vpp=nvidia-true-hdr=yes"
            );
        }
        // RTX HDR needs HDR output.
        let sdr = Preferences {
            hdr_output: HdrOutput::Off,
            ..both.clone()
        };
        assert_eq!(
            rtx_filter_string(&sdr, Profile::Live, Some(1.5), true),
            "@rtx:d3d11vpp=scale=1.50:scaling-mode=nvidia"
        );
        assert_eq!(
            rtx_filter_string(&sdr, Profile::Multiview, Some(1.5), true),
            ""
        );
        // An SDR monitor never gets RTX HDR (mpv would tone-map it straight back to SDR), but
        // Super Resolution still runs.
        assert_eq!(
            rtx_filter_string(&both, Profile::Live, Some(2.0), false),
            "@rtx:d3d11vpp=scale=2.00:scaling-mode=nvidia"
        );
        assert_eq!(rtx_filter_string(&both, Profile::Seekable, None, false), "");
        // The info panel reads the chain back.
        assert_eq!(
            rtx_filter_parts("@rtx:d3d11vpp=scale=2.00:scaling-mode=nvidia:nvidia-true-hdr=yes"),
            (Some(2.0), true)
        );
        assert_eq!(
            rtx_filter_parts("@rtx:d3d11vpp=nvidia-true-hdr=yes"),
            (None, true)
        );
        assert_eq!(rtx_filter_parts(""), (None, false));
        // Both off: no filter.
        assert_eq!(
            rtx_filter_string(&Preferences::default(), Profile::Live, Some(2.0), true),
            ""
        );
        // No RTX adapter, or not Windows: never a filter.
        assert_eq!(
            rtx_video_filter(&both, Profile::Live, Some(2.0), false, true),
            ""
        );
        if !cfg!(target_os = "windows") {
            assert_eq!(
                rtx_video_filter(&both, Profile::Live, Some(2.0), true, true),
                ""
            );
        }
    }

    fn has(o: &Opts, k: &str, v: &str) -> bool {
        o.iter().any(|(a, b)| a == k && b == v)
    }

    #[test]
    fn audio_outputs_never_include_sdl() {
        let o = base_options(&Preferences::default(), RenderBackend::RenderApi);
        let ao = o.iter().find(|(k, _)| k == "ao").map(|(_, v)| v.as_str());
        assert_eq!(ao, Some(AUDIO_OUTPUTS));
        for bad in ["sdl", "jack", "sndio", "openal"] {
            assert!(
                !AUDIO_OUTPUTS.contains(bad),
                "{bad} must not be an audio output"
            );
        }
    }

    #[test]
    fn every_profile_starts_with_subtitles_off() {
        let p = Preferences::default();
        for prof in [
            Profile::Live,
            Profile::Timeshift,
            Profile::Seekable,
            Profile::Multiview,
            Profile::MultiviewDense,
        ] {
            let o = profile_options(prof, &p);
            assert_eq!(o.iter().filter(|(k, _)| k == "sid").count(), 1);
            assert!(has(&o, "sid", "no"), "{prof:?}");
            assert!(loadfile_options(prof, &p, &[], None, false).contains("sid=%2%no"));
        }
    }

    #[test]
    fn multiview_profiles() {
        let p = Preferences::default();
        let mv = profile_options(Profile::Multiview, &p);
        assert_eq!(mv, profile_options(Profile::Live, &p));
        assert!(has(&mv, "sid", "no"));
        let dense = profile_options(Profile::MultiviewDense, &p);
        assert!(has(&dense, "vd-lavc-skiploopfilter", "nonkey"));
        assert!(has(&dense, "sid", "no"));
        assert!(Profile::MultiviewDense.is_live() && !Profile::Seekable.is_live());
    }

    #[test]
    fn base_render_api() {
        let o = base_options(&Preferences::default(), RenderBackend::RenderApi);
        assert!(has(&o, "vo", "libmpv"));
        assert!(has(&o, "hwdec", "auto-safe,auto-copy-safe"));
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
        assert!(has(&o, "target-colorspace-hint", "yes"));
        let off = Preferences {
            hdr_output: HdrOutput::Off,
            ..Preferences::default()
        };
        let o = base_options(&off, RenderBackend::WidWindow);
        assert!(has(&o, "target-colorspace-hint", "no"));
        // The render API path always tone-maps unless the surface overrides it, even on Auto.
        let o = base_options(&Preferences::default(), RenderBackend::RenderApi);
        assert!(has(&o, "target-colorspace-hint", "no"));
        let live = Preferences::default().live_options();
        assert!(live.iter().any(|(k, _)| k == "target-colorspace-hint"));
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
        for (input, want) in [(0u32, "3"), (2, "3"), (5, "5"), (10, "10"), (99, "10")] {
            let p = Preferences {
                live_buffer_secs: input,
                ..Preferences::default()
            };
            let o = profile_options(Profile::Live, &p);
            assert!(has(&o, "cache-pause-wait", want));
            assert!(has(&o, "demuxer-readahead-secs", want));
            let cache: u32 = want.parse().unwrap_or(0) * 2;
            assert!(has(&o, "cache-secs", &cache.to_string()));
        }
    }

    #[test]
    fn live_and_seekable_profiles() {
        let p = Preferences::default();
        let live = profile_options(Profile::Live, &p);
        assert!(has(&live, "demuxer-readahead-secs", "3"));
        assert!(has(&live, "cache-pause", "yes"));
        assert!(has(&live, "demuxer-max-bytes", "32MiB"));
        assert!(has(&live, "demuxer-max-back-bytes", "16MiB"));
        let seek = profile_options(Profile::Seekable, &p);
        assert!(has(&seek, "demuxer-readahead-secs", "20"));
        assert!(has(&seek, "demuxer-max-bytes", "128MiB"));
        assert!(has(&seek, "demuxer-max-back-bytes", "64MiB"));
        assert!(!seek.iter().any(|(k, _)| k == "cache-secs"));
    }

    #[test]
    fn timeshift_profile_uses_a_disk_cache_sized_from_the_setting() {
        let p = Preferences {
            timeshift_secs: 3600,
            timeshift_dir: "/tmp/ts".into(),
            ..Preferences::default()
        };
        let o = profile_options(Profile::Timeshift, &p);
        assert!(has(&o, "cache-on-disk", "yes"));
        assert!(has(&o, "force-seekable", "yes"));
        assert!(has(&o, "cache-secs", "3600"));
        assert!(has(&o, "demuxer-max-back-bytes", "5400000000"));
        assert!(has(&o, "demuxer-max-bytes", "5400000000"));
        assert!(has(&o, "demuxer-cache-dir", "/tmp/ts"));
        assert!(Profile::Timeshift.is_live());
    }

    #[test]
    fn timeshift_is_clamped_and_can_be_off() {
        let at = |secs| {
            Preferences {
                timeshift_secs: secs,
                ..Preferences::default()
            }
            .timeshift_secs_effective()
        };
        assert_eq!(at(0), None);
        assert_eq!(at(60), Some(TIMESHIFT_MIN_SECS));
        assert_eq!(at(99_999), Some(TIMESHIFT_MAX_SECS));
        let off = Preferences {
            timeshift_secs: 0,
            ..Preferences::default()
        };
        assert_eq!(
            profile_options(Profile::Timeshift, &off),
            profile_options(Profile::Live, &off)
        );
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
            false,
        );
        // inner list: "X-API-Key: k\,1" (15 bytes), the comma escaped for mpv's list parser
        assert!(s.contains("http-header-fields=%15%X-API-Key: k\\,1"), "{s}");
        assert!(s.contains("cache-secs=%1%6"));
    }

    #[test]
    fn multiple_headers_are_list_items() {
        let s = loadfile_options(
            Profile::Live,
            &Preferences::default(),
            &[
                ("Authorization".into(), "Bearer t".into()),
                ("Referer".into(), "http://r.test".into()),
            ],
            None,
            false,
        );
        assert!(
            s.contains("http-header-fields=%46%Authorization: Bearer t,Referer: http://r.test"),
            "{s}"
        );
    }

    #[test]
    fn plan_user_agent_becomes_per_file_option() {
        let s = loadfile_options(
            Profile::Live,
            &Preferences::default(),
            &[
                ("User-Agent".into(), "Agent/1, x".into()),
                ("X-API-Key".into(), "k".into()),
            ],
            None,
            false,
        );
        assert!(s.contains("user-agent=%10%Agent/1, x"), "{s}");
        assert!(s.contains("http-header-fields=%12%X-API-Key: k"), "{s}");
        assert!(!s.contains("User-Agent:"), "{s}");
    }

    #[test]
    fn global_user_agent_only_when_set() {
        let none = base_options(&Preferences::default(), RenderBackend::RenderApi);
        assert!(!none.iter().any(|(k, _)| k == "user-agent"));
        let p = Preferences {
            user_agent: "Downlink/1.0.0 (Linux)".into(),
            ..Preferences::default()
        };
        let o = base_options(&p, RenderBackend::RenderApi);
        assert!(has(&o, "user-agent", "Downlink/1.0.0 (Linux)"));
    }

    #[test]
    fn every_load_starts_unpaused() {
        for p in [Profile::Live, Profile::Timeshift, Profile::Seekable] {
            let s = loadfile_options(p, &Preferences::default(), &[], None, false);
            assert!(s.starts_with("pause=no,"), "{s}");
        }
    }

    #[test]
    fn start_paused_loads_paused() {
        let s = loadfile_options(
            Profile::Seekable,
            &Preferences::default(),
            &[],
            Some(30.0),
            true,
        );
        assert!(s.starts_with("pause=yes,"), "{s}");
        assert!(s.contains("start=%6%30.000"), "{s}");
        assert!(!s.contains("pause=no"), "{s}");
    }

    #[test]
    fn no_headers_no_header_option() {
        let s = loadfile_options(
            Profile::Seekable,
            &Preferences::default(),
            &[],
            Some(12.5),
            false,
        );
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
        assert_eq!(bad.hwdec_mpv_value(), "auto-safe,auto-copy-safe");
    }

    #[test]
    fn automatic_hwdec_falls_back_to_copy_back_modes() {
        let pick = |v: &str| {
            Preferences {
                hwdec: v.into(),
                ..Preferences::default()
            }
            .hwdec_mpv_value()
        };
        assert_eq!(pick("auto-safe"), "auto-safe,auto-copy-safe");
        assert_eq!(pick("auto"), "auto,auto-copy");
        assert_eq!(pick("vaapi"), "vaapi");
        assert_eq!(pick("no"), "no");
    }

    #[test]
    fn performance_mode_adds_the_fast_options_only_when_on() {
        let off = Preferences::default();
        let on = Preferences {
            performance_mode: true,
            ..Preferences::default()
        };
        let base_off = base_options(&off, RenderBackend::RenderApi);
        let base_on = base_options(&on, RenderBackend::RenderApi);
        for (k, v) in PERFORMANCE_OPTIONS {
            assert!(
                !base_off.iter().any(|(key, _)| key == k),
                "{k} set when off"
            );
            assert!(has(&base_on, k, v));
            assert!(has(&on.live_options(), k, v));
            assert!(!off.live_options().iter().any(|(key, _)| key == k));
        }
    }

    #[test]
    fn base_options_never_carry_headers() {
        let o = base_options(&Preferences::default(), RenderBackend::RenderApi);
        assert!(!o.iter().any(|(k, _)| k.contains("header")));
    }
}
