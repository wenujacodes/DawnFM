//! The application UI: state, playback logic and the per-frame layout.
//!
//! Layout: animated album-art background (shader.rs) → sidebar, top bar and page
//! content → player bar → now-playing overlay (slides up) → toasts.

mod account;
mod actions;
mod collections;
mod dev;
mod dialogs;
mod glyphs;
mod integrations;
mod jam;
mod karaoke;
mod lists;
mod lyric_prefs;
mod mini;
mod notes;
pub mod nowplaying;
pub mod pages;
mod playback;
pub mod queue;
mod responses;
mod scroll;
mod shell;
mod sleep;
mod titlebar;
pub use titlebar::{custom as custom_titlebar, set_hwnd as set_titlebar_hwnd};
mod transfer;
pub mod widgets;

pub use pages::{
    artist, circle_action, collection, collection_genre, filter_collection_tracks,
    hero_height, metadata_badge, paint_ambient_glow, paint_hero_gradient_dissolve,
    peach_circle, peach_pill, play_pill, CircleGlyph,
};
pub use queue::{Queue, Repeat};
pub use widgets::{paint_quality_icon, track_bpm_key, track_row, RowOpts};

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crossbeam_channel::Receiver;
use egui::{Color32, Ui};
use souvlaki::MediaControlEvent;

use crate::audio::{AudioEvent, AudioHandle};
use crate::backend::{ArtistPage, Backend, Explore, Library, Req, Resp, SearchResults};
use crate::lyrics::LyricsSet;
use crate::media::Media;
use crate::model::{Card, Chip, Shelf, Target, Track};
use crate::shader::Background;
use crate::theme;
use lists::{LIKED_MUSIC, Lists};
use dev::{dev_pace, dev_screenshot, dev_var};
use dialogs::text_field;

/// Animated sleeves are off until the decoder is fixed: the surfacing call sites stay
/// in place, so turning this back on is all it takes to try again.
const MOTION_COVERS: bool = false;

#[derive(Clone, PartialEq, Hash, Debug)]
pub enum Page {
    Home,
    New,
    Library(LibTab),
    Search(String),
    Downloads,
    Album(String),
    Playlist(String),
    Artist(String),
    Settings,
    /// Transfer from Spotify.
    Transfer,
    /// Listen together.
    Jam,
}

#[derive(Clone, Copy, PartialEq, Hash, Debug)]
pub enum LibTab {
    Playlists,
    Songs,
    Albums,
    Artists,
    History,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NpTab {
    #[default]
    Lyrics,
    UpNext,
    /// Neither panel: just the artwork.
    Art,
}

/// User intents collected while drawing; applied after the frame's UI pass.
pub enum Action {
    Play(Vec<Track>, usize),
    PlayCollection(Target, bool),
    /// Click on a card: songs start a radio, everything else opens its page.
    Open(Target),
    PlayNext(Track),
    PlayNextMany(Vec<Track>),
    Enqueue(Track),
    Radio(Track),
    Go(Page),
    QueueJump(usize),
    QueueRemove(usize),
    QueueMove(usize, usize),
    /// Open up next (the player's) or the queue drawer (the bar's).
    ViewQueue,
    Seek(f32),
    Like(Track, bool),
    /// Like (or unlike) a selection of songs.
    LikeMany(Vec<Track>, bool),
    Dislike(Track),
    Share(Target),
    ShareAt(Target, egui::Rect),
    AddToPlaylist(Vec<Track>),
    /// Rows (indices into the open playlist) to take out of it.
    RemoveFromPlaylist(Vec<usize>),
    Download(Track),
    DownloadMany(Vec<Track>),
    NewPlaylist(Vec<Track>),
    EditPlaylist,
    /// The open playlist's collaboration settings.
    Collaborate,
    DeletePlaylist,
    SaveCollection(String, bool),
    TogglePin(Card),
    ToggleFavouriteCollection(String),
    SuggestLess(String),
    /// Subscribe to (or unsubscribe from) the artist page's channel.
    SetSubscribed(bool),
    HomeChip(Option<String>),
    OpenNowPlaying(bool),
    /// Into the mini player (true) or back to the full window.
    Mini(bool),
    /// Play/pause what's already loaded (a card's button on the playing item).
    TogglePlay,
    /// Drop the hi-res stream and replay the current song from YouTube at the same spot.
    RevertQuality,
    /// Clear a quality revert and replay, asking hi-res sources again.
    UpgradeQuality,
    /// A song chosen on the search page as the Spotify transfer's match.
    TransferPick(Track),
}

enum Dialog {
    PickPlaylist(Vec<Track>),
    NewPlaylist { title: String, tracks: Vec<Track> },
    EditPlaylist { id: String, title: String, description: String, privacy: Option<String> },
    ConfirmDelete { id: String, title: String },
    /// `info` until it loads; `busy` while a change is on its way; `confirm_off` asks
    /// before turning collaboration off (that removes the collaborators).
    Collaborate { id: String, info: Option<crate::ytm::Collaboration>, busy: bool, confirm_off: bool },
}

/// Songs picked in a list for a batch action: Ctrl-click toggles one, Shift-click a range,
/// or the Select button makes every click pick.
#[derive(Default)]
pub struct Selection {
    /// The page the picks belong to; on any other page there are none.
    pub page: Option<Page>,
    /// Row indices into that page's list.
    pub rows: std::collections::BTreeSet<usize>,
    pub anchor: Option<usize>,
    /// Select mode: a plain click picks instead of playing.
    pub active: bool,
}

impl Selection {
    pub fn on(&self, page: &Page) -> bool {
        self.page.as_ref() == Some(page) && (self.active || !self.rows.is_empty())
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Start select mode on `page` with nothing picked yet.
    pub fn start(&mut self, page: &Page) {
        *self = Self { page: Some(page.clone()), active: true, ..Self::default() };
    }

    /// Toggle row `i`, or with `range` pick every row from the last one toggled to it.
    pub fn pick(&mut self, page: &Page, i: usize, range: bool) {
        if self.page.as_ref() != Some(page) {
            *self = Self { page: Some(page.clone()), ..Self::default() };
        }
        match self.anchor.filter(|_| range) {
            Some(a) => self.rows.extend(a.min(i)..=a.max(i)),
            None => {
                if !self.rows.remove(&i) {
                    self.rows.insert(i);
                }
                self.anchor = Some(i);
            }
        }
    }
}

pub struct HomeState {
    pub chips: Vec<Chip>,
    pub shelves: Vec<Shelf>,
    pub continuation: Option<String>,
    /// When the next shelves were requested (app time, s), while they load.
    pub loading_more: Option<f64>,
    pub picks_loading: bool,
    pub picks_expanded: bool,
}

pub struct Channels {
    pub resp_rx: Receiver<Resp>,
    pub audio_rx: Receiver<AudioEvent>,
    pub media_rx: Receiver<MediaControlEvent>,
}

/// Settings › Network's connection test: not run (`None`), running (`Some(None)`), or
/// its answer (works, message).
type ProxyTest = Option<Option<(bool, String)>>;

/// A thing opened from a search, shown on the search landing with its artwork.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct RecentSearch {
    /// The search it was clicked from.
    pub query: String,
    /// What was opened; `None` for entries saved by an earlier version, which only kept
    /// the query and open the search again.
    #[serde(default)]
    pub target: Option<Target>,
    pub title: String,
    pub subtitle: String,
    pub thumb: Option<String>,
    /// An artist, whose picture is round.
    pub round: bool,
}

/// A decoded motion sleeve, uploaded to a texture as frames arrive.
pub struct MotionView {
    motion: std::sync::Arc<crate::motion::Motion>,
    tex: Option<egui::TextureHandle>,
    seen: u64,
    /// The width this decoder was started at: a larger view replaces it once.
    width: u32,
}

impl MotionView {
    fn new(motion: std::sync::Arc<crate::motion::Motion>, width: u32) -> Self {
        Self { motion, tex: None, seen: 0, width }
    }

    /// The newest frame as a texture, once one has been decoded.
    fn texture(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        if let Some((version, frame)) = self.motion.latest()
            && version != self.seen
        {
            self.seen = version;
            let image = egui::ColorImage::from_rgba_unmultiplied([frame.w, frame.h], &frame.rgba);
            match &mut self.tex {
                Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
                None => self.tex = Some(ctx.load_texture("apple-motion", image, egui::TextureOptions::LINEAR)),
            }
        }
        self.tex.as_ref()
    }
}

impl Drop for MotionView {
    fn drop(&mut self) {
        // Off screen: the decoder stops at its next frame instead of looping on.
        self.motion.stop();
    }
}

pub struct App {
    pub backend: Backend,
    pub audio: AudioHandle,
    ch: Channels,
    media: Option<Media>,
    discord: Option<crate::discord::Discord>,
    discord_enabled: bool,
    /// Last state sent to Discord, to send only real changes.
    discord_sent: Option<crate::discord::Presence>,
    /// The Last.fm account scrobbles go to; a sign-in in progress and its outcome.
    lastfm_user: Option<String>,
    lastfm_busy: Arc<AtomicBool>,
    lastfm_result: Arc<std::sync::Mutex<Option<Result<String, String>>>>,
    /// The play generation Last.fm last heard about, and whether it was scrobbled.
    scrobble: Option<(u64, bool)>,
    /// Last session's song, restored paused: Play starts it from here.
    resume_at: Option<Duration>,
    /// That song came from another device's queue, so playing it here is a new play.
    resume_counts: bool,
    /// When something last played here (unix seconds), to tell whether another
    /// device has played since. When the cross-device queue was last asked for (`now`).
    listened_at: u64,
    queue_checked: Option<f64>,
    was_focused: bool,
    /// Album / playlist / artist the queue was started from, so its card shows pause.
    queue_source: Option<Target>,
    /// Asked to play (shuffled?) before its songs are here: a playlist's first page, or
    /// an album's or artist's songs (`Req::PlayCollection`).
    play_pending: Option<(Target, bool)>,
    bg: Option<Background>,

    // navigation
    page: Page,
    back: Vec<Page>,
    forward: Vec<Page>,
    page_changed: f64,
    /// A fresh launch starts each page at the top, not where the last run left it.
    scroll_top: bool,
    /// Incremented on each page visit so scroll offset resets to the top instead of persisting.
    page_visit: u64,
    loading: bool,
    /// The current page's request failed with nothing cached: show a retry, not a skeleton.
    load_failed: bool,
    search_text: String,
    /// The route query the box was last filled from, so clearing it isn't undone.
    search_synced: String,
    /// Past searches, newest first, for the search landing.
    recent_searches: Vec<RecentSearch>,
    /// A search whose results are on screen, waiting for the first result to be opened.
    search_pending: bool,
    suggestions: (String, Vec<crate::ytm::Suggestion>),
    /// Debounce timer for live Search-page queries as the user types.
    live_search_due: Option<f64>,
    /// Up to 32 query results, oldest first, scoped to the current suggestion round.
    suggest_cache: VecDeque<(String, Vec<crate::ytm::Suggestion>)>,
    /// The suggestion picked with the arrow keys, which Enter searches.
    suggest_sel: Option<usize>,
    /// Bumped when a past search is removed or the account changes: suggestions
    /// requested before that are dropped when they arrive, rather than bring it back.
    suggest_round: u64,
    /// Removals sent and not yet confirmed: the list is asked for again once they're done.
    forgets_pending: u32,
    suggest_due: Option<f64>,

    // page data
    home: Option<HomeState>,
    home_loading: bool,
    explore: Option<Explore>,
    search: Option<SearchResults>,
    /// Albums and playlists opened lately, so going back to one shows it at once (and
    /// refreshes it quietly), and their loads under way.
    lists: Lists,
    artist: Option<ArtistPage>,
    /// Recent artist pages kept in memory, so returning to an artist displays immediately.
    artists_cache: HashMap<String, (ArtistPage, std::time::Instant)>,
    library: Option<Library>,
    /// A save/remove shown in the library before YouTube Music confirms it: the card and
    /// which way it went, so a failure can put the library back.
    optimistic_save: Option<(Card, bool)>,
    /// A subscription shown flipped on the artist page before YouTube confirms it: the
    /// channel and the state it had, so a failure can put it back.
    optimistic_sub: Option<(String, bool)>,
    /// When the library was last requested; the sidebar spinner gives up after a while.
    library_requested: std::time::Instant,
    history: Option<Vec<Track>>,
    /// The artists the account follows (Library › Artists).
    library_artists: Option<Vec<crate::model::Card>>,
    liked: HashSet<String>,
    /// Songs whose unlike is queued and hasn't gone through yet.
    unliking: HashMap<String, Track>,
    selection: Selection,
    account_name: Option<String>,
    /// The signed-in account's channels and which one is used, once Settings asked.
    channels: Option<crate::backend::Channels>,
    /// When they were last asked for (`ui.input` time).
    channels_asked: Option<f64>,
    /// The channel being switched to, named for the toast once it's done.
    switching_to: Option<String>,
    /// Last launch's playlists, shown in the sidebar until the library arrives.
    sidebar_cache: Option<Vec<Card>>,

    // playback
    pub queue: Queue,
    generation: u64,
    buffering: bool,
    /// Unplayable songs skipped in a row; a run of them stops playback instead of
    /// racing through the queue.
    skip_streak: u32,
    /// Times the current song was reopened after its download failed.
    recoveries: u32,
    autoplay: bool,
    /// Turn loud tracks down to YouTube's loudness target.
    normalize: bool,
    log_volume: bool,
    high_quality: bool,
    /// Play an official music video as its song version when one is found.
    play_songs: bool,
    /// The song version playing in place of the current music video.
    playing_as: Option<PlayingAs>,
    /// The output device picked in settings (cpal's id and its name, for when it's not
    /// connected), and the devices last listed there.
    output_device: Option<(String, String)>,
    output_devices: Vec<crate::audio::OutputDevice>,
    radio_pending: bool,
    seek_drag: Option<f32>,

    // visuals
    art_key: Option<String>,
    art_changed: f64,
    /// Locked backdrop artwork key for the current Home feed so colors stay steady on Home.
    home_backdrop_key: Option<String>,
    /// Apple's page for each artist visited (their display name and hero clips).
    apple_artists: std::collections::HashMap<String, Option<crate::apple::Artwork>>,
    /// Album pages' artist photos by artist id (empty while asked for, or when none).
    artist_photos: std::collections::HashMap<String, Vec<crate::model::Thumb>>,
    /// Motion sleeve master playlist per release key (`None`: Apple has none).
    apple_covers: std::collections::HashMap<String, Option<String>>,
    /// Releases Apple had nothing for when last asked, and when: asked again after a
    /// while, since a lookup can fail on a network hiccup rather than for good.
    apple_missed: std::collections::HashMap<String, std::time::Instant>,
    /// Decoders for the motion sleeves on screen.
    apple_motion: std::collections::HashMap<String, MotionView>,
    /// Apple lookups already in flight.
    apple_waiting: std::collections::HashSet<String>,
    /// Artwork dominant colors cached by key (e.g. "album:{id}", "artist:{id}").
    pub art_colors: std::collections::HashMap<String, egui::Color32>,
    /// Artwork vivid accent colors cached by key.
    pub art_accents: std::collections::HashMap<String, egui::Color32>,
    /// Pre-blurred 64x64 RGBA album/artist artwork bytes cached by key.
    pub art_blurs: std::collections::HashMap<String, Vec<u8>>,
    /// Mean luminance `(overall, right_half)` of the blurred artwork backdrop by key.
    pub art_lumas: std::collections::HashMap<String, (f32, f32)>,
    /// Uploaded blurred artwork textures for album ambient backdrops.
    art_blur_tex: std::collections::HashMap<String, egui::TextureHandle>,
    /// Timestamp when each artwork key finished loading (for smooth fade-in).
    art_loaded_at: std::collections::HashMap<String, f64>,
    /// Artwork color requests currently in flight.
    art_waiting: std::collections::HashSet<String>,
    /// Settings › Artist pages: the listener's own signature service, or empty.
    signature_api: String,
    /// Signatures already uploaded, by name key (`artist_signature::key`).
    signature_tex: std::collections::HashMap<String, egui::TextureHandle>,
    /// Signatures fetched but not yet uploaded, keyed the same way.
    signature_images: std::collections::HashMap<String, (usize, usize, Vec<u8>)>,
    /// Names the service had nothing for, and when: asked again after a minute.
    signature_missed: std::collections::HashMap<String, std::time::Instant>,
    /// Signature lookups already in flight.
    signature_waiting: std::collections::HashSet<String>,
    /// Settings › Playback: the listener's hi-res addon links, in the order given.
    hires_sources: Vec<String>,
    /// The "+ Add a source" row is open.
    hires_adding: bool,
    /// What's typed in that row, before it's added.
    hires_draft: String,
    /// Focus the draft row as soon as it opens.
    hires_focus: bool,
    /// Ask the addons for Dolby Atmos renditions.
    prefer_atmos: bool,
    /// What the playing stream is (codec, rate, depth, size), when a hi-res source
    /// supplied it, with the generation it belongs to.
    audio_info: Option<(u64, crate::hires::Info)>,
    accent: Color32,
    pub dominant: Color32,
    /// Selected theme preset (see `preset.rs`).
    theme_id: String,
    /// Preset whose chrome (glass, corner radius) is installed in the egui style.
    chrome_applied: Option<String>,
    /// Light or dark pages (the ytm-rs theme's; see `light_pages`).
    appearance: theme::Appearance,
    reactive_bg: bool,
    home_gradient: bool,
    /// Skip the parts of music videos SponsorBlock marks as not music.
    sponsorblock: bool,
    /// Crossfade between songs, and over how many seconds.
    crossfade: bool,
    /// DJ-style handoff: the next song eases in under the last one (see `audio::Ramp`).
    automix: bool,
    crossfade_secs: u32,
    /// The next song starts because this one is ending: crossfade into it.
    handoff: bool,
    /// The handoff just begun, for the play that starts the next song to pick up.
    pending_mix: Option<MixFrom>,
    /// The song an Automix handoff came from, shown until the next one is louder.
    mix_from: Option<MixFrom>,
    /// Now playing's sleep timer; off at every launch.
    sleep: sleep::Sleep,
    /// SponsorBlock segments for a video, and when one was last skipped.
    segments: Option<(String, Vec<crate::sponsorblock::Segment>)>,
    segment_skipped: f64,
    /// Background animation clock: advances only while music plays, so pausing
    /// freezes the motion instead of jumping when resumed.
    bg_time: f64,
    last_frame: f64,
    search_rect: Option<egui::Rect>,
    search_focused: bool,
    /// Give the search box its focus back (removing a past search takes it away).
    search_refocus: bool,
    /// A past search was removed during the current press: the rows below slide up
    /// under the held pointer, and must not go too.
    forget_held: bool,
    /// The past search being removed (its token) and when: it folds away first.
    forgetting: Option<(String, f64)>,
    /// Mouse-wheel distance still to be scrolled (smooth scrolling).
    wheel_pending: egui::Vec2,
    wheel_mods: egui::Modifiers,
    /// The wheel/trackpad gesture under way: the axis and pane it's locked to.
    scroll_gesture: scroll::Gesture,
    /// The side panes (sidebar, drawers) drawn last frame, each its own scroller.
    scroll_panes: Vec<egui::Rect>,

    // now playing
    pub np_open: bool,
    pub np_tab: NpTab,
    queue_open: bool,
    /// The lyrics panel at the right of the pages (the player bar's lyrics button).
    lyrics_drawer: bool,
    pub fullscreen: bool,
    /// The mini player: the window shrunk to a strip.
    mini: mini::Mini,
    /// Every source's lyrics for the current song (by video ID).
    lyrics: Option<(String, LyricsSet)>,
    /// Romanization/translation for one source's lines: (video ID, source key, language, extras).
    lyrics_extras: Option<(String, String, String, crate::translate::Extras)>,
    /// The extras last asked for, so each is requested once.
    extras_asked: Option<(String, String, String)>,
    show_roman: bool,
    show_trans: bool,
    lyric_prefs: lyric_prefs::Prefs,
    lyric_view: karaoke::View,
    lyric_clock: lyric_prefs::Clock,
    /// Rotation of a "record" cover (radians); advances only while playing.
    record_angle: f64,

    // account / misc
    logged_in: bool,
    spotify_mode: bool,
    spotify_user: Option<String>,
    spotify_auth_busy: bool,
    spotify_email: String,
    spotify_code: String,
    spotify_password: String,
    spotify_login_pw: bool,
    spotify_challenge: Option<crate::spotify::EmailCodeChallenge>,
    spotify_library: Option<Library>,
    spotify_artists: Option<Vec<Card>>,
    /// The Spotify transfer, running on the network runtime.
    transfer: Arc<crate::transfer::Transfer>,
    xfer: transfer::Setup,
    /// Listen together: the room we're in, if any.
    jam: jam::Session,
    signing_in: Arc<AtomicBool>,
    spotify_web_signing_in: Arc<AtomicBool>,
    toast: Option<(String, f64)>,

    // updates
    updater: Option<crate::update::Updater>,
    /// When the last automatic update check ran (app time, s).
    update_checked: Option<f64>,
    update_announced: bool,
    /// Release notes dialog (update ready, or "See all notes").
    notes_dialog: Option<notes::NotesDialog>,
    /// Version whose "Updated to" card is showing, on the first launch after an update.
    whats_new: Option<String>,
    /// The user picked "Restart to update": install on exit, then relaunch.
    restart_for_update: bool,
    /// Settings › Network: the saved proxy setting (`proxy::active()` is this run's),
    /// the mode picked on the page, the custom address being typed, its error, and
    /// the last connection test (`Some(None)` while one runs).
    proxy_saved: crate::proxy::Setting,
    proxy_mode: crate::proxy::Setting,
    proxy_url: String,
    proxy_error: Option<String>,
    proxy_test: Arc<std::sync::Mutex<ProxyTest>>,
    /// Close and start again (the proxy changed).
    restart_app: bool,
    /// The media controls' cover as a local file (video id, `file://` path), fetched
    /// through the proxy: Windows would otherwise download the URL itself, directly.
    media_cover: Arc<std::sync::Mutex<Option<(String, String)>>>,

    // window
    /// Settings › Window: a notification when a new song starts while the window isn't focused.
    pub(super) notify_songs: bool,
    /// The last song `sync_notify` saw start, so each one is announced once.
    notified: Option<String>,
    /// Settings › Window: draw with wgpu (Vulkan) from the next start (renderer.rs).
    pub(super) wgpu_renderer: bool,
    /// The app icon (`appicon::ALL` id) shown on the window.
    /// Closing for real (Quit from the menu, restart to update): don't hide.
    exiting: bool,
    /// Minimized, or hidden: the window's geometry isn't worth saving.
    window_hidden: bool,
    was_covered: bool,
    /// eframe's saved window geometry ("window") from the last time it was visible.
    good_window: Option<String>,
    /// The sidebar playlists' order (by id), and the ones hidden from it.
    playlist_order: Vec<String>,
    hidden_playlists: HashSet<String>,
    /// Albums/playlists pinned by the user to the sidebar.
    pinned: Vec<Card>,
    /// Collection IDs marked as Favourite or Suggest less.
    favourite_collections: HashSet<String>,
    suggest_less: HashSet<String>,
    /// The collection page whose inline search is open, and what's typed in it.
    coll_search: Option<(String, String)>,
    pub actions: Vec<Action>,
    dialog: Option<Dialog>,
    share_pending: Option<String>,
    share_sheet_pending: Option<(String, egui::Rect)>,
    smoke: Option<String>,
    mouse_last_pos: Option<egui::Pos2>,
    mouse_last_moved: f64,
    mouse_hidden: bool,
}

/// Longest crossfade offered, in seconds.
pub(super) const CROSSFADE_MAX: u32 = 12;

/// The song an Automix handoff came from. Its last stretch is still what you hear
/// while the next one eases in, so the player keeps showing it (with its art and
/// lyrics) until the next song is the louder of the two rather than switching at
/// the first beat of the blend.
pub(super) struct MixFrom {
    pub(super) track: Track,
    /// Its length and the handoff's length, in seconds.
    pub(super) total: f32,
    pub(super) mix: f32,
    /// The audio thread has taken the handoff: the position reads the next song's again.
    pub(super) started: bool,
    /// The player has handed the screen to the next song. The mix keeps running after
    /// that, with this song's tail under it, so the "Mixing" caption stays until its end.
    pub(super) released: bool,
}

impl App {
    /// Creates a headless dummy App instance for integration testing without audio or network dependencies.
    pub fn mock_headless() -> Self {
        let (_tx, resp_rx) = crossbeam_channel::unbounded();
        let (_atx, audio_rx) = crossbeam_channel::unbounded();
        let (_mtx, media_rx) = crossbeam_channel::unbounded();
        let (btx, _brx) = crossbeam_channel::unbounded();
        let tmp = std::env::temp_dir().join(format!("dawnfm_mock_{}", std::process::id()));
        let audio = AudioHandle::dummy();
        let backend = Backend::new(tmp, btx, audio.clone(), Arc::new(|| {})).expect("backend mock");
        let ch = Channels { resp_rx, audio_rx, media_rx };
        Self::new(backend, audio, ch, None, None, None)
    }

    pub fn new(
        backend: Backend,
        audio: AudioHandle,
        ch: Channels,
        media: Option<Media>,
        bg: Option<Background>,
        storage: Option<&dyn eframe::Storage>,
    ) -> Self {
        let get = |k: &str| storage.and_then(|s| s.get_string(k));
        // Dev hook: YTM_FONT=<font name> tries a font without saving it.
        theme::set_font_choice(&dev_var("YTM_FONT").map(str::to_owned).or_else(|| get("font")).unwrap_or_default());
        let discord_enabled = get("discord_enabled").is_none_or(|v| v == "1");
        let app_icon = crate::appicon::load(backend.data_dir());
        let wgpu_renderer = crate::renderer::load(backend.data_dir()) == crate::renderer::Renderer::Wgpu;
        // Updates reinstall the shortcuts with the classic icon: always put ours back,
        // and keep the Start menu link pointing at this exe.
        crate::appicon::sync_shortcuts(app_icon, backend.data_dir());
        crate::downloads::init(backend.data_dir(), &get("download_dir").unwrap_or_default());
        crate::lastfm::init(backend.data_dir());
        let logged_in = backend.is_logged_in();
        // Before any request: a song started at launch (dev hooks) must see this setting.
        backend.set_high_quality(get("high_quality").is_none_or(|v| v == "1"));
        // Before any request: a song started at launch must see the hi-res source too.
        let hires_source = get("hires_source").unwrap_or_default();
        let hires_sources: Vec<String> = hires_source.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
        backend.set_hires_source(&hires_sources.join("\n"));
        let prefer_atmos = get("prefer_atmos").is_some_and(|v| v == "1");
        backend.set_prefer_atmos(prefer_atmos);
        let spotify_user = if logged_in { None } else { backend.saved_spotify_user() };
        let spotify_mode = !logged_in && (spotify_user.is_some() || get("spotify_mode").is_some_and(|v| v == "1"));
        if spotify_mode {
            if spotify_user.is_some() {
                backend.request(Req::SpotifyHome);
                backend.request(Req::SpotifyLibrary);
            }
        } else {
            backend.request(Req::Home(None));
        }
        backend.request(Req::SpotifyAccount);
        if logged_in {
            backend.request(Req::Library);
            backend.request(Req::Account);
            backend.warm_signed_in();
        }
        let transfer_handle = {
            let b = backend.clone();
            crate::transfer::Transfer::new(backend.data_dir(), Arc::new(move || b.wake()))
        };
        // Not a field: the built-in look reads its colour from the presets.
        crate::preset::set_colour(get("colour").as_deref().unwrap_or_default());
        let mut app = Self {
            backend,
            audio,
            ch,
            media,
            discord: discord_enabled.then(crate::discord::Discord::spawn).flatten(),
            discord_enabled,
            discord_sent: None,
            lastfm_user: crate::lastfm::user(),
            lastfm_busy: Default::default(),
            lastfm_result: Default::default(),
            scrobble: None,
            resume_at: None,
            resume_counts: false,
            listened_at: 0,
            queue_checked: None,
            was_focused: false,
            queue_source: None,
            play_pending: None,
            bg,
            page: Page::Home,
            back: Vec::new(),
            forward: Vec::new(),
            page_changed: 0.0,
            scroll_top: true,
            page_visit: 0,
            loading: false,
            load_failed: false,
            search_text: String::new(),
            search_synced: String::new(),
            recent_searches: get("recent_searches").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default(),
            search_pending: false,
            suggestions: Default::default(),
            live_search_due: None,
            suggest_cache: VecDeque::new(),
            suggest_sel: None,
            suggest_round: 0,
            forgets_pending: 0,
            suggest_due: None,
            home: None,
            home_loading: true,
            explore: None,
            search: None,
            lists: Lists::default(),
            artist: None,
            artists_cache: HashMap::new(),
            library: None,
            optimistic_save: None,
            optimistic_sub: None,
            library_requested: std::time::Instant::now(),
            history: None,
            library_artists: None,
            liked: HashSet::new(),
            unliking: HashMap::new(),
            selection: Selection::default(),
            account_name: None,
            channels: None,
            channels_asked: None,
            switching_to: None,
            sidebar_cache: None,
            spotify_mode,
            spotify_user,
            spotify_auth_busy: false,
            spotify_email: String::new(),
            spotify_code: String::new(),
            spotify_password: String::new(),
            spotify_login_pw: false,
            spotify_challenge: None,
            spotify_library: None,
            spotify_artists: None,
            queue: Queue::new(match get("repeat").as_deref() {
                Some("all") => Repeat::All,
                Some("one") => Repeat::One,
                _ => Repeat::Off,
            }),
            generation: 0,
            buffering: false,
            skip_streak: 0,
            recoveries: 0,
            autoplay: get("autoplay").is_none_or(|v| v == "1"),
            normalize: get("normalize").is_none_or(|v| v == "1"),
            log_volume: get("log_volume").is_none_or(|v| v == "1"),
            high_quality: get("high_quality").is_none_or(|v| v == "1"),
            play_songs: get("play_songs").is_none_or(|v| v == "1"),
            playing_as: None,
            output_device: get("output_device").filter(|d| !d.is_empty()).map(|id| (id, get("output_device_name").unwrap_or_default())),
            output_devices: Vec::new(),
            updater: None,
            notify_songs: get("notify_songs").is_some_and(|v| v == "1"),
            notified: None,
            wgpu_renderer,
            exiting: false,
            window_hidden: false,
            was_covered: false,
            good_window: get("window"),
            playlist_order: get("playlist_order").map(|v| v.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()).unwrap_or_default(),
            hidden_playlists: get("hidden_playlists").map(|v| v.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()).unwrap_or_default(),
            pinned: get("pinned_items").and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default(),
            favourite_collections: get("favourite_collections").map(|v| v.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()).unwrap_or_default(),
            suggest_less: get("suggest_less").map(|v| v.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()).unwrap_or_default(),
            coll_search: None,
            update_checked: None,
            update_announced: false,
            notes_dialog: None,
            // First launch of a newer version than last time: show what changed.
            // YTM_WHATS_NEW=1 shows the card for this version regardless.
            whats_new: {
                let current = crate::update::current_version();
                let last = get("last_version");
                (std::env::var_os("YTM_WHATS_NEW").is_some() || last.is_some_and(|v| crate::update::newer(&current, &v))).then_some(current)
            },
            restart_for_update: false,
            proxy_saved: crate::proxy::active().clone(),
            proxy_mode: crate::proxy::active().clone(),
            proxy_url: match crate::proxy::active() {
                crate::proxy::Setting::Custom(u) => u.clone(),
                _ => String::new(),
            },
            proxy_error: None,
            proxy_test: Default::default(),
            restart_app: false,
            media_cover: Default::default(),
            radio_pending: false,
            seek_drag: None,
            art_key: None,
            home_backdrop_key: None,
            apple_artists: Default::default(),
            artist_photos: Default::default(),
            apple_covers: Default::default(),
            apple_missed: Default::default(),
            apple_motion: Default::default(),
            apple_waiting: Default::default(),
            art_colors: Default::default(),
            art_accents: Default::default(),
            art_blurs: Default::default(),
            art_lumas: Default::default(),
            art_blur_tex: Default::default(),
            art_loaded_at: Default::default(),
            art_waiting: Default::default(),
            signature_api: get("signature_api").unwrap_or_default(),
            signature_tex: Default::default(),
            signature_images: Default::default(),
            signature_missed: Default::default(),
            signature_waiting: Default::default(),
            hires_sources,
            hires_adding: false,
            hires_draft: String::new(),
            hires_focus: false,
            prefer_atmos,
            audio_info: None,
            art_changed: -10.0,
            accent: theme::BLUE,
            dominant: Color32::from_rgb(0x18, 0x14, 0x1c),
            theme_id: crate::preset::DEFAULT_ID.to_owned(),
            chrome_applied: None,
            // Dev hook: YTM_APPEARANCE=light|dark|system tries a mode without saving it.
            appearance: theme::Appearance::from_key(&dev_var("YTM_APPEARANCE").map(str::to_owned).or_else(|| get("appearance")).unwrap_or_default()),
            reactive_bg: get("reactive_bg").is_none_or(|v| v == "1"),
            home_gradient: get("home_gradient").is_some_and(|v| v == "1"),
            sponsorblock: get("sponsorblock").is_some_and(|v| v == "1"),
            crossfade: get("crossfade").is_some_and(|v| v == "1"),
            automix: get("automix").is_some_and(|v| v == "1"),
            handoff: false,
            pending_mix: None,
            mix_from: None,
            sleep: sleep::Sleep::Off,
            crossfade_secs: get("crossfade_secs").and_then(|v| v.parse().ok()).unwrap_or(5).clamp(1, CROSSFADE_MAX),
            segments: None,
            segment_skipped: f64::NEG_INFINITY,
            bg_time: 0.0,
            last_frame: 0.0,
            search_rect: None,
            search_focused: false,
            search_refocus: false,
            forget_held: false,
            forgetting: None,
            wheel_pending: egui::Vec2::ZERO,
            wheel_mods: egui::Modifiers::NONE,
            scroll_gesture: scroll::Gesture::default(),
            scroll_panes: Vec::new(),
            np_open: false,
            np_tab: if get("np_art").is_some_and(|v| v == "1") { NpTab::Art } else { NpTab::Lyrics },
            queue_open: false,
            lyrics_drawer: get("lyrics_drawer").is_some_and(|v| v == "1"),
            fullscreen: false,
            mini: mini::Mini::load(get),
            lyrics: None,
            lyrics_extras: None,
            extras_asked: None,
            show_roman: get("show_roman").is_none_or(|v| v == "1"),
            show_trans: get("show_trans").is_none_or(|v| v == "1"),
            lyric_prefs: lyric_prefs::Prefs::load(&get),
            lyric_view: Default::default(),
            lyric_clock: Default::default(),
            record_angle: 0.0,
            logged_in,
            signing_in: Default::default(),
            spotify_web_signing_in: Default::default(),
            transfer: transfer_handle,
            xfer: Default::default(),
            jam: Default::default(),
            toast: None,
            actions: Vec::new(),
            dialog: None,
            share_pending: None,
            share_sheet_pending: None,
            smoke: None,
            mouse_last_pos: None,
            mouse_last_moved: 0.0,
            mouse_hidden: false,
        };
        app.request_liked(false);
        // Dev hooks: YTM_SMOKE="query" searches and plays the first song ("id:<videoId>[,<videoId>…]"
        // plays that video); YTM_SEARCH
        // only searches; YTM_PAGE opens a page; YTM_NOWPLAYING opens the overlay.
        if let Some(s) = get(SESSION_KEY).filter(|_| std::env::var_os("YTM_SMOKE").is_none()) {
            app.restore_session(&s);
        }
        let smoke = std::env::var("YTM_SMOKE").ok().filter(|q| !q.is_empty());
        if let Some(ids) = smoke.as_deref().and_then(|q| q.strip_prefix("id:")) {
            // "id:a,b,c" queues several.
            let track = |id: &str| Track { id: id.to_owned(), title: id.to_owned(), artists: vec![], album: None, duration: None, thumbs: vec![], track_nr: None, plays: None, music_video: false, explicit: false };
            app.play_list(ids.split(',').map(track).collect(), 0);
        } else if let Some(q) = smoke {
            app.go(Page::Search(q.clone()));
            app.smoke = Some(q);
        } else if let Some(q) = std::env::var("YTM_SEARCH").ok().filter(|q| !q.is_empty()) {
            app.go(Page::Search(q));
        }
        match std::env::var("YTM_PAGE").as_deref() {
            Ok("home") => app.go(Page::Home),
            Ok("new") | Ok("explore") => app.go(Page::New),
            Ok("library") => app.go(Page::Library(LibTab::Playlists)),
            Ok("liked") => app.go(Page::Library(LibTab::Songs)),
            Ok("history") => app.go(Page::Library(LibTab::History)),
            Ok("artists") => app.go(Page::Library(LibTab::Artists)),
            Ok("settings") => app.go(Page::Settings),
            Ok("transfer") => app.go(Page::Transfer),
            Ok("jam") => app.go(Page::Jam),
            Ok(p) if p.starts_with("album:") => app.go(Page::Album(p[6..].to_owned())),
            Ok(p) if p.starts_with("artist:") => app.go(Page::Artist(p[7..].to_owned())),
            Ok(p) if p.starts_with("playlist:") => app.go(Page::Playlist(p[9..].to_owned())),
            _ => {}
        }
        // Dev hook: YTM_PROXY_INPUT=<address> opens Settings › Network on Custom with it typed in.
        if let Some(v) = dev_var("YTM_PROXY_INPUT") {
            app.proxy_mode = crate::proxy::Setting::Custom(String::new());
            app.proxy_url = v.to_owned();
        }
        if std::env::var("YTM_NOWPLAYING").is_ok() {
            app.np_open = true;
        }
        app.jam.name = get("jam_name").unwrap_or_default();
        // Last launch's name, picture and playlists, so they show at once; the account
        // menu and the library replace them when they arrive.
        if app.logged_in
            && let Some(c) = get("account_cache").and_then(|j| serde_json::from_str::<AccountCache>(&j).ok())
        {
            app.account_name = c.name;
            app.sidebar_cache = Some(c.playlists).filter(|p| !p.is_empty());
        }
        // Dev hook: YTM_QUEUE opens the queue drawer.
        if std::env::var_os("YTM_QUEUE").is_some() {
            app.queue_open = true;
        }
        jam::dev_hooks(&mut app);
        app.audio.set_normalize(app.normalize);
        app.set_crossfade();
        app
    }

    // ---------------------------------------------------------------- navigation

    fn go(&mut self, page: Page) {
        self.np_open = false;
        self.live_search_due = None;
        if let Page::Search(q) = &page {
            let trimmed = q.trim().to_owned();
            let target = Page::Search(trimmed.clone());
            if trimmed.is_empty() {
                self.search_refocus = true;
            }
            if target == self.page {
                self.page_visit = self.page_visit.wrapping_add(1);
                self.scroll_top = true;
                self.refresh();
                return;
            }
            self.apple_motion.clear();
            let prev = std::mem::replace(&mut self.page, target);
            self.back.push(prev);
            self.forward.clear();
            self.page_loaded();
            return;
        }
        self.search_text.clear();
        self.search_synced.clear();
        if page == self.page {
            // Clicking the page you're on refreshes it and resets scroll to the top.
            self.page_visit = self.page_visit.wrapping_add(1);
            self.scroll_top = true;
            self.refresh();
            return;
        }
        // Leaving a page deloads its motion clips: no decoder keeps running unseen.
        self.apple_motion.clear();
        let prev = std::mem::replace(&mut self.page, page);
        self.back.push(prev);
        self.forward.clear();
        self.page_loaded();
    }

    fn nav_back(&mut self) {
        if self.np_open {
            self.np_open = false;
        } else if let Some(p) = self.back.pop() {
            self.apple_motion.clear();
            self.forward.push(std::mem::replace(&mut self.page, p));
            self.page_loaded();
        }
    }

    fn nav_forward(&mut self) {
        if let Some(p) = self.forward.pop() {
            self.apple_motion.clear();
            self.back.push(std::mem::replace(&mut self.page, p));
            self.page_loaded();
        }
    }

    /// Reload the current page's data in place: no page transition, the page stays as
    /// it is until the new data arrives.
    fn refresh(&mut self) {
        if self.spotify_mode && matches!(self.page, Page::Library(LibTab::Playlists | LibTab::Albums | LibTab::Artists)) && self.spotify_user.is_some() {
            self.backend.request(Req::SpotifyLibrary);
        }
        let changed = self.page_changed;
        self.page_loaded();
        self.page_changed = changed;
    }

    /// Start the page transition and (re)load the page's data. Every page refreshes on
    /// each visit (things change outside the app: likes, other devices, new releases),
    /// showing the cached copy meanwhile.
    fn page_loaded(&mut self) {
        self.page_visit = self.page_visit.wrapping_add(1);
        self.scroll_top = true;
        self.page_changed = -1.0; // stamped with the frame time on the next pass
        // The search box holds the query of the search page you're on, and is empty elsewhere.
        self.search_synced = match &self.page {
            Page::Search(q) => q.clone(),
            _ => String::new(),
        };
        self.search_text = self.search_synced.clone();
        self.suggestions = Default::default();
        self.suggest_sel = None;
        // Choosing a transfer match lasts while searching; any other page ends it.
        if !matches!(self.page, Page::Search(_)) {
            self.xfer.picking = None;
        }
        if let Some(id) = self.page_list().map(str::to_owned) {
            self.lists.move_to_front(&id);
        }
        let req = match &self.page.clone() {
            Page::Home if !self.home_loading => {
                if self.spotify_mode {
                    if self.spotify_user.is_some() {
                        self.home_loading = true;
                        Some(Req::SpotifyHome)
                    } else {
                        None
                    }
                } else {
                    self.home_loading = true;
                    Some(Req::Home(None))
                }
            }
            Page::New => {
                if self.spotify_mode {
                    self.spotify_user.is_some().then_some(Req::SpotifyExplore)
                } else {
                    Some(Req::Explore(self.preferred_artist_ids()))
                }
            }
            // The empty search is the page's landing state: nothing to ask for.
            Page::Search(q) if !q.trim().is_empty() && self.search.as_ref().is_none_or(|s| &s.query != q) => Some(self.search_req(q.clone())),
            Page::Album(id) | Page::Playlist(id) => {
                let is_album = matches!(self.page, Page::Album(_));
                if is_album {
                    let thumbs = self.opened(id).map(|o| o.c.thumbs.clone());
                    if let Some(thumbs) = thumbs {
                        self.request_album_art(id, &thumbs);
                    }
                }
                self.visit_list(id, is_album);
                None
            }
            Page::Artist(id) => {
                let cached_artist = self.artists_cache.get(id).and_then(|(cached, at)| {
                    if at.elapsed() < std::time::Duration::from_secs(10 * 60) {
                        Some(cached.clone())
                    } else {
                        None
                    }
                });
                if let Some(cached) = cached_artist {
                    let thumbs = if !cached.banner.is_empty() {
                        cached.banner.clone()
                    } else if !cached.photo.is_empty() {
                        cached.photo.clone()
                    } else {
                        cached.albums.first().map(|c| c.thumbs.clone()).unwrap_or_default()
                    };
                    self.artist = Some(cached);
                    if !thumbs.is_empty() {
                        self.request_artist_art(id, &thumbs);
                    }
                }
                Some(Req::Artist(id.clone()))
            }
            Page::Library(LibTab::History) if self.logged_in && !self.spotify_mode => Some(Req::History),
            Page::Library(LibTab::Artists) if self.spotify_mode && self.spotify_user.is_some() => self.spotify_artists.is_none().then_some(Req::SpotifyLibrary),
            Page::Library(LibTab::Artists) if self.logged_in => {
                // The "From your history" rows need History too.
                if self.history.is_none() {
                    self.backend.request(Req::History);
                } else if let Some(h) = &self.history {
                    for a in pages::played_artists(h).into_iter().take(pages::HISTORY_ARTISTS_LIMIT) {
                        if let Some(id) = a.id {
                            if !self.artist_photos.contains_key(&id) {
                                self.artist_photos.insert(id.clone(), Vec::new());
                                self.backend.request(Req::ArtistPhoto(id));
                            }
                        }
                    }
                }
                Some(Req::Artists)
            }
            Page::Library(LibTab::Songs) => {
                let liked_id = self.active_liked_id();
                if self.active_logged_in() {
                    self.visit_list(liked_id, false);
                }
                None
            }
            Page::Library(_) if self.spotify_mode && self.spotify_user.is_some() => self.spotify_library.is_none().then_some(Req::SpotifyLibrary),
            Page::Library(_) if self.logged_in => Some(Req::Library),
            _ => None,
        };
        // The spinner is only for a page with nothing to show yet.
        let cached = match &self.page {
            Page::Home => self.home.is_some(),
            Page::New => self.explore.is_some(),
            Page::Album(id) | Page::Playlist(id) => self.opened(id).is_some(),
            Page::Artist(id) => self.artist.as_ref().is_some_and(|a| &a.id == id),
            Page::Library(LibTab::History) => self.history.is_some(),
            Page::Library(LibTab::Artists) => self.active_artists().is_some(),
            Page::Library(LibTab::Songs) => self.opened(self.active_liked_id()).is_some(),
            Page::Library(_) => self.active_library().is_some(),
            _ => false,
        };
        let coming = self.page_list().is_some_and(|id| self.lists.loading(id));
        self.loading = (req.is_some() || coming) && !cached;
        self.load_failed = false;
        match req {
            Some(Req::Library) => self.request_library(),
            Some(Req::SpotifyLibrary) => {
                self.library_requested = std::time::Instant::now();
                self.backend.request(Req::SpotifyLibrary);
            }
            Some(r) => self.backend.request(r),
            None => {}
        }
    }

    /// The album or playlist the page shows (the Library's liked songs are Liked Music).
    fn page_list(&self) -> Option<&str> {
        match &self.page {
            Page::Album(id) | Page::Playlist(id) => Some(id),
            Page::Library(LibTab::Songs) => Some(self.active_liked_id()),
            _ => None,
        }
    }

    pub(super) fn active_liked_id(&self) -> &'static str {
        if self.spotify_mode {
            crate::spotify::SPOTIFY_LIKED_ID
        } else {
            LIKED_MUSIC
        }
    }

    pub(super) fn active_library(&self) -> Option<&Library> {
        if self.spotify_mode {
            self.spotify_library.as_ref()
        } else {
            self.library.as_ref()
        }
    }

    pub(super) fn active_artists(&self) -> Option<&[Card]> {
        if self.spotify_mode {
            self.spotify_artists.as_deref()
        } else {
            self.library_artists.as_deref()
        }
    }

    pub(super) fn active_logged_in(&self) -> bool {
        if self.spotify_mode {
            self.spotify_user.is_some()
        } else {
            self.logged_in
        }
    }

    /// Whether the account backing `id` (Spotify or YouTube Music) is signed in.
    pub(super) fn signed_in_for(&self, id: &str) -> bool {
        if crate::spotify::is_spotify_id(id) {
            self.spotify_user.is_some()
        } else {
            self.logged_in
        }
    }

    pub(super) fn sign_in_prompt_for(&self, id: &str, general_action: &str, item_action: &str) -> String {
        if crate::spotify::is_spotify_id(id) && self.logged_in {
            format!("Sign in to Spotify to {item_action}")
        } else if !crate::spotify::is_spotify_id(id) && self.spotify_user.is_some() {
            format!("Sign in to YouTube Music to {item_action}")
        } else {
            format!("Sign in to {general_action}")
        }
    }

    pub(super) fn search_req(&self, query: String) -> Req {
        if self.spotify_mode {
            Req::SpotifySearch(query)
        } else {
            Req::Search(query)
        }
    }

    pub(super) fn set_spotify_mode(&mut self, on: bool) {
        if self.spotify_mode == on {
            return;
        }
        self.spotify_mode = on;
        self.search = None;
        self.home = None;
        self.home_loading = false;
        self.explore = None;
        if on && self.spotify_user.is_some() && self.spotify_library.is_none() {
            self.request_library();
        }
        match &self.page {
            Page::Home | Page::New | Page::Search(_) | Page::Library(_) => self.page_loaded(),
            _ => {}
        }
    }

    /// Ask Apple, once, for the release's motion sleeve.
    fn apple_cover(&mut self, album: &str, artist: &str, title: Option<&str>) {
        if !MOTION_COVERS || album.is_empty() || artist.is_empty() {
            return;
        }
        let key = format!("{album}\u{1}{artist}");
        if self.apple_covers.contains_key(&key) {
            return;
        }
        if self.apple_missed.get(&key).is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(300)) {
            return;
        }
        if !self.apple_waiting.insert(key.clone()) {
            return;
        }
        self.backend.request(Req::AppleCover {
            key,
            album: album.to_owned(),
            artist: artist.to_owned(),
            title: title.map(str::to_owned),
        });
    }

    /// The newest frame of the release's motion sleeve, started the first time it's seen.
    /// `None` until Apple has answered (or if it has no clip for the release).
    fn apple_sprite(&mut self, ctx: &egui::Context, album: &str, artist: &str, max_width: u32) -> Option<widgets::MotionSprite> {
        if !MOTION_COVERS {
            return None;
        }
        let key = format!("{album}\u{1}{artist}");
        let url = self.apple_covers.get(&key)?.clone()?;
        self.motion_sprite(ctx, key, &url, max_width)
    }

    /// The artist's handwritten signature, from the service in Settings: asked for once
    /// per name, uploaded the frame it arrives. `None` while it's on its way or when
    /// the service has none for them.
    fn signature_texture(&mut self, ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
        let endpoint = crate::artist_signature::endpoint(&self.signature_api);
        let canon = crate::artist_signature::key(name);
        if endpoint.is_empty() || canon.is_empty() {
            return None;
        }
        // The link is part of the key: an answer from a service the field no longer
        // holds is not this artist's signature.
        let key = format!("{endpoint}\u{1}{canon}");
        if let Some(tex) = self.signature_tex.get(&key) {
            return Some(tex.clone());
        }
        if let Some((w, h, rgba)) = self.signature_images.remove(&key) {
            let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            let tex = ctx.load_texture("artist-signature", image, egui::TextureOptions::LINEAR);
            // A few artists' marks at most: the header never shows two at once.
            if self.signature_tex.len() >= 8 {
                self.signature_tex.clear();
            }
            self.signature_tex.insert(key, tex.clone());
            return Some(tex);
        }
        if self.signature_missed.get(&key).is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(60)) {
            return None;
        }
        if self.signature_waiting.insert(key.clone()) {
            self.backend.request(Req::ArtistSignature { key, endpoint, name: name.to_owned() });
        }
        None
    }

    /// The newest frame of a motion clip, started the first time it's seen. The decoders
    /// on screen are capped so old pages don't leave threads behind.
    fn motion_sprite(&mut self, ctx: &egui::Context, key: String, url: &str, max_width: u32) -> Option<widgets::MotionSprite> {
        // One decoder per release. The strip and the player ask for different sizes, and
        // replacing it per caller would restart the clip every frame and never show a
        // frame: a bigger view takes over only when it needs a wider decode.
        let stale = self.apple_motion.get(&key).is_none_or(|v| v.width < max_width);
        if stale {
            if self.apple_motion.len() >= 8 {
                self.apple_motion.clear();
            }
            self.apple_motion.insert(key.clone(), MotionView::new(crate::motion::play(ctx, url, max_width), max_width));
        }
        let tex = self.apple_motion.get_mut(&key)?.texture(ctx)?;
        Some(widgets::MotionSprite { id: tex.id(), size: tex.size_vec2() })
    }

    fn accent_color(&self) -> Color32 {
        let c = self.look().accent(self.accent);
        if theme::light() {
            // A pale cover colour would barely show on light pages: deepen it.
            let luma = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
            if luma > 150.0 {
                return theme::lerp_color(c, Color32::BLACK, ((luma - 150.0) / 105.0 * 0.45).min(0.45));
            }
        }
        c
    }

    pub fn dominant_color(&self) -> Color32 {
        self.dominant
    }

    pub(super) fn request_album_art(&mut self, id: &str, thumbs: &[crate::model::Thumb]) {
        let key = format!("album:{id}");
        if self.art_colors.contains_key(&key) || self.art_waiting.contains(&key) {
            return;
        }
        if let Some(t) = crate::model::pick_thumb(thumbs, 544).or_else(|| thumbs.first()) {
            self.art_waiting.insert(key.clone());
            self.backend.request(crate::backend::Req::Art { key, url: t.url.clone() });
        }
    }

    pub(super) fn request_artist_art(&mut self, id: &str, thumbs: &[crate::model::Thumb]) {
        let key = format!("artist:{id}");
        if self.art_colors.contains_key(&key) || self.art_waiting.contains(&key) {
            return;
        }
        if let Some(t) = thumbs.iter().max_by_key(|t| t.width).or_else(|| thumbs.first()) {
            self.art_waiting.insert(key.clone());
            self.backend.request(crate::backend::Req::Art { key, url: t.url.clone() });
        }
    }

    fn home_art_candidates(&self) -> Vec<(String, String)> {
        let Some(h) = self.home.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        for s in &h.shelves {
            let mut push_thumbs = |thumbs: &[crate::model::Thumb]| -> bool {
                if let Some(t) = crate::model::pick_thumb(thumbs, 544).or_else(|| thumbs.first()) {
                    let key = format!("home:{}", t.url);
                    if !out.iter().any(|(k, _)| k == &key) {
                        out.push((key, t.url.clone()));
                    }
                }
                out.len() >= 16
            };
            match &s.kind {
                crate::model::ShelfKind::Cards(cards) => {
                    for c in cards.iter().filter(|c| !c.thumbs.is_empty()).take(10) {
                        if push_thumbs(&c.thumbs) {
                            return out;
                        }
                    }
                }
                crate::model::ShelfKind::Songs { tracks, .. } => {
                    for t in tracks.iter().filter(|t| !t.thumbs.is_empty()).take(10) {
                        if push_thumbs(&t.thumbs) {
                            return out;
                        }
                    }
                }
            }
        }
        out
    }

    fn is_vibrant_art(&self, key: &str) -> bool {
        let Some(&dom) = self.art_colors.get(key) else { return false };
        let (_, s, v) = rgb_to_hsv(dom.r() as f32 / 255.0, dom.g() as f32 / 255.0, dom.b() as f32 / 255.0);
        // Skip white/pale, near-black, neutral grayscale, or cool blue covers for Home backdrop.
        !is_blue_hue(dom) && s >= 0.22 && v >= 0.14 && !(v > 0.85 && s < 0.30)
    }

    pub(super) fn request_home_art(&mut self) {
        if self.home_backdrop_key.is_some() {
            return;
        }
        let candidates = self.home_art_candidates();
        let mut fallback: Option<String> = None;
        for (key, url) in &candidates {
            if self.art_colors.contains_key(key) && self.art_blurs.contains_key(key) {
                if self.is_vibrant_art(key) {
                    self.home_backdrop_key = Some(key.clone());
                    return;
                }
                if fallback.is_none() && self.art_colors.get(key).is_some_and(|&c| !is_blue_hue(c)) {
                    fallback = Some(key.clone());
                }
                continue;
            }
            if !self.art_waiting.contains(key) {
                self.art_waiting.insert(key.clone());
                self.backend.request(crate::backend::Req::Art { key: key.clone(), url: url.clone() });
            }
            return;
        }
        if let Some(k) = fallback {
            self.home_backdrop_key = Some(k);
        }
    }
}

fn is_blue_hue(c: Color32) -> bool {
    let (h, s, _) = rgb_to_hsv(c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0);
    let h_deg = h * 360.0;
    s >= 0.14 && (185.0..=262.0).contains(&h_deg)
}

fn warm_non_blue(c: Color32) -> Color32 {
    if !is_blue_hue(c) {
        return c;
    }
    let luma = (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32).clamp(32.0, 220.0);
    Color32::from_rgb(
        (luma * 1.05).min(255.0) as u8,
        (luma * 0.96).min(255.0) as u8,
        (luma * 0.92).min(255.0) as u8,
    )
}

fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let v = max;
    let s = if max > 0.0 { delta / max } else { 0.0 };
    if delta <= 0.0001 {
        return (0.0, 0.0, v);
    }
    let h = if (max - r).abs() < 0.0001 {
        let mut seg = (g - b) / delta;
        if seg < 0.0 {
            seg += 6.0;
        }
        seg / 6.0
    } else if (max - g).abs() < 0.0001 {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    (h, s, v)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    if s <= 0.0 {
        return (v, v, v);
    }
    let h_deg = (h.fract() * 6.0).rem_euclid(6.0);
    let i = h_deg.floor() as i32;
    let f = h_deg - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

impl App {

    /// Target page background color:
    /// In dark mode, starts from #111111 base and adds a clean, non-muddy chromatic tint.
    /// In light mode, starts from the #f6f4f8 base and adds a subtle, fresh pastel ambient wash.
    pub fn page_target_color(&self) -> Color32 {
        let is_light = theme::light();
        let base_default = if is_light { theme::bg() } else { Color32::from_rgb(0x11, 0x11, 0x11) };
        if self.look().id != crate::preset::DEFAULT_ID {
            return if is_light { theme::bg() } else { self.look().solid() };
        }

        // Only tint background for Artist and Album pages matching that page's artwork
        let page_dom = match &self.page {
            Page::Album(id) => self.art_colors.get(&format!("album:{id}")).copied(),
            Page::Artist(id) => self.art_colors.get(&format!("artist:{id}")).copied(),
            _ => None,
        };

        let Some(dom) = page_dom else {
            return base_default;
        };

        let (r, g, b) = (dom.r() as f32 / 255.0, dom.g() as f32 / 255.0, dom.b() as f32 / 255.0);
        let (h, s, v) = rgb_to_hsv(r, g, b);
        let h_deg = h * 360.0;

        // Perceptual grayscale cutoff: covers perceived as B&W / neutral gray stay obsidian dark.
        if s < 0.17 || v < 0.08 {
            return base_default;
        }

        if is_light {
            // Fresh, clean pastel ambient wash on light mode background
            let light_s = (0.06 + 0.06 * s).clamp(0.06, 0.12);
            let light_v = (0.98 - 0.02 * s).clamp(0.95, 0.99);
            let (pr, pg, pb) = hsv_to_rgb(h_deg / 360.0, light_s, light_v);
            let (br, bg, bb) = (246.0 / 255.0, 244.0 / 255.0, 248.0 / 255.0);
            let lr = ((br * 0.45 + pr * 0.55) * 255.0).round().clamp(220.0, 255.0) as u8;
            let lg = ((bg * 0.45 + pg * 0.55) * 255.0).round().clamp(220.0, 255.0) as u8;
            let lb = ((bb * 0.45 + pb * 0.55) * 255.0).round().clamp(220.0, 255.0) as u8;
            Color32::from_rgb(lr, lg, lb)
        } else {
            // Rich, authentic dark tint preserving true artwork hue without distorting blues or snapping to green.
            let dark_s = (0.38 + 0.44 * s).clamp(0.38, 0.80);
            let dark_v = (0.19 + 0.08 * s).clamp(0.19, 0.27);
            let (dr, dg, db) = hsv_to_rgb(h_deg / 360.0, dark_s, dark_v);
            Color32::from_rgb(
                (dr * 255.0).round() as u8,
                (dg * 255.0).round() as u8,
                (db * 255.0).round() as u8,
            )
        }
    }

    pub(super) fn blurred_art_tex(&mut self, ctx: &egui::Context, key: &str) -> Option<egui::TextureHandle> {
        if let Some(tex) = self.art_blur_tex.get(key) {
            return Some(tex.clone());
        }
        let rgba = self.art_blurs.get(key)?;
        if rgba.len() != (crate::art::SIZE * crate::art::SIZE * 4) as usize {
            return None;
        }
        let img = egui::ColorImage::from_rgba_unmultiplied([crate::art::SIZE as usize, crate::art::SIZE as usize], rgba);
        let tex = ctx.load_texture(format!("album-blur:{key}"), img, egui::TextureOptions::LINEAR);
        self.art_blur_tex.insert(key.to_owned(), tex.clone());
        Some(tex)
    }

    /// Paint the window-wide backdrop with warm blurred artwork and alive ambient glow.
    fn paint_page_backdrop(&mut self, ctx: &egui::Context, painter: &egui::Painter, area: egui::Rect, now: f64) {
        let base = self.page_color(ctx);
        if self.look().id == crate::preset::DEFAULT_ID {
            painter.rect_filled(area, 0.0, base);
        }

        let is_home = matches!(self.page, Page::Home);
        let home_vis = theme::anim_bool(ctx, "page-backdrop-home", is_home && self.home_gradient, 0.70);
        if is_home && !self.home_gradient && home_vis <= 0.001 {
            return;
        }

        let key = match &self.page {
            Page::Album(id) => format!("album:{id}"),
            Page::Artist(id) => format!("artist:{id}"),
            Page::Home => {
                self.request_home_art();
                let Some(k) = self.home_backdrop_key.clone() else {
                    return;
                };
                k
            }
            _ => return,
        };
        let Some(&dom) = self.art_colors.get(&key) else {
            return;
        };
        if is_home && is_blue_hue(dom) {
            return;
        }
        let (_, s, v) = rgb_to_hsv(dom.r() as f32 / 255.0, dom.g() as f32 / 255.0, dom.b() as f32 / 255.0);
        if !is_home && (s < 0.17 || v < 0.08) {
            return;
        }

        let blur_tex = self.blurred_art_tex(ctx, &key);

        let loaded_t = self.art_loaded_at.get(&key).copied().unwrap_or(now);
        let load_fade = theme::ease_out(((now - loaded_t) / 0.65).clamp(0.0, 1.0) as f32);
        let fade = load_fade * if is_home { theme::ease_out(home_vis) } else { 1.0 };
        if fade > 0.0 && fade < 1.0 {
            ctx.request_repaint_after(Duration::from_millis(16));
        }

        let light = theme::light();
        let p = painter.with_clip_rect(area);
        let glow_h = (area.height() * 0.90).clamp(520.0, 840.0).min(area.height());

        if let Some(tex) = blur_tex {
            // Keep Home completely calm so the gradient never drifts or shifts colors while browsing Home.
            let t = if !is_home && self.playing() { (now * 0.28) as f32 } else { 0.0 };
            let dx = t.sin() * 0.045;
            let dy = (t * 0.73).cos() * 0.045;
            let u0 = (0.08 + dx).clamp(0.02, 0.20);
            let u1 = (0.92 + dx).clamp(0.80, 0.98);
            let v0 = (0.08 + dy).clamp(0.02, 0.20);
            let v1 = (0.92 + dy).clamp(0.80, 0.98);

            let wash_rect = egui::Rect::from_min_max(area.min, egui::pos2(area.right(), area.top() + glow_h));

            let max_a = match (is_home, light) {
                (true, true) => 0.34f32,
                (true, false) => 0.48f32,
                (false, true) => 0.20f32,
                (false, false) => 0.50f32,
            };
            const ROWS: usize = 12;
            let xs = [0.0f32, 0.5, 1.0];
            let x_boost = [0.95f32, 1.0, 0.95];

            let mut mesh = egui::Mesh::with_texture(tex.id());
            for row in 0..ROWS {
                let ty = row as f32 / (ROWS - 1) as f32;
                let curve = (0.5 * (1.0 + (ty * std::f32::consts::PI).cos())).powf(1.35);
                let base_a = max_a * curve;
                let y = wash_rect.top() + ty * wash_rect.height();
                let v = v0 + ty * (v1 - v0);
                for (&tx, &xb) in xs.iter().zip(x_boost.iter()) {
                    let x = wash_rect.left() + tx * wash_rect.width();
                    let u = u0 + tx * (u1 - u0);
                    let a = (base_a * fade * xb).clamp(0.0, 1.0);
                    // Subtle warm tint on dark backgrounds so colors feel alive rather than cold.
                    let (wr, wg, wb) = if light { (1.0, 1.0, 1.0) } else { (1.0, 0.96, 0.91) };
                    let col = Color32::from_rgba_premultiplied(
                        (255.0 * a * wr).min(255.0) as u8,
                        (255.0 * a * wg).min(255.0) as u8,
                        (255.0 * a * wb).min(255.0) as u8,
                        (255.0 * a).min(255.0) as u8,
                    );
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: egui::pos2(x, y),
                        uv: egui::pos2(u, v),
                        color: col,
                    });
                }
            }
            let cols = xs.len() as u32;
            for r in 0..(ROWS as u32 - 1) {
                for c in 0..(cols - 1) {
                    let i0 = r * cols + c;
                    let i1 = i0 + 1;
                    let i2 = i0 + cols;
                    let i3 = i2 + 1;
                    mesh.add_triangle(i0, i1, i2);
                    mesh.add_triangle(i1, i3, i2);
                }
            }
            p.add(egui::Shape::mesh(mesh));
        }

        // Smooth ambient halo softly blending across the top of the page (never blue on Home).
        if let Some(&raw_acc) = self.art_accents.get(&key) {
            let acc = if is_home { warm_non_blue(raw_acc) } else { raw_acc };
            let breath = if !is_home && self.playing() { 1.0 + 0.06 * (now as f32 * 0.8).sin() } else { 1.0 };
            let warm_acc = if light {
                theme::lerp_color(acc, Color32::WHITE, 0.20)
            } else {
                theme::lerp_color(acc, Color32::from_rgb(255, 224, 196), 0.16)
            };
            let center = egui::pos2(area.left() + shell::SIDEBAR_W + (area.width() - shell::SIDEBAR_W) * 0.35, area.top() + 140.0);
            let rad_x = (area.width() * 0.65).clamp(420.0, 820.0);
            let rad_y = glow_h * 0.55;
            let mut halo = egui::Mesh::default();
            let halo_alpha = (if light {
                if is_home { 0.16 } else { 0.08 }
            } else {
                if is_home { 0.14 } else { 0.17 }
            }) * fade * breath;
            halo.colored_vertex(center, theme::with_alpha(warm_acc, halo_alpha));
            const SEGS: usize = 36;
            for i in 0..=SEGS {
                let ang = (i as f32 / SEGS as f32) * std::f32::consts::TAU;
                halo.colored_vertex(
                    center + egui::vec2(ang.cos() * rad_x, ang.sin() * rad_y),
                    Color32::TRANSPARENT,
                );
                if i > 0 {
                    halo.add_triangle(0, i as u32, i as u32 + 1);
                }
            }
            p.add(egui::Shape::mesh(halo));
        }
    }

    /// The page background color animated smoothly across transitions.
    pub fn page_color(&self, ctx: &egui::Context) -> Color32 {
        if self.look().id != crate::preset::DEFAULT_ID {
            return self.page_target_color();
        }
        let target = self.page_target_color();
        let ch = |i: usize, val: u8| theme::anim(ctx, ("page-ambient-ch", i), f32::from(val), 0.8).round() as u8;
        Color32::from_rgb(ch(0, target.r()), ch(1, target.g()), ch(2, target.b()))
    }

    /// Sidebar background fill: translucent frosted wash harmonizing with the active page
    /// backdrop so blurred album/artist colors flow softly through the sidebar in both
    /// light and dark modes (matching Apple Music).
    pub fn sidebar_fill(&self, ctx: &egui::Context, alpha: f32) -> Color32 {
        let p_col = self.page_color(ctx);
        if theme::light() {
            let tinted = theme::lerp_color(p_col, Color32::WHITE, 0.25);
            if alpha >= 1.0 {
                tinted
            } else {
                theme::with_alpha(tinted, (alpha + 0.15).clamp(0.35, 0.65))
            }
        } else {
            let tinted = theme::lerp_color(p_col, Color32::from_rgb(0x18, 0x18, 0x1a), 0.22);
            if alpha >= 1.0 {
                tinted
            } else {
                theme::with_alpha(tinted, (alpha + 0.12).clamp(0.32, 0.60))
            }
        }
    }

    /// Background fill for the side lyrics panel and queue drawer, matching the ambient
    /// page tint with a subtle glassy lift.
    pub fn lyrics_panel_fill(&self, ctx: &egui::Context) -> Color32 {
        if theme::light() {
            theme::with_alpha(theme::lerp_color(self.page_color(ctx), Color32::WHITE, 0.12), 0.42)
        } else {
            theme::with_alpha(theme::lerp_color(self.page_color(ctx), Color32::WHITE, 0.035), 0.72)
        }
    }

    pub fn bg_is_light(&self) -> bool {
        use crate::preset::{BgMode, Layout};
        let luma = |c: Color32| (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0;
        let look = self.look();
        let b = &look.background;
        match b.mode {
            BgMode::Kawarp | BgMode::CoverStatic | BgMode::CoverFullbleed if self.bg.is_some() => {
                let key = self.art_key.as_deref().or_else(|| self.current_id());
                let (all, right) = key
                    .and_then(|k| self.art_lumas.get(k).copied())
                    .unwrap_or_else(|| {
                        let dl = luma(self.dominant);
                        (dl, dl)
                    });
                let base = if b.mode == BgMode::CoverFullbleed && look.cover.layout == Layout::LyricsOnly {
                    right
                } else {
                    all
                };
                // Shader multiplies by u_dim (`np_dim`) and ~0.92 vignette across the lyrics area.
                base * b.np_dim * 0.92 > 0.52
            }
            BgMode::Solid => luma(look.solid()) > 0.52,
            _ => theme::light() && luma(self.dominant) > 0.52,
        }
    }

    /// Whether pages draw light: only the ytm-rs theme has a light mode, and it follows
    /// Settings › Appearance › Mode.
    fn light_pages(&self, ctx: &egui::Context) -> bool {
        self.look().id == crate::preset::DEFAULT_ID && self.appearance.is_light(ctx)
    }

    /// The active theme preset. Synced once per frame (`sync_look`), so this is a plain
    /// index, not a search.
    fn look(&self) -> &'static crate::preset::Preset {
        crate::preset::active()
    }

    fn sync_look(&self) {
        let id = self.theme_id.as_str();
        if crate::preset::active().id != id {
            crate::preset::set_active(id);
        }
    }

    /// Height of the bottom player area: the floating capsule's 52 points plus its floating margin.
    fn bar_height(&self) -> f32 {
        if self.page == Page::Settings {
            0.0
        } else if shell::docked(self) {
            shell::DOCKED_H
        } else {
            80.0
        }
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), -1.0));
    }


}



impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        dev_pace();
        theme::watch_system_appearance(ctx);
        theme::align_macos_traffic_lights();
        self.lyric_view.release_hidden(ctx.cumulative_pass_nr());
        let now = ctx.input(|i| i.time);

        // Mouse visibility in full-screen player: hides after 3.5 seconds of inactivity.
        let pos = ctx.input(|i| i.pointer.latest_pos());
        let mouse_moved = ctx.input(|i| {
            i.pointer.any_pressed() || i.pointer.any_down() || !i.events.is_empty()
        }) || (pos.is_some() && pos != self.mouse_last_pos);

        if mouse_moved {
            self.mouse_last_pos = pos;
            self.mouse_last_moved = now;
            if self.mouse_hidden {
                self.mouse_hidden = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
            }
        }

        let in_fullscreen_np = (self.fullscreen || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))) && self.np_open;
        if in_fullscreen_np {
            if now - self.mouse_last_moved >= 5.0 {
                if !self.mouse_hidden {
                    self.mouse_hidden = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
                }
                ctx.set_cursor_icon(egui::CursorIcon::None);
            } else {
                let remaining = 5.0 - (now - self.mouse_last_moved);
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining.max(0.05)));
            }
        } else if self.mouse_hidden {
            self.mouse_hidden = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        }
        self.sync_look();
        self.pump(now);
        self.skip_segments(ctx, now);
        // System media controls and taskbar buttons follow the player however it changed.
        let playing = self.playing() || self.buffering;
        if let Some(m) = &mut self.media {
            m.set_playing(playing);
        }
        self.request_lyric_extras();
        transfer::poll(self, ctx, now);
        if crate::deeplink::take_show() {
            self.show_window(ctx);
        }
        for code in crate::deeplink::take() {
            self.show_window(ctx);
            jam::open_link(self, code);
        }
        for a in std::mem::take(&mut self.actions) {
            self.apply(a);
        }
        jam::tick(self);
        jam::dev_script(self, ctx, now);
        if self.page_changed < 0.0 {
            self.page_changed = now;
        }
        // Dev hook: YTM_NP_TOGGLE_AT=<secs>[,<secs>…] clicks the Lyrics chip at those times.
        if let Some(v) = dev_var("YTM_NP_TOGGLE_AT") {
            let times: Vec<f64> = v.split(',').filter_map(|t| t.parse().ok()).collect();
            let id = egui::Id::unique("dev-np-toggles");
            let done: usize = ctx.data(|d| d.get_temp(id)).unwrap_or(0);
            if let Some(&t) = times.get(done) {
                if now >= t {
                    ctx.data_mut(|d| d.insert_temp(id, done + 1));
                    self.np_tab = if self.np_tab == NpTab::Lyrics { NpTab::Art } else { NpTab::Lyrics };
                } else {
                    ctx.request_repaint_after(Duration::from_secs_f64(t - now));
                }
            }
        }
        fit_window_to_monitor(ctx);
        // Dev hook: YTM_MINI=<secs>[,<secs>…] switches between the mini player and the
        // full window at those times (YTM_MINI=1: the mini player after a second).
        if let Some(v) = dev_var("YTM_MINI") {
            let times: Vec<f64> = v.split(',').filter_map(|t| t.parse().ok()).collect();
            let id = egui::Id::unique("dev-mini");
            let done: usize = ctx.data(|d| d.get_temp(id)).unwrap_or(0);
            if let Some(&t) = times.get(done) {
                if now >= t {
                    ctx.data_mut(|d| d.insert_temp(id, done + 1));
                    self.mini.request(done.is_multiple_of(2));
                } else {
                    ctx.request_repaint_after(Duration::from_secs_f64((t - now).min(0.2)));
                }
            }
        }
        if let Some(on) = self.mini.take_request() {
            self.set_mini(ctx, on, now);
        }
        self.mini.tick(ctx, now);
        self.tray_window(ctx);
        self.window_hidden = ctx.input(|i| i.viewport().minimized == Some(true) || i.viewport().visible() == Some(false));
        self.sync_discord();
        self.sync_lastfm();
        self.sync_dock(ctx);
        self.sync_menubar(ctx);
        self.sync_notify(ctx);
        self.sync_server_queue(ctx, now);
        self.tick_sleep(ctx);
        self.tick_updates(ctx, now);
        let np = widgets::NowPlaying {
            song: self.current_id().map(str::to_owned),
            source: self.queue_source.clone(),
            playing: self.playing() || self.buffering,
        };
        ctx.data_mut(|d| d.insert_temp(widgets::NowPlaying::id(), np));
        if let Some((_, t)) = &mut self.toast {
            if *t < 0.0 {
                *t = now;
            } else if now - *t > 4.5 {
                self.toast = None;
            }
        }
        // Frame pacing: idle costs nothing. The background drifts slowly, so ~15 fps
        // reads as smooth on pages; the now-playing view runs at 30 fps (25 on a page
        // showing the playing row's bars), and the synced
        // lyrics ask for every frame while they animate (the karaoke sweep must be
        // smooth). Each egui frame re-lays-out the UI, so frame rate is the main CPU cost.
        let playing = self.playing();
        let (covered, focused) = ctx.input(|i| (i.viewport().occluded == Some(true), i.viewport().focused != Some(false)));
        let is_covered = self.window_hidden || covered;
        if is_covered && !self.was_covered {
            self.was_covered = true;
            crate::sync::trim_memory();
        } else if !is_covered && self.was_covered {
            self.was_covered = false;
        }
        if is_covered {
            // In the tray, minimized or covered: nothing to draw, only the player
            // bookkeeping. Track changes and media keys wake the loop themselves.
            if playing || self.buffering {
                // Coming back from minimized or from under another window wakes nothing
                // by itself (eframe only notes it), so the first frame is this tick:
                // short, or the window sits on its old frame for up to a second.
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        } else if self.np_open && playing && !self.mini.busy() {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else if playing && widgets::equalizer_shown(ctx) {
            // The playing row's bars need a steady, full rate to read as smooth; 30 fps
            // behind other windows.
            ctx.request_repaint_after(Duration::from_millis(if focused { 16 } else { 33 }));
        } else if playing && (shell::pages_bg_moves(self.look()) || (self.look().id == crate::preset::DEFAULT_ID && matches!(self.page, Page::Album(_) | Page::Artist(_)))) {
            ctx.request_repaint_after(Duration::from_millis(66));
        } else if playing || self.buffering {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);
        let now = ctx.input(|i| i.time);
        let screen = if ctx.viewport_rect().is_positive() { ctx.viewport_rect() } else { ctx.content_rect() };

        self.sync_look();
        let look = self.look();
        // Now playing is dark; once it's most of the way up, so is the player bar under it.
        let np = theme::anim_bool(&ctx, "np-open", self.np_open && self.current().is_some(), 0.42);
        let is_fs = self.fullscreen || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let np_fullscreen = is_fs && self.np_open;
        let light = self.light_pages(&ctx) && !np_fullscreen;
        theme::set_mode(&ctx, light);
        if self.chrome_applied.as_deref() != Some(look.id.as_str()) {
            if self.chrome_applied.is_none() {
                let textures = Arc::new(crate::images::Textures::default());
                crate::images::register_textures(textures.clone());
                ctx.add_texture_loader(textures);
            }
            theme::apply_chrome(&ctx, look);
            self.chrome_applied = Some(look.id.clone());
        }
        let accent = self.accent_color();
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("accent"), accent));
        let app_style = &look.app;
        // Once now playing has slid all the way up it hides everything but the player
        // bar, so the pages and the full-screen background under it aren't drawn.
        let covered = !np_fullscreen && np >= 1.0;
        let bar_strip = egui::Rect::from_min_max(egui::pos2(screen.left(), screen.bottom() - self.bar_height()), screen.max);
        titlebar::set_fullscreen(&ctx, is_fs);
        // The mini player, or nothing but the background while the window changes size.
        let layout = self.mini.layout(&ctx);
        // Drawn again below, in the full layout.
        self.scroll_panes.clear();
        // The strip's window is cut to the same corner as the artwork it frames; the
        // full window keeps its corners.
        crate::fullscreen::set_rounded((layout == mini::Layout::Mini).then(|| (theme::thumb_radius() + 1.0) * ctx.pixels_per_point()));
        if layout != mini::Layout::Full {
            shell::background(self, ui, screen, now, None);
            if layout == mini::Layout::Mini {
                mini::strip(self, ui, now);
                titlebar::resize_sides(&ctx);
                // "Sign in to like songs" and such: shown here too, not timed out unseen.
                shell::toast(self, &ctx, now);
                self.mini.fade_in(&ctx, now, shell::base_color(self));
            }
            dev_screenshot(&ctx, now);
            // A click takes effect in the next frame's `logic`: have that frame now, not at
            // the idle pacing's next tick.
            if !self.actions.is_empty() {
                ctx.request_repaint();
            }
            return;
        }
        shell::background(self, ui, screen, now, (covered || np_fullscreen).then_some(bar_strip));
        // Set again by the sidebar's search box when it's drawn this frame.
        self.search_rect = None;
        if !is_fs && !covered {
            // The top of the window, down to the search box's row: what's on it comes after.
            titlebar::drag_area(ui, egui::Rect::from_min_size(screen.min, egui::vec2(screen.width(), titlebar::HEIGHT)), "main");
            // And the sidebar under it, down the window: its empty parts move the window
            // too (a double-click there does nothing).
            let side_w = shell::SIDEBAR_W;
            let side = egui::Rect::from_min_max(egui::pos2(screen.left(), screen.top() + titlebar::HEIGHT), egui::pos2(screen.left() + side_w, screen.bottom()));
            titlebar::drag_strip(ui, side, "sidebar");
        }
        if !np_fullscreen && !covered {
            // Paint the smooth ambient artwork gradient across the window (Home in all themes; Album/Artist in default look).
            if look.id == crate::preset::DEFAULT_ID || matches!(self.page, Page::Home) {
                self.paint_page_backdrop(&ctx, ui.painter(), screen, now);
            }
            use crate::preset::SidebarStyle;
            let side_alpha = app_style.sidebar_alpha.clamp(0.0, 1.0);
            // The logo's row is 32 points tall.
            // The macOS window buttons sit over the sidebar's top-left: start below them.
            let side_margin = egui::Margin { left: 14, right: 14, top: titlebar::top_margin(16, 32.0).max(titlebar::MAC_BUTTONS_CLEAR), bottom: 16 };
            let side_fill = self.sidebar_fill(&ctx, side_alpha);
            let glass_sidebar = matches!(app_style.sidebar, SidebarStyle::Glass | SidebarStyle::Floating);
            let side_frame = match app_style.sidebar {
                SidebarStyle::Glass => egui::Frame::NONE,
                SidebarStyle::Transparent => egui::Frame::new().inner_margin(side_margin),
                SidebarStyle::Solid => egui::Frame::new().fill(side_fill).inner_margin(side_margin),
                SidebarStyle::Floating => egui::Frame::NONE.outer_margin(egui::Margin { left: 10, right: 4, top: 10, bottom: 10 }),
            };
            let inner_side_margin = match app_style.sidebar {
                SidebarStyle::Floating => egui::Margin::symmetric(12, 14),
                _ => side_margin,
            };
            // The sidebar first, so it runs the window's full height: the player floats
            // over the main section only, and the page content scrolls under it.
            let side = egui::Panel::left("nav").resizable(false)
                .exact_size(shell::SIDEBAR_W)
                .frame(side_frame)
                .show_separator_line(false)
                .show(ui, |ui| {
                    if glass_sidebar {
                        let corner = if app_style.sidebar == SidebarStyle::Floating {
                            crate::glass::Corner::Radius(theme::panel_radius().max(14) as f32)
                        } else {
                            crate::glass::Corner::Circular(0.0)
                        };
                        let tint = theme::with_alpha(side_fill, if light { 0.24 } else { 0.28 });
                        let mut style = crate::glass::Style::thick(corner, Some(tint), if light { 0.08 } else { 0.90 }, 1.0);
                        // Render frosted sidebar glass without a white specular border or divider line.
                        style.material.rim = 0.0;
                        style.material.rim_base = 0.0;
                        style.material.inner_glow = 0.0;
                        style.material.light_spill = 0.0;
                        crate::glass::paint(ui, egui::Id::unique("sidebar-liquid-glass"), ui.max_rect(), style);
                        egui::Frame::NONE.inner_margin(inner_side_margin).show(ui, |ui| shell::sidebar(self, ui));
                    } else {
                        shell::sidebar(self, ui);
                    }
                });
            self.scroll_panes.push(side.response.rect);
            let section = ui.available_rect_before_wrap();
            const DRAWER_W: f32 = 340.0;
            let q = crate::glass::spring_bool(&ctx, egui::Id::unique("queue-drawer"), self.queue_open && !self.np_open, crate::glass::Tuning::SMOOTH).clamp(0.0, 1.0);
            let l = crate::glass::spring_bool(&ctx, egui::Id::unique("lyrics-drawer"), self.lyrics_drawer && !self.queue_open && !self.np_open, crate::glass::Tuning::SMOOTH).clamp(0.0, 1.0);
            let active_drawer = q.max(l);
            let drawer_w = DRAWER_W.min((screen.width() - shell::SIDEBAR_W - 80.0).max(240.0));
            let drawer_left = screen.right() - drawer_w * active_drawer;
            ctx.data_mut(|d| d.insert_temp(egui::Id::unique("drawer-left"), drawer_left));
            let is_artist = matches!(self.page, Page::Artist(_));
            let left_margin = if is_artist { 0 } else { 28 };
            let top_margin = if is_artist { 0 } else { titlebar::top_margin(14, 42.0) };
            egui::CentralPanel::default()
                // No right margin: the page's scroll bar sits at the very edge; the page
                // keeps the same gap inside (see `pages::page`).
                .frame(egui::Frame::new().inner_margin(egui::Margin { left: left_margin, right: 0, top: top_margin, bottom: 0 }))
                .show(ui, |ui| {
                    if !is_artist {
                        ui.add_space(2.0);
                    }
                    pages::page(self, ui, now);
                });
            // The player: a frosted strip over the bottom of the main section, staying left
            // of any open right-side overlay drawer.
            if self.page != Page::Settings && np < 0.99 {
                let bar_reveal = (1.0 - np).clamp(0.0, 1.0);
                let y_offset = np * 70.0;
                let strip = egui::Rect::from_min_max(
                    egui::pos2(section.left(), screen.bottom() - self.bar_height() + y_offset),
                    egui::pos2(drawer_left, screen.bottom() + y_offset),
                );
                egui::Area::new(egui::Id::unique("player-bar"))
                    .fixed_pos(strip.min)
                    .order(egui::Order::Middle)
                    .show(&ctx, |ui| {
                        ui.set_opacity(bar_reveal * bar_reveal);
                        ui.scope_builder(egui::UiBuilder::new().max_rect(strip).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                            shell::player_bar(self, ui, now, true);
                        });
                    });
            }
            // Apple Music-style full-height frosted overlay drawers sliding over the right
            // side of the page without reflowing page content.
            for (is_queue, t) in [(true, q), (false, l)] {
                if t <= 0.01 {
                    continue;
                }
                let drawer_fill = self.lyrics_panel_fill(&ctx);
                let slide_x = screen.right() - drawer_w * t;
                let full_rect = egui::Rect::from_min_size(
                    egui::pos2(slide_x, screen.top()),
                    egui::vec2(drawer_w, screen.height()),
                );
                let visible_rect = full_rect.intersect(screen);
                let area_id = if is_queue { "queue" } else { "lyrics-drawer" };
                let glass_id = if is_queue { "queue-liquid-glass" } else { "lyrics-drawer-liquid-glass" };
                let top_pad = if is_queue { titlebar::top_margin(14, 32.0) } else { titlebar::top_margin(18, 32.0) };
                let margin = if is_queue {
                    egui::Margin { left: 16, right: 16, top: top_pad, bottom: 16 }
                } else {
                    egui::Margin { left: 22, right: 20, top: top_pad, bottom: 18 }
                };
                egui::Area::new(egui::Id::unique(area_id))
                    .fixed_pos(full_rect.min)
                    .constrain(false)
                    .order(egui::Order::Middle)
                    .show(&ctx, |ui| {
                        ui.set_clip_rect(visible_rect);
                        ui.set_opacity(t);
                        ui.allocate_rect(visible_rect, egui::Sense::click());
                        let tint = theme::with_alpha(drawer_fill, if light { 0.24 } else { 0.36 });
                        let mut style = crate::glass::Style::thick(crate::glass::Corner::Circular(0.0), Some(tint), if light { 0.08 } else { 0.90 }, 1.0);
                        style.material.rim = 0.0;
                        style.material.rim_base = 0.0;
                        style.material.inner_glow = 0.0;
                        style.material.light_spill = 0.0;
                        crate::glass::paint(ui, egui::Id::unique(glass_id), full_rect, style);
                        ui.painter().vline(full_rect.left(), full_rect.y_range(), egui::Stroke::new(1.0, theme::glass(if light { 0.06 } else { 0.08 })));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(full_rect).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                            egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                                if is_queue {
                                    shell::queue_drawer(self, ui);
                                } else {
                                    nowplaying::lyrics_drawer(self, ui, now);
                                }
                            });
                        });
                    });
                self.scroll_panes.push(visible_rect);
            }
        }
        if np > 0.001 {
            let pages = theme::set_mode(&ctx, false);
            nowplaying::overlay(self, &ctx, now, screen);
            theme::set_mode(&ctx, pages);
        }
        self.draw_dialog(&ctx);
        notes::card(self, &ctx);
        notes::dialog(self, &ctx);
        if let Some(url) = self.share_pending.take() { ctx.copy_text(url); }
        if let Some((url, rect)) = self.share_sheet_pending.take() {
            let z = ctx.zoom_factor();
            let scaled = egui::Rect::from_min_max(rect.min * z, rect.max * z);
            if !theme::show_share_sheet(&url, scaled) {
                ctx.copy_text(url);
                self.toast("Link copied");
            }
        }
        shell::suggestions(self, &ctx);
        shell::search_results(self, &ctx);
        shell::toast(self, &ctx, now);
        shell::zoom_badge(&ctx, now);
        // A click takes effect in the next frame's `logic`: have that frame now, not at
        // the idle pacing's next tick.
        if !self.actions.is_empty() {
            ctx.request_repaint();
        }

        // The player keeps the window buttons too: the top right stays the window's.
        titlebar::window_buttons(&ctx);
        titlebar::resize_edges(&ctx);
        // Back from the mini player.
        self.mini.fade_in(&ctx, now, shell::base_color(self));

        if dev_var("YTM_THEME_TOUR").is_some() {
            self.theme_tour(&ctx, now);
        } else {
            dev_screenshot(&ctx, now);
        }
    }

    /// Dev hook: `YTM_FAKE_POINTER=x,y` parks a synthetic pointer there, for testing
    /// hover behaviour without touching the real mouse.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Dev hook: YTM_FAKE_WHEEL="secs,notches" injects one wheel event, for testing.
        if let Some((at, n)) = dev_var("YTM_FAKE_WHEEL").and_then(|v| {
            let (a, n) = v.split_once(',')?;
            Some((a.parse::<f64>().ok()?, n.parse::<f32>().ok()?))
        }) {
            let fired = egui::Id::unique("dev-wheel-fired");
            let t = raw.time.unwrap_or(0.0);
            if t >= at && !ctx.data(|d| d.get_temp::<bool>(fired).unwrap_or(false)) {
                ctx.data_mut(|d| d.insert_temp(fired, true));
                raw.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, -n),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                    source: egui::MouseWheelSource::Wheel,
                });
            } else if t < at {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        // Each gesture scrolls one pane along one axis: the page, not the sidebar too.
        if !self.scroll_gesture.route(&mut raw.events, raw.time.unwrap_or(0.0), &self.scroll_panes) {
            self.wheel_pending = egui::Vec2::ZERO;
        }
        smooth_wheel(self, ctx, raw);
        if let Some((x, y)) = dev_var("YTM_FAKE_POINTER").and_then(|v| {
            let (x, y) = v.split_once(',')?;
            Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?))
        }) {
            raw.events.retain(|e| !matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerGone));
            raw.events.push(egui::Event::PointerMoved(egui::pos2(x, y)));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if std::env::var_os("YTM_MUTE").is_none() {
            storage.set_string("volume", self.audio.volume().to_string());
        }
        let repeat = match self.queue.repeat {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        };
        storage.set_string("repeat", repeat.into());
        let b = |v: bool| if v { "1" } else { "0" }.to_owned();
        storage.set_string("autoplay", b(self.autoplay));
        storage.set_string("spotify_mode", b(self.spotify_mode));
        storage.set_string("recent_searches", serde_json::to_string(&self.recent_searches).unwrap_or_default());
        storage.set_string("jam_name", self.jam.name.clone());
        let cache = self.logged_in.then(|| AccountCache {
            name: self.account_name.clone(),
            avatar: None,
            playlists: self.library.as_ref().map(|l| l.playlists.clone()).or_else(|| self.sidebar_cache.clone()).unwrap_or_default(),
        });
        storage.set_string("account_cache", cache.and_then(|c| serde_json::to_string(&c).ok()).unwrap_or_default());
        storage.set_string("normalize", b(self.normalize));
        storage.set_string("log_volume", b(self.log_volume));
        storage.set_string("high_quality", b(self.high_quality));
        storage.set_string("play_songs", b(self.play_songs));
        let folder = crate::downloads::folder();
        storage.set_string("download_dir", if folder == crate::downloads::default_folder() { String::new() } else { folder.display().to_string() });
        let (device, device_name) = self.output_device.clone().unzip();
        storage.set_string("output_device", device.unwrap_or_default());
        storage.set_string("output_device_name", device_name.unwrap_or_default());
        // eframe has just saved the window's geometry. Minimized or in the tray that's
        // 0×0 at -32000, which would reopen as a speck: keep the last real one instead.
        if self.window_hidden {
            if let Some(w) = &self.good_window {
                storage.set_string("window", w.clone());
            }
        } else {
            self.good_window = storage.get_string("window");
        }
        // In the mini player, the full window's place is the one to reopen at.
        self.mini.save(storage);
        if let Some(w) = self.mini.full_window() {
            storage.set_string("window", w.clone());
            self.good_window = Some(w);
        }
        storage.set_string("notify_songs", b(self.notify_songs));
        storage.set_string("lyrics_drawer", b(self.lyrics_drawer));
        // The sidebar's playlist order, completed with any playlist that appeared since
        // it was last changed (so a drag can place them all).
        if let Some(l) = &self.library {
            for c in &l.playlists {
                if let Target::Playlist(id) = &c.target
                    && !self.playlist_order.contains(id)
                {
                    self.playlist_order.push(id.clone());
                }
            }
        }
        storage.set_string("playlist_order", self.playlist_order.join("\n"));
        storage.set_string("hidden_playlists", self.hidden_playlists.iter().cloned().collect::<Vec<_>>().join("\n"));
        storage.set_string("pinned_items", serde_json::to_string(&self.pinned).unwrap_or_default());
        storage.set_string("favourite_collections", self.favourite_collections.iter().cloned().collect::<Vec<_>>().join("\n"));
        storage.set_string("suggest_less", self.suggest_less.iter().cloned().collect::<Vec<_>>().join("\n"));
        // Pretend versions (dev hook) mustn't decide the real "what's new".
        if std::env::var_os("YTM_UPDATE_AS").is_none() {
            storage.set_string("last_version", crate::update::current_version());
        }
        storage.set_string("discord_enabled", b(self.discord_enabled));
        if dev_var("YTM_APPEARANCE").is_none() {
            storage.set_string("appearance", self.appearance.key().to_owned());
        }
        if std::env::var_os("YTM_THEME").is_none() {
            storage.set_string("theme", self.theme_id.clone());
            storage.set_string("colour", crate::preset::colour().id.to_owned());
        }
        if dev_var("YTM_FONT").is_none() {
            storage.set_string("font", theme::font_choice());
        }
        storage.set_string("reactive_bg", b(self.reactive_bg));
        storage.set_string("home_gradient", b(self.home_gradient));
        storage.set_string("signature_api", self.signature_api.clone());
        storage.set_string("hires_source", self.hires_sources.join("\n"));
        storage.set_string("prefer_atmos", b(self.prefer_atmos));
        storage.set_string("sponsorblock", b(self.sponsorblock));
        storage.set_string("crossfade", b(self.crossfade));
        storage.set_string("automix", b(self.automix));
        storage.set_string("crossfade_secs", self.crossfade_secs.to_string());
        storage.set_string("show_roman", b(self.show_roman));
        storage.set_string("show_trans", b(self.show_trans));
        // Background test runs mustn't change the listener's sources or per-song picks.
        if std::env::var_os("YTM_BACKGROUND").is_none() || std::env::var_os("YTM_SAVE_SESSION").is_some() {
            self.lyric_prefs.save(storage);
        }
        storage.set_string("np_art", b(self.np_tab == NpTab::Art));
        // Background test runs leave the user's real "last song" alone unless asked.
        let test_run = std::env::var_os("YTM_BACKGROUND").is_some() && std::env::var_os("YTM_SAVE_SESSION").is_none();
        if let Some(s) = self.session_json().filter(|_| !test_run) {
            storage.set_string(SESSION_KEY, s);
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.audio.send(crate::audio::AudioCmd::Stop);
        crate::dock::stop_activity();
        if let (Some(bg), Some(gl)) = (&self.bg, gl) {
            bg.destroy(gl);
        }
        // A downloaded update installs only when the user restarted for it; nothing is
        // installed behind their back as the app closes.
        let update_ready = self.updater.as_ref().is_some_and(|u| matches!(u.state(), crate::update::State::Ready { .. }));
        if let Some(u) = &self.updater
            && self.restart_for_update
            && update_ready
        {
            u.install(true);
        } else if self.restart_for_update || self.restart_app {
            crate::update::relaunch_current();
        }
    }
}

const SESSION_KEY: &str = "session";

/// What the sidebar and the account button show, kept for the next launch.
#[derive(serde::Serialize, serde::Deserialize)]
struct AccountCache {
    name: Option<String>,
    avatar: Option<String>,
    playlists: Vec<Card>,
}

/// What's saved between launches so the player picks up where it left off.
#[derive(serde::Serialize, serde::Deserialize)]
struct Session {
    queue: Vec<Track>,
    index: usize,
    position_ms: u64,
    duration_ms: u64,
    #[serde(default)]
    listened_at: u64,
    /// The song version that was playing for the current music video: the position is its.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    playing_as: Option<Track>,
}

/// The song version playing in place of the music video `listed`, in play `generation`.
struct PlayingAs {
    generation: u64,
    listed: String,
    song: Track,
}


/// A restored window size can exceed the monitor (e.g. saved after maximizing),
/// leaving the player bar partly off-screen. Once at startup, maximize instead.
fn fit_window_to_monitor(ctx: &egui::Context) {
    let done = egui::Id::unique("fit-window-checked");
    if ctx.data(|d| d.get_temp::<bool>(done).unwrap_or(false)) {
        return;
    }
    if std::env::var_os("YTM_BACKGROUND").is_some() {
        // eframe re-applies the saved (often maximized) window state regardless of the
        // builder, so push the test window off-screen here, before it's ever shown.
        ctx.data_mut(|d| d.insert_temp(done, true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(-4000.0, 0.0)));
        // YTM_SIZE=WxH tests other window sizes (e.g. the narrow layout).
        let size = std::env::var("YTM_SIZE").ok().and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some(egui::vec2(w.parse().ok()?, h.parse().ok()?))
        });
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.unwrap_or(egui::vec2(1920.0, 1057.0))));
        return;
    }
    let (outer, monitor, maximized) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().monitor_size, i.viewport().maximized));
    let (Some(outer), Some(monitor)) = (outer, monitor) else { return };
    ctx.data_mut(|d| d.insert_temp(done, true));
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, monitor);
    // A saved size below the minimum (closed while minimized saves a tiny one, which
    // eframe restores as-is) would open a window too small to see: start over centred.
    let min = egui::Vec2::from(crate::MIN_SIZE);
    if maximized != Some(true) && (outer.width() < min.x || outer.height() < min.y) {
        let size = egui::Vec2::from(crate::DEFAULT_SIZE).min(monitor * 0.9).max(min);
        log::info!("window {outer:?} is below the minimum size; resetting to {size:?}");
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(((monitor - size) / 2.0).max(egui::Vec2::ZERO).to_pos2()));
        return;
    }
    // A little slack for the invisible resize borders Windows adds around the frame.
    if maximized != Some(true) && !screen.expand(12.0).contains_rect(outer) {
        log::info!("window {outer:?} exceeds monitor {monitor:?}; maximizing");
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
    }
}

/// Smooth scrolling: mouse-wheel notches (line units) are collected and replayed as
/// small pixel steps with an exponential ease-out (~0.3 s glide), like browsers do.
/// Touchpads already report smooth pixel deltas and pass through untouched, as does
/// Ctrl+wheel zoom.
fn smooth_wheel(app: &mut App, ctx: &egui::Context, raw: &mut egui::RawInput) {
    const NOTCH_PX: f32 = 70.0;
    const GLIDE: f32 = 0.06; // time constant, seconds (settles in ~0.18 s)
    let viewport_h = raw.screen_rect.map_or(800.0, |r| r.height());
    raw.events.retain(|e| match e {
        egui::Event::MouseWheel { unit, delta, modifiers, .. } if !modifiers.command && !modifiers.ctrl => {
            let px = match unit {
                egui::MouseWheelUnit::Line => *delta * NOTCH_PX,
                egui::MouseWheelUnit::Page => *delta * viewport_h * 0.8,
                egui::MouseWheelUnit::Point => return true,
            };
            // Reversing direction cancels the remaining glide instead of fighting it.
            if app.wheel_pending.y * px.y < 0.0 || app.wheel_pending.x * px.x < 0.0 {
                app.wheel_pending = egui::Vec2::ZERO;
            }
            app.wheel_pending += px;
            app.wheel_mods = *modifiers;
            false
        }
        _ => true,
    });
    if app.wheel_pending.length() < 0.5 {
        app.wheel_pending = egui::Vec2::ZERO;
        return;
    }
    let dt = raw.predicted_dt.clamp(0.001, 0.05);
    let step = app.wheel_pending * (1.0 - (-dt / GLIDE).exp());
    app.wheel_pending -= step;
    raw.events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: step,
        phase: egui::TouchPhase::Move,
        modifiers: app.wheel_mods,
        source: egui::MouseWheelSource::Wheel,
    });
    ctx.request_repaint();
}

/// Ask for the next frame of an animation: every frame while the window has focus,
/// ~30 fps behind other windows. Hidden or covered, eframe runs no ui pass, so the
/// pacing in `logic` decides alone.
pub fn animate(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().focused == Some(false)) {
        ctx.request_repaint_after(Duration::from_millis(33));
    } else {
        ctx.request_repaint();
    }
}

/// Fisher-Yates with a clock-seeded xorshift; shuffling doesn't need a real RNG crate.
pub fn shuffle<T>(v: &mut [T]) {
    let mut x = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x9e37_79b9_7f4a_7c15, |d| d.as_nanos() as u64)
        | 1;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

/// Whether this is the installed or the portable app (updates only apply there). It
/// can't change while running, and the check touches the disk, so it runs once.
fn updatable() -> bool {
    static UPDATABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *UPDATABLE.get_or_init(|| crate::update::installed() || crate::update::portable_dir().is_some())
}

pub fn fmt_time(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn sessions_saved_before_song_versions_still_load() {
        let old = r#"{"queue":[{"id":"a","title":"A","artists":[],"album":null,"duration":null,"thumbs":[],"track_nr":null,"plays":null}],"index":0,"position_ms":5000,"duration_ms":0}"#;
        let s: Session = serde_json::from_str(old).unwrap();
        assert!(!s.queue[0].music_video && s.playing_as.is_none());
        // A song (or a session without a song version) saves as before, so an older
        // version, or a jam guest on one, reads it too.
        let saved = serde_json::to_string(&s).unwrap();
        assert!(!saved.contains("music_video") && !saved.contains("playing_as"), "{saved}");
        let video = Track { music_video: true, ..s.queue[0].clone() };
        let back: Track = serde_json::from_str(&serde_json::to_string(&video).unwrap()).unwrap();
        assert!(back.music_video);
    }
}
