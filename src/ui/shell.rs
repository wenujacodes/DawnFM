//! App chrome: background, sidebar, top bar, player bar, queue drawer, toasts.

use egui::{Align2, Color32, Pos2, Rect, Sense, Ui, Vec2, pos2, vec2};

use super::widgets::{self, icon_at, named, paint_cover, text_at};
use super::{Action, App, LibTab, NpTab, Page, Repeat, fmt_time, glyphs};
use crate::i18n::t;
use crate::model::Target;
use crate::shader::Params;
use crate::theme::{self, icon};

// ------------------------------------------------------------------ background

/// Vertical gradient (top colour → bottom colour) over `rect`.
pub fn vgradient(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    gradient(painter, rect, [top, top, bottom, bottom]);
}

/// Vertical gradient over a rounded rectangle, fading smoothly with ease-in.
#[allow(dead_code)]
pub fn rounded_vgradient(painter: &egui::Painter, rect: Rect, radius: f32, top: Color32, bottom: Color32) {
    if radius <= 0.0 {
        vgradient(painter, rect, top, bottom);
        return;
    }
    let r = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let mut mesh = egui::Mesh::default();
    let center = rect.center();
    let c_mid = {
        let t = 0.5_f32;
        theme::lerp_color(top, bottom, t * t)
    };
    mesh.colored_vertex(center, c_mid);

    let mut add_arc = |cx: f32, cy: f32, start_a: f32, end_a: f32| {
        const STEPS: usize = 6;
        for i in 0..=STEPS {
            let a = start_a + (end_a - start_a) * (i as f32 / STEPS as f32);
            let (s, c) = a.sin_cos();
            let p = pos2(cx + c * r, cy + s * r);
            let t = ((p.y - (rect.top() + rect.height() * 0.15)) / (rect.height() * 0.85)).clamp(0.0, 1.0);
            mesh.colored_vertex(p, theme::lerp_color(top, bottom, t * t));
        }
    };

    use std::f32::consts::PI;
    add_arc(rect.right() - r, rect.top() + r, -PI * 0.5, 0.0);
    add_arc(rect.right() - r, rect.bottom() - r, 0.0, PI * 0.5);
    add_arc(rect.left() + r, rect.bottom() - r, PI * 0.5, PI);
    add_arc(rect.left() + r, rect.top() + r, PI, PI * 1.5);

    let n = mesh.vertices.len() as u32;
    for i in 1..n {
        let next = if i + 1 < n { i + 1 } else { 1 };
        mesh.add_triangle(0, i, next);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Horizontal gradient (left colour → right colour) over `rect`.
#[allow(dead_code)]
pub fn hgradient(painter: &egui::Painter, rect: Rect, left: Color32, right: Color32) {
    gradient(painter, rect, [left, right, left, right]);
}

/// Corner colours: left top, right top, left bottom, right bottom.
fn gradient(painter: &egui::Painter, rect: Rect, c: [Color32; 4]) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), c[0]);
    mesh.colored_vertex(rect.right_top(), c[1]);
    mesh.colored_vertex(rect.left_bottom(), c[2]);
    mesh.colored_vertex(rect.right_bottom(), c[3]);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(egui::Shape::mesh(mesh));
}

/// The background mode browse pages use: the theme's own choice, else whatever now
/// playing uses.
fn page_mode(look: &crate::preset::Preset) -> crate::preset::BgMode {
    use crate::preset::{BgMode, PageBg};
    match look.app.page_bg {
        PageBg::Same => look.background.mode,
        PageBg::Static => BgMode::CoverStatic,
        PageBg::Accent => BgMode::AccentGradient,
        PageBg::Solid => BgMode::Solid,
    }
}

/// Whether the pages' background moves while music plays (static themes need no frames).
pub fn pages_bg_moves(look: &crate::preset::Preset) -> bool {
    use crate::preset::BgMode;
    let b = &look.background;
    matches!(page_mode(look), BgMode::Kawarp | BgMode::CoverFullbleed) && b.speed > 0.0 && b.intensity > 0.0
}

/// `only`: paint just this part (the rest is covered by now playing).
/// The background's flat colour, under its art and gradients.
pub fn base_color(app: &App) -> Color32 {
    app.page_target_color()
}

pub fn background(app: &mut App, ui: &mut Ui, screen: Rect, now: f64, only: Option<Rect>) {
    use crate::preset::BgMode;
    let dt = (now - app.last_frame).clamp(0.0, 0.1);
    app.last_frame = now;
    let look = app.look();
    let b = &look.background;
    let mode = page_mode(look);
    // Light pages are the art at full strength under a white wash, a pale tint of the cover.
    let light = theme::light();
    let dim = if light { 1.0 } else { look.app.page_dim.unwrap_or(b.page_dim).clamp(0.02, 1.0) };
    // Now playing animates the shader itself, so keep the clock running for it too.
    let animated = matches!(b.mode, BgMode::Kawarp | BgMode::CoverFullbleed) && b.speed > 0.0;
    let moving = animated && app.playing();
    if moving {
        app.bg_time += dt * b.speed as f64;
    }
    let painter = match only {
        Some(r) => ui.painter().with_clip_rect(r),
        None => ui.painter().clone(),
    };
    painter.rect_filled(screen, 0.0, app.page_color(ui.ctx()));
    let art = matches!(mode, BgMode::Kawarp | BgMode::CoverStatic | BgMode::CoverFullbleed) && !matches!(app.page, Page::Home);
    if let Some(bg) = app.bg.as_ref().filter(|_| art) {
        let blend = theme::ease_in_out(((now - app.art_changed) / 1.4) as f32);
        if blend < 1.0 {
            // The crossfade to new art finishes even while paused.
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
        let level = if app.reactive_bg && app.playing() { app.audio.status.level() * b.reactive } else { 0.0 };
        let still = mode == BgMode::CoverStatic;
        bg.paint(
            &painter,
            screen,
            Params {
                time: app.bg_time as f32,
                blend,
                intensity: if still { 0.0 } else { b.intensity * 0.9 },
                saturation: b.saturation * 0.9,
                dim,
                scale: (b.scale - 0.08).max(1.0) + if still { 0.0 } else { level.min(0.5) * 0.08 },
            },
        );
    } else if mode == BgMode::Solid || look.id == crate::preset::DEFAULT_ID {
        // Plain colour or custom ambient page colour: nothing over it.
    } else {
        painter.rect_filled(screen, 0.0, look.solid());
    }
    if light && (art || look.id != crate::preset::DEFAULT_ID) {
        painter.rect_filled(screen, 0.0, Color32::from_white_alpha(212));
    }
}

// ------------------------------------------------------------------ sidebar

/// How the theme draws nav items.
#[derive(Clone, Copy)]
struct NavLook {
    style: crate::preset::NavStyle,
    radius: f32,
    accent: Color32,
}

impl NavLook {
    fn of(app: &App) -> Self {
        Self { style: app.look().app.nav, radius: (app.look().chrome.radius * 0.6).clamp(4.0, 16.0), accent: app.accent_color() }
    }
}

/// A nav item's background in the theme's nav style; returns its icon colour.
/// `h` and `s` are the hover and selected animations (0..1).
fn nav_bg(ui: &Ui, look: NavLook, rect: Rect, h: f32, s: f32) -> Color32 {
    use crate::preset::NavStyle;
    let (nav_r, accent) = (look.radius, look.accent);
    let icon_fg = theme::lerp_color(theme::text_dim(), theme::text(), (h + s).min(1.0));
    match look.style {
        NavStyle::Pill => {
            ui.painter().rect_filled(rect, nav_r, theme::glass(0.05 * h + 0.10 * s));
            icon_fg
        }
        NavStyle::Accent => {
            ui.painter().rect_filled(rect, nav_r, theme::glass(0.05 * h + 0.08 * s));
            accent
        }
        NavStyle::Bar => {
            ui.painter().rect_filled(rect, nav_r, theme::glass(0.05 * h));
            let bar = Rect::from_center_size(pos2(rect.left() + 2.0, rect.center().y), vec2(3.0, 20.0 * s));
            ui.painter().rect_filled(bar, 2.0, accent);
            icon_fg
        }
    }
}

/// Sidebar width (points).
pub const SIDEBAR_W: f32 = 220.0;

/// A button on the sidebar's top line, at `rect`: a rounded rectangle under the
/// pointer, with `paint` drawing its mark at the centre in the given colour.
fn top_button(ui: &mut Ui, rect: Rect, key: &str, name: &str, paint: impl FnOnce(&Ui, Pos2, Color32)) -> bool {
    let resp = ui.interact(rect, ui.scope_id().with(("side-top", key)), Sense::click());
    let resp = named(resp, name).on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand);
    let press = crate::glass::press(ui, &resp, rect, 0.75);
    widgets::paint_glass_hover(ui, resp.id, &press, press.hover);
    paint(ui, press.rect.center(), theme::lerp_color(theme::text_dim(), theme::text(), press.hover));
    resp.clicked()
}

/// The chevron pair that walks the page history.
fn history_button(ui: &mut Ui, rect: Rect, glyph: char, name: &str) -> bool {
    top_button(ui, rect, name, name, |ui, at, c| icon_at(ui, at, glyph, 16.0, c))
}

/// Animates a collapsible sidebar section's height and opacity over `k` (`0.0..=1.0`).
fn animated_section(ui: &mut Ui, id_salt: &str, k: f32, add_contents: impl FnOnce(&mut Ui)) {
    if k <= 0.001 {
        return;
    }
    let h_key = egui::Id::unique(("sidebar-sec-h", id_salt));
    if k >= 0.999 {
        let start_y = ui.cursor().min.y;
        add_contents(ui);
        let full_h = (ui.cursor().min.y - start_y).max(0.0);
        ui.ctx().data_mut(|d| d.insert_temp(h_key, full_h));
        return;
    }
    let cached_h = ui.ctx().data(|d| d.get_temp::<f32>(h_key)).unwrap_or(150.0);
    let vis_h = (cached_h * k).max(0.0);
    let top = ui.cursor().min;
    let clip = Rect::from_min_size(top, vec2(ui.available_width(), vis_h));
    ui.allocate_exact_size(vec2(ui.available_width(), vis_h), Sense::hover());
    let content_rect = Rect::from_min_size(
        pos2(top.x, top.y - (cached_h - vis_h) * 0.35),
        vec2(ui.available_width(), 2000.0),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
    child.set_clip_rect(clip.intersect(ui.clip_rect()));
    child.set_opacity(k);
    add_contents(&mut child);
    let measured = (child.cursor().min.y - content_rect.top()).max(0.0);
    ui.ctx().data_mut(|d| d.insert_temp(h_key, measured));
}

fn is_liked_playlist_card(c: &crate::model::Card) -> bool {
    let Target::Playlist(id) = &c.target else { return false };
    id == super::LIKED_MUSIC
        || id == "LM"
        || id == "VLLM"
        || id == crate::spotify::SPOTIFY_LIKED_ID
        || c.thumbs.iter().any(|t| t.url.contains("/images/pbg/liked-songs"))
}

/// The sidebar: the search box, the app's pages and the playlists.
pub fn sidebar(app: &mut App, ui: &mut Ui) {
    const ICON_X: f32 = 24.0;
    // Content width: text keeps its truncation inside it.
    let full_w = ui.available_width();

    // The page history buttons, on the macOS window buttons' line (~22 pt centre) and right
    // after them. A floating sidebar starts below the window buttons.
    let top = ui.cursor().min;
    let screen = ui.ctx().content_rect();
    let below = pos2(top.x + ICON_X - 14.0, top.y + 3.0);
    let on_line = top.y - screen.top() >= 36.0;
    let at = if on_line { pos2(screen.left() + 86.0, screen.top() + 9.0) } else { below };
    let button = |i: usize| Rect::from_min_size(at + vec2([0.0, 30.0][i], 0.0), vec2(28.0, 26.0));
    if history_button(ui, button(0), icon::CHEVRON_LEFT, "Back") {
        app.nav_back();
    }
    if history_button(ui, button(1), icon::CHEVRON_RIGHT, "Forward") {
        app.nav_forward();
    }
    // The row's own room: all of it under the window buttons, none beside them.
    ui.add_space(if on_line { 4.0 } else { 46.0 });

    let mut go_to: Option<Page> = None;
    let look = NavLook::of(app);
    // Draws one row, with the measures of Music's sidebar (30 points, 13-point labels):
    // `paint` puts its icon at the given point in the given colour.
    let row = |ui: &mut Ui, paint: &dyn Fn(&Ui, Pos2, Color32), label: &str, page: Page, selected: bool, go_to: &mut Option<Page>| {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        let resp = named(resp, label).on_hover_cursor(egui::CursorIcon::PointingHand);
        let h = if resp.hovered() { 1.0 } else { 0.0 };
        let s = theme::anim_bool(ui.ctx(), resp.id.with("s"), selected, 0.2);
        let icon_fg = nav_bg(ui, look, rect, h, s);
        paint(ui, pos2(rect.left() + ICON_X, rect.center().y), icon_fg);
        let fg = theme::label();
        let font = if selected { theme::semibold(13.0) } else { theme::regular(13.0) };
        ui.painter().text(pos2(rect.left() + 44.0, rect.center().y), Align2::LEFT_CENTER, label, font, fg);
        if resp.clicked() {
            *go_to = Some(page);
        }
    };
    let icon_row = |ui: &mut Ui, glyph: char, label: &str, page: Page, selected: bool, go_to: &mut Option<Page>| {
        row(ui, &|ui: &Ui, at: Pos2, c: Color32| icon_at(ui, at, glyph, 15.0, c), label, page, selected, go_to);
    };
    // A section's heading; a click folds the section away (remembered across launches).
    let section = |ui: &mut Ui, title: &str| -> f32 {
        let key = egui::Id::unique(("sidebar-section", title));
        let mut open = ui.data_mut(|d| *d.get_persisted_mut_or_insert_with(key, || true));
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        let resp = named(resp, title).on_hover_cursor(egui::CursorIcon::PointingHand);
        let h = if resp.hovered() { 1.0 } else { 0.0 };
        let c = theme::lerp_color(theme::text_faint(), theme::text_dim(), h);
        ui.painter().text(pos2(r.left() + 8.0, r.center().y), Align2::LEFT_CENTER, title, theme::semibold(11.0), c);
        let k = theme::anim_bool(ui.ctx(), resp.id.with("open"), open, 0.22);
        let chev = pos2(r.right() - 12.0, r.center().y);
        if k < 0.99 {
            icon_at(ui, chev, icon::CHEVRON_RIGHT, 12.0, c.gamma_multiply(1.0 - k));
        }
        if k > 0.01 {
            icon_at(ui, chev, icon::CHEVRON_DOWN, 12.0, c.gamma_multiply(k));
        }
        if resp.clicked() {
            open = !open;
            ui.data_mut(|d| d.insert_persisted(key, open));
        }
        k
    };
    let nav = |ui: &mut Ui, glyph: &'static [u8], size: f32, label: &str, page: Page, selected: bool, go_to: &mut Option<Page>| {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        let resp = named(resp, label).on_hover_cursor(egui::CursorIcon::PointingHand);
        let h = if resp.hovered() { 1.0 } else { 0.0 };
        let s = theme::anim_bool(ui.ctx(), resp.id.with("s"), selected, 0.2);
        let icon_fg = nav_bg(ui, look, rect, h, s);
        glyphs::draw(ui, pos2(rect.left() + ICON_X, rect.center().y), size, glyph, icon_fg);
        let fg = theme::label();
        let font = if selected { theme::semibold(13.0) } else { theme::regular(13.0) };
        ui.painter().text(pos2(rect.left() + 44.0, rect.center().y), Align2::LEFT_CENTER, label, font, fg);
        if resp.clicked() {
            *go_to = Some(page);
        }
    };
    let p = app.page.clone();
    // A size down from the desktop app's, for the shorter rows.
    const APPLE: f32 = glyphs::APPLE_GLYPH - 3.0;
    const LUCIDE: f32 = glyphs::LUCIDE_GLYPH - 2.0;

    // Drag-to-reorder and right-click-to-hide results, applied once the rows are done.
    let mut reorder: Option<(String, String)> = None;
    let mut hide: Option<String> = None;
    let mut unpin: Option<crate::model::Card> = None;

    let transferring = super::transfer::sidebar_visible(app, ui.input(|i| i.time));
    let ready = app.updater.as_ref().and_then(|u| match u.state() {
        crate::update::State::Ready { version, .. } => Some(version),
        _ => None,
    });
    // Room under the list: Settings, the app's sign-off, and the transfer card.
    let bottom_h = (if transferring { 106.0 } else { 42.0 }) + if ready.is_some() { 58.0 } else { 0.0 };

    {
        icon_row(ui, icon::SEARCH, &t!("search"), Page::Search(String::new()), matches!(p, Page::Search(_)), &mut go_to);
        nav(ui, glyphs::HOME, APPLE, &t!("home"), Page::Home, p == Page::Home, &mut go_to);
        nav(ui, glyphs::EXPLORE, APPLE, &t!("new"), Page::New, p == Page::New, &mut go_to);
        ui.add_space(4.0);
        let lib_k = section(ui, &t!("library"));
        animated_section(ui, "library", lib_k, |ui| {
            let lib = |tab: LibTab| p == Page::Library(tab);
            nav(ui, glyphs::LIBRARY, APPLE, &t!("playlists"), Page::Library(LibTab::Playlists), lib(LibTab::Playlists), &mut go_to);
            icon_row(ui, icon::CIRCLE_USER, &t!("artists"), Page::Library(LibTab::Artists), lib(LibTab::Artists), &mut go_to);
            icon_row(ui, icon::DISC_3, &t!("albums"), Page::Library(LibTab::Albums), lib(LibTab::Albums), &mut go_to);
            icon_row(ui, icon::HEART, &t!("liked_songs"), Page::Library(LibTab::Songs), lib(LibTab::Songs), &mut go_to);
            if !app.spotify_mode {
                icon_row(ui, icon::HISTORY, &t!("recently_played"), Page::Library(LibTab::History), lib(LibTab::History), &mut go_to);
            }
            nav(ui, glyphs::DOWNLOAD, APPLE, &t!("downloads"), Page::Downloads, p == Page::Downloads, &mut go_to);
        });
        if !app.pinned.is_empty() {
            ui.add_space(4.0);
            let pin_k = section(ui, "Pinned");
            animated_section(ui, "pinned", pin_k, |ui| {
                for c in &app.pinned {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                    let resp = named(resp, &c.title).on_hover_cursor(egui::CursorIcon::PointingHand);
                    let h = if resp.hovered() { 1.0 } else { 0.0 };
                    let selected = matches!(
                        (&c.target, &p),
                        (Target::Album(a), Page::Album(b))
                            | (Target::Playlist(a), Page::Playlist(b))
                            | (Target::Artist(a), Page::Artist(b))
                            if a == b
                    );
                    ui.painter().rect_filled(rect, look.radius, theme::glass(0.05 * h + if selected { 0.08 } else { 0.0 }));
                    let img = Rect::from_center_size(pos2(rect.left() + ICON_X, rect.center().y), Vec2::splat(20.0));
                    paint_cover(ui, img, &c.thumbs, if c.round { 10.0 } else { theme::thumb_radius() }, 1.0);
                    let w = full_w - 54.0;
                    text_at(ui, pos2(rect.left() + 44.0, rect.center().y - 8.5), &c.title, theme::regular(13.0), theme::label(), w);
                    widgets::context_menu(&resp, |ui| {
                        ui.set_min_width(150.0);
                        if widgets::menu_action(ui, None, "Unpin") {
                            unpin = Some(c.clone());
                        }
                    });
                    if resp.clicked() {
                        app.actions.push(Action::Open(c.target.clone()));
                    }
                }
            });
        }
        ui.add_space(4.0);
        let playlists_k = section(ui, &t!("playlists"));

        // Playlists that appeared since the order was last changed join it at the end, so a
        // drag can place any of them.
        if let Some(l) = if app.spotify_mode { &app.spotify_library } else { &app.library } {
            for c in &l.playlists {
                if let Target::Playlist(id) = &c.target
                    && !app.playlist_order.contains(id)
                {
                    app.playlist_order.push(id.clone());
                }
            }
        }

        let pl_h = (ui.available_height() - bottom_h).max(0.0);
        ui.allocate_ui(vec2(ui.available_width(), pl_h), |ui| {
            if !app.active_logged_in() {
                if app.spotify_mode {
                    ui.label(egui::RichText::new("Sign in to Spotify to see your playlists.").color(theme::text_dim()));
                    ui.add_space(8.0);
                    if widgets::pill(ui, "Open Settings", None, false).clicked() {
                        go_to = Some(Page::Settings);
                    }
                } else {
                    ui.label(egui::RichText::new(t!("sign_in_to_see_playlists")).color(theme::text_dim()));
                    ui.add_space(8.0);
                    if widgets::pill(ui, &t!("sign_in"), None, false).clicked() {
                        app.start_sign_in(ui.ctx());
                    }
                }
                return;
            }
            if playlists_k <= 0.001 {
                return;
            }
            if playlists_k < 0.999 {
                let r = ui.available_rect_before_wrap();
                let clip_h = (pl_h * playlists_k).max(0.0);
                ui.set_clip_rect(Rect::from_min_size(r.min, vec2(r.width(), clip_h)).intersect(ui.clip_rect()));
                ui.set_opacity(playlists_k);
            }
            egui::ScrollArea::vertical()
                .id_salt("sidebar-pl")
                .auto_shrink([false, false])
                .max_height(pl_h)
                .show(ui, |ui| {
                // Last launch's list until the library arrives.
                let fallback_cache = if app.spotify_mode { None } else { app.sidebar_cache.as_ref() };
                let Some(playlists) = (if app.spotify_mode { app.spotify_library.as_ref() } else { app.library.as_ref() }).map(|l| &l.playlists).or(fallback_cache).cloned() else {
                    // The spinner animates every frame: give up on a request that never answered.
                    if app.library_requested.elapsed() < std::time::Duration::from_secs(20) {
                        ui.horizontal(|ui| {
                            ui.add_space(ICON_X - 8.0);
                            ui.spinner();
                        });
                    } else {
                        ui.label(egui::RichText::new(t!("could_not_load_playlists")).color(theme::text_dim()));
                        ui.add_space(8.0);
                        if widgets::pill(ui, &t!("try_again"), None, false).clicked() {
                            app.request_library();
                        }
                    }
                    return;
                };
                let mut visible: Vec<&crate::model::Card> = playlists
                    .iter()
                    .filter(|c| !is_liked_playlist_card(c) && !matches!(&c.target, Target::Playlist(id) if app.hidden_playlists.contains(id)))
                    .collect();
                visible.sort_by_key(|c| match &c.target {
                    Target::Playlist(id) => app.playlist_order.iter().position(|x| x == id).unwrap_or(usize::MAX),
                    _ => usize::MAX,
                });
                let drag_key = egui::Id::unique("sidebar-pl-drag");
                let dragging: Option<String> = ui.data(|d| d.get_temp(drag_key));
                let down = ui.input(|i| i.pointer.any_down());
                let pointer = ui.input(|i| i.pointer.interact_pos());
                for c in &visible {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click_and_drag());
                    let id = match &c.target {
                        Target::Playlist(id) => id.clone(),
                        _ => continue,
                    };
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let is_dragged = dragging.as_deref() == Some(id.as_str());
                    let resp = named(resp, &c.title).on_hover_cursor(if is_dragged { egui::CursorIcon::Grabbing } else { egui::CursorIcon::PointingHand });
                    if resp.drag_started() {
                        ui.data_mut(|d| d.insert_temp(drag_key, id.clone()));
                    }
                    if let Some(did) = &dragging
                        && did != &id
                        && down
                        && pointer.is_some_and(|p| rect.contains(p))
                    {
                        reorder = Some((did.clone(), id.clone()));
                    }
                    let h = if resp.hovered() { 1.0 } else { 0.0 };
                    let selected = matches!(&c.target, Target::Playlist(id) if app.page == Page::Playlist(id.clone()));
                    let lift = if is_dragged { 0.10 } else { 0.0 };
                    ui.painter().rect_filled(rect, look.radius, theme::glass(0.05 * h + if selected { 0.08 } else { 0.0 } + lift));
                    let img = Rect::from_center_size(pos2(rect.left() + ICON_X, rect.center().y), Vec2::splat(22.0));
                    paint_cover(ui, img, &c.thumbs, theme::thumb_radius(), 1.0);
                    let w = full_w - 54.0;
                    let fg = theme::label();
                    text_at(ui, pos2(rect.left() + 44.0, rect.center().y - 8.5), &c.title, theme::regular(13.0), fg, w);
                    widgets::context_menu(&resp, |ui| {
                        ui.set_min_width(170.0);
                        if widgets::menu_action(ui, None, &t!("hide_from_sidebar")) {
                            hide = Some(id.clone());
                        }
                    });
                    if resp.clicked() && !is_dragged {
                        app.actions.push(Action::Open(c.target.clone()));
                    }
                }
                if ui.input(|i| i.pointer.any_released()) {
                    ui.data_mut(|d| d.remove_temp::<String>(drag_key));
                }
            });
        });
    }
    if let Some(id) = hide {
        app.hidden_playlists.insert(id);
    }
    if let Some(card) = unpin {
        app.actions.push(Action::TogglePin(card));
    }
    if let Some((dragged, target)) = reorder
        && let (Some(from), Some(to)) = (app.playlist_order.iter().position(|x| x == &dragged), app.playlist_order.iter().position(|x| x == &target))
    {
        let id = app.playlist_order.remove(from);
        app.playlist_order.insert(to.min(app.playlist_order.len()), id);
    }
    let mut restart = false;
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        nav(ui, glyphs::SETTINGS, LUCIDE, &t!("settings"), Page::Settings, p == Page::Settings, &mut go_to);
        if transferring {
            super::transfer::sidebar_card(app, ui, ICON_X);
        }
        if let Some(v) = &ready {
            ui.add_space(6.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
            let resp = named(resp, &t!("restart_to_update")).on_hover_cursor(egui::CursorIcon::PointingHand);
            let h = if resp.hovered() { 1.0 } else { 0.0 };
            ui.painter().rect_filled(rect, 12.0, theme::with_alpha(look.accent, 0.22 + 0.12 * h));
            glyphs::draw(ui, pos2(rect.left() + ICON_X, rect.center().y), LUCIDE, glyphs::SPARKLES, theme::text());
            ui.painter().text(pos2(rect.left() + 46.0, rect.top() + 17.0), Align2::LEFT_CENTER, t!("restart_to_update"), theme::semibold(13.5), theme::text());
            ui.painter().text(pos2(rect.left() + 46.0, rect.top() + 35.0), Align2::LEFT_CENTER, format!("Dawn FM {v}"), theme::regular(12.0), theme::with_alpha(theme::text(), 0.75));
            restart = resp.clicked();
        }
    });
    if restart {
        app.restart_to_update(ui.ctx());
    }
    if let Some(page) = go_to {
        if matches!(page, Page::Search(_)) {
            app.search_refocus = true;
            if !matches!(app.page, Page::Search(_)) {
                app.actions.push(Action::Go(Page::Search(app.search_text.clone())));
            }
        } else {
            app.actions.push(Action::Go(page));
        }
    }
}

/// The search box rendered centered at the top of the Search page: live typeahead,
/// autocomplete suggestions, and Enter to search immediately.
pub fn search_field(app: &mut App, ui: &mut Ui, width: f32) {
    // Anywhere on it focuses the box, not just on the text area inside it.
    let (rect, field) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click());
    let field = field.on_hover_cursor(egui::CursorIcon::Text);
    let focus = theme::anim_bool(ui.ctx(), "search-focus", app.search_focused, 0.2);
    let radius = 18.0;
    if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
        crate::glass::paint(ui, egui::Id::unique("page-search-glass"), rect, crate::glass::control(crate::glass::Corner::Capsule, theme::light(), focus * 0.65, 1.0));
        if focus > 0.01 {
            let stroke_color = theme::with_alpha(app.accent_color(), focus);
            ui.painter().rect_stroke(rect, radius, egui::Stroke::new(1.5, stroke_color), egui::StrokeKind::Inside);
        }
    } else if theme::light() {
        ui.painter().rect_filled(rect, radius, theme::glass(0.08));
        if focus > 0.0 {
            let stroke_color = theme::with_alpha(app.accent_color(), focus);
            ui.painter().rect_stroke(rect, radius, egui::Stroke::new(1.5, stroke_color), egui::StrokeKind::Inside);
        }
    } else {
        ui.painter().rect_filled(rect, radius, Color32::from_rgba_unmultiplied(255, 255, 255, 18));
        if focus > 0.0 {
            let stroke_color = theme::with_alpha(app.accent_color(), focus);
            ui.painter().rect_stroke(rect, radius, egui::Stroke::new(1.5, stroke_color), egui::StrokeKind::Inside);
        }
    }
    icon_at(ui, pos2(rect.left() + 18.0, rect.center().y), icon::SEARCH, 15.0, theme::text_dim());
    let clearable = !app.search_text.is_empty();
    let inner = Rect::from_min_max(pos2(rect.left() + 36.0, rect.top() + 4.0), pos2(rect.right() - if clearable { 36.0 } else { 14.0 }, rect.bottom() - 4.0));

    let fresh = app.suggestions.0 == app.search_text.trim();
    let n = if fresh { app.suggestions.1.len() } else { 0 };
    if app.search_focused && n > 0 && ui.input(|i| i.modifiers.is_none()) {
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                app.suggest_sel = match app.suggest_sel {
                    None => Some(0),
                    Some(s) if s + 1 < n => Some(s + 1),
                    Some(_) => None,
                };
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                app.suggest_sel = match app.suggest_sel {
                    None => Some(n - 1),
                    Some(0) => None,
                    Some(s) => Some(s - 1),
                };
            }
        });
    }
    let edit = ui.put(
        inner,
        egui::TextEdit::singleline(&mut app.search_text)
            .hint_text(egui::RichText::new("Artists, Songs, Albums and Playlists").color(theme::text_faint()))
            .font(theme::regular(13.5))
            .frame(egui::Frame::NONE)
            .align(egui::Align2::LEFT_CENTER)
            .desired_width(inner.width()),
    );
    app.search_rect = Some(rect);
    if std::mem::take(&mut app.search_refocus) {
        edit.request_focus();
    }
    app.search_focused = edit.has_focus();
    let mut cleared = false;
    if clearable {
        let x = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(24.0));
        let resp = named(ui.interact(x, ui.scope_id().with("search-clear"), Sense::click()), "Clear search").on_hover_cursor(egui::CursorIcon::PointingHand);
        let h = if resp.hovered() { 1.0 } else { 0.0 };
        if h > 0.0 {
            ui.painter().circle_filled(x.center(), 12.0, theme::glass(0.10 * h));
        }
        icon_at(ui, x.center(), icon::X, 14.0, theme::lerp_color(theme::text_dim(), theme::text(), h));
        if resp.clicked() {
            app.search_text.clear();
            app.search_synced = String::new();
            app.suggestions = Default::default();
            app.suggest_sel = None;
            app.live_search_due = None;
            cleared = true;
            app.page = Page::Search(String::new());
            edit.request_focus();
        }
    }
    if field.clicked() {
        edit.request_focus();
    }
    if edit.changed() || edit.gained_focus() || cleared {
        app.suggest_sel = None;
        let now = ui.input(|i| i.time);
        let first = app.suggestions.1.is_empty();
        let query = app.search_text.trim().to_owned();
        app.suggestions = app.suggest_cache.iter()
            .find(|(q, _)| q == &query)
            .cloned()
            .unwrap_or_default();
        let delay = if first || cleared || edit.gained_focus() { 0.0 } else { 0.06 };
        app.suggest_due = Some(now + delay);
        if edit.changed() {
            app.live_search_due = if query.is_empty() {
                app.page = Page::Search(String::new());
                app.search_synced.clear();
                None
            } else {
                Some(now + 0.28)
            };
        }
    }
    // Keep the event loop awake until suggest / live-search debounce deadlines fire.
    let now = ui.input(|i| i.time);
    for due in [app.suggest_due, app.live_search_due].into_iter().flatten() {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((due - now).max(0.0)));
    }
    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        let picked = app.suggest_sel.filter(|_| fresh).and_then(|s| app.suggestions.1.get(s)).map(|s| s.text.clone());
        let q = picked.unwrap_or_else(|| app.search_text.trim().to_owned());
        app.live_search_due = None;
        if !q.is_empty() {
            app.search_text = q.clone();
            app.suggestions = Default::default();
            app.actions.push(Action::Go(Page::Search(q)));
        }
    }
}

/// Legacy no-ops (search now lives on its dedicated page).
pub fn suggestions(_app: &mut App, _ctx: &egui::Context) {}
pub fn search_results(_app: &mut App, _ctx: &egui::Context) {}

// ------------------------------------------------------------------ player bar

/// The player: one frosted capsule floating over the bottom of the window, as in the
/// BitChord desktop app. Transport on the left, the sleeve with its titles and seek
/// line in the middle, favourites / lyrics / queue / volume on the right.
/// Height of the docked player strip.
pub const DOCKED_H: f32 = 96.0;

/// Whether the theme docks the player to the window's bottom edge.
pub fn docked(app: &App) -> bool {
    app.look().chrome.player_bar == crate::preset::PlayerBar::Docked
}

fn paint_blurred_capsule(painter: &egui::Painter, cap: Rect, radius: f32, tex: &egui::TextureHandle, tint: Color32) {
    let mut mesh = egui::Mesh::with_texture(tex.id());
    let to_uv = |p: Pos2| {
        let ux = ((p.x - cap.left()) / cap.width().max(1.0)).clamp(0.0, 1.0);
        let vy = ((p.y - cap.top()) / cap.height().max(1.0)).clamp(0.0, 1.0);
        pos2(0.10 + ux * 0.80, 0.25 + vy * 0.50)
    };
    mesh.vertices.push(egui::epaint::Vertex { pos: cap.center(), uv: to_uv(cap.center()), color: tint });

    let mut ring: Vec<Pos2> = Vec::with_capacity(28);
    let left_c = pos2(cap.left() + radius, cap.center().y);
    let right_c = pos2(cap.right() - radius, cap.center().y);
    const ARC_SEGS: usize = 12;
    for i in 0..=ARC_SEGS {
        let a = std::f32::consts::FRAC_PI_2 + (i as f32 / ARC_SEGS as f32) * std::f32::consts::PI;
        ring.push(left_c + vec2(a.cos() * radius, a.sin() * radius));
    }
    for i in 0..=ARC_SEGS {
        let a = -std::f32::consts::FRAC_PI_2 + (i as f32 / ARC_SEGS as f32) * std::f32::consts::PI;
        ring.push(right_c + vec2(a.cos() * radius, a.sin() * radius));
    }
    let count = ring.len() as u32;
    for (idx, p) in ring.into_iter().enumerate() {
        mesh.vertices.push(egui::epaint::Vertex { pos: p, uv: to_uv(p), color: tint });
        let cur = 1 + idx as u32;
        let next = 1 + ((idx as u32 + 1) % count);
        mesh.add_triangle(0, cur, next);
    }
    painter.add(egui::Shape::mesh(mesh));
}

pub fn player_bar(app: &mut App, ui: &mut Ui, now: f64, _frosted: bool) {
    let full = ui.max_rect();
    // A docked strip is opaque and takes clicks across its whole width.
    if docked(app) {
        ui.interact(full, ui.scope_id().with("player-strip"), Sense::hover());
    }
    let ctx = ui.ctx().clone();
    let cap = if docked(app) {
        // Docked: the app's own background, opaque, so rows scrolling under the controls
        // don't show through.
        let cap = Rect::from_min_max(pos2(full.left(), full.bottom() - DOCKED_H), full.max);
        let base = if theme::light() { theme::bg() } else { app.page_color(&ctx) };
        ui.painter().rect_filled(cap, 0.0, base);
        cap.shrink2(vec2(12.0, 0.0))
    } else {
        const H: f32 = 52.0;
        let radius = H / 2.0;
        let max_w = 700.0;
        let w = (full.width() - 80.0).clamp(400.0, max_w).min(full.width() - 24.0);
        let cap = Rect::from_min_size(pos2(full.center().x - w / 2.0, full.bottom() - 14.0 - H), vec2(w, H));
        ui.interact(cap, ui.scope_id().with("player-strip"), Sense::hover());

        let light = theme::light();
        let p_col = app.page_color(&ctx);
        if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
            let tint = if light {
                theme::with_alpha(theme::lerp_color(p_col, Color32::WHITE, 0.78), 0.22)
            } else {
                theme::with_alpha(theme::lerp_color(p_col, Color32::from_rgb(0x16, 0x16, 0x18), 0.55), 0.22)
            };
            let style = crate::glass::Style::regular(crate::glass::Corner::Capsule, Some(tint), if light { 0.08 } else { 0.90 }, 1.0);
            crate::glass::paint(ui, egui::Id::unique("player-bar-liquid-glass"), cap, style);
        } else {
            let fill = if light {
                theme::with_alpha(theme::lerp_color(p_col, Color32::WHITE, 0.72), 0.72)
            } else {
                theme::with_alpha(theme::lerp_color(p_col, Color32::from_rgb(0x16, 0x16, 0x18), 0.45), 0.72)
            };
            ui.painter().rect_filled(cap, radius, fill);

            let blur_key = match &app.page {
                Page::Album(id) => Some(format!("album:{id}")),
                Page::Artist(id) => Some(format!("artist:{id}")),
                _ => app.art_key.clone().or_else(|| app.current_id().map(str::to_owned)),
            };
            if let Some(k) = blur_key
                && let Some(dom) = app.art_colors.get(&k).copied()
            {
                let r = dom.r() as f32 / 255.0;
                let g = dom.g() as f32 / 255.0;
                let b = dom.b() as f32 / 255.0;
                let mx = r.max(g).max(b);
                let mn = r.min(g).min(b);
                let sat = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
                if sat >= 0.17
                    && mx >= 0.08
                    && let Some(tex) = app.blurred_art_tex(&ctx, &k)
                {
                    let tint = if light {
                        Color32::from_rgba_premultiplied(46, 46, 46, 46)
                    } else {
                        Color32::from_rgba_premultiplied(78, 76, 74, 78)
                    };
                    paint_blurred_capsule(ui.painter(), cap, radius, &tex, tint);
                }
            }
        }

        cap
    };

    let status = app.audio.status.clone();
    // An Automix handoff keeps the outgoing song on this strip until the next one is
    // the louder of the two.
    let hold = app.mix_hold().map(|h| (h.total, h.mix, app.mix_hold_pos(h)));
    let track = match &hold {
        Some(_) => app.shown().cloned(),
        None => app.current().cloned(),
    };
    let enabled = track.is_some();
    let playing = app.playing();
    let accent = app.accent_color();
    let cy = cap.center().y;

    let ink = |h: f32| {
        if theme::light() {
            if enabled { Color32::BLACK } else { Color32::from_black_alpha(115) }
        } else {
            let c = theme::lerp_color(Color32::from_white_alpha(224), Color32::WHITE, h);
            if enabled { c } else { c.gamma_multiply(0.45) }
        }
    };

    // ------------------------------------------------------------------ Left Zone: Transport Controls
    let tbtn = |ui: &mut Ui, x: f32, key: &str, name: &str| -> egui::Response {
        let r = Rect::from_center_size(pos2(x, cy), Vec2::splat(28.0));
        let resp = ui.interact(r, ui.scope_id().with(("bar-t", key)), Sense::click());
        named(resp, name).on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand)
    };

    let left_start = cap.left() + 18.0;
    let show_modes = cap.width() >= 480.0;
    let (x_shuf, x_prev, x_play, x_next, x_rep, left_boundary) = if show_modes {
        let s = left_start + 12.0;
        let p = s + 26.0;
        let pl = p + 28.0;
        let n = pl + 28.0;
        let r = n + 26.0;
        (s, p, pl, n, r, r + 18.0)
    } else {
        let p = left_start + 12.0;
        let pl = p + 28.0;
        let n = pl + 28.0;
        (0.0, p, pl, n, 0.0, n + 18.0)
    };

    // Shuffle (if show_modes)
    let light_bar = theme::light();
    if show_modes {
        let r_shuf = tbtn(ui, x_shuf, "shuffle", &t!("shuffle_up_next"));
        let (h, s) = widgets::paint_icon_touch(ui, &r_shuf, r_shuf.rect, light_bar);
        glyphs::draw(ui, r_shuf.rect.center(), 17.0 * s, glyphs::SHUFFLE, ink(h));
        if r_shuf.clicked() {
            app.shuffle_upcoming();
        }
    }

    // Previous
    let r_prev = tbtn(ui, x_prev, "prev", &t!("previous"));
    let (h, s) = widgets::paint_icon_touch(ui, &r_prev, r_prev.rect, light_bar);
    glyphs::draw(ui, r_prev.rect.center(), 19.5 * s, glyphs::PREVIOUS, ink(h));
    if r_prev.clicked() && enabled {
        app.prev();
    }

    // Play / Pause
    let play_label = if playing { t!("pause") } else { t!("play") };
    let r_play = tbtn(ui, x_play, "play", &play_label);
    let (h, s) = widgets::paint_icon_touch(ui, &r_play, r_play.rect, light_bar);
    let c = r_play.rect.center();
    let morph = theme::anim_bool(&ctx, "bar-play-morph", playing || app.buffering, 0.18);
    if app.buffering {
        let a = now as f32 * 6.0;
        let pts: Vec<Pos2> = (0..16).map(|i| c + vec2((a + i as f32 * 0.24).cos(), (a + i as f32 * 0.24).sin()) * 7.5).collect();
        ui.painter().add(egui::Shape::Path(egui::epaint::PathShape::line(pts, egui::epaint::PathStroke::new(2.0, ink(0.6)))));
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    } else {
        let fg = ink(0.9 + 0.1 * h);
        if morph < 1.0 {
            glyphs::draw(ui, c, 22.5 * s, glyphs::PLAY, theme::with_alpha(fg, 1.0 - morph));
        }
        if morph > 0.0 {
            glyphs::draw(ui, c, 22.5 * s, glyphs::PAUSE, theme::with_alpha(fg, morph));
        }
    }
    if r_play.clicked() && enabled {
        app.toggle();
    }

    // Next
    let r_next = tbtn(ui, x_next, "next", &t!("next"));
    let (h, s) = widgets::paint_icon_touch(ui, &r_next, r_next.rect, light_bar);
    glyphs::draw(ui, r_next.rect.center(), 19.5 * s, glyphs::NEXT, ink(h));
    if r_next.clicked() && enabled {
        app.next();
    }

    // Repeat (if show_modes)
    if show_modes {
        let (glyph, on) = match app.queue.repeat {
            Repeat::Off => (glyphs::REPEAT, false),
            Repeat::All => (glyphs::REPEAT, true),
            Repeat::One => (glyphs::REPEAT_ONE, true),
        };
        let r_rep = tbtn(ui, x_rep, "repeat", &t!("repeat"));
        let (h, s) = widgets::paint_icon_touch(ui, &r_rep, r_rep.rect, light_bar);
        glyphs::draw(ui, r_rep.rect.center(), 17.0 * s, glyph, if on { accent } else { ink(h) });
        if r_rep.clicked() {
            app.queue.repeat = match app.queue.repeat {
                Repeat::Off => Repeat::All,
                Repeat::All => Repeat::One,
                Repeat::One => Repeat::Off,
            };
        }
    }

    // ------------------------------------------------------------------ Right Zone: Lyrics, Queue, Star, Volume
    let r_pad = 18.0;
    let r_base = cap.right() - r_pad - 12.0;
    let r_spacing = if cap.width() < 540.0 { 26.0 } else { 28.0 };
    let vol_x = r_base;
    let star_x = r_base - r_spacing;
    let q_x = r_base - 2.0 * r_spacing;
    let lyr_x = r_base - 3.0 * r_spacing;
    let right_boundary = lyr_x - 18.0;

    // Click-to-expand inline volume capsule slider
    let r_vol = Rect::from_center_size(pos2(vol_x, cy), Vec2::splat(28.0));
    let pill_right = r_vol.right() + 5.0;
    let expanded_w = (r_spacing * 3.0 + 34.0).clamp(108.0, 128.0);
    let vol_open_id = ui.scope_id().with("bar-vol-open");
    let mut vol_open = ui.data(|d| d.get_temp::<bool>(vol_open_id).unwrap_or(false));
    let v_reveal = theme::anim_bool(&ctx, "bar-vol-reveal-inline", vol_open, 0.20);
    let cur_w = egui::lerp(32.0..=expanded_w, v_reveal);
    let vol_pill_rect = Rect::from_min_max(pos2(pill_right - cur_w, cy - 19.0), pos2(pill_right, cy + 19.0));
    let slider_rect = Rect::from_min_max(pos2(pill_right - expanded_w + 14.0, cy - 10.0), pos2(r_vol.left() - 6.0, cy + 10.0));
    if vol_open {
        let clicked_outside = ui.input(|i| i.pointer.any_pressed())
            && ui.input(|i| i.pointer.interact_pos()).is_none_or(|p| !vol_pill_rect.contains(p));
        if clicked_outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            vol_open = false;
        }
    }

    // ------------------------------------------------------------------ Center Zone: Art + Title/Artist + Scrubber + Options
    let center_avail = (right_boundary - left_boundary).max(80.0);
    let center_w = center_avail.min(370.0);
    let center_cx = (left_boundary + right_boundary) / 2.0;
    let center_box = Rect::from_center_size(pos2(center_cx, cy), vec2(center_w, 42.0));

    // Cover art (left edge of center_box)
    let cover_size = 36.0;
    let cover_rect = Rect::from_center_size(pos2(center_box.left() + cover_size / 2.0, center_box.center().y - 1.5), Vec2::splat(cover_size));
    match &track {
        Some(t) => {
            widgets::paint_cover_xfade(ui, egui::Id::unique("player-bar-art"), cover_rect, &t.thumbs, 6.0, 1.0);
            let ar = ui.interact(cover_rect, ui.scope_id().with("bar-art"), Sense::click());
            let ar = named(ar, "Open now playing").on_hover_cursor(egui::CursorIcon::PointingHand);
            let ah = if ar.hovered() { 1.0 } else { 0.0 };
            if ah > 0.0 {
                ui.painter().rect_filled(cover_rect, 6.0, theme::glass(0.16 * ah));
                ui.painter().rect_filled(cover_rect, 6.0, theme::shade(0.3 * ah));
                glyphs::draw(ui, cover_rect.center(), 14.0, glyphs::MAXIMIZE_2, theme::with_alpha(Color32::WHITE, ah));
            }
            if ar.clicked() {
                app.actions.push(Action::OpenNowPlaying(true));
            }
        }
        None => {
            ui.painter().rect_filled(cover_rect, 6.0, theme::glass(0.08));
            paint_mark(ui, cover_rect.shrink(6.0));
        }
    }

    // Options button (...) at the right edge of center_box
    let opt_size = 22.0;
    let r_opt = Rect::from_center_size(pos2(center_box.right() - opt_size / 2.0, center_box.center().y - 1.5), Vec2::splat(opt_size));
    if let Some(t) = &track {
        super::nowplaying::options_menu(app, ui, r_opt, t, false, Some(&ink));
    } else {
        let resp = ui.interact(r_opt, ui.scope_id().with("bar-opt-disabled"), Sense::hover());
        let _ = named(resp, &t!("options"));
        for dx in [-4.5, 0.0, 4.5] {
            ui.painter().circle_filled(r_opt.center() + vec2(dx, 0.0), 1.7, ink(0.0));
        }
    }

    // Text between cover art and options button
    let text_l = cover_rect.right() + 10.0;
    let text_r = r_opt.left() - 6.0;
    let text_w = (text_r - text_l).max(10.0);

    let row1_y = center_box.top() + 9.0;
    let row2_y = center_box.top() + 24.0;

    let (title, artists, album, explicit) = match &track {
        Some(t) => (t.title.clone(), t.artists.clone(), t.album.as_ref().map(|a| a.name.clone()), t.explicit),
        None => ("Dawn FM".to_owned(), Vec::new(), None, false),
    };

    // Row 1: Title (bold)
    let title_col = if theme::light() { Color32::BLACK } else { theme::text() };
    let title_font = theme::semibold(12.5);
    let probe_g = widgets::galley(ui, &title, title_font.clone(), title_col, text_w);
    let title_pos = pos2(text_l, row1_y - probe_g.size().y / 2.0);
    let title_sz = widgets::title_with_explicit(ui, title_pos, &title, title_font, title_col, text_w, explicit);

    let title_hit = Rect::from_min_size(title_pos, vec2(title_sz.x.min(text_w), 15.0));
    let tre = ui.interact(title_hit, ui.scope_id().with("bar-title"), Sense::click());
    if named(tre, &title).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        app.actions.push(Action::OpenNowPlaying(true));
    }

    // Row 2: Artist — Album (matches Apple Music style in screenshot)
    let mut subtitle = artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ");
    if let Some(alb) = &album {
        if !subtitle.is_empty() {
            subtitle.push_str(" — ");
        }
        subtitle.push_str(alb);
    }
    let artist_col = if theme::light() { Color32::from_black_alpha(150) } else { theme::text_dim() };
    let sub_g = widgets::galley(ui, &subtitle, theme::regular(11.0), artist_col, text_w);
    let sub_pos = pos2(text_l, row2_y - sub_g.size().y / 2.0);
    ui.painter().galley(sub_pos, sub_g.clone(), artist_col);

    let sub_hit = Rect::from_min_size(sub_pos, vec2(sub_g.size().x.min(text_w), 13.0));
    let sub_resp = ui.interact(sub_hit, ui.scope_id().with("bar-artist"), Sense::click());
    if named(sub_resp, &subtitle).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        if let Some(a0) = artists.first().and_then(|a| a.id.as_ref()) {
            app.actions.push(Action::Open(Target::Artist(a0.clone())));
        } else {
            app.actions.push(Action::OpenNowPlaying(true));
        }
    }

    // Scrubber line along the bottom of center_box
    let held = hold.is_some();
    let (total, mix) = match &hold {
        Some((total, mix, _)) => (*total, *mix),
        None => {
            let total = status
                .duration()
                .map(|d| d.as_secs_f32())
                .or(track.as_ref().and_then(|t| t.duration).map(|d| d as f32))
                .unwrap_or(0.0);
            let next = app.queue.upcoming().is_some() || app.autoplay;
            let mix = match (total > 0.0, next, app.queue.repeat) {
                (true, true, repeat) if repeat != Repeat::One => {
                    status.mix_secs(status.duration().or_else(|| track.as_ref().and_then(|t| t.duration).map(|s| std::time::Duration::from_secs(s as u64)))).min(total * 0.5)
                }
                _ => 0.0,
            };
            (total, mix)
        }
    };
    let smooth = match &hold {
        Some((_, _, pos)) => *pos,
        None => status.position_smooth().as_secs_f32(),
    };
    let pos = app.seek_drag.unwrap_or(smooth).min(total.max(0.0));

    let sx0 = center_box.left();
    let sx1 = center_box.right();
    let sy = center_box.bottom() - 1.0;
    let seek_hit = Rect::from_min_max(pos2(sx0, sy - 7.0), pos2(sx1, sy + 7.0));
    let sresp = ui.interact(seek_hit, ui.scope_id().with("bar-seek"), if held { Sense::hover() } else { Sense::click_and_drag() });
    let sresp = if total > 0.0 && !held { sresp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { sresp };
    let time_label = format!("{} / {}", fmt_time(pos as u32), if total > 0.0 { fmt_time(total as u32) } else { "0:00".to_string() });
    let sresp = named(sresp, &time_label).on_hover_text(&time_label);
    sresp.widget_info(|| egui::WidgetInfo::slider(total > 0.0 && !held, pos as f64, "Seek"));
    let focus = theme::anim_bool(&ctx, "bar-seek-h", total > 0.0 && !held && (sresp.hovered() || sresp.dragged()), 0.15);

    let thick = 2.0 + 1.2 * focus;
    let line_r = Rect::from_min_max(pos2(sx0, sy - thick / 2.0), pos2(sx1, sy + thick / 2.0));
    ui.painter().rect_filled(line_r, thick / 2.0, theme::glass(0.18));

    if mix > 0.5 {
        let edge = sx0 + (sx1 - sx0) * ((total - mix) / total);
        let band = Rect::from_min_max(pos2(edge, line_r.top()), line_r.max);
        ui.painter().rect_filled(band, thick / 2.0, theme::with_alpha(accent, 0.30));
        let tick = Rect::from_min_size(pos2(edge - 0.75, line_r.top()), vec2(1.5, thick));
        ui.painter().rect_filled(tick, 0.75, theme::with_alpha(accent, 0.9));
    }

    let frac = if total > 0.0 { (pos / total).clamp(0.0, 1.0) } else { 0.0 };
    if frac > 0.0 {
        let head_col = if theme::light() { Color32::BLACK } else { theme::with_alpha(theme::text(), 0.85 + 0.15 * focus) };
        ui.painter().rect_filled(Rect::from_min_size(line_r.min, vec2(line_r.width() * frac, thick)), thick / 2.0, head_col);
        if focus > 0.0 {
            let grab_pos = pos2(sx0 + (sx1 - sx0) * frac, sy);
            ui.painter().circle_filled(grab_pos, 3.0 + 0.8 * focus, if theme::light() { Color32::BLACK } else { Color32::WHITE });
        }
    }

    let at = |p: Pos2| (((p.x - sx0) / (sx1 - sx0)).clamp(0.0, 1.0) * total).max(0.0);
    if total > 0.0 && let Some(p) = sresp.interact_pointer_pos() {
        if sresp.dragged() {
            app.seek_drag = Some(at(p));
        }
        if sresp.drag_stopped() || sresp.clicked() {
            app.seek_drag = None;
            app.actions.push(Action::Seek(at(p)));
        }
    }

    // Now render Right Zone: Star, Volume, Lyrics, Queue
    let heard = if held { track.clone() } else { app.playing_track().cloned().or_else(|| track.clone()) };
    let liked = heard.as_ref().is_some_and(|t| app.liked.contains(&t.id));
    let like_label = if liked { t!("remove_from_favourites") } else { t!("add_to_favourites") };
    let r_star = Rect::from_center_size(pos2(star_x, cy), Vec2::splat(28.0));

    // Star, Lyrics & Queue fade out when volume slider expands
    if v_reveal < 0.95 {
        let r_alpha = (1.0 - v_reveal).clamp(0.0, 1.0);
        let r_ink = |h: f32| theme::with_alpha(ink(h), r_alpha);
        let interactive_r = r_alpha > 0.5 && !vol_open;

        // Star / Like
        let r_star_hit = if interactive_r {
            let star_resp = ui.interact(r_star, ui.scope_id().with(("bar-r", "like")), Sense::click());
            let star_resp = named(star_resp, &like_label).on_hover_cursor(egui::CursorIcon::PointingHand);
            let (star_h, star_s) = widgets::paint_icon_touch(ui, &star_resp, r_star, light_bar);
            Some((star_resp, star_h, star_s))
        } else {
            None
        };
        let (star_h, star_s) = r_star_hit.as_ref().map_or((0.0, 1.0), |(_, h, s)| (*h, *s));
        let pop = heard.as_ref().map_or(0.0, |t| theme::anim_bool(&ctx, ("liked", &t.id), liked, 0.3));
        let dim = if heard.is_some() { 1.0 } else { 0.4 };
        let tint = theme::lerp_color(r_ink(star_h), accent, pop).gamma_multiply(dim);
        let size = 17.5 * star_s * (1.0 + 0.25 * (pop * std::f32::consts::PI).sin());
        glyphs::draw(ui, r_star.center(), size, glyphs::STAR, tint);
        if pop > 0.0 {
            glyphs::draw(ui, r_star.center(), size, glyphs::STAR_FILLED, theme::with_alpha(tint, tint.a() as f32 / 255.0 * pop));
        }
        if let Some((star_resp, _, _)) = r_star_hit
            && star_resp.clicked()
            && let Some(t) = heard
        {
            app.actions.push(Action::Like(t, !liked));
        }

        // Queue / Up Next
        let queue_open = if app.np_open { app.np_tab == NpTab::UpNext } else { app.queue_open };
        let r_q = Rect::from_center_size(pos2(q_x, cy), Vec2::splat(28.0));
        if interactive_r {
            let q_resp = ui.interact(r_q, ui.scope_id().with(("bar-r", "queue")), Sense::click());
            let q_resp = named(q_resp, &t!("up_next")).on_hover_cursor(egui::CursorIcon::PointingHand);
            let (q_h, q_s) = widgets::paint_icon_touch(ui, &q_resp, r_q, light_bar);
            glyphs::draw(ui, r_q.center(), 17.5 * q_s, glyphs::QUEUE, if queue_open { accent } else { r_ink(q_h) });
            if q_resp.clicked() {
                if app.np_open {
                    app.np_tab = if queue_open { NpTab::Art } else { NpTab::UpNext };
                } else {
                    app.queue_open = !app.queue_open;
                    if app.queue_open {
                        app.lyrics_drawer = false;
                    }
                }
            }
        } else {
            glyphs::draw(ui, r_q.center(), 17.5, glyphs::QUEUE, if queue_open { theme::with_alpha(accent, r_alpha) } else { r_ink(0.0) });
        }

        // Lyrics
        let lyrics_on = if app.np_open { app.np_tab == NpTab::Lyrics && !app.no_lyrics() } else { app.lyrics_drawer && !app.queue_open };
        let lyrics_label = format!("{} (⌘L)", t!("lyrics"));
        let r_lyr = Rect::from_center_size(pos2(lyr_x, cy), Vec2::splat(28.0));
        if interactive_r {
            let lyr_resp = ui.interact(r_lyr, ui.scope_id().with(("bar-r", "lyrics")), Sense::click());
            let lyr_resp = named(lyr_resp, &lyrics_label).on_hover_cursor(egui::CursorIcon::PointingHand);
            let (lyr_h, lyr_s) = widgets::paint_icon_touch(ui, &lyr_resp, r_lyr, light_bar);
            glyphs::draw(ui, r_lyr.center(), 17.5 * lyr_s, glyphs::LYRICS_OFF, if lyrics_on { accent } else { r_ink(lyr_h) });
            if lyr_resp.clicked() {
                app.toggle_lyrics();
            }
        } else {
            glyphs::draw(ui, r_lyr.center(), 17.5, glyphs::LYRICS_OFF, if lyrics_on { theme::with_alpha(accent, r_alpha) } else { r_ink(0.0) });
        }
    }

    // Expanded Liquid Glass capsule for the volume slider when clicked open.
    if v_reveal > 0.01 {
        if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
            let style = crate::glass::control(crate::glass::Corner::Capsule, light_bar, 0.35, v_reveal);
            crate::glass::paint(ui, egui::Id::unique("bar-vol-capsule-glass"), vol_pill_rect, style);
        } else {
            let pill_fill = if light_bar {
                Color32::from_black_alpha((24.0 * v_reveal) as u8)
            } else {
                Color32::from_white_alpha((32.0 * v_reveal) as u8)
            };
            ui.painter().rect_filled(vol_pill_rect, vol_pill_rect.height() * 0.5, pill_fill);
        }
        if v_reveal > 0.25 {
            let mut slider_ui = ui.new_child(egui::UiBuilder::new().max_rect(vol_pill_rect));
            slider_ui.set_clip_rect(vol_pill_rect.intersect(ui.clip_rect()));
            slider_ui.set_opacity(v_reveal);
            volume_slider(app, &mut slider_ui, slider_rect, "bar-vol-inline");
        }
    }

    // Volume icon (click expands/collapses the volume slider capsule; scroll adjusts volume).
    let vol = app.audio.volume();
    let vol_resp = ui.interact(r_vol, ui.scope_id().with(("bar-r", "vol")), Sense::click());
    let vol_resp = named(vol_resp, "Volume").on_hover_cursor(egui::CursorIcon::PointingHand);
    let (vol_hover, vol_scale) = if v_reveal < 0.05 {
        widgets::paint_icon_touch(ui, &vol_resp, r_vol, light_bar)
    } else {
        (if vol_resp.hovered() { 1.0 } else { 0.0 }, 1.0)
    };
    let vol_h = vol_hover.max(if vol_open { 1.0 } else { 0.0 });
    let step = glyphs::volume_step(vol);
    glyphs::draw(ui, r_vol.center(), 17.5 * vol_scale, glyphs::VOLUME[step], ink(vol_h));
    if vol_resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            app.audio.set_volume((vol + scroll / 600.0).clamp(0.0, 1.0));
        }
    }
    if vol_resp.clicked() {
        vol_open = !vol_open;
    }
    ui.data_mut(|d| d.insert_temp(vol_open_id, vol_open));
}

/// The BitChord mark, white, fitted inside `r` (the sleeve slot when nothing plays).
fn paint_mark(ui: &Ui, r: Rect) {
    mark(ui, r, Color32::from_white_alpha(170));
}

/// The Dawn FM mark (a rising sun), fitted inside `r`, tinted `tint`.
fn mark(ui: &Ui, r: Rect, tint: Color32) {
    const MARK: &[u8] = include_bytes!("../../assets/icon/mark.png");
    const W: f32 = 256.0;
    const H: f32 = 171.0;
    let key = egui::Id::unique("dawnfm-mark");
    let tex = match ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        Some(t) => t,
        None => {
            let Ok(img) = image::load_from_memory(MARK) else { return };
            let img = img.into_rgba8();
            let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
            let t = ui.ctx().load_texture("dawnfm-mark", color, egui::TextureOptions::LINEAR);
            ui.ctx().data_mut(|d| d.insert_temp(key, t.clone()));
            t
        }
    };
    let k = (r.width() / W).min(r.height() / H);
    let dst = Rect::from_center_size(r.center(), vec2(W * k, H * k));
    ui.painter().image(tex.id(), dst, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
}

/// Where Mute keeps the volume to go back to (the player bar's and the mini player's).
pub fn unmute_id() -> egui::Id {
    egui::Id::unique("unmute")
}

/// The volume track: click or drag to set, scroll over it to nudge.
fn volume_slider(app: &mut App, ui: &mut Ui, slider: Rect, salt: &str) {
    let vol = app.audio.volume();
    let vresp = ui.interact(slider, ui.scope_id().with(salt), Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::PointingHand);
    vresp.widget_info(|| egui::WidgetInfo::slider(true, vol as f64, "Volume"));
    let vh = theme::anim_bool(ui.ctx(), ui.scope_id().with((salt, "h")), vresp.hovered() || vresp.dragged(), 0.15);
    let thick = 5.5 + 1.5 * vh;
    let track_r = Rect::from_center_size(slider.center(), vec2(slider.width(), thick));
    let r = thick * 0.5;
    ui.painter().rect_filled(track_r, r, theme::glass(0.22));
    let fill = Rect::from_min_size(track_r.min, vec2(track_r.width() * vol, thick));
    let fill_col = if theme::light() { Color32::BLACK } else { Color32::WHITE };
    ui.painter().rect_filled(fill, r, fill_col);
    if let Some(p) = vresp.interact_pointer_pos()
        && (vresp.dragged() || vresp.clicked())
    {
        app.audio.set_volume(((p.x - track_r.left()) / track_r.width()).clamp(0.0, 1.0));
    }
    if vresp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            app.audio.set_volume((vol + scroll / 600.0).clamp(0.0, 1.0));
        }
    }
}

// ------------------------------------------------------------------ queue drawer

/// Draws a crisp infinity glyph (`∞`) centered at `c` with half-span `r`.
fn paint_infinity(painter: &egui::Painter, c: Pos2, r: f32, stroke: egui::Stroke) {
    let loop_r = r * 0.42;
    let dx = r * 0.46;
    painter.circle_stroke(pos2(c.x - dx, c.y), loop_r, stroke);
    painter.circle_stroke(pos2(c.x + dx, c.y), loop_r, stroke);
}

/// Draws the AutoMix overlapping-circles glyph (`◐●`) centered at `c`.
fn paint_automix_icon(painter: &egui::Painter, c: Pos2, r: f32, col: Color32) {
    let cr = r * 0.46;
    let dx = r * 0.36;
    painter.circle_filled(pos2(c.x - dx, c.y), cr, col);
    painter.circle_stroke(pos2(c.x + dx, c.y), cr, egui::Stroke::new(1.6, col));
}

/// Pill toggle button for the Queue panel top row (`AutoPlay` / `AutoMix`).
fn queue_top_pill(ui: &mut Ui, width: f32, key: &str, tip: &str, active: bool, draw_icon: impl FnOnce(&egui::Painter, Pos2, Color32)) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    let resp = ui.interact(rect, ui.scope_id().with(("q-pill", key)), Sense::click());
    let resp = named(resp, tip).on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand);
    let light = theme::light();
    let press = crate::glass::press(ui, &resp, rect, 0.75);
    let draw_r = press.rect;
    let h = press.hover;
    let s = theme::anim_bool(ui.ctx(), resp.id.with("on"), active, 0.16);
    let off_fill = if light { Color32::from_black_alpha((24.0 + 14.0 * h) as u8) } else { Color32::from_white_alpha((32.0 + 18.0 * h) as u8) };
    let on_fill = if light { Color32::from_rgb(0x28, 0x26, 0x2c) } else { Color32::from_white_alpha((200.0 + 25.0 * h) as u8) };
    let fill = theme::lerp_color(off_fill, on_fill, s);
    ui.painter().rect_filled(draw_r, draw_r.height() * 0.5, fill);
    let off_ink = if light { Color32::BLACK } else { Color32::from_white_alpha(230) };
    let on_ink = if light { Color32::WHITE } else { Color32::from_rgb(0x18, 0x18, 0x1b) };
    draw_icon(ui.painter(), draw_r.center(), theme::lerp_color(off_ink, on_ink, s));
    resp.clicked()
}

pub fn queue_drawer(app: &mut App, ui: &mut Ui) {
    let ink = if theme::light() { Color32::BLACK } else { Color32::WHITE };
    let right_pad = super::titlebar::right_gap(ui.ctx(), ui.max_rect(), 0.0);
    let avail = (ui.available_width() - right_pad).max(160.0);
    let gap = 10.0;
    let pill_w = ((avail - gap) * 0.5).max(70.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        if queue_top_pill(ui, pill_w, "autoplay", "AutoPlay", app.autoplay, |p, c, col| {
            paint_infinity(p, c, 9.5, egui::Stroke::new(1.8, col));
        }) {
            app.autoplay = !app.autoplay;
        }
        if queue_top_pill(ui, pill_w, "automix", "AutoMix", app.automix, |p, c, col| {
            paint_automix_icon(p, c, 9.0, col);
        }) {
            app.automix = !app.automix;
            app.set_crossfade();
        }
    });
    ui.add_space(12.0);
    if app.autoplay {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
        paint_infinity(ui.painter(), pos2(r.left() + 8.0, r.center().y), 6.5, egui::Stroke::new(1.5, ink));
        ui.painter().text(pos2(r.left() + 20.0, r.center().y), Align2::LEFT_CENTER, "AutoPlay", theme::semibold(15.0), ink);
        ui.add_space(2.0);
        ui.label(egui::RichText::new("Similar songs will play").font(theme::regular(12.0)).color(theme::text_dim()));
    } else {
        ui.label(egui::RichText::new("Up next").font(theme::semibold(15.0)).color(ink));
    }
    ui.add_space(8.0);
    queue_list(app, ui, "drawer");
}

/// Current track plus everything after it. Shared by the drawer and now playing.
pub fn queue_list(app: &mut App, ui: &mut Ui, salt: &str) {
    let rows = super::pages::Rows::new(app, ui.input(|i| i.time));
    let drag_key = egui::Id::unique(("queue-drag", salt));
    let dragging: Option<usize> = ui.data(|d| d.get_temp(drag_key));
    let mut reorder: Option<(usize, usize)> = None;
    let down = ui.input(|i| i.pointer.any_down());
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let ink = if theme::light() { Color32::BLACK } else { Color32::WHITE };

    egui::ScrollArea::vertical().id_salt(("queue", salt)).auto_shrink(false).show(ui, |ui| {
        if app.queue.tracks.is_empty() {
            ui.label(egui::RichText::new("Nothing queued yet").color(ink));
            return;
        }
        let start = app.queue.index.unwrap_or(0);
        if salt != "drawer" {
            ui.checkbox(&mut app.autoplay, egui::RichText::new("Autoplay similar songs").color(ink)).on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.add_space(6.0);
        }
        let accent = app.accent_color();
        let (queue, liked, acts, has_current, by) = (&app.queue.tracks, &app.liked, &mut app.actions, app.queue.index.is_some(), &app.jam.by);
        widgets::visible_rows(ui, queue.len() - start, |ui, k| {
            let i = start + k;
            let t = &queue[i];
            // The first row is the current song.
            let current = k == 0 && has_current;
            let can_drag = !current;
            let is_dragged = dragging == Some(i);
            let o = widgets::RowOpts {
                current,
                draggable: can_drag,
                is_dragged,
                remove_queue: (!current).then_some(i),
                added_by: by.get(&t.id).cloned(),
                alt: k % 2 == 0,
                plain_text: true,
                ..rows.opts(liked, t, None, false)
            };
            let resp = widgets::track_row(ui, t, &o, acts);
            if can_drag && resp.drag_started() {
                ui.data_mut(|d| d.insert_temp(drag_key, i));
            }
            if let Some(from) = dragging {
                if can_drag && from != i && down && pointer.is_some_and(|p| resp.rect.contains(p)) {
                    reorder = Some((from, i));
                }
                if can_drag && from != i && pointer.is_some_and(|p| resp.rect.contains(p)) {
                    let line_y = if from < i { resp.rect.bottom() } else { resp.rect.top() };
                    ui.painter().hline(resp.rect.x_range(), line_y, egui::Stroke::new(2.0, accent));
                }
            }
            if resp.clicked() && dragging.is_none() {
                // The playing row pauses/resumes instead of doing nothing.
                acts.push(if current { Action::TogglePlay } else { Action::QueueJump(i) });
            }
        });
        if ui.input(|i| i.pointer.any_released()) {
            ui.data_mut(|d| d.remove_temp::<usize>(drag_key));
        }
    });

    if let Some(from) = dragging
        && down
        && let Some(pos) = pointer
        && let Some(t) = app.queue.tracks.get(from)
    {
        let t = t.clone();
        egui::Area::new(egui::Id::unique("queue-drag-preview"))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .fixed_pos(pos + egui::vec2(14.0, 14.0))
            .show(ui.ctx(), |ui| {
                let stroke = egui::Stroke::new(1.0, theme::glass(0.18));
                let shadow = if theme::light() {
                    egui::epaint::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(45) }
                } else {
                    egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(150) }
                };
                let fallback = egui::Frame::new()
                    .fill(theme::menu_fill())
                    .corner_radius(10)
                    .stroke(stroke)
                    .shadow(shadow);
                crate::glass::surface(
                    ui,
                    egui::Id::unique("queue-drag-preview-glass"),
                    egui::Margin::symmetric(10, 8),
                    crate::glass::panel(10.0, theme::light(), 1.0),
                    fallback,
                    |ui| {
                        let card_w = 230.0;
                        let card_h = 36.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(card_w, card_h), Sense::hover());
                        let cy = rect.center().y;
                        let img_rect = Rect::from_center_size(pos2(rect.left() + 18.0, cy), Vec2::splat(32.0));
                        widgets::paint_cover(ui, img_rect, &t.thumbs, theme::thumb_radius(), 1.0);

                        let text_x = rect.left() + 40.0;
                        let max_w = card_w - 60.0;
                        widgets::title_with_explicit(ui, pos2(text_x, cy - 8.5), &t.title, theme::semibold(13.0), ink, max_w, t.explicit);
                        widgets::text_at(ui, pos2(text_x, cy + 4.5), &t.artist_line(), theme::regular(11.5), ink, max_w);

                        let handle_x = rect.right() - 8.0;
                        let handle_stroke = egui::Stroke::new(1.5, ink);
                        for dy in [-3.5, 0.0, 3.5] {
                            ui.painter().line_segment([pos2(handle_x - 5.0, cy + dy), pos2(handle_x + 5.0, cy + dy)], handle_stroke);
                        }
                    },
                );
            });
    }

    if let Some((from, to)) = reorder {
        app.actions.push(Action::QueueMove(from, to));
        ui.data_mut(|d| d.insert_temp(drag_key, to));
    }
}

// ------------------------------------------------------------------ toast

/// The app's size, as egui's zoom: ⌘+ and ⌘− step it by a tenth and ⌘0 resets it
/// (egui's own shortcuts), and Settings › Appearance has the same steps.
pub const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.7..=1.6;

/// One step larger (`up`) or smaller, on tenths and inside [`ZOOM_RANGE`].
pub fn zoom_step(ctx: &egui::Context, up: bool) {
    let z = ctx.zoom_factor() + if up { 0.1 } else { -0.1 };
    ctx.set_zoom_factor(((z * 10.0).round() / 10.0).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end()));
}

/// Keeps the size inside its range and, for a moment after it changes, shows it as a
/// percentage at the window's top right.
pub fn zoom_badge(ctx: &egui::Context, now: f64) {
    let z = ctx.zoom_factor();
    if !ZOOM_RANGE.contains(&z) {
        ctx.set_zoom_factor(z.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end()));
    }
    // The size last seen and when it changed; the one at launch isn't a change.
    let id = egui::Id::unique("zoom-badge");
    let (seen, at) = ctx.data(|d| d.get_temp::<(f32, f64)>(id)).unwrap_or((z, f64::NEG_INFINITY));
    let at = if (seen - z).abs() > 0.001 { now } else { at };
    ctx.data_mut(|d| d.insert_temp(id, (z, at)));
    let age = (now - at) as f32;
    if age >= 1.6 {
        return;
    }
    let t = theme::ease_out(age / 0.15) * (1.0 - ((age - 1.3) / 0.3).clamp(0.0, 1.0));
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("zoom-badge-area"))
        .order(egui::Order::Tooltip)
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(pos2(screen.right() - 16.0, screen.top() + 14.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.set_opacity(t);
            let shadow_color = if theme::light() {
                Color32::from_black_alpha(20)
            } else {
                Color32::from_black_alpha(60)
            };
            let stroke = if theme::light() {
                egui::Stroke::new(0.5, Color32::from_black_alpha(25))
            } else {
                egui::Stroke::new(0.5, Color32::from_white_alpha(32))
            };
            let fallback = egui::Frame::new()
                .fill(theme::menu_fill())
                .corner_radius(6)
                .stroke(stroke)
                .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 8, spread: 0, color: shadow_color });
            crate::glass::surface(
                ui,
                egui::Id::unique("zoom-badge-glass"),
                egui::Margin::symmetric(10, 5),
                crate::glass::control(8.0, theme::light(), 0.0, 1.0),
                fallback,
                |ui| {
                    ui.set_min_width(44.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("{:.0}%", z * 100.0))
                                    .font(theme::semibold(12.5))
                                    .color(theme::text()),
                            )
                            .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
                },
            );
        });
    ctx.request_repaint();
}

pub fn toast(app: &mut App, ctx: &egui::Context, now: f64) {    let Some((msg, shown)) = &app.toast else { return };
    if *shown < 0.0 {
        return;
    }
    let age = (now - shown) as f32;
    let t = crate::glass::spring_bool(ctx, egui::Id::unique("toast-spring"), age < 4.1, crate::glass::Tuning::BOUNCY).clamp(0.0, 1.0);
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("toast"))
        .order(egui::Order::Tooltip)
        .pivot(Align2::CENTER_BOTTOM)
        // Above the player bar; in the mini player, over the strip's middle.
        .fixed_pos(Pos2::new(screen.center().x, if screen.height() < 200.0 { screen.center().y + 22.0 } else { screen.bottom() - 104.0 } + 14.0 * (1.0 - t)))
        .show(ctx, |ui| {
            ui.set_opacity(t);
            let font = theme::regular(14.0);
            let g = ui.fonts_mut(|f| f.layout(msg.clone(), font, theme::text(), 520.0));
            let size = g.size() + vec2(36.0, 20.0);
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let light = theme::light();
            let tint = theme::with_alpha(theme::menu_fill(), 0.28);
            let style = crate::glass::Style::regular(crate::glass::Corner::Capsule, Some(tint), if light { 0.08 } else { 0.90 }, 1.0);
            crate::glass::paint(ui, egui::Id::unique("toast-liquid-glass"), rect, style);
            ui.painter().galley(rect.min + vec2(18.0, 10.0), g, theme::text());
        });
    // Frames only while it fades in or out; in between it just waits.
    if (0.25..4.1).contains(&age) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f32(4.1 - age));
    } else {
        ctx.request_repaint();
    }
}
