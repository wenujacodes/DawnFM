//! Test fixtures and deterministic test vectors for Dawn FM integration tests.

#![allow(dead_code)]

use dawnfm::model::{Link, Thumb, Track};

/// Standard sample track fixture for general UI and playback testing.
pub fn sample_track() -> Track {
    Track {
        id: "test_track_001".to_string(),
        title: "Blinding Lights".to_string(),
        artists: vec![Link {
            name: "The Weeknd".to_string(),
            id: Some("UC0WP5P-ufpRfjbNrmOWwLBQ".to_string()),
        }],
        album: Some(Link {
            name: "After Hours".to_string(),
            id: Some("MPREb_sample_album_001".to_string()),
        }),
        duration: Some(200), // 3m 20s
        thumbs: sample_thumbs(),
        track_nr: Some(9),
        plays: Some(3_500_000_000),
        music_video: false,
        explicit: false,
    }
}

/// Sample track that represents an official music video.
pub fn sample_music_video() -> Track {
    Track {
        id: "test_video_002".to_string(),
        title: "Blinding Lights (Official Music Video)".to_string(),
        artists: vec![Link {
            name: "The Weeknd".to_string(),
            id: Some("UC0WP5P-ufpRfjbNrmOWwLBQ".to_string()),
        }],
        album: None,
        duration: Some(262), // 4m 22s
        thumbs: sample_thumbs(),
        track_nr: None,
        plays: Some(850_000_000),
        music_video: true,
        explicit: false,
    }
}

/// Generates `count` sample tracks with sequential metadata.
pub fn sample_tracks(count: usize) -> Vec<Track> {
    (0..count)
        .map(|i| Track {
            id: format!("test_track_{:04}", i),
            title: format!("Test Song #{:03}", i + 1),
            artists: vec![Link {
                name: format!("Artist {}", (i % 5) + 1),
                id: Some(format!("UC_artist_{}", (i % 5) + 1)),
            }],
            album: Some(Link {
                name: format!("Album {}", (i % 10) + 1),
                id: Some(format!("MPREb_album_{}", (i % 10) + 1)),
            }),
            duration: Some(150 + ((i as u32 * 17) % 180)), // 2m30s to 5m30s
            thumbs: sample_thumbs(),
            track_nr: Some(((i % 12) + 1) as u16),
            plays: Some(100_000 * ((i as u64) + 1)),
            music_video: i % 7 == 0,
            explicit: false,
        })
        .collect()
}

/// Generates a realistic 14-track album tracklist.
pub fn sample_album_tracks() -> Vec<Track> {
    let titles = [
        "Alone Again",
        "Too Late",
        "Hardest to Love",
        "Scared to Live",
        "Snowchild",
        "Escape from LA",
        "Heartless",
        "Faith",
        "Blinding Lights",
        "In Your Eyes",
        "Save Your Tears",
        "Repeat After Me",
        "After Hours",
        "Until I Bleed Out",
    ];
    let durations = [250, 239, 211, 191, 247, 355, 198, 283, 200, 237, 215, 195, 361, 190];

    titles
        .iter()
        .zip(durations.iter())
        .enumerate()
        .map(|(idx, (&title, &duration))| Track {
            id: format!("after_hours_{:02}", idx + 1),
            title: title.to_string(),
            artists: vec![Link {
                name: "The Weeknd".to_string(),
                id: Some("UC0WP5P-ufpRfjbNrmOWwLBQ".to_string()),
            }],
            album: Some(Link {
                name: "After Hours".to_string(),
                id: Some("MPREb_sample_album_001".to_string()),
            }),
            duration: Some(duration),
            thumbs: sample_thumbs(),
            track_nr: Some((idx + 1) as u16),
            plays: Some(50_000_000 + (idx as u64 * 10_000_000)),
            music_video: false,
            explicit: false,
        })
        .collect()
}

/// Standard thumbnail set with small, medium, and large variants.
pub fn sample_thumbs() -> Vec<Thumb> {
    vec![
        Thumb {
            url: "https://lh3.googleusercontent.com/sample_thumb=w120-h120-l90-rj".to_string(),
            width: 120,
            height: 120,
        },
        Thumb {
            url: "https://lh3.googleusercontent.com/sample_thumb=w544-h544-l90-rj".to_string(),
            width: 544,
            height: 544,
        },
        Thumb {
            url: "https://lh3.googleusercontent.com/sample_thumb=w1200-h1200-l90-rj".to_string(),
            width: 1200,
            height: 1200,
        },
    ]
}

/// Sample artist link.
pub fn sample_artist() -> Link {
    Link {
        name: "The Weeknd".to_string(),
        id: Some("UC0WP5P-ufpRfjbNrmOWwLBQ".to_string()),
    }
}

/// Sample album link.
pub fn sample_album() -> Link {
    Link {
        name: "Dawn FM".to_string(),
        id: Some("MPREb_dawn_fm_001".to_string()),
    }
}

/// Sample credentials and tokens for auth and single-instance tests.
pub const SAMPLE_TOKEN: &str = "ya29.a0AfH6SMB_SAMPLE_TOKEN_VECTOR_DAWNFM_TEST_1234567890";
pub const SAMPLE_COOKIES: &str = "HSID=AYzTest; SSID=ApqTest; SID=sTestCookieValue; SAPISID=x123Secure";
pub const SAMPLE_ACCOUNT_NAME: &str = "Dawn FM Test Listener";
pub const SAMPLE_ACCOUNT_EMAIL: &str = "listener@dawnfm.test";

/// Realistic Inno Setup installer script content for verifying Feature 26 (F26).
pub const SAMPLE_INNO_SETUP_SCRIPT: &str = r#"; Dawn FM Inno Setup Script
#define MyAppName "Dawn FM"
#define MyAppVersion "2.6.0"
#define MyAppPublisher "Dawn FM Team"
#define MyAppURL "https://github.com/wenujacodes/DawnFM"
#define MyAppExeName "dawnfm.exe"

[Setup]
AppId={{9B73C488-8422-4B1D-8FE9-DAWNFM2024}}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=dawnfm-setup-v{#MyAppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
Root: HKCR; Subkey: "ytm-rs"; ValueType: string; ValueData: "URL:YouTube Music Protocol"; Flags: uninsdeletekey
Root: HKCR; Subkey: "ytm-rs"; ValueName: "URL Protocol"; ValueType: string; ValueData: ""
Root: HKCR; Subkey: "ytm-rs\DefaultIcon"; ValueType: string; ValueData: "{app}\{#MyAppExeName},0"
Root: HKCR; Subkey: "ytm-rs\shell\open\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" ""%1"""

[Code]
// WebView2 Bootstrapper check
function NeedsWebView2(): Boolean;
begin
  Result := True;
end;
"#;

/// Generates a deterministic byte block of length `len` using a linear congruential generator.
pub fn sample_byte_block(seed: u8, len: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(len);
    let mut state = seed as u32;
    for _ in 0..len {
        state = (state.wrapping_mul(1664525).wrapping_add(1013904223)) & 0xFFFFFFFF;
        data.push((state >> 16) as u8);
    }
    data
}

/// Generates deterministic stereo sine wave PCM samples.
pub fn sample_pcm_stereo_sine(duration_secs: f32, sample_rate: u32, freq_hz: f32) -> Vec<f32> {
    let total_samples = (duration_secs * sample_rate as f32) as usize;
    let mut pcm = Vec::with_capacity(total_samples * 2);
    let angular_step = (2.0 * std::f32::consts::PI * freq_hz) / sample_rate as f32;

    for i in 0..total_samples {
        let sample = (i as f32 * angular_step).sin();
        pcm.push(sample); // Left
        pcm.push(sample); // Right
    }
    pcm
}

/// Internationalization and boundary test strings.
pub mod strings {
    pub const ENGLISH_TITLE: &str = "Blinding Lights";
    pub const ARABIC_RTL: &str = "أضواء ساطعة";
    pub const HEBREW_RTL: &str = "אורות מסנוורים";
    pub const SINHALA_TEXT: &str = "දිදුලන එළි";
    pub const JAPANESE_TEXT: &str = "ブラインディング・ライツ";
    pub const EMOJI_TEXT: &str = "✨ 🎶 Dawn FM 📻 🔥";
    pub const HTML_SPECIAL_CHARS: &str = "Rock & Roll <Live> 'At' The \"O2\"";
    pub const UNCLOSED_BRACE: &str = "Hello {name, missing closing brace";
    pub const WHITESPACE_ONLY: &str = "   \t\n\r   ";

    /// Generates a long string of `len` characters.
    pub fn long_string(len: usize) -> String {
        "A".repeat(len)
    }
}
