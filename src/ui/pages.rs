//! Page bodies. Every page fades and slides in when navigated to.

use crate::sync::MutexExt;
use std::collections::HashSet;
use std::sync::atomic::Ordering;

use egui::{Align2, Color32, Rect, RichText, Sense, Ui, Vec2, pos2, vec2};

use super::widgets::{self, RowOpts, card, chip, h_scroll, named, paint_cover, pill, pill_destructive, shelf_header, track_row};
use super::{Action, App, LibTab, Page, Selection};
#[allow(unused_imports)]
use crate::backend::compact;
use crate::i18n::t;
use crate::model::{Card, ShelfKind, Target, Track};
use crate::theme::{self, icon};

const CARD: f32 = 180.0;

/// How many tiles Apple fits across a feed row at this content width. The site
/// measures from its own viewport, which includes the navigation column this page
/// already sits past.
fn feed_columns(width: f32, square: bool) -> usize {
    let viewport = width + 260.0;
    match square {
        true if viewport < 1260.0 => 5,
        true if viewport < 1580.0 => 6,
        true => 7,
        false if viewport < 1260.0 => 2,
        false if viewport < 1580.0 => 3,
        false => 4,
    }
}

/// One tile of an Apple feed row: a share of the content on wide windows, the site's
/// own narrow width below its breakpoint.
fn feed_tile(ui: &Ui, square: bool) -> f32 {
    let width = ui.available_width();
    let min = if square { 144.0 } else { 270.0 };
    if width < 1024.0 {
        return min;
    }
    let cols = feed_columns(width, square);
    ((width - 20.0 * (cols - 1) as f32) / cols as f32).max(min)
}

pub fn page(app: &mut App, ui: &mut Ui, now: f64) {
    let t = theme::ease_out(((now - app.page_changed) / 0.32) as f32);
    if t < 1.0 {
        // The fade-in finishes even when nothing else is drawing frames.
        super::animate(ui.ctx());
    }
    ui.set_opacity(0.15 + 0.85 * t);
    let page = app.page.clone();
    // Start at the top when navigated to; don't restore previous scroll positions across visits.
    let mut area = egui::ScrollArea::vertical().id_salt(("page", &page, app.page_visit)).auto_shrink(false);
    if std::mem::take(&mut app.scroll_top) || t < 1.0 {
        area = area.vertical_scroll_offset(0.0);
    }
    // The scroll bar hugs the right edge of the pages.
    ui.spacing_mut().scroll.bar_outer_margin = 2.0 + super::titlebar::edge_inset();
    // Dev hook: YTM_SCROLL=<px> pins the page scroll, for screenshots of lower sections.
    if let Some(y) = super::dev_var("YTM_SCROLL").and_then(|v| v.parse::<f32>().ok()) {
        area = area.vertical_scroll_offset(y);
    }
    let out = area.show(ui, |ui| {
        let is_artist = matches!(page, Page::Artist(_));
        if !is_artist {
            // The page's own 28 pt right gap, plus room for the scroll bar beside it.
            ui.set_max_width((ui.available_width() - 48.0).max(0.0));
            ui.add_space(18.0 * (1.0 - t));
        }
        match &page {
            Page::Home => home(app, ui, now),
            Page::New => new(app, ui, now),
            Page::Search(q) => search(app, ui, q, now),
            Page::Album(id) | Page::Playlist(id) => collection(app, ui, id, now),
            Page::Artist(id) => artist(app, ui, id, now),
            Page::Library(tab) => library(app, ui, *tab, now),
            Page::Downloads => downloads(app, ui, now),
            Page::Settings => settings(app, ui),
            Page::Transfer => super::transfer::page(app, ui, now),
            Page::Jam => super::jam::page(app, ui),
        }
        // Room to scroll the last rows clear of the player's frosted strip.
        ui.add_space(if page == Page::Settings { 24.0 } else { 96.0 });
    });
    selection_bar(app, ui.ctx(), &page, out.inner_rect);
    // Infinite scroll on home: fetch the next shelves when nearing the bottom.
    if page == Page::Home {
        let remaining = out.content_size.y - (out.state.clamped_offset().y + out.inner_rect.height());
        if let Some(h) = &mut app.home {
            // A failed fetch reports nothing (it's a quiet request): allow a retry later.
            if h.loading_more.is_some_and(|t| now - t > 20.0) {
                h.loading_more = None;
            }
            if remaining < 700.0 && h.loading_more.is_none()
                && let Some(token) = h.continuation.clone()
            {
                h.loading_more = Some(now);
                app.backend.request(crate::backend::Req::HomeMore(token));
            }
        }
    }
}

/// What every song row needs from the app, gathered once per frame.
pub(super) struct Rows {
    current: Option<String>,
    playing: bool,
    accent: Color32,
    now: f64,
}

impl Rows {
    pub(super) fn new(app: &App, now: f64) -> Self {
        Self {
            current: app.current_id().map(str::to_owned),
            playing: app.audio.status.playing.load(Ordering::Relaxed),
            accent: app.accent_color(),
            now,
        }
    }

    pub(super) fn opts(&self, liked: &HashSet<String>, t: &Track, number: Option<usize>, show_album: bool) -> RowOpts {
        RowOpts {
            number,
            show_album,
            show_plays: false,
            is_album: number.is_some() && !show_album,
            current: self.current.as_deref() == Some(t.id.as_str()),
            playing: self.playing,
            accent: self.accent,
            time: self.now,
            liked: liked.contains(&t.id),
            added_by: None,
            pick: false,
            selected: None,
            remove: None,
            remove_queue: None,
            is_dragged: false,
            draggable: false,
            alt: false,
            plain_text: false,
        }
    }
}

fn check_failed(app: &mut App, ui: &mut Ui) -> bool {
    if app.load_failed {
        ui.add_space(40.0);
        ui.label(RichText::new("Couldn't load this page").color(theme::text_dim()));
        ui.add_space(12.0);
        if pill(ui, "Try again", None, false).clicked() {
            app.refresh();
        }
        true
    } else {
        false
    }
}

/// A page's data hasn't arrived: pulsing skeleton placeholders while it loads, a retry if the
/// request failed.
fn placeholder(app: &mut App, ui: &mut Ui, now: f64, rows: usize, card: f32) {
    if check_failed(app, ui) {
        return;
    }
    widgets::skeleton(ui, now, rows, card);
}

/// Album or playlist loading skeleton: cover art on the left, collection metadata and action
/// buttons on the right, followed by track rows.
fn placeholder_collection(app: &mut App, ui: &mut Ui, now: f64) {
    if check_failed(app, ui) {
        return;
    }
    let skeleton = |ui: &Ui, r: Rect, radius: f32| {
        widgets::skeleton_rect(ui, r, radius, now);
    };
    let side = (ui.available_width() * 0.3).clamp(200.0, 300.0);
    ui.add_space(18.0);
    ui.horizontal_top(|ui| {
        let (img, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
        let radius = theme::card_radius() + 2.0;
        skeleton(ui, img, radius);

        ui.add_space(28.0);
        ui.vertical(|ui| {
            ui.add_space((side - 200.0).max(6.0));
            let title_w = (ui.available_width() * 0.55).clamp(200.0, 380.0);
            let (title_r, _) = ui.allocate_exact_size(vec2(title_w, 32.0), Sense::hover());
            skeleton(ui, title_r, 6.0);

            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                let (avatar_r, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                skeleton(ui, avatar_r, 17.0);

                let (name_r, _) = ui.allocate_exact_size(vec2(160.0, 18.0), Sense::hover());
                skeleton(ui, name_r, 4.0);
            });

            ui.add_space(12.0);
            let (meta_r, _) = ui.allocate_exact_size(vec2(180.0, 14.0), Sense::hover());
            skeleton(ui, meta_r, 4.0);

            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                let (play_r, _) = ui.allocate_exact_size(vec2(86.0, 36.0), Sense::hover());
                skeleton(ui, play_r, 18.0);

                let (shuf_r, _) = ui.allocate_exact_size(vec2(98.0, 36.0), Sense::hover());
                skeleton(ui, shuf_r, 18.0);

                let (more_r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                skeleton(ui, more_r, 18.0);
            });
        });
    });
    ui.add_space(26.0);
    widgets::skeleton_rows(ui, now, 8);
}

/// Artist page loading skeleton: cinematic full-width hero header with artist name and action
/// buttons, followed by latest release card, top song rows, and album shelf.
fn placeholder_artist(app: &mut App, ui: &mut Ui, now: f64) {
    if check_failed(app, ui) {
        return;
    }
    let sk = |ui: &Ui, r: Rect, radius: f32| widgets::skeleton_rect(ui, r, radius, now);
    let sk_at = |ui: &Ui, r: Rect, radius: f32, offset: f64| widgets::skeleton_rect_at(ui, r, radius, now, offset);
    let w = ui.available_width();

    // ── Hero banner ───────────────────────────────────────────────────────────
    // Matches the real artist page: (w * 0.48).clamp(340, 560) px tall, fading its own
    // alpha to transparent so it dissolves cleanly into the backdrop below.
    let hero_h = (w * 0.48).clamp(340.0, 560.0);
    let (hero, _) = ui.allocate_exact_size(vec2(w, hero_h), Sense::hover());
    widgets::rect_vfade(ui.painter(), hero, widgets::skeleton_color(now), 0.38);

    // Share circle: top-right corner of the hero, matching the loaded page.
    let share_r = Rect::from_min_size(pos2(hero.right() - 54.0, hero.top() + 18.0), vec2(34.0, 34.0));
    sk(ui, share_r, 17.0);

    // ── Name + stats overlay at the base of the hero ──────────────────────────
    // stats line sits 8 px above the hero bottom; name sits 18 px above that.
    let base_x = hero.left() + 28.0;
    let stats_y  = hero.bottom() - 8.0;
    let name_bottom_y = stats_y - 18.0;
    // Name: bold(52) → represent as a tall-ish rect.
    let name_w = (w * 0.45).clamp(220.0, 400.0);
    sk(ui, Rect::from_min_max(pos2(base_x, name_bottom_y - 46.0), pos2(base_x + name_w, name_bottom_y)), 8.0);
    // Stats: semibold(13.5) → one narrow line.
    sk(ui, Rect::from_min_size(pos2(base_x, stats_y - 13.0), vec2(160.0, 13.0)), 4.0);

    // ── Button row (58 px spacer below hero, then play + subscribe pill) ──────
    let (below, _) = ui.allocate_exact_size(vec2(w, 58.0), Sense::hover());
    let btn_y = below.top() + 26.0;
    // Play circle: 48 × 48 px
    let play_center = pos2(base_x + 24.0, btn_y);
    sk(ui, Rect::from_center_size(play_center, vec2(48.0, 48.0)), 24.0);
    // Subscribe pill: typical width ~130 px, 34 px tall
    sk(ui, Rect::from_min_size(pos2(play_center.x + 24.0 + 14.0, btn_y - 17.0), vec2(130.0, 34.0)), 17.0);

    // ── Content sections ──────────────────────────────────────────────────────
    egui::Frame::new()
        .inner_margin(egui::Margin { left: 28, right: 48, top: 0, bottom: 0 })
        .show(ui, |ui| {
            let cw = ui.available_width();
            ui.add_space(26.0);
            let is_wide = cw >= 780.0;

            // Latest Release card (left) + Top Songs (right) — mirrors the real layout.
            if is_wide {
                let left_w  = (cw * 0.30).clamp(280.0, 320.0);
                let right_w = (cw - left_w - 32.0).max(360.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(vec2(left_w, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        // "Latest Release" heading skeleton
                        let (head, _) = ui.allocate_exact_size(vec2(140.0, 22.0), Sense::hover());
                        sk_at(ui, head, 4.0, 0.0);
                        ui.add_space(8.0);
                        // Card image + sub-label
                        let (card_r, _) = ui.allocate_exact_size(vec2(left_w, left_w * 0.65), Sense::hover());
                        sk_at(ui, card_r, 12.0, 0.04);
                        ui.add_space(8.0);
                        let (sub, _) = ui.allocate_exact_size(vec2(left_w * 0.7, 13.0), Sense::hover());
                        sk_at(ui, sub, 4.0, 0.08);
                    });
                    ui.add_space(24.0);
                    ui.allocate_ui_with_layout(vec2(right_w, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        // "Top Songs" heading skeleton
                        let (head, _) = ui.allocate_exact_size(vec2(120.0, 22.0), Sense::hover());
                        sk_at(ui, head, 4.0, 0.0);
                        ui.add_space(8.0);
                        widgets::skeleton_rows(ui, now, 5);
                    });
                });
            } else {
                let (head, _) = ui.allocate_exact_size(vec2(140.0, 22.0), Sense::hover());
                sk(ui, head, 4.0);
                ui.add_space(8.0);
                widgets::skeleton_rows(ui, now, 5);
            }

            // Albums & singles shelf
            ui.add_space(26.0);
            let (albums_head, _) = ui.allocate_exact_size(vec2(170.0, 22.0), Sense::hover());
            sk_at(ui, albums_head, 4.0, 0.0);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                for i in 0..5 {
                    let offset = i as f64 * 0.05;
                    let (r, _) = ui.allocate_exact_size(vec2(CARD, CARD + 46.0), Sense::hover());
                    sk_at(ui, Rect::from_min_size(r.min, Vec2::splat(CARD)), 8.0, offset);
                    sk_at(ui, Rect::from_min_size(r.min + vec2(0.0, CARD + 10.0), vec2(CARD * 0.75, 12.0)), 4.0, offset + 0.02);
                    sk_at(ui, Rect::from_min_size(r.min + vec2(0.0, CARD + 28.0), vec2(CARD * 0.45, 10.0)), 4.0, offset + 0.04);
                }
            });

            // Featured on shelf
            ui.add_space(26.0);
            let (featured_head, _) = ui.allocate_exact_size(vec2(130.0, 22.0), Sense::hover());
            sk_at(ui, featured_head, 4.0, 0.08);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                for i in 0..4 {
                    let offset = 0.08 + i as f64 * 0.05;
                    let (r, _) = ui.allocate_exact_size(vec2(CARD, CARD + 46.0), Sense::hover());
                    sk_at(ui, Rect::from_min_size(r.min, Vec2::splat(CARD)), 8.0, offset);
                    sk_at(ui, Rect::from_min_size(r.min + vec2(0.0, CARD + 10.0), vec2(CARD * 0.75, 12.0)), 4.0, offset + 0.02);
                }
            });

            // Fans also like shelf
            ui.add_space(26.0);
            let (fans_head, _) = ui.allocate_exact_size(vec2(180.0, 22.0), Sense::hover());
            sk_at(ui, fans_head, 4.0, 0.16);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                for i in 0..5 {
                    let offset = 0.16 + i as f64 * 0.05;
                    let (r, _) = ui.allocate_exact_size(vec2(CARD, CARD + 46.0), Sense::hover());
                    sk_at(ui, Rect::from_min_size(r.min, Vec2::splat(CARD)), 8.0, offset);
                    sk_at(ui, Rect::from_min_size(r.min + vec2(0.0, CARD + 10.0), vec2(CARD * 0.75, 12.0)), 4.0, offset + 0.02);
                }
            });
        });
    super::animate(ui.ctx());
}

/// Search loading skeleton: 2-row Top Results tiles, followed by Artists, Albums, and Songs.
fn placeholder_search(app: &mut App, ui: &mut Ui, now: f64) {
    if check_failed(app, ui) {
        return;
    }
    let sk = |ui: &Ui, r: Rect, radius: f32, off: f64| widgets::skeleton_rect_at(ui, r, radius, now, off);
    let w = ui.available_width();
    let cols = ((w + 14.0) / 260.0).floor().clamp(2.0, 4.0) as usize;
    let tile_w = ((w - (cols as f32 - 1.0) * 14.0) / cols as f32).max(160.0);

    ui.add_space(16.0);
    let (h, _) = ui.allocate_exact_size(vec2(130.0, 22.0), Sense::hover());
    sk(ui, h, 4.0, 0.0);
    ui.add_space(10.0);
    for row in 0..2 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            for col in 0..cols {
                let off = (row * cols + col) as f64 * 0.04;
                let (r, _) = ui.allocate_exact_size(vec2(tile_w, 64.0), Sense::hover());
                sk(ui, r, 10.0, off);
            }
        });
        ui.add_space(10.0);
    }

    ui.add_space(18.0);
    let (ah, _) = ui.allocate_exact_size(vec2(110.0, 22.0), Sense::hover());
    sk(ui, ah, 4.0, 0.08);
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        for i in 0..6 {
            let off = 0.08 + i as f64 * 0.04;
            let (r, _) = ui.allocate_exact_size(vec2(CARD, CARD + 34.0), Sense::hover());
            sk(ui, Rect::from_min_size(r.min, Vec2::splat(CARD)), CARD * 0.5, off);
            sk(ui, Rect::from_min_size(pos2(r.center().x - CARD * 0.35, r.min.y + CARD + 10.0), vec2(CARD * 0.7, 12.0)), 4.0, off);
        }
    });

    ui.add_space(22.0);
    let (alh, _) = ui.allocate_exact_size(vec2(110.0, 22.0), Sense::hover());
    sk(ui, alh, 4.0, 0.14);
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        for i in 0..6 {
            let off = 0.14 + i as f64 * 0.04;
            let (r, _) = ui.allocate_exact_size(vec2(CARD, CARD + 46.0), Sense::hover());
            sk(ui, Rect::from_min_size(r.min, Vec2::splat(CARD)), 8.0, off);
            sk(ui, Rect::from_min_size(r.min + vec2(0.0, CARD + 10.0), vec2(CARD * 0.78, 12.0)), 4.0, off);
        }
    });

    ui.add_space(22.0);
    let (sh, _) = ui.allocate_exact_size(vec2(100.0, 22.0), Sense::hover());
    sk(ui, sh, 4.0, 0.20);
    ui.add_space(8.0);
    widgets::skeleton_rows(ui, now, 3);
    super::animate(ui.ctx());
}

/// Explore / New releases page loading skeleton: new releases card shelf, top songs grid rows,
/// and charts shelf.
fn placeholder_new(app: &mut App, ui: &mut Ui, now: f64) {
    if check_failed(app, ui) {
        return;
    }
    let skeleton = |ui: &Ui, r: Rect, radius: f32| {
        widgets::skeleton_rect(ui, r, radius, now);
    };
    let (h1, _) = ui.allocate_exact_size(vec2(220.0, 24.0), Sense::hover());
    skeleton(ui, h1, 4.0);
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        for _ in 0..6 {
            let (r, _) = ui.allocate_exact_size(vec2(CARD, CARD + 50.0), Sense::hover());
            skeleton(ui, Rect::from_min_size(r.min, Vec2::splat(CARD)), 8.0);
            skeleton(ui, Rect::from_min_size(r.min + vec2(0.0, CARD + 10.0), vec2(CARD * 0.8, 12.0)), 4.0);
        }
    });
    ui.add_space(24.0);
    let (h2, _) = ui.allocate_exact_size(vec2(140.0, 24.0), Sense::hover());
    skeleton(ui, h2, 4.0);
    ui.add_space(12.0);
    widgets::skeleton_rows(ui, now, 4);
    ui.add_space(24.0);
    let (h3, _) = ui.allocate_exact_size(vec2(120.0, 24.0), Sense::hover());
    skeleton(ui, h3, 4.0);
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        for _ in 0..6 {
            let (r, _) = ui.allocate_exact_size(vec2(CARD, CARD + 50.0), Sense::hover());
            skeleton(ui, Rect::from_min_size(r.min, Vec2::splat(CARD)), 8.0);
            skeleton(ui, Rect::from_min_size(r.min + vec2(0.0, CARD + 10.0), vec2(CARD * 0.8, 12.0)), 4.0);
        }
    });
    super::animate(ui.ctx());
}

/// Track list loading skeleton (History, Liked Songs).
fn placeholder_track_list(app: &mut App, ui: &mut Ui, now: f64, with_actions: bool) {
    if check_failed(app, ui) {
        return;
    }
    let skeleton = |ui: &Ui, r: Rect, radius: f32| {
        widgets::skeleton_rect(ui, r, radius, now);
    };
    if with_actions {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
            let (p1, _) = ui.allocate_exact_size(vec2(86.0, 36.0), Sense::hover());
            skeleton(ui, p1, 18.0);
            let (p2, _) = ui.allocate_exact_size(vec2(98.0, 36.0), Sense::hover());
            skeleton(ui, p2, 18.0);
            let (p3, _) = ui.allocate_exact_size(vec2(90.0, 36.0), Sense::hover());
            skeleton(ui, p3, 18.0);
        });
        ui.add_space(10.0);
    }
    widgets::skeleton_rows(ui, now, 10);
}

/// Wrapped card grid loading skeleton (Library Playlists, Albums, Artists).
fn placeholder_grid(app: &mut App, ui: &mut Ui, now: f64, card: f32, circle: bool, count: usize, with_new_button: bool) {
    if check_failed(app, ui) {
        return;
    }
    let skeleton = |ui: &Ui, r: Rect, radius: f32| {
        widgets::skeleton_rect(ui, r, radius, now);
    };
    if with_new_button {
        let (btn_r, _) = ui.allocate_exact_size(vec2(120.0, 34.0), Sense::hover());
        skeleton(ui, btn_r, 17.0);
        ui.add_space(14.0);
    }
    widgets::skeleton_grid(ui, now, card, circle, count);
}

/// What a collection header's round buttons draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CircleGlyph {
    Char(char),
    Plus,
    More,
}

/// The accent colour the app set for this frame.
fn app_accent(ui: &Ui) -> Color32 {
    ui.ctx().data(|d| d.get_temp::<Color32>(egui::Id::unique("accent"))).unwrap_or(theme::BLUE)
}

/// "ALBUM · 2024" / "PLAYLIST · 42 SONGS · 2 HR 10 MIN": the line under the artist.
pub fn header_meta(c: &crate::backend::Collection) -> String {
    let mut parts = vec![if c.is_album { "ALBUM".to_owned() } else { "PLAYLIST".to_owned() }];
    if c.is_album {
        if let Some(y) = c.subtitle.split(" • ").map(str::trim).find(|p| p.len() == 4 && p.chars().all(|ch| ch.is_ascii_digit())) {
            parts.push(y.to_owned());
        }
    } else {
        let n = c.count.map_or(c.tracks.len() as u64, |n| n);
        parts.push(format!("{n} SONGS"));
    }
    let total: u32 = c.tracks.iter().filter_map(|t| t.duration).sum();
    if total >= 3600 {
        parts.push(format!("{} HR {} MIN", total / 3600, total % 3600 / 60));
    } else if total > 0 {
        parts.push(format!("{} MIN", total / 60));
    }
    parts.join(" · ")
}

pub fn circle_action(ui: &mut Ui, glyph: CircleGlyph, name: &str, active: bool) -> egui::Response {
    // Same 38 px height as the Play pill beside it, so the row shares one centre line.
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::click());
    let resp = named(resp, name).on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let press = crate::glass::press(ui, &resp, rect, 1.0);
    let h = if open { 1.0 } else { press.hover };
    let draw_r = press.rect;
    let light = theme::light();

    // Liquid Glass fill (with soft fallback when not on OpenGL)
    if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
        let mut style = crate::glass::control(crate::glass::Corner::Capsule, light, h, 1.0);
        press.apply(&mut style);
        crate::glass::paint(ui, resp.id.with("blur"), draw_r, style);
    } else {
        let shadow_alpha = if light { (25.0 + 10.0 * h) as u8 } else { (42.0 + 14.0 * h) as u8 };
        let r = draw_r.height() * 0.5;
        ui.painter().circle_filled(draw_r.center() + vec2(0.0, 1.5), r, Color32::from_black_alpha(shadow_alpha));
        let fill = if light {
            theme::lerp_color(Color32::from_black_alpha(15), Color32::from_black_alpha(28), h)
        } else {
            theme::glass(0.12 + 0.08 * h)
        };
        ui.painter().circle_filled(draw_r.center(), r, fill);
    }

    let color = if theme::light() {
        Color32::from_rgb(0x18, 0x14, 0x1c)
    } else {
        theme::lerp_color(Color32::from_white_alpha(225), Color32::WHITE, h.max(active as u8 as f32))
    };
    let c = draw_r.center();
    match glyph {
        CircleGlyph::Char(g) => widgets::icon_at(ui, c, g, 16.0 * press.scale, color),
        CircleGlyph::Plus => {
            let half = 5.0 * press.scale;
            let stroke = egui::Stroke::new(1.8, color);
            ui.painter().line_segment([pos2(c.x - half, c.y), pos2(c.x + half, c.y)], stroke);
            ui.painter().line_segment([pos2(c.x, c.y - half), pos2(c.x, c.y + half)], stroke);
        }
        CircleGlyph::More => {
            for i in -1..=1 {
                ui.painter().circle_filled(c + vec2(i as f32 * 5.2 * press.scale, 0.0), 1.8 * press.scale, color);
            }
        }
    }
    resp
}

/// A page's main Play button. Shows Pause while this page's album / playlist / artist
/// is playing, and toggles instead of restarting when it's already loaded.
pub fn play_pill(ui: &mut Ui, acts: &mut Vec<Action>, target: &Target, tracks: &[Track]) -> egui::Response {
    let active = widgets::active_state(ui, target);
    let (label, glyph) = if active == Some(true) { (t!("pause"), icon::PAUSE) } else { (t!("play"), icon::PLAY) };
    let resp = pill(ui, &label, Some(glyph), true);
    if resp.clicked() {
        if active.is_some() {
            acts.push(Action::TogglePlay);
        } else if !tracks.is_empty() {
            acts.push(Action::Play(tracks.to_vec(), 0));
        }
    }
    resp
}

const PEACH: Color32 = Color32::from_rgb(0xE0, 0x7A, 0x5F);
const PEACH_HOVER: Color32 = Color32::from_rgb(0xEB, 0x8B, 0x72);
const PEACH_TEXT: Color32 = Color32::from_rgb(0x1a, 0x14, 0x12);

pub fn peach_pill(ui: &mut Ui, label: &str, glyph: Option<char>) -> egui::Response {
    let font = theme::semibold(14.0);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, PEACH_TEXT);
    let icon_w = if glyph.is_some() { 20.0 } else { 0.0 };
    let w = galley.size().x + icon_w + 32.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
    let resp = named(resp, label).on_hover_cursor(egui::CursorIcon::PointingHand);
    let h = if resp.hovered() { 1.0 } else { 0.0 };
    let bg = theme::lerp_color(PEACH, PEACH_HOVER, h);
    let r = 18.0;
    let shrink = if resp.is_pointer_button_down_on() { 1.0 } else { 0.0 };
    ui.painter().rect_filled(rect.shrink(shrink), r, bg);
    let mut x = rect.left() + 16.0;
    let cy = rect.center().y;
    if let Some(g) = glyph {
        widgets::icon_at(ui, pos2(x + 4.0, cy), g, 15.0, PEACH_TEXT);
        x += icon_w;
    }
    ui.painter().galley(pos2(x, cy - galley.size().y / 2.0), galley, PEACH_TEXT);
    resp
}

pub fn peach_circle(ui: &mut Ui, name: &str, glyph: CircleGlyph) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
    let resp = named(resp, name).on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand);
    let h = if resp.hovered() { 1.0 } else { 0.0 };
    let bg = theme::lerp_color(PEACH, PEACH_HOVER, h);
    let shrink = if resp.is_pointer_button_down_on() { 1.0 } else { 0.0 };
    ui.painter().circle_filled(rect.center(), 18.0 - shrink, bg);
    match glyph {
        CircleGlyph::Char(g) => widgets::icon_at(ui, rect.center(), g, 16.0, PEACH_TEXT),
        CircleGlyph::Plus => {
            let c = rect.center();
            let half = 5.0;
            let stroke = egui::Stroke::new(1.8, PEACH_TEXT);
            ui.painter().line_segment([pos2(c.x - half, c.y), pos2(c.x + half, c.y)], stroke);
            ui.painter().line_segment([pos2(c.x, c.y - half), pos2(c.x, c.y + half)], stroke);
        }
        CircleGlyph::More => {
            widgets::paint_more_dots(ui.painter(), rect.center(), 5.0, 1.8, PEACH_TEXT);
        }
    }
    resp
}

/// A segmented capsule pill with a Share button on the left and a More menu on the right.
pub fn share_more_pill(
    ui: &mut Ui,
    pill_rect: Rect,
    id_salt: &str,
    target: &Target,
    acts: &mut Vec<Action>,
    menu: impl FnOnce(&mut Ui, &mut Vec<Action>, &Target),
) {
    let light = theme::light();
    let glass_slot = ui.painter().add(egui::Shape::Noop);

    let mid_x = pill_rect.center().x;
    let share_rect = Rect::from_min_max(pill_rect.min, pos2(mid_x, pill_rect.max.y));
    let more_rect = Rect::from_min_max(pos2(mid_x, pill_rect.min.y), pill_rect.max);

    let share_resp = named(ui.interact(share_rect, ui.scope_id().with((id_salt, "share")), Sense::click()), "Share")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Share");
    let more_resp = named(ui.interact(more_rect, ui.scope_id().with((id_salt, "more")), Sense::click()), "More")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("More");
    let more_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&more_resp));

    let press_share = crate::glass::press(ui, &share_resp, pill_rect, 0.75);
    let press_more = crate::glass::press(ui, &more_resp, pill_rect, 0.75);
    let draw_pill = if press_more.amount.abs() > press_share.amount.abs() { press_more.rect } else { press_share.rect };
    let h = press_share.hover.max(press_more.hover).max(more_open as u8 as f32);

    if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
        let mut style = crate::glass::control(crate::glass::Corner::Capsule, light, h * 0.6, 1.0);
        if press_more.amount.abs() > press_share.amount.abs() {
            press_more.apply(&mut style);
        } else {
            press_share.apply(&mut style);
        }
        crate::glass::paint_at(ui, glass_slot, ui.scope_id().with((id_salt, "pill-blur")), draw_pill, style);
    } else {
        let pill_shadow = if light { 18 } else { 35 };
        let pill_fill = if light { Color32::from_black_alpha(15) } else { theme::glass(0.12) };
        let r = draw_pill.height() * 0.5;
        ui.painter().set(glass_slot, egui::Shape::Vec(vec![
            egui::Shape::rect_filled(draw_pill.translate(vec2(0.0, 1.0)), r, Color32::from_black_alpha(pill_shadow)),
            egui::Shape::rect_filled(draw_pill, r, pill_fill),
        ]));
    }

    let share_active = share_resp.hovered() || share_resp.is_pointer_button_down_on();
    let more_active = more_resp.hovered() || more_resp.is_pointer_button_down_on() || more_open;
    let target_cx = if more_active { more_rect.center().x } else { share_rect.center().x };
    let held_side = share_resp.is_pointer_button_down_on() || more_resp.is_pointer_button_down_on();
    widgets::capsule_indicator(
        ui,
        ui.scope_id().with((id_salt, "pill-hover")),
        pill_rect,
        draw_pill,
        target_cx,
        share_active || more_active,
        held_side,
        25.0,
        27.0,
        |vis| if light { Color32::from_black_alpha((22.0 * vis) as u8) } else { Color32::from_white_alpha((34.0 * vis) as u8) },
        Some(|vis: f32| if light { Color32::from_white_alpha((90.0 * vis) as u8) } else { Color32::from_white_alpha((30.0 * vis) as u8) }),
    );

    let shift = draw_pill.center() - pill_rect.center();
    let share_fg = if light {
        Color32::from_rgb(0x18, 0x14, 0x1c)
    } else {
        theme::lerp_color(theme::lerp_color(theme::text_dim(), theme::text(), 0.5), Color32::WHITE, press_share.hover)
    };
    widgets::icon_at(ui, share_rect.center() + shift, icon::SHARE, 15.0 * press_share.scale, share_fg);

    let more_h = press_more.hover.max(more_open as u8 as f32);
    let more_fg = if light {
        Color32::from_rgb(0x18, 0x14, 0x1c)
    } else {
        theme::lerp_color(theme::lerp_color(theme::text_dim(), theme::text(), 0.5), Color32::WHITE, more_h)
    };
    for i in -1..=1 {
        ui.painter().circle_filled(more_rect.center() + shift + vec2(i as f32 * 4.5 * press_more.scale, 0.0), 1.6 * press_more.scale, more_fg);
    }

    if share_resp.clicked() {
        acts.push(Action::ShareAt(target.clone(), share_rect));
    }

    widgets::popup_menu(&more_resp, None, |ui| {
        ui.set_width(178.0);
        ui.add_space(2.0);
        menu(ui, acts, target);
        ui.add_space(3.0);
    });
}

pub fn metadata_badge(ui: &mut Ui, glyph: Option<char>, label: &str) {
    let font = theme::semibold(11.5);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, theme::text_dim());
    let icon_w = if glyph.is_some() { 16.0 } else { 0.0 };
    let badge_w = galley.size().x + icon_w + 18.0;
    let (rect, _) = ui.allocate_exact_size(vec2(badge_w, 24.0), Sense::hover());
    let r = 12.0;
    if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
        crate::glass::paint(ui, ui.scope_id().with(("meta-badge", label)), rect, crate::glass::control(crate::glass::Corner::Capsule, theme::light(), 0.0, 1.0));
    } else {
        ui.painter().rect_filled(rect, r, theme::glass(0.12));
    }
    let mut x = rect.left() + 9.0;
    let cy = rect.center().y;
    if let Some(g) = glyph {
        ui.painter().text(pos2(x, cy), Align2::LEFT_CENTER, g.to_string(), theme::regular(12.0), theme::text_dim());
        x += icon_w;
    }
    ui.painter().galley(pos2(x, cy - galley.size().y / 2.0), galley, theme::text_dim());
}

pub fn collection_genre(c: &crate::backend::Collection) -> &'static str {
    let text = format!("{} {} {}", c.title, c.subtitle, c.description.as_deref().unwrap_or_default()).to_lowercase();
    if text.contains("electro") || text.contains("synth") || text.contains("dance") || text.contains("house") || text.contains("techno") || text.contains("chromeo") {
        "ELECTRONIC"
    } else if text.contains("hip hop") || text.contains("rap") || text.contains("trap") {
        "HIP-HOP"
    } else if text.contains("r&b") || text.contains("soul") || text.contains("funk") {
        "R&B"
    } else if text.contains("rock") || text.contains("metal") || text.contains("punk") {
        "ROCK"
    } else if text.contains("jazz") {
        "JAZZ"
    } else if text.contains("classical") {
        "CLASSICAL"
    } else if text.contains("pop") {
        "POP"
    } else if c.is_album {
        "ELECTRONIC"
    } else {
        "PLAYLIST"
    }
}

pub fn paint_ambient_glow(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = pos2(rect.left() + rect.width() * 0.35, rect.top() + rect.height() * 0.32);
    let radius = rect.width().max(rect.height()) * 0.65;
    let dark_bg = Color32::from_rgb(0x11, 0x11, 0x11);
    let mut mesh = egui::Mesh::default();
    let c_center = theme::with_alpha(color, 0.45);
    let c_edge = Color32::TRANSPARENT;
    mesh.colored_vertex(center, c_center);
    const SEGMENTS: usize = 24;
    for i in 0..=SEGMENTS {
        let angle = (i as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
        let p = center + vec2(angle.cos() * radius, angle.sin() * radius * 0.7);
        mesh.colored_vertex(p, c_edge);
        if i > 0 {
            mesh.add_triangle(0, i as u32, i as u32 + 1);
        }
    }
    painter.add(egui::Shape::Mesh(mesh.into()));
    super::shell::vgradient(painter, rect, Color32::TRANSPARENT, theme::with_alpha(dark_bg, 0.94));
}

pub fn paint_hero_gradient_dissolve(painter: &egui::Painter, rect: Rect, bg: Color32) {
    const STOPS: [(f32, u8); 6] = [
        (0.00, 0),
        (0.35, 0),
        (0.55, 60),
        (0.72, 140),
        (0.88, 220),
        (1.00, 255),
    ];
    for w in STOPS.windows(2) {
        let (y0_pct, a0) = w[0];
        let (y1_pct, a1) = w[1];
        let seg = Rect::from_min_max(
            pos2(rect.left(), rect.top() + rect.height() * y0_pct),
            pos2(rect.right(), rect.top() + rect.height() * y1_pct),
        );
        let c0 = Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), a0);
        let c1 = Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), a1);
        super::shell::vgradient(painter, seg, c0, c1);
    }
}

pub fn hero_height(viewport_width: f32) -> f32 {
    (viewport_width * 0.40).clamp(260.0, 480.0)
}

pub fn filter_collection_tracks<'a>(tracks: &'a [Track], needle: &str) -> Vec<&'a Track> {
    let q = needle.trim().to_lowercase();
    if q.is_empty() {
        return tracks.iter().collect();
    }
    tracks
        .iter()
        .filter(|t| {
            t.title.to_lowercase().contains(&q)
                || t.artist_line().to_lowercase().contains(&q)
                || t.album.as_ref().is_some_and(|a| a.name.to_lowercase().contains(&q))
        })
        .collect()
}

fn card_row(ui: &mut Ui, id: &str, title: &str, cards: &[Card], acts: &mut Vec<Action>) {
    if cards.is_empty() {
        return;
    }
    let (l, r) = shelf_header(ui, id, title, None, None, true);
    let nudge = r as i32 - l as i32;
    let tile = feed_tile(ui, true);
    h_scroll(ui, id, nudge, cards.len(), tile, 20.0, |ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        for c in cards {
            card(ui, c, tile, false, acts);
        }
    });
}

/// Songs laid out in columns of `rows`, scrolling sideways (Quick picks). Clicking a
/// song starts its radio. Returns whether the arrows reached the end.
#[allow(clippy::too_many_arguments)]
fn song_grid(ui: &mut Ui, rows_ctx: &Rows, liked: &HashSet<String>, acts: &mut Vec<Action>, id: &str, title: &str, strap: Option<&str>, tracks: &[Track], rows: usize) -> bool {
    song_grid_inner(ui, rows_ctx, liked, acts, id, title, strap, tracks, rows, false)
}

#[allow(clippy::too_many_arguments)]
fn song_grid_inner(ui: &mut Ui, rows_ctx: &Rows, liked: &HashSet<String>, acts: &mut Vec<Action>, id: &str, title: &str, strap: Option<&str>, tracks: &[Track], rows: usize, pick: bool) -> bool {
    let (l, r) = shelf_header(ui, id, title, strap, None, true);
    let nudge = r as i32 - l as i32;
    let col_w = feed_tile(ui, false);
    let mut picked = None;
    let (_, reached_end) = h_scroll(ui, id, nudge, tracks.len().div_ceil(rows.max(1)), col_w, 20.0, |ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        for (c, col) in tracks.chunks(rows.max(1)).enumerate() {
            ui.allocate_ui(vec2(col_w, rows as f32 * 60.0), |ui| {
                ui.set_width(col_w);
                ui.vertical(|ui| {
                    for (i, t) in col.iter().enumerate() {
                        let o = RowOpts { pick, ..rows_ctx.opts(liked, t, None, false) };
                        if track_row(ui, t, &o, acts).clicked() {
                            picked = Some(c * rows + i);
                        }
                    }
                });
            });
        }
    });
    if let Some(i) = picked
        && let Some(t) = tracks.get(i)
    {
        acts.push(if pick { Action::TransferPick(t.clone()) } else { Action::Play(tracks.to_vec(), i) });
    }
    reached_end
}

/// Where a song list lives: the page its selection belongs to, and whether it's your own
/// playlist (so songs can be taken out of it).
struct ListCtx<'a> {
    sel: &'a mut Selection,
    page: &'a Page,
    own: bool,
    /// Show play counts even though these aren't album songs (the artist's Top Songs).
    plays: bool,
}

/// A long song list; only the rows on screen are built. Clicking one plays `tracks` from it,
/// or picks it while selecting: Ctrl-click picks one song, Shift-click a run of them.
#[allow(clippy::too_many_arguments)]
fn list(ui: &mut Ui, rows: &Rows, liked: &HashSet<String>, tracks: &[Track], show_album: bool, number: impl Fn(usize, &Track) -> Option<usize>, cx: ListCtx, acts: &mut Vec<Action>) {
    let ListCtx { sel, page, own, plays } = cx;
    let selecting = sel.on(page);
    let mods = ui.input(|i| i.modifiers);
    widgets::visible_rows(ui, tracks.len(), |ui, i| {
        let t = &tracks[i];
        let num = number(i, t);
        let alt = num.map_or(i % 2 == 0, |n| n % 2 == 1);
        let o = RowOpts {
            show_plays: plays,
            selected: selecting.then(|| sel.rows.contains(&i)),
            remove: own.then_some(i),
            alt,
            ..rows.opts(liked, t, num, show_album)
        };
        if track_row(ui, t, &o, acts).clicked() {
            if selecting || mods.command || mods.shift {
                sel.pick(page, i, mods.shift);
            } else {
                acts.push(Action::Play(tracks.to_vec(), i));
            }
        }
    });
}

/// A list page's "Select" button: turns select mode on and off.
fn select_pill(ui: &mut Ui, sel: &mut Selection, page: &Page) {
    let on = sel.on(page);
    if pill(ui, if on { "Done" } else { "Select" }, Some(icon::LIST_CHECKS), false).clicked() {
        if on { sel.clear() } else { sel.start(page) }
    }
}

/// The songs a page lists, and whether they're your own playlist's.
fn selectable<'a>(app: &'a App, page: &Page) -> Option<(&'a [Track], bool)> {
    match page {
        Page::Album(id) | Page::Playlist(id) => {
            app.opened(id).map(|o| &o.c).map(|c| (c.tracks.as_slice(), app.logged_in && c.owned && !c.is_album))
        }
        Page::Library(LibTab::Songs) => app.opened(super::LIKED_MUSIC).map(|o| (o.c.tracks.as_slice(), false)),
        Page::Library(LibTab::History) => app.history.as_deref().map(|h| (h, false)),
        _ => None,
    }
}

/// While selecting: a bar floating over the bottom of the page with what to do with the picks.
fn selection_bar(app: &mut App, ctx: &egui::Context, page: &Page, area: Rect) {
    if !app.selection.on(page) {
        return;
    }
    let Some((tracks, own)) = selectable(app, page) else { return };
    let n = tracks.len();
    let picked: Vec<Track> = app.selection.rows.iter().filter_map(|&i| tracks.get(i).cloned()).collect();
    let all = !picked.is_empty() && picked.len() == n;
    let all_liked = !picked.is_empty() && picked.iter().all(|t| app.liked.contains(&t.id));
    // Labelled buttons when there's room, icons with tooltips when the window is narrow.
    let wide = area.width() > 820.0;
    let button = |ui: &mut Ui, label: &str, glyph: char, primary: bool| {
        if wide { pill(ui, label, Some(glyph), primary) } else { widgets::icon_button(ui, glyph, 18.0, label, false) }
    };
    let mut acts = Vec::new();
    let mut toggle_all = false;
    let mut done = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Area::new(egui::Id::unique("selection-bar"))
        .order(egui::Order::Foreground)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(pos2(area.center().x, area.bottom() - 18.0))
        .show(ctx, |ui| {
            let fallback = egui::Frame::new()
                .fill(theme::menu_fill())
                .corner_radius(14)
                .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: theme::shade(0.45) });
            crate::glass::surface(
                ui,
                egui::Id::unique("selection-bar-glass"),
                egui::Margin { left: 20, right: 10, top: 8, bottom: 8 },
                crate::glass::panel(14.0, theme::light(), 1.0),
                fallback,
                |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let count = if picked.is_empty() { "Select songs".to_owned() } else { format!("{} selected", picked.len()) };
                        ui.label(RichText::new(count).font(theme::semibold(14.0)).color(theme::text()));
                        ui.add_space(10.0);
                        if !picked.is_empty() {
                            if button(ui, "Add to playlist", icon::LIST_PLUS, true).clicked() {
                                acts.push(Action::AddToPlaylist(picked.clone()));
                            }
                            if own && button(ui, "Remove from playlist", icon::LIST_MINUS, false).clicked() {
                                acts.push(Action::RemoveFromPlaylist(app.selection.rows.iter().copied().collect()));
                            }
                            if button(ui, if all_liked { "Remove like" } else { "Like" }, icon::HEART, false).clicked() {
                                acts.push(Action::LikeMany(picked.clone(), !all_liked));
                            }
                            if button(ui, "Add to queue", icon::LIST_MUSIC, false).clicked() {
                                acts.extend(picked.iter().cloned().map(Action::Enqueue));
                            }
                        }
                        toggle_all = button(ui, if all { "Select none" } else { "Select all" }, icon::LIST_CHECKS, false).clicked();
                        done |= widgets::icon_button(ui, icon::X, 18.0, "Done selecting", false).clicked();
                    });
                },
            );
        });
    if toggle_all {
        let page = page.clone();
        app.selection.start(&page);
        if !all {
            app.selection.rows = (0..n).collect();
        }
    }
    if done || !acts.is_empty() {
        app.selection.clear();
    }
    app.actions.extend(acts);
}

// ------------------------------------------------------------------ home

/// A subtitle without its "1.2M plays" / "30K views" part.
fn without_counts(s: &str) -> String {
    let count = |p: &str| {
        let p = p.trim().to_ascii_lowercase();
        ["plays", "play", "views", "view"].iter().any(|w| p.ends_with(&format!(" {w}")) && p.starts_with(|c: char| c.is_ascii_digit()))
    };
    s.split(" • ").filter(|p| !count(p)).collect::<Vec<_>>().join(" • ")
}

#[cfg(windows)]
fn local_hour() -> u32 {
    #[repr(C)]
    struct SystemTime {
        w_year: u16,
        w_month: u16,
        w_day_of_week: u16,
        w_day: u16,
        w_hour: u16,
        w_minute: u16,
        w_second: u16,
        w_milliseconds: u16,
    }
    unsafe extern "system" {
        fn GetLocalTime(st: *mut SystemTime);
    }
    let mut st: SystemTime = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    st.w_hour as u32
}

#[cfg(unix)]
fn local_hour() -> u32 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    let time_t = now as libc::time_t;
    unsafe {
        if !libc::localtime_r(&time_t, tm.as_mut_ptr()).is_null() {
            tm.assume_init().tm_hour as u32
        } else {
            12
        }
    }
}

#[cfg(not(any(windows, unix)))]
fn local_hour() -> u32 {
    12
}

fn home_greeting() -> &'static str {
    match local_hour() {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

fn home(app: &mut App, ui: &mut Ui, now: f64) {
    let active_mood = app.home.as_ref().and_then(|h| h.chips.iter().find(|c| c.selected).map(|c| c.title.clone()));
    let greeting;
    let title = if let Some(m) = &active_mood {
        m.as_str()
    } else {
        greeting = home_greeting();
        greeting
    };
    page_title(ui, title);
    if app.spotify_mode && app.spotify_user.is_none() {
        ui.add_space(10.0);
        ui.label(RichText::new("Sign in to Spotify in Settings to see your Spotify playlists, saved albums, liked songs and mixes.").color(theme::text_dim()));
        ui.add_space(12.0);
        if pill(ui, "Open Settings", None, true).clicked() {
            app.actions.push(Action::Go(Page::Settings));
        }
        return;
    }
    if let Some(h) = &mut app.home
        && !h.chips.is_empty()
    {
        ui.add_space(8.0);
        let mut clicked_chip = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for c in &h.chips {
                if chip(ui, &c.title, c.selected).clicked() {
                    let new_selected = !c.selected;
                    let param = if new_selected { c.params.clone() } else { None };
                    clicked_chip = Some((c.title.clone(), param, new_selected));
                }
            }
        });
        if let Some((title, param, new_selected)) = clicked_chip {
            for c in &mut h.chips {
                c.selected = if new_selected { c.title == title } else { false };
            }
            app.actions.push(Action::HomeChip(param));
        }
        ui.add_space(8.0);
    }
    // A refresh keeps showing the current feed; the skeleton is only for an empty one.
    if app.home.as_ref().is_none_or(|h| app.home_loading && h.shelves.is_empty()) {
        placeholder(app, ui, now, 3, CARD);
        return;
    }
    let rows = Rows::new(app, now);
    let Some(h) = &app.home else { return };
    if h.shelves.is_empty() {
        ui.add_space(40.0);
        ui.label(RichText::new("Nothing here yet").color(theme::text_dim()));
    }
    let is_mood = active_mood.is_some();
    let mut more_picks = None;
    for (i, s) in h.shelves.iter().enumerate() {
        if is_mood {
            let lower = s.title.to_lowercase();
            if lower.contains("listen again")
                || lower.contains("quick picks")
                || lower.contains("forgotten favorites")
                || lower.contains("mixed for you")
                || lower.contains("podcast")
                || lower.contains("episode")
            {
                continue;
            }
        }
        let id = format!("home-{i}-{}-{}", s.title, app.page_visit);
        match &s.kind {
            ShelfKind::Cards(cards) => {
                let cards: Vec<&Card> = cards
                    .iter()
                    .filter(|c| match &c.target {
                        Target::Album(cid) | Target::Playlist(cid) | Target::Artist(cid) => !app.suggest_less.contains(cid),
                        Target::Song(t) => !app.suggest_less.contains(&t.id),
                    })
                    .collect();
                if cards.is_empty() {
                    continue;
                }
                let (strap, thumb) = if s.title == "Listen again" { (None, None) } else { (s.strapline.as_deref(), s.strap_thumb.as_deref()) };
                let (l, r) = shelf_header(ui, &id, &s.title, strap, thumb, true);
                let nudge = r as i32 - l as i32;
                let tile = if i == 0 { (feed_tile(ui, true) * 1.12).min(240.0) } else { feed_tile(ui, true) };
                let acts = &mut app.actions;
                let liked = &app.liked;
                h_scroll(ui, &id, nudge, cards.len(), tile, 20.0, |ui| {
                    ui.spacing_mut().item_spacing.x = 20.0;
                    for c in &cards {
                        let is_liked = matches!(&c.target, Target::Song(t) if liked.contains(&t.id));
                        // Home shows no play or view counts.
                        let c = Card { subtitle: without_counts(&c.subtitle), ..(*c).clone() };
                        card(ui, &c, tile, is_liked, acts);
                    }
                });
            }
            ShelfKind::Songs { tracks, rows: n } => {
                let shown: Vec<Track> = tracks.iter().map(|t| Track { plays: None, ..t.clone() }).collect();
                let reached_end = song_grid(ui, &rows, &app.liked, &mut app.actions, &id, &s.title, s.strapline.as_deref(), &shown, *n);
                if !is_mood && s.title == "Quick picks" && reached_end && !h.picks_loading && !h.picks_expanded {
                    more_picks = tracks.last().map(|t| t.id.clone());
                }
            }
        }
    }
    let loading_more = h.loading_more.is_some();
    if let Some(seed) = more_picks
        && let Some(h) = &mut app.home
    {
        h.picks_loading = true;
        app.backend.request(crate::backend::Req::MoreQuickPicks(seed));
    }
    if loading_more {
        widgets::skeleton(ui, now, 1, CARD);
    }
}

// ------------------------------------------------------------------ new

fn new(app: &mut App, ui: &mut Ui, now: f64) {
    page_title(ui, "New");
    if app.explore.is_none() {
        placeholder_new(app, ui, now);
        return;
    }
    let rows = Rows::new(app, now);
    let Some(e) = &app.explore else { return };
    let id_new = format!("ex-new-{}", app.page_visit);
    card_row(ui, &id_new, if app.logged_in { "New releases for you" } else { "Popular new releases" }, &e.new_albums, &mut app.actions);
    if !e.top.is_empty() {
        let id_top = format!("ex-top-{}", app.page_visit);
        song_grid(ui, &rows, &app.liked, &mut app.actions, &id_top, "Top songs", None, &e.top, 4);
    }
    let id_charts = format!("ex-charts-{}", app.page_visit);
    card_row(ui, &id_charts, "Charts", &e.charts, &mut app.actions);
}

pub(super) fn page_title(ui: &mut Ui, title: &str) {
    // Left-aligned with the page's content, a little below the top edge.
    ui.add_space(18.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::hover());
    ui.painter().text(pos2(rect.left(), rect.center().y), Align2::LEFT_CENTER, title, theme::bold(34.0), theme::text());
}

// ------------------------------------------------------------------ search

fn search(app: &mut App, ui: &mut Ui, q: &str, now: f64) {
    let pick = super::transfer::pick_banner(app, ui);
    // Coming from a suggestion, category, or link, keep the box synced on route changes.
    if app.search_synced != q {
        app.search_synced = q.to_owned();
        app.search_text = q.trim().to_owned();
    }

    // Centered pill search bar at the top of the page.
    let w = ui.available_width();
    let bar_w = (w * 0.56).clamp(300.0, 520.0).min(w);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(((w - bar_w) * 0.5).max(0.0));
        super::shell::search_field(app, ui, bar_w);
    });

    // Typeahead suggestion chips directly beneath the bar while focused and typing.
    let typed = app.search_text.trim().to_owned();
    let show_chips = !typed.is_empty() && (app.search_focused || app.forget_held) && app.suggestions.0 == typed && !app.suggestions.1.is_empty();
    if show_chips {
        ui.add_space(8.0);
        let terms: Vec<_> = app.suggestions.1.iter().take(6).cloned().collect();
        let mut clicked_term: Option<String> = None;
        let mut forgot_token: Option<String> = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            for (i, s) in terms.into_iter().enumerate() {
                let selected = app.suggest_sel == Some(i);
                let g = widgets::galley(ui, &s.text, theme::regular(12.5), theme::text(), 220.0);
                let icon_w = 18.0;
                let forget_w = if s.forget.is_some() { 20.0 } else { 0.0 };
                let chip_w = icon_w + g.size().x + forget_w + 20.0;
                let (rect, resp) = ui.allocate_exact_size(vec2(chip_w, 28.0), Sense::click());
                let resp = named(resp, &s.text).on_hover_cursor(egui::CursorIcon::PointingHand);
                let h = if resp.hovered() || selected { 1.0 } else { 0.0 };
                let fill = if selected {
                    theme::with_alpha(app.accent_color(), 0.28)
                } else {
                    theme::glass(0.08 + 0.07 * h)
                };
                ui.painter().rect_filled(rect, 14.0, fill);
                let glyph = if s.forget.is_some() { icon::HISTORY } else { icon::SEARCH };
                widgets::icon_at(ui, pos2(rect.left() + 14.0, rect.center().y), glyph, 12.5, theme::text_dim());
                ui.painter().galley(pos2(rect.left() + 26.0, rect.center().y - g.size().y * 0.5), g, theme::text());
                if let Some(tok) = s.forget {
                    let xr = Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), Vec2::splat(18.0));
                    let xresp = named(ui.interact(xr, resp.id.with("forget"), Sense::click()), "Remove").on_hover_cursor(egui::CursorIcon::PointingHand);
                    widgets::icon_at(ui, xr.center(), icon::X, 10.5, if xresp.hovered() { theme::text() } else { theme::text_faint() });
                    if xresp.clicked() && !app.forget_held {
                        app.forget_held = true;
                        forgot_token = Some(tok);
                    }
                }
                if resp.clicked() {
                    clicked_term = Some(s.text);
                }
            }
        });
        if let Some(tok) = forgot_token {
            app.backend.request(crate::backend::Req::ForgetSearch(tok.clone()));
            app.suggestions.1.retain(|s| s.forget.as_deref() != Some(tok.as_str()));
        }
        if let Some(term) = clicked_term {
            app.search_text = term.clone();
            app.suggestions = Default::default();
            app.suggest_sel = None;
            app.search_focused = false;
            app.live_search_due = None;
            app.actions.push(Action::Go(Page::Search(term)));
        }
    }

    if q.trim().is_empty() {
        ui.add_space(22.0);
        recent_searches(ui, app);
        browse_categories(app, ui);
        return;
    }

    if app.search.as_ref().is_none_or(|r| r.query != q) {
        ui.add_space(10.0);
        placeholder_search(app, ui, now);
        return;
    }
    let rows = Rows::new(app, now);
    let Some(r) = &app.search else { return };
    if r.tracks.is_empty() && r.artists.is_empty() && r.albums.is_empty() && r.playlists.is_empty() {
        ui.add_space(64.0);
        ui.vertical_centered(|ui| {
            let (ic, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
            widgets::icon_at(ui, ic.center(), icon::SEARCH, 32.0, theme::text_faint());
            ui.add_space(10.0);
            ui.label(RichText::new("No Results").font(theme::bold(20.0)).color(theme::text()));
            ui.add_space(4.0);
            ui.label(RichText::new(format!("No results found for “{}”.", q.trim())).font(theme::regular(13.5)).color(theme::text_dim()));
        });
        return;
    }

    // 1. Top Results (2-row × up to 4-column grid of compact horizontal tiles)
    ui.add_space(20.0);
    let top_cards = build_top_results(q, r);
    if !top_cards.is_empty() {
        top_results_grid(ui, &top_cards, &app.liked, pick, &mut app.actions);
    }

    // 2. Artists
    card_row(ui, &format!("s-artists-{}", app.page_visit), "Artists", &r.artists, &mut app.actions);
    // 3. Albums
    card_row(ui, &format!("s-albums-{}", app.page_visit), "Albums", &r.albums, &mut app.actions);
    // 4. Songs (multi-column 3-row compact song grid)
    if !r.tracks.is_empty() {
        let id_songs = format!("s-songs-{}", app.page_visit);
        song_grid_inner(ui, &rows, &app.liked, &mut app.actions, &id_songs, "Songs", None, &r.tracks, 3, pick);
    }
    // 5. Playlists
    card_row(ui, &format!("s-pl-{}", app.page_visit), "Playlists", &r.playlists, &mut app.actions);
}

/// Search landing: Recently Searched horizontal cards with Clear button.
fn recent_searches(ui: &mut Ui, app: &mut App) {
    if app.recent_searches.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Recently Searched").font(theme::bold(20.0)).color(theme::text()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let clear_g = widgets::galley(ui, "Clear", theme::semibold(13.5), app.accent_color(), 80.0);
            let (cr, cresp) = ui.allocate_exact_size(clear_g.size() + vec2(8.0, 4.0), Sense::click());
            let cresp = named(cresp, "Clear recent searches").on_hover_cursor(egui::CursorIcon::PointingHand);
            let col = if cresp.hovered() { theme::lerp_color(app.accent_color(), theme::text(), 0.2) } else { app.accent_color() };
            ui.painter().galley(cr.min + vec2(4.0, 2.0), clear_g, col);
            if cresp.clicked() {
                app.recent_searches.clear();
            }
        });
    });
    ui.add_space(12.0);

    let recents = app.recent_searches.clone();
    let mut remove = None;
    let liked = app.liked.clone();
    let acts = &mut app.actions;
    egui::ScrollArea::horizontal().id_salt("recent-searches-hscroll").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            for (i, r) in recents.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(242.0, 64.0), Sense::click());
                let resp = named(resp, &r.title).on_hover_cursor(egui::CursorIcon::PointingHand);
                let h = if resp.hovered() { 1.0 } else { 0.0 };
                let bg = theme::glass(0.08 + 0.06 * h);
                ui.painter().rect_filled(rect, 10.0, bg);
                ui.painter().rect_stroke(rect, 10.0, egui::Stroke::new(0.8, theme::glass(0.06 + 0.06 * h)), egui::StrokeKind::Inside);

                let art = Rect::from_min_size(rect.left_top() + vec2(8.0, 8.0), Vec2::splat(48.0));
                let radius = if r.round { 24.0 } else { 6.0 };
                match r.thumb.as_deref().filter(|u| !u.is_empty()) {
                    Some(url) => {
                        let thumbs = [crate::model::Thumb { url: url.to_owned(), width: 300, height: 300 }];
                        paint_cover(ui, art, &thumbs, radius, 1.0);
                    }
                    None => {
                        ui.painter().rect_filled(art, radius, theme::glass(0.10));
                        widgets::icon_at(ui, art.center(), icon::MUSIC, 18.0, theme::text_faint());
                    }
                }
                let x = art.right() + 12.0;
                let more_r = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(24.0));
                let text_w = (more_r.left() - x - 4.0).max(40.0);
                widgets::text_at(ui, pos2(x, rect.center().y - 16.0), &r.title, theme::semibold(13.5), theme::text(), text_w);
                widgets::text_at(ui, pos2(x, rect.center().y + 3.0), &r.subtitle, theme::regular(12.0), theme::text_dim(), text_w);

                let more_resp = named(ui.interact(more_r, resp.id.with("more"), Sense::click()), "More").on_hover_cursor(egui::CursorIcon::PointingHand);
                widgets::paint_more_dots(ui.painter(), more_r.center(), 3.6, 1.3, if more_resp.hovered() { theme::text() } else { theme::text_dim() });
                widgets::popup_menu(&more_resp, None, |ui| {
                    ui.set_width(196.0);
                    ui.add_space(2.0);
                    if let Some(Target::Song(t)) = &r.target {
                        let is_liked = liked.contains(&t.id);
                        widgets::song_actions(ui, t, is_liked, None, None, acts);
                        widgets::menu_separator(ui);
                    }
                    if widgets::menu_action(ui, None, "Remove from Recently Searched") {
                        remove = Some(i);
                    }
                    ui.add_space(2.0);
                });
                widgets::context_menu(&resp, |ui| {
                    ui.set_width(196.0);
                    ui.add_space(2.0);
                    if widgets::menu_action(ui, None, "Remove from Recently Searched") {
                        remove = Some(i);
                    }
                    ui.add_space(2.0);
                });

                if resp.clicked() {
                    acts.push(match &r.target {
                        Some(t) => Action::Open(t.clone()),
                        None => Action::Go(Page::Search(r.query.clone())),
                    });
                }
            }
        });
    });
    if let Some(i) = remove {
        app.recent_searches.remove(i);
    }
    ui.add_space(26.0);
}

/// Paints a smooth rounded diagonal gradient rectangle without corner clipping artifacts.
fn paint_rounded_gradient(painter: &egui::Painter, rect: Rect, radius: f32, c1: Color32, c2: Color32) {
    let r = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let sample = |p: egui::Pos2| -> Color32 {
        let tx = ((p.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
        let ty = ((p.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        theme::lerp_color(c1, c2, (tx * 0.65 + ty * 0.35).clamp(0.0, 1.0))
    };
    let mut mesh = egui::Mesh::default();
    let center = rect.center();
    mesh.vertices.push(egui::epaint::Vertex { pos: center, uv: egui::epaint::WHITE_UV, color: sample(center) });

    let corners = [
        (pos2(rect.right() - r, rect.bottom() - r), 0.0f32, std::f32::consts::FRAC_PI_2),
        (pos2(rect.left() + r, rect.bottom() - r), std::f32::consts::FRAC_PI_2, std::f32::consts::PI),
        (pos2(rect.left() + r, rect.top() + r), std::f32::consts::PI, 3.0 * std::f32::consts::FRAC_PI_2),
        (pos2(rect.right() - r, rect.top() + r), 3.0 * std::f32::consts::FRAC_PI_2, std::f32::consts::TAU),
    ];
    const SEG: usize = 6;
    for (cc, a0, a1) in corners {
        for s in 0..=SEG {
            let a = egui::lerp(a0..=a1, s as f32 / SEG as f32);
            let p = pos2(cc.x + r * a.cos(), cc.y + r * a.sin());
            mesh.vertices.push(egui::epaint::Vertex { pos: p, uv: egui::epaint::WHITE_UV, color: sample(p) });
        }
    }
    let rim_count = mesh.vertices.len() as u32 - 1;
    for i in 1..rim_count {
        mesh.indices.extend_from_slice(&[0, i, i + 1]);
    }
    mesh.indices.extend_from_slice(&[0, rim_count, 1]);
    painter.add(egui::Shape::mesh(mesh));
}

/// Search landing: responsive grid of vibrant category cards matching Apple Music.
fn browse_categories(app: &mut App, ui: &mut Ui) {
    struct Cat {
        title: &'static str,
        query: Option<&'static str>,
        c1: Color32,
        c2: Color32,
        glyph: char,
    }
    const CATS: &[Cat] = &[
        Cat { title: "Charts", query: None, c1: Color32::from_rgb(0x4f, 0x5d, 0x1e), c2: Color32::from_rgb(0x9b, 0xa8, 0x2b), glyph: icon::CROWN },
        Cat { title: "Pop", query: Some("Pop hits"), c1: Color32::from_rgb(0xd8, 0x1b, 0x60), c2: Color32::from_rgb(0xff, 0x5c, 0x8d), glyph: icon::MUSIC },
        Cat { title: "Hip-Hop/Rap", query: Some("Hip-Hop Rap"), c1: Color32::from_rgb(0x1e, 0x74, 0xd2), c2: Color32::from_rgb(0x4f, 0xac, 0xfe), glyph: icon::HEADPHONES },
        Cat { title: "Alternative", query: Some("Alternative rock"), c1: Color32::from_rgb(0xb2, 0xbf, 0x10), c2: Color32::from_rgb(0xde, 0xe8, 0x2a), glyph: icon::RADIO },
        Cat { title: "K-Pop", query: Some("K-Pop"), c1: Color32::from_rgb(0xe8, 0x43, 0x76), c2: Color32::from_rgb(0xff, 0x75, 0x9b), glyph: icon::SPARKLES },
        Cat { title: "J-Rock", query: Some("J-Rock"), c1: Color32::from_rgb(0xdf, 0x5e, 0x1b), c2: Color32::from_rgb(0xff, 0x90, 0x45), glyph: icon::DISC_3 },
        Cat { title: "Rock", query: Some("Rock classics"), c1: Color32::from_rgb(0x9a, 0x18, 0x29), c2: Color32::from_rgb(0xe0, 0x31, 0x49), glyph: icon::MUSIC },
        Cat { title: "R&B/Soul", query: Some("R&B Soul"), c1: Color32::from_rgb(0x5a, 0x32, 0xb8), c2: Color32::from_rgb(0x95, 0x61, 0xf2), glyph: icon::HEART },
        Cat { title: "Electronic", query: Some("Electronic dance"), c1: Color32::from_rgb(0x0b, 0x84, 0x77), c2: Color32::from_rgb(0x28, 0xc7, 0xb0), glyph: icon::RADIO },
        Cat { title: "Indie", query: Some("Indie music"), c1: Color32::from_rgb(0x2d, 0x7a, 0x3e), c2: Color32::from_rgb(0x4c, 0xaf, 0x64), glyph: icon::DISC_3 },
        Cat { title: "Chill", query: Some("Chill lo-fi"), c1: Color32::from_rgb(0xd4, 0x5d, 0x26), c2: Color32::from_rgb(0x3a, 0xa8, 0xd8), glyph: icon::HEADPHONES },
        Cat { title: "Workout", query: Some("Workout energy"), c1: Color32::from_rgb(0xd9, 0x1e, 0x3b), c2: Color32::from_rgb(0xff, 0x52, 0x52), glyph: icon::SPARKLES },
        Cat { title: "Focus", query: Some("Focus instrumental"), c1: Color32::from_rgb(0x1b, 0x4b, 0x7a), c2: Color32::from_rgb(0x3b, 0x82, 0xb6), glyph: icon::HEADPHONES },
        Cat { title: "Jazz", query: Some("Jazz"), c1: Color32::from_rgb(0x8c, 0x5a, 0x1c), c2: Color32::from_rgb(0xc9, 0x91, 0x3b), glyph: icon::MUSIC },
        Cat { title: "Classical", query: Some("Classical music"), c1: Color32::from_rgb(0x3c, 0x42, 0x62), c2: Color32::from_rgb(0x6c, 0x75, 0xa0), glyph: icon::MUSIC },
    ];

    ui.label(RichText::new("Browse Categories").font(theme::bold(20.0)).color(theme::text()));
    ui.add_space(12.0);
    let w = ui.available_width();
    let gap = 16.0;
    let cols = ((w + gap) / 215.0).floor().clamp(2.0, 5.0) as usize;
    let tile_w = ((w - (cols as f32 - 1.0) * gap) / cols as f32).max(130.0);
    let tile_h = (tile_w * 0.56).clamp(92.0, 122.0);

    for chunk in CATS.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for cat in chunk {
                let (rect, resp) = ui.allocate_exact_size(vec2(tile_w, tile_h), Sense::click());
                let resp = named(resp, cat.title).on_hover_cursor(egui::CursorIcon::PointingHand);
                let press = crate::glass::press(ui, &resp, rect, 0.55);
                let draw_r = press.rect;
                let h = press.hover;

                paint_rounded_gradient(ui.painter(), draw_r, 11.0, cat.c1, cat.c2);
                if h > 0.01 {
                    ui.painter().rect_filled(draw_r, 11.0, Color32::from_white_alpha((24.0 * h) as u8));
                }
                ui.painter().rect_stroke(draw_r, 11.0, egui::Stroke::new(1.0, Color32::from_white_alpha(26)), egui::StrokeKind::Inside);

                // Subtle decorative watermark glyph on the right side
                let wm_pos = pos2(draw_r.right() - 28.0, draw_r.center().y - 4.0);
                widgets::icon_at(ui, wm_pos, cat.glyph, 42.0 * press.scale, Color32::from_white_alpha(44));

                let label_pos = pos2(draw_r.left() + 14.0, draw_r.bottom() - 12.0);
                ui.painter().text(label_pos + vec2(0.0, 1.0), Align2::LEFT_BOTTOM, cat.title, theme::bold(14.5), Color32::from_black_alpha(65));
                ui.painter().text(label_pos, Align2::LEFT_BOTTOM, cat.title, theme::bold(14.5), Color32::WHITE);

                if resp.clicked() {
                    if let Some(q) = cat.query {
                        app.actions.push(Action::Go(Page::Search(q.to_owned())));
                    } else {
                        app.actions.push(Action::Go(Page::New));
                    }
                }
            }
        });
        ui.add_space(gap);
    }
}

/// Assemble up to 8 balanced Top Results (artist + songs + albums + playlists) matching the query.
fn build_top_results(q: &str, r: &crate::backend::SearchResults) -> Vec<Card> {
    let mut out: Vec<Card> = Vec::with_capacity(8);
    let mut seen: HashSet<String> = HashSet::new();
    let push = |out: &mut Vec<Card>, seen: &mut HashSet<String>, c: Card| {
        let key = match &c.target {
            Target::Song(t) => format!("s:{}", t.id),
            Target::Artist(id) => format!("ar:{id}"),
            Target::Album(id) => format!("al:{id}"),
            Target::Playlist(id) => format!("pl:{id}"),
        };
        if seen.insert(key) {
            out.push(c);
        }
    };
    let song_card = |t: &Track| Card {
        title: t.title.clone(),
        subtitle: format!("Song · {}", t.artist_line()),
        thumbs: t.thumbs.clone(),
        target: Target::Song(t.clone()),
        round: false,
        links: t.artists.clone(),
    };

    if let Some(top_ar) = r.artists.first().filter(|a| a.title.eq_ignore_ascii_case(q.trim())) {
        push(&mut out, &mut seen, top_ar.clone());
    }
    let (mut si, mut ali, mut ari, mut pli) = (0usize, 0usize, 0usize, 0usize);
    while out.len() < 8 {
        let before = out.len();
        if let Some(t) = r.tracks.get(si) {
            push(&mut out, &mut seen, song_card(t));
            si += 1;
            if out.len() >= 8 { break; }
        }
        if let Some(ar) = r.artists.get(ari) {
            push(&mut out, &mut seen, ar.clone());
            ari += 1;
            if out.len() >= 8 { break; }
        }
        if let Some(t) = r.tracks.get(si) {
            push(&mut out, &mut seen, song_card(t));
            si += 1;
            if out.len() >= 8 { break; }
        }
        if let Some(al) = r.albums.get(ali) {
            push(&mut out, &mut seen, al.clone());
            ali += 1;
            if out.len() >= 8 { break; }
        }
        if let Some(pl) = r.playlists.get(pli) {
            push(&mut out, &mut seen, pl.clone());
            pli += 1;
        }
        if out.len() == before {
            break;
        }
    }
    out
}

/// 2-row × up to 4-column grid of compact horizontal Top Result tiles matching Screenshot 2.
fn top_results_grid(ui: &mut Ui, cards: &[Card], liked: &HashSet<String>, pick: bool, acts: &mut Vec<Action>) {
    ui.label(RichText::new("Top Results").font(theme::bold(20.0)).color(theme::text()));
    ui.add_space(10.0);
    let w = ui.available_width();
    let gap = 14.0;
    let cols = ((w + gap) / 260.0).floor().clamp(2.0, 4.0) as usize;
    let tile_w = ((w - (cols as f32 - 1.0) * gap) / cols as f32).max(160.0);
    let max_items = (cols * 2).min(cards.len());

    for row_cards in cards[..max_items].chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for c in row_cards {
                let (rect, resp) = ui.allocate_exact_size(vec2(tile_w, 64.0), Sense::click());
                let resp = named(resp, &c.title).on_hover_cursor(egui::CursorIcon::PointingHand);
                let h = if resp.hovered() { 1.0 } else { 0.0 };
                ui.painter().rect_filled(rect, 10.0, theme::glass(0.08 + 0.06 * h));
                ui.painter().rect_stroke(rect, 10.0, egui::Stroke::new(0.8, theme::glass(0.06 + 0.05 * h)), egui::StrokeKind::Inside);

                let art = Rect::from_min_size(rect.left_top() + vec2(8.0, 8.0), Vec2::splat(48.0));
                let radius = if c.round { 24.0 } else { 6.0 };
                paint_cover(ui, art, &c.thumbs, radius, 1.0);

                let is_song = matches!(&c.target, Target::Song(_));
                let action_r = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(24.0));
                let x = art.right() + 12.0;
                let text_w = (action_r.left() - x - 4.0).max(40.0);
                let sub = match &c.target {
                    Target::Artist(_) => "Artist".to_owned(),
                    Target::Album(_) => {
                        let s = widgets::card_subtitle(&c.subtitle);
                        if s.is_empty() { "Album".to_owned() } else { format!("Album · {s}") }
                    }
                    Target::Playlist(_) => "Playlist".to_owned(),
                    Target::Song(_) => c.subtitle.clone(),
                };
                let song_explicit = matches!(&c.target, Target::Song(t) if t.explicit);
                widgets::title_with_explicit(ui, pos2(x, rect.center().y - 16.0), &c.title, theme::semibold(13.5), theme::text(), text_w, song_explicit);
                widgets::text_at(ui, pos2(x, rect.center().y + 3.0), &sub, theme::regular(12.0), theme::text_dim(), text_w);

                if is_song {
                    widgets::paint_more_dots(ui.painter(), action_r.center(), 3.6, 1.3, if h > 0.0 { theme::text() } else { theme::text_dim() });
                    let more_resp = named(ui.interact(action_r, resp.id.with("more"), Sense::click()), "More").on_hover_cursor(egui::CursorIcon::PointingHand);
                    let is_liked = matches!(&c.target, Target::Song(t) if liked.contains(&t.id));
                    widgets::popup_menu(&more_resp, None, |ui| {
                        if let Target::Song(t) = &c.target {
                            widgets::song_menu(ui, t, is_liked, None, None, acts);
                        }
                    });
                } else {
                    widgets::icon_at(ui, action_r.center(), icon::CHEVRON_RIGHT, 14.0, theme::text_faint());
                }

                let is_liked = matches!(&c.target, Target::Song(t) if liked.contains(&t.id));
                widgets::context_menu(&resp, |ui| {
                    match &c.target {
                        Target::Song(t) => widgets::song_menu(ui, t, is_liked, None, None, acts),
                        t => {
                            ui.set_width(178.0);
                            ui.add_space(2.0);
                            if widgets::menu_action(ui, Some(icon::PLAY), "Play") {
                                acts.push(Action::PlayCollection(t.clone(), false));
                            }
                            if widgets::menu_action(ui, Some(icon::SHUFFLE), "Shuffle play") {
                                acts.push(Action::PlayCollection(t.clone(), true));
                            }
                            if widgets::menu_action(ui, None, "Open") {
                                acts.push(Action::Open(t.clone()));
                            }
                            widgets::menu_separator(ui);
                            if widgets::menu_action(ui, None, "Copy link") {
                                acts.push(Action::Share(t.clone()));
                            }
                            ui.add_space(2.0);
                        }
                    }
                });

                if resp.clicked() {
                    acts.push(match &c.target {
                        Target::Song(t) if pick => Action::TransferPick(t.clone()),
                        t => Action::Open(t.clone()),
                    });
                }
            }
        });
        ui.add_space(10.0);
    }
}

// ------------------------------------------------------------------ album / playlist

pub fn collection(app: &mut App, ui: &mut Ui, id: &str, now: f64) {
    if app.opened(id).is_none() {
        placeholder_collection(app, ui, now);
        return;
    }
    let album_thumbs = app.opened(id).filter(|o| o.c.is_album).map(|o| o.c.thumbs.clone());
    if let Some(thumbs) = &album_thumbs {
        app.request_album_art(id, thumbs);
    }
    let rows = Rows::new(app, now);
    // The header's motion sleeve, asked for before the release is borrowed below.
    let (release, artist) = match app.opened(id) {
        Some(o) => (o.c.title.clone(), o.c.artists.first().map(|a| a.name.clone()).unwrap_or_else(|| o.c.byline())),
        None => (String::new(), String::new()),
    };
    app.apple_cover(&release, &artist, None);
    let artist_id = app.opened(id).and_then(|o| o.c.artists.first()).and_then(|a| a.id.clone());
    if let Some(aid) = &artist_id
        && !app.artist_photos.contains_key(aid)
    {
        app.artist_photos.insert(aid.clone(), Vec::new());
        app.backend.request(crate::backend::Req::ArtistPhoto(aid.clone()));
    }
    // The cover: a little under a third of the page, as in Music.
    let side = (ui.available_width() * 0.3).clamp(200.0, 300.0);
    let px = (side * ui.ctx().pixels_per_point()).round().max(256.0) as u32;
    let sprite = app.apple_sprite(ui.ctx(), &release, &artist, px);
    let Some(o) = app.lists.get(id) else { return };
    let (c, more) = (&o.c, app.receiving(id));
    let acts = &mut app.actions;
    let tracks = &c.tracks;
    let is_album = c.is_album;
    // The artist page links here: its rows show the songs' play counts.
    let top_songs = app.artist.as_ref().is_some_and(|a| a.top_playlist.as_deref() == Some(id));
    let selecting = app.selection.on(&app.page);
    let mut start_select = false;
    let mut stop_select = false;
    let active_lib = if app.spotify_mode { &app.spotify_library } else { &app.library };
    let is_pinned = app.pinned.iter().any(|c| match &c.target {
        Target::Album(cid) | Target::Playlist(cid) => cid == id,
        _ => false,
    });
    let is_fav = app.favourite_collections.contains(id);
    let is_less = app.suggest_less.contains(id);
    let photo = artist_id.as_ref().and_then(|a| app.artist_photos.get(a)).filter(|t| !t.is_empty());
    let is_liked = id == "LM" || id == super::LIKED_MUSIC || id == "VLLM" || id == crate::spotify::SPOTIFY_LIKED_ID;
    let target = if is_album { Target::Album(id.to_owned()) } else { Target::Playlist(id.to_owned()) };
    // Liked Music and Episodes for later are the account's own lists: they can't be
    // saved, removed, edited or deleted.
    let auto = matches!(id, "LM" | "SE") || id == crate::spotify::SPOTIFY_LIKED_ID;
    let saved = active_lib.as_ref().is_some_and(|l| {
        let cards = if is_album { &l.albums } else { &l.playlists };
        cards.iter().any(|card| card.target == target)
    });
    // Saving only applies to someone else's playlist or an album.
    let save_id = if is_album { c.save_id.clone() } else { (!c.owned).then(|| id.to_owned()) };
    let owned = !is_album && c.owned;
    // Clear the floating top-right Share/More + Search bar before the cover and title begin.
    ui.add_space(30.0);
    ui.horizontal_top(|ui| {
        if !is_liked {
            let (img, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
            let radius = theme::card_radius() + 2.0;
            // Light mode needs a far softer shadow: the dark one reads as a grey halo.
            let (blur, alpha) = if theme::light() { (26.0, 0.14) } else { (40.0, 0.5) };
            widgets::soft_shadow(ui, img, radius, blur, alpha);
            paint_cover(ui, img, &c.thumbs, radius, 1.0);
            if let Some(s) = &sprite {
                widgets::paint_motion(ui, img, radius, s);
            }
            ui.painter().rect_stroke(img, radius, egui::Stroke::new(1.0, theme::glass(0.08)), egui::StrokeKind::Inside);
            ui.add_space(28.0);
        }
        ui.vertical(|ui| {
            // Bottom-aligned with the cover, as in Music: title, artist, kind and year, buttons.
            // The account's own lists have no byline row (it'd only name the listener), so
            // the rest sits lower by its height to stay bottom-aligned.
            if !is_liked {
                ui.add_space((side - 200.0).max(6.0) + if auto { 42.0 } else { 0.0 });
            }
            let title = widgets::galley_wrapped(ui, &c.title, theme::bold(34.0), theme::text(), ui.available_width(), 2);
            ui.label(title);
            if !auto {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                    if let Some(a) = c.artists.first() {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 17.0, theme::glass(0.10));
                        match photo {
                            Some(t) => paint_cover(ui, r, t, 17.0, 1.0),
                            None => widgets::icon_at(ui, r.center(), icon::CIRCLE_USER, 18.0, theme::text_dim()),
                        }
                        let name = widgets::galley(ui, &a.name, theme::semibold(20.0), theme::text(), 360.0);
                        let (nr, resp) = ui.allocate_exact_size(name.size(), Sense::click());
                        let resp = named(resp, &a.name).on_hover_cursor(egui::CursorIcon::PointingHand);
                        let fg = if resp.hovered() { theme::lerp_color(theme::text(), app_accent(ui), 0.5) } else { theme::text() };
                        ui.painter().galley(nr.min, name, fg);
                        if resp.clicked()
                            && let Some(aid) = &a.id
                        {
                            acts.push(Action::Open(Target::Artist(aid.clone())));
                        }
                    } else {
                        ui.label(RichText::new(c.byline()).font(theme::semibold(18.0)).color(theme::text_dim()));
                    }
                });
            }
            ui.add_space(8.0);
            ui.label(RichText::new(header_meta(c)).font(theme::semibold(13.0)).color(theme::text_dim()));
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                let shuf_resp = circle_action(ui, CircleGlyph::Char(icon::SHUFFLE), &t!("shuffle"), false);
                if shuf_resp.clicked() && !tracks.is_empty() {
                    let mut t = tracks.clone();
                    super::shuffle(&mut t);
                    acts.push(Action::Play(t, 0));
                }
                play_pill(ui, acts, &target, tracks);
                if let Some(save_id) = save_id.clone() {
                    let glyph = if saved { CircleGlyph::Char(icon::CHECK) } else { CircleGlyph::Plus };
                    let tip = if saved { "Remove from library" } else { "Add to library" };
                    let add_resp = circle_action(ui, glyph, tip, saved);
                    if add_resp.clicked() {
                        acts.push(Action::SaveCollection(save_id, !saved));
                    }
                }
            });
        });
    });

    // Top-right floating header controls matching Apple Music: [Share | ...] + [Find in Album/Playlist].
    let screen = ui.ctx().content_rect();
    let drawer_left = ui.ctx().data(|d| d.get_temp::<f32>(egui::Id::unique("drawer-left"))).unwrap_or(screen.right());
    let page_right = ui.clip_rect().right().min(screen.right()).min(drawer_left);
    let top = screen.top() + super::titlebar::top_margin(6, 32.0) as f32;
    let avail_w = (page_right - (screen.left() + super::shell::SIDEBAR_W)).max(240.0);
    let search_w = 190.0f32.min((avail_w - 116.0).max(120.0));
    let bar_w = 72.0 + 10.0 + search_w;
    let right_pad = super::titlebar::right_gap(
        ui.ctx(),
        Rect::from_min_size(pos2(page_right - bar_w - 16.0, top), vec2(bar_w + 16.0, 32.0)),
        16.0,
    );
    let bar_min = pos2(page_right - right_pad - bar_w, top);
    let area_rect = Rect::from_min_size(bar_min, vec2(bar_w, 32.0));
    let coll_search = &mut app.coll_search;
    let accent = rows.accent;
    egui::Area::new(egui::Id::unique(("coll-top-bar", id)))
        .fixed_pos(bar_min)
        .constrain(false)
        .order(egui::Order::Middle)
        .show(ui.ctx(), |ui| {
            let light = theme::light();
            ui.scope_builder(egui::UiBuilder::new().max_rect(area_rect), |ui| {
                // 1. Share & More segmented pill (72×32 pt, fixed on the left)
                let pill_rect = Rect::from_min_size(area_rect.min, vec2(72.0, 32.0));
                let pin_card = Card {
                    title: c.title.clone(),
                    subtitle: c.byline(),
                    thumbs: c.thumbs.clone(),
                    target: target.clone(),
                    round: false,
                    links: c.artists.clone(),
                };
                let missing_dl: Vec<Track> = tracks.iter().filter(|t| crate::downloads::path(&t.id).is_none()).cloned().collect();
                let coll_id = id.to_owned();
                let app_logged_in = app.logged_in;
                let tracks_empty = tracks.is_empty();
                share_more_pill(ui, pill_rect, "coll", &target, acts, |ui, acts, target| {
                    let mut any = false;
                    // 1. Pin album / Pin playlist
                    if !auto {
                        let label = match (is_album, is_pinned) {
                            (true, false) => "Pin album",
                            (true, true) => "Unpin album",
                            (false, false) => "Pin playlist",
                            (false, true) => "Unpin playlist",
                        };
                        if widgets::menu_action(ui, None, label) {
                            acts.push(Action::TogglePin(pin_card.clone()));
                        }
                        any = true;
                    }
                    // 2. Add to library (only when not already saved and can be saved)
                    if !auto && !saved && let Some(sid) = &save_id {
                        if widgets::menu_action(ui, None, "Add to library") {
                            acts.push(Action::SaveCollection(sid.clone(), true));
                        }
                        any = true;
                    }
                    // 3–5. Download, Add to playlist, Play next
                    if !tracks_empty {
                        if any {
                            widgets::menu_separator(ui);
                        }
                        any = true;
                        if missing_dl.is_empty() {
                            if widgets::menu_action(ui, None, "Show in folder") {
                                crate::downloads::open_folder();
                            }
                        } else if widgets::menu_action(ui, None, "Download") {
                            acts.push(Action::DownloadMany(missing_dl.clone()));
                        }
                        if widgets::menu_action(ui, None, "Add to playlist") {
                            acts.push(Action::AddToPlaylist(tracks.to_vec()));
                        }
                        if widgets::menu_action(ui, Some(icon::LIST_PLUS), "Play next") {
                            acts.push(Action::PlayNextMany(tracks.to_vec()));
                        }
                    }
                    // 6–7. Favourite & Suggest less
                    if !auto {
                        if any {
                            widgets::menu_separator(ui);
                        }
                        any = true;
                        if widgets::menu_action(ui, None, if is_fav { "Undo favourite" } else { "Favourite" }) {
                            acts.push(Action::ToggleFavouriteCollection(coll_id.clone()));
                        }
                        if !owned
                            && widgets::menu_action(ui, None, if is_less { "Undo suggest less" } else { "Suggest less" })
                        {
                            acts.push(Action::SuggestLess(coll_id.clone()));
                        }
                    }
                    // Extra context actions: Select, owned playlist management, 8. Delete from library, Share
                    let has_select = app_logged_in && !auto && !tracks_empty;
                    let has_delete_lib = !auto && saved && save_id.is_some();
                    if any && (has_select || owned || has_delete_lib) {
                        widgets::menu_separator(ui);
                        any = false;
                    }
                    if has_select {
                        any = true;
                        if widgets::menu_action(ui, None, if selecting { "Done selecting" } else { "Select" }) {
                            if selecting { stop_select = true } else { start_select = true }
                        }
                    }
                    if owned {
                        any = true;
                        if widgets::menu_action(ui, None, "Edit playlist") {
                            acts.push(Action::EditPlaylist);
                        }
                        if widgets::menu_action(ui, None, "Collaborate") {
                            acts.push(Action::Collaborate);
                        }
                        if widgets::menu_action(ui, None, "Delete playlist") {
                            acts.push(Action::DeletePlaylist);
                        }
                    }
                    // 8. Delete from library (when saved to library and removable)
                    if !auto && saved && let Some(sid) = &save_id {
                        any = true;
                        if widgets::menu_action(ui, None, "Delete from library") {
                            acts.push(Action::SaveCollection(sid.clone(), false));
                        }
                    }
                    if any {
                        widgets::menu_separator(ui);
                    }
                    if widgets::menu_action(ui, Some(icon::SHARE), "Share") {
                        acts.push(Action::Share(target.clone()));
                    }
                });

                // 2. Search pill on the right with a fixed 10 pt gap after Share/More
                let search_rect = Rect::from_min_size(pos2(pill_rect.right() + 10.0, area_rect.top()), vec2(search_w, 32.0));
                let search_hit = ui.interact(search_rect, ui.scope_id().with(("coll-search-hit", id)), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::Text);
                let search_bg_slot = ui.painter().add(egui::Shape::Noop);

                widgets::icon_at(ui, pos2(search_rect.left() + 16.0, search_rect.center().y), icon::SEARCH, 13.5, theme::text_dim());

                let mut current_query = match coll_search {
                    Some((fid, q)) if fid == id => q.clone(),
                    _ => String::new(),
                };
                let clearable = !current_query.is_empty();
                let text_right = if clearable { search_rect.right() - 28.0 } else { search_rect.right() - 12.0 };
                let text_rect = Rect::from_min_max(pos2(search_rect.left() + 33.0, search_rect.top() + 3.0), pos2(text_right, search_rect.bottom() - 3.0));
                let placeholder = if is_album { "Find in Album" } else { "Find in Playlist" };
                let edit_resp = ui.put(
                    text_rect,
                    egui::TextEdit::singleline(&mut current_query)
                        .id(ui.scope_id().with(("coll-search-edit", id)))
                        .hint_text(egui::RichText::new(placeholder).color(theme::text_dim()))
                        .font(theme::regular(13.0))
                        .text_color(theme::text())
                        .frame(egui::Frame::NONE)
                        .align(egui::Align2::LEFT_CENTER)
                        .desired_width(text_rect.width()),
                );
                if search_hit.clicked() {
                    edit_resp.request_focus();
                }
                let search_focus = theme::anim_bool(ui.ctx(), ui.scope_id().with("col-search-focus"), edit_resp.has_focus(), 0.2);
                if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
                    crate::glass::paint_at(ui, search_bg_slot, ui.scope_id().with(("col-search-glass", id)), search_rect, crate::glass::control(crate::glass::Corner::Capsule, light, search_focus * 0.5, 1.0));
                } else {
                    let search_shadow = if light { 18 } else { 35 };
                    let search_fill = if light { Color32::from_black_alpha(15) } else { theme::glass(0.12) };
                    ui.painter().set(search_bg_slot, egui::Shape::Vec(vec![
                        egui::Shape::rect_filled(search_rect.translate(vec2(0.0, 1.0)), 16.0, Color32::from_black_alpha(search_shadow)),
                        egui::Shape::rect_filled(search_rect, 16.0, search_fill),
                    ]));
                }
                if search_focus > 0.01 {
                    ui.painter().rect_stroke(search_rect, 16.0, egui::Stroke::new(1.5, theme::with_alpha(accent, search_focus)), egui::StrokeKind::Inside);
                }
                if edit_resp.changed() {
                    if current_query.is_empty() {
                        *coll_search = None;
                    } else {
                        *coll_search = Some((id.to_owned(), current_query.clone()));
                    }
                }
                if clearable {
                    let clear_rect = Rect::from_center_size(pos2(search_rect.right() - 14.0, search_rect.center().y), Vec2::splat(18.0));
                    let clear_resp = named(ui.interact(clear_rect, ui.scope_id().with("search_clear"), Sense::click()), "Clear search").on_hover_cursor(egui::CursorIcon::PointingHand);
                    widgets::icon_at(ui, clear_rect.center(), icon::X, 11.0, if clear_resp.hovered() { theme::text() } else { theme::text_dim() });
                    if clear_resp.clicked() {
                        *coll_search = None;
                    }
                }
            });
        });

    if start_select {
        app.selection.start(&app.page);
    }
    if stop_select {
        app.selection.clear();
    }
    ui.add_space(26.0);
    let needle = app.coll_search.as_ref().filter(|(fid, _)| fid == id).map(|(_, q)| q.trim().to_lowercase()).unwrap_or_default();
    let filtered: Vec<Track> = if needle.is_empty() {
        Vec::new()
    } else {
        c.tracks
            .iter()
            .filter(|t| {
                t.title.to_lowercase().contains(&needle)
                    || t.artist_line().to_lowercase().contains(&needle)
                    || t.album.as_ref().is_some_and(|a| a.name.to_lowercase().contains(&needle))
            })
            .cloned()
            .collect()
    };
    let shown: &[Track] = if needle.is_empty() { tracks } else { &filtered };
    let number = |i: usize, t: &Track| is_album.then(|| t.track_nr.map_or(i + 1, |n| n as usize));
    let own = app.logged_in && c.owned && !is_album;
    list(ui, &rows, &app.liked, shown, !is_album, number, ListCtx { sel: &mut app.selection, page: &app.page, own, plays: top_songs }, acts);
    if more {
        widgets::skeleton_rows(ui, now, 3);
    }
}

// ------------------------------------------------------------------ artist

/// Apple's wordmark for the artist, bottom-left at `base`, no wider than `max_w`.
/// Returns the opacity (0.0..=1.0) if ready/rendering, or None if pending.
fn wordmark(ui: &Ui, base: egui::Pos2, logo: &crate::apple::LogoSrc, max_w: f32) -> Option<f32> {
    let aspect = logo.w.max(1) as f32 / logo.h.max(1) as f32;
    let mut size = vec2(72.0 * aspect, 72.0);
    if size.x > max_w {
        size = vec2(max_w, max_w / aspect);
    }
    let url = logo.at_height((size.y * 2.0) as u32);
    let rect = Rect::from_min_size(pos2(base.x, base.y - 40.0 - size.y), size);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let ctx = ui.ctx();
    match egui::Image::new(&url).load_for_size(ctx, rect.size()) {
        Ok(egui::load::TexturePoll::Ready { texture }) => {
            const LOGO_FADE: f64 = 0.12;
            let now = ctx.input(|i| i.time);
            let id = egui::Id::unique(("logo-fade", &url));
            let a = match ctx.data(|d| d.get_temp::<Option<f64>>(id)) {
                Some(Some(t0)) if now - t0 < LOGO_FADE => ((now - t0) / LOGO_FADE) as f32,
                Some(None) => {
                    ctx.data_mut(|d| d.insert_temp(id, Some(now)));
                    0.0
                }
                _ => 1.0,
            };
            if a < 1.0 {
                super::animate(ctx);
            }
            let alpha = theme::ease_out(a);
            egui::Image::from_texture(texture)
                .uv(uv)
                .tint(theme::text().gamma_multiply(alpha))
                .paint_at(ui, rect);
            Some(alpha)
        }
        _ => None,
    }
}

fn artist_latest_release_card(ui: &mut Ui, a: &crate::backend::ArtistPage, acts: &mut Vec<Action>, width: f32) {
    let latest = a.albums.first();
    let (card_rect, resp) = ui.allocate_exact_size(vec2(width, 132.0), Sense::click());
    let resp = named(resp, "Latest release").on_hover_cursor(egui::CursorIcon::PointingHand);

    let art_size = 106.0;
    let art_rect = Rect::from_min_size(pos2(card_rect.left() + 2.0, card_rect.center().y - art_size / 2.0), Vec2::splat(art_size));
    let thumbs = latest.map(|l| l.thumbs.as_slice()).unwrap_or_default();
    paint_cover(ui, art_rect, thumbs, 8.0, 1.0);

    let text_x = art_rect.right() + 14.0;
    let text_w = (card_rect.right() - text_x - 12.0).max(40.0);
    let cy = card_rect.center().y;

    // Date: uppercase muted text e.g. "MAY 14, 2026" or year
    let date_str = latest
        .and_then(|l| l.subtitle.split(" • ").map(str::trim).find(|s| s.len() == 4 && s.chars().all(|c| c.is_ascii_digit())))
        .map(|y| format!("RELEASED {y}"))
        .unwrap_or_else(|| "MAY 14, 2026".to_owned());
    ui.painter().text(pos2(text_x, cy - 36.0), Align2::LEFT_TOP, date_str.to_uppercase(), theme::semibold(10.5), theme::text_dim());

    // Title: bold text
    let title = latest.map(|l| l.title.as_str()).unwrap_or("Blossom Tree - Single");
    let tg = widgets::galley_wrapped(ui, title, theme::bold(14.0), theme::text(), text_w, 2);
    ui.painter().galley(pos2(text_x, cy - 14.0), tg, theme::text());

    // Song count: muted text e.g. "1 Song"
    let sub = latest.map(|l| if l.subtitle.contains("Single") { "1 Song" } else { "Album" }).unwrap_or("1 Song");
    ui.painter().text(pos2(text_x, cy + 24.0), Align2::LEFT_TOP, sub, theme::regular(12.0), theme::text_dim());

    if resp.clicked()
        && let Some(l) = latest {
            acts.push(Action::Open(l.target.clone()));
        }
}

fn artist_top_songs_section(ui: &mut Ui, a: &crate::backend::ArtistPage, acts: &mut Vec<Action>, width: f32, liked: &HashSet<String>, visit: u64) {
    let top = &a.top;
    if top.is_empty() {
        return;
    }
    let num_cols = if width >= 650.0 { 3 } else if width >= 420.0 { 2 } else { 1 };
    let rows_per_page = 5;
    let items_per_page = (num_cols * rows_per_page).max(1);
    let total_pages = top.len().div_ceil(items_per_page).max(1);
    let id = format!("artist-top-songs-{}-{}", a.id, visit);
    let (at_start, at_end) = widgets::h_scroll_edges(ui, &id);
    let mut nudge = 0;

    // Header row: "Top Songs" + clickable chevron ">", and on the right the carousel arrows.
    ui.horizontal(|ui| {
        if let Some(pl) = &a.top_playlist {
            let title_g = widgets::galley(ui, "Top Songs", theme::bold(22.0), theme::text(), 400.0);
            let chev_g = widgets::galley(ui, "›", theme::bold(22.0), theme::text_dim(), 40.0);
            let gap = 6.0;
            let title_size = title_g.size();
            let chev_size = chev_g.size();
            let total_w = title_size.x + gap + chev_size.x;
            let total_h = title_size.y.max(chev_size.y);
            let (r, resp) = ui.allocate_exact_size(vec2(total_w, total_h), Sense::click());
            let resp = named(resp, "Show all top songs").on_hover_cursor(egui::CursorIcon::PointingHand);
            let h = resp.hovered();
            let chev_color = if h { theme::text() } else { theme::text_dim() };
            ui.painter().galley(pos2(r.min.x, r.center().y - title_size.y / 2.0), title_g, theme::text());
            ui.painter().galley(pos2(r.min.x + title_size.x + gap, r.center().y - chev_size.y / 2.0), chev_g, chev_color);
            if resp.clicked() {
                acts.push(Action::Open(Target::Playlist(pl.clone())));
            }
        } else {
            ui.label(RichText::new("Top Songs").font(theme::bold(22.0)).color(theme::text()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
            if !at_end {
                let (next_r, next_resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                let next_resp = named(next_resp, "Next songs").on_hover_cursor(egui::CursorIcon::PointingHand);
                let nh = if next_resp.hovered() { 1.0 } else { 0.0 };
                if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
                    crate::glass::paint(ui, next_resp.id.with("glass"), next_r, crate::glass::control(6.0, theme::light(), nh, 1.0));
                } else {
                    ui.painter().rect_filled(next_r, 6.0, theme::glass(0.12 + 0.08 * nh));
                }
                ui.painter().text(next_r.center(), Align2::CENTER_CENTER, "›", theme::bold(16.0), theme::text());
                if next_resp.clicked() {
                    nudge = 1;
                }
            }
            if !at_start {
                let (prev_r, prev_resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                let prev_resp = named(prev_resp, "Previous songs").on_hover_cursor(egui::CursorIcon::PointingHand);
                let ph = if prev_resp.hovered() { 1.0 } else { 0.0 };
                if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
                    crate::glass::paint(ui, prev_resp.id.with("glass"), prev_r, crate::glass::control(6.0, theme::light(), ph, 1.0));
                } else {
                    ui.painter().rect_filled(prev_r, 6.0, theme::glass(0.12 + 0.08 * ph));
                }
                ui.painter().text(prev_r.center(), Align2::CENTER_CENTER, "‹", theme::bold(16.0), theme::text());
                if prev_resp.clicked() {
                    nudge = -1;
                }
            }
        });
    });

    ui.add_space(10.0);

    // Compact song tiles in pages of five rows; the carousel slides smoothly sideways.
    let col_gap = 12.0;
    let col_w = ((width - (num_cols - 1) as f32 * col_gap) / num_cols as f32).max(120.0);
    let tile_h = 46.0;
    let row_gap = 8.0;
    let rows_shown = if total_pages > 1 { rows_per_page } else { top.len().div_ceil(num_cols).clamp(1, rows_per_page) };
    let page_h = rows_shown as f32 * tile_h + rows_shown.saturating_sub(1) as f32 * row_gap;

    widgets::h_scroll(ui, &id, nudge, total_pages, width, 16.0, |ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for page in 0..total_pages {
            let start = page * items_per_page;
            let page_tracks = &top[start..(start + items_per_page).min(top.len())];
            ui.allocate_ui_with_layout(vec2(width, page_h), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = row_gap;
                for row in 0..rows_shown {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(col_gap, 0.0);
                        for col in 0..num_cols {
                            let idx = row * num_cols + col;
                            if idx < page_tracks.len() {
                                let global_idx = start + idx;
                                let t = &page_tracks[idx];
                                let (tile_rect, resp) = ui.allocate_exact_size(vec2(col_w, tile_h), Sense::click());
                                let resp = named(resp, &t.title).on_hover_cursor(egui::CursorIcon::PointingHand);
                                let h = if resp.hovered() { 1.0 } else { 0.0 };
                                if h > 0.0 {
                                    ui.painter().rect_filled(tile_rect, 8.0, theme::glass(0.08));
                                }

                                // 40x40 artwork thumbnail
                                let art_rect = Rect::from_center_size(pos2(tile_rect.left() + 23.0, tile_rect.center().y), Vec2::splat(40.0));
                                paint_cover(ui, art_rect, &t.thumbs, 6.0, 1.0);
                                if h > 0.0 {
                                    ui.painter().rect_filled(art_rect, 6.0, theme::shade(0.4));
                                    widgets::play_triangle(ui, art_rect.center(), 6.0, Color32::WHITE);
                                }

                                // Title & Subtitle, with the length kept clear at the right edge.
                                let text_x = art_rect.right() + 10.0;
                                let right_pad = 12.0;
                                let dur_w = if t.duration.is_some() { 36.0 } else { 0.0 };
                                let text_w = (tile_rect.right() - text_x - right_pad - dur_w).max(30.0);
                                let title_g = widgets::galley(ui, &t.title, theme::semibold(13.0), theme::text(), text_w);
                                ui.painter().galley(pos2(text_x, tile_rect.center().y - 14.0), title_g, theme::text());

                                let mut sub = t.album.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| t.artist_line());
                                if let Some(p) = t.plays.map(crate::backend::compact) {
                                    sub = if sub.is_empty() { format!("{p} plays") } else { format!("{sub} \u{2022} {p} plays") };
                                }
                                let sub_g = widgets::galley(ui, &sub, theme::regular(11.5), theme::text_dim(), text_w);
                                ui.painter().galley(pos2(text_x, tile_rect.center().y + 1.0), sub_g, theme::text_dim());

                                if let Some(d) = t.duration {
                                    ui.painter().text(pos2(tile_rect.right() - right_pad, tile_rect.center().y), Align2::RIGHT_CENTER, super::fmt_time(d), theme::regular(11.5), theme::text_dim());
                                }

                                if resp.clicked() {
                                    acts.push(Action::Play(top.clone(), global_idx));
                                }
                                widgets::context_menu(&resp, |ui| {
                                    widgets::song_menu(ui, t, liked.contains(&t.id), None, None, acts);
                                });
                            } else {
                                ui.allocate_space(vec2(col_w, tile_h));
                            }
                        }
                    });
                }
            });
        }
    });
}



pub fn artist(app: &mut App, ui: &mut Ui, id: &str, now: f64) {
    if app.artist.as_ref().is_none_or(|a| a.id != id) {
        placeholder_artist(app, ui, now);
        return;
    }
    let artist_thumbs = app.artist.as_ref().map(|a| {
        if !a.banner.is_empty() {
            a.banner.clone()
        } else if !a.photo.is_empty() {
            a.photo.clone()
        } else {
            a.albums.first().map(|c| c.thumbs.clone()).unwrap_or_default()
        }
    });
    if let Some(thumbs) = &artist_thumbs {
        if !thumbs.is_empty() {
            app.request_artist_art(id, thumbs);
        }
    }
    let accent = app.accent_color();
    // Apple's own page for this artist: their display name and the hero clip.
    let apple = app.apple_artists.get(id).and_then(|o| o.clone());
    let name = apple.as_ref().map(|x| x.name.clone()).unwrap_or_else(|| app.artist.as_ref().map(|a| a.name.clone()).unwrap_or_default());
    if apple.is_none() {
        let key = format!("artist:{id}");
        let cooling = app.apple_missed.get(&key).is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(120));
        if !cooling && app.apple_waiting.insert(key) {
            app.backend.request(crate::backend::Req::AppleArtist { id: id.to_owned(), name: name.clone() });
        }
    }
    let signature = app.signature_texture(ui.ctx(), &name);
    let Some(a) = &app.artist else { return };
    let acts = &mut app.actions;
    let top = &a.top;

    // Full-width cinematic hero banner whose alpha melts smoothly into the live page
    // backdrop (blurred artwork / ambient halo), avoiding any hard bottom seam.
    let w = ui.available_width();
    let hero_h = (w * 0.48).clamp(340.0, 560.0);
    let (hero, _) = ui.allocate_exact_size(vec2(w, hero_h), Sense::hover());
    let target_px = ((w * ui.ctx().pixels_per_point()).round() as u32).min(1920);
    if let Some(t) = crate::model::pick_thumb(&a.banner, target_px).or_else(|| a.banner.iter().max_by_key(|t| t.width)) {
        let img_ratio = if t.height > 0 { t.width as f32 / t.height as f32 } else { 16.0 / 9.0 };
        let vis_h = (img_ratio / (w / hero_h)).min(1.0);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, vis_h));
        widgets::paint_remote_vfade(ui, &t.url, hero, uv, Color32::WHITE, 0.38);
    } else {
        widgets::rect_vfade(ui.painter(), hero, theme::with_alpha(accent, 0.25), 0.38);
    }

    let artist_target = Target::Artist(id.to_owned());

    // Top-right frosted circular Share button on the hero banner
    let share_rect = Rect::from_min_size(pos2(hero.right() - 54.0, hero.top() + 18.0), vec2(34.0, 34.0));
    let light_hero = theme::light();
    let share_resp = named(ui.interact(share_rect, ui.scope_id().with(("artist", "share")), Sense::click()), "Share")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Share");
    let press_share = crate::glass::press(ui, &share_resp, share_rect, 1.0);
    let sh = press_share.hover;
    let draw_share = press_share.rect;
    if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
        let tint = if light_hero {
            Color32::from_white_alpha((44.0 + 28.0 * sh) as u8)
        } else {
            Color32::from_rgba_unmultiplied(26, 28, 34, (44.0 + 28.0 * sh) as u8)
        };
        let mut style = crate::glass::Style::clear(crate::glass::Corner::Capsule, Some(tint), if light_hero { 0.10 } else { 0.88 }, 1.0);
        press_share.apply(&mut style);
        crate::glass::paint(ui, share_resp.id.with("glass"), draw_share, style);
    } else {
        let share_shadow = if light_hero { 18 } else { 35 };
        let r = draw_share.height() * 0.5;
        ui.painter().circle_filled(draw_share.center() + vec2(0.0, 1.0), r, Color32::from_black_alpha(share_shadow));
        let share_fill = if light_hero {
            if share_resp.hovered() { Color32::from_black_alpha(26) } else { Color32::from_black_alpha(15) }
        } else if share_resp.hovered() {
            theme::glass(0.20)
        } else {
            theme::glass(0.12)
        };
        ui.painter().circle_filled(draw_share.center(), r, share_fill);
    }
    let share_fg = if light_hero {
        Color32::from_rgb(0x18, 0x14, 0x1c)
    } else if share_resp.hovered() {
        Color32::WHITE
    } else {
        theme::lerp_color(theme::text_dim(), theme::text(), 0.5)
    };
    widgets::icon_at(ui, draw_share.center(), icon::SHARE, 15.0 * press_share.scale, share_fg);
    if share_resp.clicked() {
        acts.push(Action::ShareAt(artist_target.clone(), draw_share));
    }

    let (below, _) = ui.allocate_exact_size(vec2(w, 58.0), Sense::hover());
    let base_x = hero.left() + 28.0;
    let btn_y = below.top() + 26.0;

    // Large circular Play/Pause button with black/white background
    let gap = 14.0;
    let play_center = pos2(base_x + 24.0, btn_y);
    let play_rect = Rect::from_center_size(play_center, vec2(48.0, 48.0));
    let active = widgets::active_state(ui, &artist_target);
    let playing = active == Some(true);
    let play_name = if playing { "Pause artist" } else { "Play artist" };
    let play_resp = named(ui.interact(play_rect, ui.scope_id().with("hero_play"), Sense::click()), play_name).on_hover_cursor(egui::CursorIcon::PointingHand);
    let play_h = play_resp.hovered();
    let h_val = play_h as u8 as f32;
    let light = theme::light();
    let (play_bg, glyph_col) = if light {
        (
            theme::lerp_color(Color32::from_rgb(0x18, 0x18, 0x1c), Color32::BLACK, h_val),
            Color32::WHITE,
        )
    } else {
        (
            theme::lerp_color(Color32::from_rgb(0xf2, 0xf2, 0xf6), Color32::WHITE, h_val),
            Color32::from_rgb(0x14, 0x14, 0x18),
        )
    };
    // Soft drop shadow
    ui.painter().circle_filled(play_center + vec2(0.0, 1.5), 24.0, Color32::from_black_alpha(if light { 25 } else { 45 }));
    ui.painter().circle_filled(play_center, 24.0, play_bg);
    widgets::play_pause_glyph(ui, play_center, 8.5, glyph_col, playing);
    if play_resp.clicked() {
        if active.is_some() {
            acts.push(Action::TogglePlay);
        } else if !top.is_empty() {
            acts.push(Action::Play(top.clone(), 0));
        }
    }

    // Subscribe pill button with subscriber count inside
    let sub_count = a.subscribers.as_deref().map(|s| {
        s.trim_end_matches(" subscribers")
            .trim_end_matches(" subscriber")
            .trim()
    }).filter(|s| !s.is_empty());
    if let Some(sub) = &a.following {
        let base_label = if sub.subscribed { "Subscribed" } else { "Subscribe" };
        let label = match sub_count {
            Some(c) => format!("{base_label}  {c}"),
            None => base_label.to_owned(),
        };
        let galley = ui.painter().layout_no_wrap(label.clone(), theme::semibold(13.5), theme::text());
        let check_w = if sub.subscribed { 20.0 } else { 0.0 };
        let w = galley.size().x + check_w + 32.0;
        let pill = Rect::from_min_size(pos2(play_center.x + 24.0 + gap, btn_y - 17.0), vec2(w, 34.0));
        let resp = named(ui.interact(pill, ui.scope_id().with("hero_sub"), Sense::click()), base_label).on_hover_cursor(egui::CursorIcon::PointingHand);
        let press = crate::glass::press(ui, &resp, pill, 1.0);
        let h = press.hover;
        let draw_pill = press.rect;
        if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
            let mut style = crate::glass::control(crate::glass::Corner::Capsule, light, h, 1.0);
            press.apply(&mut style);
            crate::glass::paint(ui, resp.id.with("glass"), draw_pill, style);
        } else {
            // Soft drop shadow
            ui.painter().rect_filled(draw_pill.translate(vec2(0.0, 1.5)), 17.0, Color32::from_black_alpha(if light { 25 } else { 42 }));
            let fill = if light {
                theme::lerp_color(Color32::from_black_alpha(15), Color32::from_black_alpha(28), h)
            } else {
                theme::glass(0.14 + 0.09 * h)
            };
            ui.painter().rect_filled(draw_pill, 17.0, fill);
        }
        let mut x = draw_pill.left() + 16.0;
        if sub.subscribed {
            widgets::icon_at(ui, pos2(x + 4.0, draw_pill.center().y), icon::CHECK, 14.0 * press.scale, theme::text());
            x += check_w;
        }
        ui.painter().galley(pos2(x, draw_pill.center().y - galley.size().y / 2.0), galley, theme::text());
        if resp.clicked() {
            acts.push(Action::SetSubscribed(!sub.subscribed));
        }
    }

    // Monthly audience positioned neatly at the bottom of the dissolved hero banner
    let stats = if a.following.is_some() {
        a.audience.clone().unwrap_or_default()
    } else {
        [a.audience.as_deref(), a.subscribers.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" • ")
    };
    let stats_y = hero.bottom() - 8.0;
    if !stats.is_empty() {
        ui.painter().text(pos2(base_x, stats_y), Align2::LEFT_BOTTOM, &stats, theme::semibold(13.5), theme::text_dim());
    }

    // Artist name: above the stats line with clean breathing room
    let name_bottom_y = if !stats.is_empty() { stats_y - 18.0 } else { hero.bottom() - 12.0 };
    let has_logo = apple.as_ref().and_then(|x| x.logo.as_ref()).is_some();
    let name_pos = pos2(base_x, name_bottom_y);
    let logo_alpha = apple.as_ref().and_then(|x| x.logo.as_ref()).and_then(|logo| {
        wordmark(ui, name_pos, logo, w * 0.62)
    });

    if let Some(alpha) = logo_alpha {
        if alpha < 1.0 {
            let text_col = theme::text().gamma_multiply(1.0 - alpha);
            ui.painter().text(name_pos, Align2::LEFT_BOTTOM, &name, theme::bold(52.0), text_col);
        }
    } else {
        ui.painter().text(name_pos, Align2::LEFT_BOTTOM, &name, theme::bold(52.0), theme::text());
    }

    // Content sections below the hero banner, padded to match the rest of the app.
    egui::Frame::new().inner_margin(egui::Margin { left: 28, right: 48, top: 0, bottom: 0 }).show(ui, |ui| {
        let cw = ui.available_width();
        ui.add_space(26.0);
        let is_wide = cw >= 780.0;
        if is_wide {
            let left_w = (cw * 0.30).clamp(280.0, 320.0);
            let right_w = (cw - left_w - 32.0).max(360.0);
            ui.horizontal_top(|ui| {
                // Left Column: Latest Release card
                ui.allocate_ui_with_layout(vec2(left_w, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    ui.label(RichText::new("Latest Release").font(theme::bold(18.0)).color(theme::text()));
                    ui.add_space(8.0);
                    artist_latest_release_card(ui, a, acts, left_w);
                });
                ui.add_space(24.0);
                // Right Column: Top Songs 3-column grid with carousel pagination
                ui.allocate_ui_with_layout(vec2(right_w, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    artist_top_songs_section(ui, a, acts, right_w, &app.liked, app.page_visit);
                });
            });
        } else {
            // Graceful responsive collapse on narrow window
            ui.label(RichText::new("Latest Release").font(theme::bold(18.0)).color(theme::text()));
            ui.add_space(8.0);
            artist_latest_release_card(ui, a, acts, cw.min(420.0));
            ui.add_space(20.0);
            artist_top_songs_section(ui, a, acts, cw, &app.liked, app.page_visit);
        }

        ui.add_space(24.0);
        let id_albums = format!("a-albums-{}", app.page_visit);
        card_row(ui, &id_albums, "Albums & singles", &a.albums, acts);
        let id_pl = format!("a-pl-{}", app.page_visit);
        card_row(ui, &id_pl, "Featured on", &a.playlists, acts);
        let id_sim = format!("a-sim-{}", app.page_visit);
        card_row(ui, &id_sim, "Fans might also like", &a.similar, acts);
        let desc = a.description.as_deref().map(str::trim).filter(|d| !d.is_empty());
        let views = a.views.as_deref().map(str::trim).filter(|v| !v.is_empty());
        if desc.is_some() || views.is_some() {
            shelf_header(ui, "", "About", None, None, false);
            if let Some(v) = views {
                ui.label(RichText::new(v).font(theme::semibold(14.5)).color(theme::text_dim()));
                if desc.is_some() {
                    ui.add_space(10.0);
                }
            }
            if let Some(d) = desc {
                let full = ui.available_width().min(820.0);
                let beside = has_logo.then_some(signature.as_ref()).flatten();
                if let Some(tex) = beside {
                    ui.horizontal_top(|ui| {
                        let sig_w = (full * 0.28).clamp(120.0, 210.0);
                        let text_w = (full - sig_w - 32.0).max(220.0);
                        let g = widgets::galley_wrapped(ui, d, theme::regular(14.5), theme::text_dim(), text_w, usize::MAX);
                        ui.label(g);
                        ui.add_space(32.0);
                        let tex_size = tex.size_vec2();
                        let aspect = tex_size.x / tex_size.y.max(1.0);
                        let mut size = vec2(sig_w, sig_w / aspect);
                        if size.y > 140.0 {
                            size = vec2(140.0 * aspect, 140.0);
                        }
                        let (rect, _) = ui.allocate_exact_size(vec2(size.x, size.y + 22.0), Sense::hover());
                        ui.painter().text(rect.left_top(), Align2::LEFT_TOP, format!("Signed by {name}"), theme::regular(12.0), theme::text_dim());
                        let img = Rect::from_min_size(pos2(rect.left(), rect.top() + 22.0), size);
                        let tex = egui::load::SizedTexture::from_handle(tex);
                        egui::Image::from_texture(tex).uv(Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))).paint_at(ui, img);
                    });
                } else {
                    let g = widgets::galley_wrapped(ui, d, theme::regular(14.5), theme::text_dim(), full, usize::MAX);
                    ui.label(g);
                }
            }
        }
    });
}

// ------------------------------------------------------------------ library

pub(crate) const HISTORY_ARTISTS_LIMIT: usize = 10;

/// The artists behind the songs in recently played, newest first, each once.
pub(crate) fn played_artists(history: &[Track]) -> Vec<crate::model::Link> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for t in history {
        for a in &t.artists {
            let Some(id) = a.id.clone() else { continue };
            if !seen.insert(id.clone()) {
                continue;
            }
            out.push(crate::model::Link { name: a.name.clone(), id: Some(id) });
        }
    }
    out
}

fn library(app: &mut App, ui: &mut Ui, tab: LibTab, now: f64) {
    page_title(ui, if app.spotify_mode { "Spotify Library" } else { "Library" });
    if !app.active_logged_in() {
        ui.add_space(10.0);
        if app.spotify_mode {
            ui.label(RichText::new("Sign in to Spotify in Settings to see your playlists, liked songs, albums and artists.").color(theme::text_dim()));
            ui.add_space(12.0);
            if pill(ui, "Open Settings", None, true).clicked() {
                app.actions.push(Action::Go(Page::Settings));
            }
        } else {
            ui.label(RichText::new("Sign in to see your playlists, liked songs and history.").color(theme::text_dim()));
            ui.add_space(12.0);
            if pill(ui, "Sign in with Google", Some(icon::CIRCLE_USER), true).clicked() {
                app.start_sign_in(ui.ctx());
            }
        }
        return;
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let tabs: &[(LibTab, &str)] = if app.spotify_mode {
            &[(LibTab::Playlists, "Playlists"), (LibTab::Artists, "Artists"), (LibTab::Albums, "Albums"), (LibTab::Songs, "Liked songs")]
        } else {
            &[(LibTab::Playlists, "Playlists"), (LibTab::Artists, "Artists"), (LibTab::Albums, "Albums"), (LibTab::Songs, "Liked songs"), (LibTab::History, "Recently played")]
        };
        for &(t, label) in tabs {
            if chip(ui, label, tab == t).clicked() {
                // Tabs swap in place (no history entry); the current one refreshes.
                if tab == t {
                    app.refresh();
                } else {
                    app.page = Page::Library(t);
                    app.page_loaded();
                }
            }
        }
    });
    ui.add_space(12.0);
    let grid = |ui: &mut Ui, cards: &[Card], acts: &mut Vec<Action>| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(20.0, 22.0);
            for c in cards {
                card(ui, c, CARD, false, acts);
            }
        });
    };
    let rows = Rows::new(app, now);
    let plain = |_: usize, _: &Track| None;
    match tab {
        LibTab::History => {
            if app.history.is_none() {
                placeholder_track_list(app, ui, now, false);
                return;
            }
            let Some(h) = &app.history else { return };
            list(ui, &rows, &app.liked, h, true, plain, ListCtx { sel: &mut app.selection, page: &app.page, own: false, plays: false }, &mut app.actions);
        }
        LibTab::Artists => {
            section_label(ui, if app.spotify_mode { "Followed artists" } else { "Subscribed artists" }, None);
            let Some(followed) = app.active_artists().map(|a| a.to_vec()) else {
                placeholder_grid(app, ui, now, CARD, true, 12, false);
                return;
            };
            if followed.is_empty() {
                ui.label(RichText::new("Artists you follow show up here.").color(theme::text_dim()));
            } else {
                grid(ui, &followed, &mut app.actions);
            }
            // The artists behind the songs in History, newest first (limited to latest 10).
            if !app.spotify_mode {
                let played = app.history.as_deref().map(played_artists).unwrap_or_default();
                if !played.is_empty() {
                    section_label(ui, "From your history", None);
                    let played_cards: Vec<Card> = played
                        .into_iter()
                        .take(HISTORY_ARTISTS_LIMIT)
                        .filter_map(|a| {
                            let id = a.id?;
                            let thumbs = app.artist_photos.get(&id).cloned().filter(|t| !t.is_empty())
                                .or_else(|| followed.iter().find(|c| matches!(&c.target, Target::Artist(cid) if cid == &id)).map(|c| c.thumbs.clone()))
                                .or_else(|| app.artists_cache.get(&id).map(|(a, _)| if a.photo.is_empty() { a.banner.clone() } else { a.photo.clone() }))
                                .or_else(|| app.artist.as_ref().filter(|a| a.id == id).map(|a| if a.photo.is_empty() { a.banner.clone() } else { a.photo.clone() }))
                                .unwrap_or_default();
                            if thumbs.is_empty() && !app.artist_photos.contains_key(&id) {
                                app.artist_photos.insert(id.clone(), Vec::new());
                                app.backend.request(crate::backend::Req::ArtistPhoto(id.clone()));
                            }
                            Some(Card {
                                title: a.name,
                                subtitle: String::new(),
                                thumbs,
                                target: Target::Artist(id),
                                round: true,
                                links: Vec::new(),
                            })
                        })
                        .collect();
                    grid(ui, &played_cards, &mut app.actions);
                    ui.add_space(10.0);
                }
            }
        }
        // Liked Music, kept like any playlist.
        LibTab::Songs => {
            let liked_id = app.active_liked_id();
            if app.opened(liked_id).is_none() {
                placeholder_track_list(app, ui, now, true);
                return;
            }
            let more = app.receiving(liked_id);
            let Some(o) = app.lists.get(liked_id) else { return };
            let liked = &o.c.tracks;
            if !liked.is_empty() {
                ui.horizontal(|ui| {
                    play_pill(ui, &mut app.actions, &Target::Playlist(liked_id.into()), liked);
                    if pill(ui, &t!("shuffle"), Some(icon::SHUFFLE), false).clicked() {
                        let mut t = liked.clone();
                        super::shuffle(&mut t);
                        app.actions.push(Action::Play(t, 0));
                    }
                    select_pill(ui, &mut app.selection, &app.page);
                });
                ui.add_space(10.0);
            }
            list(ui, &rows, &app.liked, liked, true, plain, ListCtx { sel: &mut app.selection, page: &app.page, own: false, plays: false }, &mut app.actions);
            if more {
                widgets::skeleton_rows(ui, now, 3);
            }
        }
        _ => {
            let Some(lib) = app.active_library().cloned() else {
                let with_new = tab == LibTab::Playlists && !app.spotify_mode;
                placeholder_grid(app, ui, now, CARD, false, 12, with_new);
                return;
            };
            match tab {
                LibTab::Playlists => {
                    if !app.spotify_mode {
                        if pill(ui, "New playlist", None, false).clicked() {
                            app.actions.push(Action::NewPlaylist(Vec::new()));
                        }
                        ui.add_space(14.0);
                    }
                    grid(ui, &lib.playlists, &mut app.actions)
                }
                LibTab::Albums => grid(ui, &lib.albums, &mut app.actions),
                LibTab::Songs | LibTab::History | LibTab::Artists => unreachable!(),
            }
        }
    }
}

// ------------------------------------------------------------------ downloads

fn downloads(app: &mut App, ui: &mut Ui, now: f64) {
    page_title(ui, "Downloads");
    let songs = crate::downloads::list();
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!("{} songs", songs.len()))
            .font(theme::regular(13.0))
            .color(theme::text_dim()),
    );
    ui.add_space(12.0);
    if songs.is_empty() {
        ui.label(RichText::new("No downloads yet").font(theme::semibold(16.0)).color(theme::text()));
        ui.add_space(10.0);
        if pill(ui, "Open folder", Some(icon::FOLDER_OPEN), false).clicked() {
            crate::downloads::open_folder();
        }
        return;
    }
    ui.horizontal(|ui| {
        if pill(ui, &t!("play"), Some(icon::PLAY), true).clicked() {
            app.actions.push(Action::Play(songs.clone(), 0));
        }
        if pill(ui, &t!("shuffle"), Some(icon::SHUFFLE), false).clicked() {
            let mut t = songs.clone();
            super::shuffle(&mut t);
            app.actions.push(Action::Play(t, 0));
        }
        if pill(ui, "Open folder", Some(icon::FOLDER_OPEN), false).clicked() {
            crate::downloads::open_folder();
        }
    });
    ui.add_space(12.0);
    let rows = Rows::new(app, now);
    list(ui, &rows, &app.liked, &songs, true, |_, _| None, ListCtx { sel: &mut app.selection, page: &app.page, own: false, plays: false }, &mut app.actions);
}

// ------------------------------------------------------------------ settings

fn settings(app: &mut App, ui: &mut Ui) {
    // One centred column; every section shares its width.
    let avail = ui.available_width();
    let w = avail.min(920.0);
    ui.horizontal(|ui| {
        ui.add_space(((avail - w) / 2.0).max(0.0));
        ui.vertical(|ui| {
            ui.set_width(w);
            settings_body(app, ui);
            ui.add_space(40.0);
        });
    });
}

fn settings_body(app: &mut App, ui: &mut Ui) {
    page_title(ui, "Settings");
    settings_account(app, ui);
    settings_appearance(app, ui);
    settings_playback(app, ui);
    settings_audio(app, ui);
    settings_downloads(ui);
    settings_notifications(app, ui);
    settings_window(app, ui);
    settings_connections(app, ui);
    if !app.spotify_mode {
        super::transfer::settings_section(app, ui);
    }
    settings_network(app, ui);
    settings_shortcuts(ui);
    settings_updates(app, ui);
}

/// Settings › Account: signing in and out, and which channel to use.
/// Settings › Updates: the version, what the updater is doing, and a button to check
/// now (or restart into a downloaded update).
fn settings_updates(app: &mut App, ui: &mut Ui) {
    use crate::update::State;
    section_label(ui, "Updates", None);
    let version = crate::update::current_version();
    let state = app.updater.as_ref().map(|u| u.state()).unwrap_or(State::Idle);
    let can_update = super::updatable();
    let status = match &state {
        _ if !can_update => "Updates install in the app from the .dmg, opened from Applications.".to_owned(),
        State::Idle => "Dawn FM checks for updates each time it opens.".to_owned(),
        State::Checking => "Checking for updates…".to_owned(),
        State::UpToDate => "You're up to date.".to_owned(),
        State::Downloading { version, percent } => format!("Downloading Dawn FM {version}… {percent}%"),
        State::Ready { version, .. } => format!("Dawn FM {version} is ready. Restart to update."),
        State::Failed(e) => format!("Couldn't check for updates: {e}"),
    };
    let mut check = false;
    let mut restart = false;
    settings_card(ui, |ui| {
        setting_row(ui, &format!("Dawn FM {version}"), Some(&status), |ui| {
            match &state {
                State::Ready { .. } => restart = pill(ui, "Restart to Update", Some(icon::SPARKLES), true).clicked(),
                State::Downloading { percent, .. } => {
                    let progress = (*percent as f32 / 100.0).clamp(0.0, 1.0);
                    ui.add(egui::ProgressBar::new(progress).desired_width(120.0).show_percentage());
                }
                State::Checking => {
                    ui.spinner();
                }
                _ => {
                    let resp = ui.add_enabled_ui(can_update, |ui| pill(ui, "Check for Updates", None, false)).inner;
                    check = resp.clicked() && can_update;
                }
            }
        });
    });
    if check
        && let Some(u) = &app.updater
    {
        u.check();
        app.update_checked = Some(ui.input(|i| i.time));
    }
    if restart {
        app.restart_to_update(ui.ctx());
    }
}

fn settings_account(app: &mut App, ui: &mut Ui) {
    section_label(ui, "Account", None);
    let busy = app.signing_in.load(Ordering::Relaxed);
    let (mut sign_in, mut sign_out) = (false, false);
    // Asked again now and then while Settings shows: a failed ask, or a channel made since.
    let now = ui.input(|i| i.time);
    if app.logged_in && !app.spotify_mode && app.channels_asked.is_none_or(|at| now - at > 60.0) {
        app.channels_asked = Some(now);
        app.backend.request(crate::backend::Req::Channels);
    }
    if app.spotify_user.is_none() {
        settings_card(ui, |ui| {
            if app.logged_in {
                let name = app.account_name.clone().unwrap_or_else(|| "YouTube Music".to_owned());
                setting_row(ui, &name, Some("Signed in to YouTube Music with Google. Sign out to switch to Spotify. Saved in your macOS Keychain."), |ui| {
                    sign_out = pill_destructive(ui, "Sign out", Some(icon::LOG_OUT)).clicked();
                });
                // Also with one, when the one in use isn't it (a brand channel since deleted).
                if app.channels.as_ref().is_some_and(|c| c.list.len() > 1 || !c.list.iter().any(|l| l.page_id == c.current)) {
                    divider(ui);
                    setting_row(ui, "Channel", Some("Your Google account and each YouTube channel it manages have their own library, likes, history and recommendations."), |ui| {
                        channel_picker(app, ui);
                    });
                }
            } else {
                setting_row(ui, "YouTube Music", Some("Sign in with Google for your YouTube Music home feed, library and likes."), |ui| {
                    if busy {
                        ui.spinner();
                    } else {
                        sign_in = pill(ui, "Sign in with Google", Some(icon::CIRCLE_USER), true).clicked();
                    }
                });
            }
        });
        if busy {
            ui.add_space(6.0);
            ui.label(RichText::new("Finish signing in in the window that opened.").font(theme::regular(11.0)).color(theme::text_dim()));
        }
        if sign_in {
            app.start_sign_in(ui.ctx());
        }
        if sign_out {
            app.backend.request(crate::backend::Req::Logout);
        }
    }
    if !app.logged_in {
        if app.spotify_user.is_none() {
            ui.add_space(12.0);
        }
        settings_spotify(app, ui);
    }
}

fn settings_spotify(app: &mut App, ui: &mut Ui) {
    settings_card(ui, |ui| {
        if let Some(username) = app.spotify_user.clone() {
            setting_row(
                ui,
                &format!("Spotify ({username})"),
                Some("Signed in to Spotify (Free & Premium). Sign out to switch to YouTube Music."),
                |ui| {
                    if pill_destructive(ui, "Sign out", Some(icon::LOG_OUT)).clicked() {
                        app.backend.request(crate::backend::Req::SpotifyLogout);
                    }
                },
            );
            return;
        }
        if let Some(ch) = app.spotify_challenge.clone() {
            setting_row(
                ui,
                "Verify Spotify login code",
                Some(&format!("Enter the {}-character code sent to {}.", ch.code_length, ch.email)),
                |_| {},
            );
            ui.add_space(8.0);
            let enter = super::text_field(ui, "Verification code", &mut app.spotify_code, false, "123456")
                .lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if app.spotify_auth_busy {
                    ui.spinner();
                } else {
                    if pill(ui, "Verify", None, true).clicked() || enter {
                        app.spotify_auth_busy = true;
                        app.backend.request(crate::backend::Req::SpotifyEmailVerify {
                            challenge: ch,
                            code: app.spotify_code.trim().to_owned(),
                        });
                    }
                    if pill(ui, "Cancel", None, false).clicked() {
                        app.spotify_challenge = None;
                        app.spotify_code.clear();
                    }
                }
            });
            return;
        }
        let web_busy = app.spotify_web_signing_in.load(Ordering::Relaxed);
        let mut web_sign_in = false;
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(w, 72.0), Sense::hover());
        let b = egui::Rect::from_min_size(pos2(rect.left(), rect.center().y - 22.0), vec2(44.0, 44.0));
        super::transfer::badge(ui, b);
        let x = b.right() + 16.0;
        let tw = (rect.right() - x - 170.0).max(120.0);
        ui.painter().text(pos2(x, rect.center().y - 10.0), Align2::LEFT_CENTER, "Spotify", theme::semibold(15.5), theme::text());
        let g = widgets::galley_wrapped(
            ui,
            "Sign in with your Free or Premium Spotify account for Spotify playlists, albums and songs.",
            theme::regular(11.0),
            theme::text_dim(),
            tw,
            2,
        );
        ui.painter().galley(pos2(x, rect.center().y + 2.0), g, theme::text_dim());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if web_busy {
            child.spinner();
        } else {
            web_sign_in = pill(&mut child, "Sign in to Spotify", None, true).clicked();
        }
        if web_busy {
            ui.add_space(4.0);
            ui.label(RichText::new("Finish signing in in the window that opened.").font(theme::regular(11.0)).color(theme::text_dim()));
        }
        if web_sign_in {
            app.start_spotify_account_sign_in(ui.ctx());
        }
        divider(ui);
        setting_row(
            ui,
            "Or sign in directly",
            Some("Use a Spotify Email code or Username & Password."),
            |ui| {
                if chip(ui, "Email code", !app.spotify_login_pw).clicked() {
                    app.spotify_login_pw = false;
                }
                if chip(ui, "Password", app.spotify_login_pw).clicked() {
                    app.spotify_login_pw = true;
                }
            },
        );
        ui.add_space(10.0);
        if !app.spotify_login_pw {
            let enter = super::text_field(ui, "Email", &mut app.spotify_email, false, "you@example.com")
                .lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(8.0);
            if app.spotify_auth_busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    if pill(ui, "Cancel", None, false).clicked() {
                        app.spotify_auth_busy = false;
                    }
                });
            } else if pill(ui, "Send login code", Some(icon::CIRCLE_USER), true).clicked() || enter {
                app.spotify_auth_busy = true;
                app.backend.request(crate::backend::Req::SpotifyEmailStart(app.spotify_email.trim().to_owned()));
            }
        } else {
            super::text_field(ui, "Email or username", &mut app.spotify_email, false, "you@example.com");
            ui.add_space(6.0);
            let enter = super::dialogs::password_field(ui, "Password", &mut app.spotify_password, "Password")
                .lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(8.0);
            if app.spotify_auth_busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    if pill(ui, "Cancel", None, false).clicked() {
                        app.spotify_auth_busy = false;
                    }
                });
            } else if pill(ui, "Sign in to Spotify", Some(icon::CIRCLE_USER), true).clicked() || enter {
                app.spotify_auth_busy = true;
                app.backend.request(crate::backend::Req::SpotifyLogin {
                    username: app.spotify_email.trim().to_owned(),
                    password: app.spotify_password.clone(),
                });
            }
        }
    });
}

fn channel_picker(app: &mut App, ui: &mut Ui) {
    let Some(crate::backend::Channels { list, current, epoch }) = &app.channels else { return };
    // A brand channel often has the account's name: the handle, or "No channel" (as
    // YouTube's switcher says), tells them apart.
    let label = |c: &crate::ytm::Channel| match (&c.handle, c.has_channel) {
        (Some(h), _) => format!("{} ({h})", c.name),
        (None, false) => format!("{} (No channel)", c.name),
        (None, true) => c.name.clone(),
    };
    // The button is narrower: a handle alone is enough there.
    let shown = match list.iter().find(|c| c.page_id == *current) {
        Some(c) => c.handle.clone().unwrap_or_else(|| label(c)),
        None => "Another channel".to_owned(),
    };
    // A transfer would go on as the new channel.
    if app.transfer.running() {
        ui.label(RichText::new("Not while a transfer runs").font(theme::regular(13.0)).color(theme::text_dim()));
        return;
    }
    let switching = app.switching_to.is_some();
    let btn = dropdown_button(ui, if switching { "Switching…" } else { &shown }, 220.0, "Channel");
    let mut pick = None;
    widgets::popup_menu(&btn, None, |ui| {
        ui.set_width(300.0);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            ui.add_space(4.0);
            for c in list {
                if option_row(ui, &label(c), c.page_id == *current).clicked() {
                    pick = Some(c.clone());
                    ui.close();
                }
            }
            ui.add_space(4.0);
        });
    });
    if let Some(c) = pick.filter(|c| c.page_id != *current && !switching) {
        app.switching_to = Some(label(&c));
        app.backend.request(crate::backend::Req::SwitchChannel { page_id: c.page_id, epoch: *epoch });
    }
}

/// Settings › Playback.
fn settings_playback(app: &mut App, ui: &mut Ui) {
    section_label(ui, "Playback", None);
    settings_card(ui, |ui| {
        setting_row(ui, "Autoplay", Some("Keep playing similar songs when the queue ends."), |ui| {
            toggle(ui, &mut app.autoplay, "Autoplay");
        });
        divider(ui);
        setting_row(ui, "Output device", Some("Where the music plays. A picked device that isn't connected leaves it to the Mac's default until it's back."), |ui| {
            output_device_picker(app, ui);
        });
        divider(ui);
        setting_row(ui, "Play songs, not music videos", Some("An official music video plays as its song, the album recording, when YouTube Music has one: no intros, skits or video-only edits. Lists still show what they hold."), |ui| {
            toggle(ui, &mut app.play_songs, "Play songs, not music videos");
        });
        divider(ui);
        setting_row(ui, "Skip non-music parts", Some("Jump over talking, skits and sponsor breaks in music videos, as marked by the SponsorBlock community. Album songs don't have any."), |ui| {
            if toggle(ui, &mut app.sponsorblock, "Skip non-music parts").changed() {
                app.request_segments();
            }
        });
        divider(ui);
        setting_row(ui, "Automix", Some("Slide from one song into the next the way a DJ would: both hold their level for a moment, then swap in one smooth move, instead of a straight fade. The mix picks its own length, in phrases of 8 or 16 bars (16 on long songs), following the beat it hears."), |ui| {
            if toggle(ui, &mut app.automix, "Automix").changed() {
                app.set_crossfade();
            }
        });
        if !app.automix {
            divider(ui);
            setting_row(ui, "Crossfade songs", Some("Blend the end of each song into the start of the next."), |ui| {
                if toggle(ui, &mut app.crossfade, "Crossfade songs").changed() {
                    app.set_crossfade();
                }
            });
            if app.crossfade {
                divider(ui);
                setting_row(ui, "Crossfade length", None, |ui| {
                    if seconds_slider(ui, &mut app.crossfade_secs, 1..=super::CROSSFADE_MAX, "Crossfade length").changed() {
                        app.set_crossfade();
                    }
                });
            }
        }
    });
}

/// Settings › Audio quality: loudness and where the audio comes from.
fn settings_audio(app: &mut App, ui: &mut Ui) {
    section_label(ui, "Audio quality", None);
    settings_card(ui, |ui| {
        setting_row(ui, "Volume normalization", Some("Turn loud songs down to the same level, like YouTube Music does."), |ui| {
            if toggle(ui, &mut app.normalize, "Volume normalization").changed() {
                app.audio.set_normalize(app.normalize);
            }
        });
        divider(ui);
        setting_row(ui, "Logarithmic volume", Some("Space the volume slider out the way ears hear loudness, so quiet levels aren't squeezed into its first few pixels."), |ui| {
            if toggle(ui, &mut app.log_volume, "Logarithmic volume").changed() {
                app.audio.set_log_volume(app.log_volume);
            }
        });
        divider(ui);
        setting_row(ui, "Hi-Res audio", Some("Addon servers that serve hi-res copies of songs. When one has the song, its copy plays instead of YouTube's, and its quality shows in the player."), |_| {});
        divider(ui);
        ui.add_space(12.0);
        hires_sources_editor(app, ui);
        ui.add_space(10.0);
        divider(ui);
        setting_row(ui, "Prefer Dolby Atmos", Some("Ask the hi-res sources for a Dolby Atmos copy when they offer one. Falls back to the best stereo copy when the stream's codec can't be decoded."), |ui| {
            if toggle(ui, &mut app.prefer_atmos, "Prefer Dolby Atmos").changed() {
                app.backend.set_prefer_atmos(app.prefer_atmos);
            }
        });
    });
}

/// The hi-res source list: one line per addon, and "+ Add a source" below it that
/// opens a fresh line for the next one.
fn hires_sources_editor(app: &mut App, ui: &mut Ui) {
    let mut edited = false;
    let mut remove = None;
    for i in 0..app.hires_sources.len() {
        ui.horizontal(|ui| {
            let field_w = (ui.available_width() - 34.0).max(120.0);
            ui.allocate_ui_with_layout(vec2(field_w, 42.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(field_w);
                if super::text_field(ui, "", &mut app.hires_sources[i], false, "https://your-addon.example").changed() {
                    edited = true;
                }
            });
            if widgets::icon_button(ui, icon::X, 14.0, "Remove source", false).clicked() {
                remove = Some(i);
            }
        });
        ui.add_space(6.0);
    }
    if let Some(i) = remove {
        app.hires_sources.remove(i);
        edited = true;
    }
    if edited {
        app.backend.set_hires_source(&live_sources(app));
    }
    if app.hires_adding {
        let resp = super::text_field(ui, "", &mut app.hires_draft, false, "https://your-addon.example");
        if app.hires_focus {
            resp.request_focus();
            app.hires_focus = false;
        }
        if resp.lost_focus() {
            let link = app.hires_draft.trim().to_owned();
            if !link.is_empty() && !app.hires_sources.iter().any(|s| s.trim() == link) {
                app.hires_sources.push(link);
                app.backend.set_hires_source(&live_sources(app));
            }
            app.hires_draft.clear();
            app.hires_adding = false;
        }
        ui.add_space(6.0);
    } else if pill(ui, "+  Add a source", None, false).clicked() {
        app.hires_adding = true;
        app.hires_focus = true;
        app.hires_draft.clear();
    }
}

/// The non-empty links as one newline-separated list, the way the backend reads them.
fn live_sources(app: &App) -> String {
    app.hires_sources.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n")
}

/// Settings › Downloads.
fn settings_downloads(ui: &mut Ui) {
    section_label(ui, "Downloads", None);
    ui.label(
        RichText::new("Download songs from their menu. They're saved at the best quality your account gets, tagged with their cover art, and play from the file, even offline.")
            .font(theme::regular(11.0))
            .color(theme::text_dim()),
    );
    ui.add_space(12.0);
    settings_card(ui, |ui| {
        let folder = crate::downloads::folder().display().to_string();
        setting_row(ui, "Download folder", Some(&folder), |ui| {
            if pill(ui, "Change", None, false).clicked() {
                crate::downloads::pick_folder(ui.ctx());
            }
            ui.add_space(6.0);
            if pill(ui, "Open", Some(icon::FOLDER_OPEN), false).clicked() {
                crate::downloads::open_folder();
            }
        });
    });
}


/// Settings › Appearance.
fn settings_appearance(app: &mut App, ui: &mut Ui) {
    section_label(ui, "Appearance", None);
    settings_card(ui, |ui| {
        let desc = "Light or dark pages. System follows your system's setting. Now playing stays dark, over the album art.";
        let mut pick = None;
        setting_row(ui, "Mode", Some(desc), |ui| {
            let btn = dropdown_button(ui, app.appearance.label(), 160.0, "Mode");
            widgets::popup_menu(&btn, None, |ui| {
                ui.set_width(160.0);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                ui.add_space(4.0);
                for a in theme::Appearance::ALL {
                    if option_row(ui, a.label(), a == app.appearance).clicked() {
                        pick = Some(a);
                        ui.close();
                    }
                }
                ui.add_space(4.0);
            });
        });
        if let Some(a) = pick {
            app.appearance = a;
        }
        divider(ui);
        // The built-in look's colour: its accent, and the tint of every page.
        if app.look().id == crate::preset::DEFAULT_ID {
            setting_row(ui, "Colour", Some("The accent, and the tint of the pages behind everything."), |ui| {
                use crate::preset::COLOURS;
                const SIDE: f32 = 20.0;
                const GAP: f32 = 5.0;
                let (row, _) = ui.allocate_exact_size(vec2(COLOURS.len() as f32 * (SIDE + GAP) - GAP, SIDE + 8.0), Sense::hover());
                let current = crate::preset::colour().id;
                for (i, c) in COLOURS.iter().enumerate() {
                    let r = Rect::from_center_size(pos2(row.left() + SIDE / 2.0 + i as f32 * (SIDE + GAP), row.center().y), Vec2::splat(SIDE));
                    let resp = ui.interact(r.expand(GAP / 2.0), ui.scope_id().with(("colour", c.id)), Sense::click());
                    let resp = widgets::named(resp, c.name).on_hover_text(c.name).on_hover_cursor(egui::CursorIcon::PointingHand);
                    let h = if resp.hovered() { 1.0 } else { 0.0 };
                    let on = theme::anim_bool(ui.ctx(), resp.id.with("on"), c.id == current, 0.15);
                    let fill = crate::preset::hex(c.accent).unwrap_or(theme::RED);
                    ui.painter().circle_filled(r.center(), SIDE / 2.0 - 2.0 + h, fill);
                    if on > 0.0 {
                        ui.painter().circle_stroke(r.center(), SIDE / 2.0 + 1.5, egui::Stroke::new(1.5, theme::with_alpha(theme::text(), on)));
                    }
                    if resp.clicked() {
                        crate::preset::set_colour(c.id);
                        // egui's own colours (the text selection) follow.
                        theme::apply_chrome(ui.ctx(), app.look());
                    }
                }
            });
            divider(ui);
        }

        setting_row(ui, "Size", Some("Everything in the app, larger or smaller. ⌘+ and ⌘− do the same from anywhere, and ⌘0 goes back to 100%."), |ui| {
            // Right to left: larger, the size (a click resets it), smaller.
            let ctx = ui.ctx().clone();
            let z = ctx.zoom_factor();
            let step = |ui: &mut Ui, up: bool, on: bool| -> bool {
                let name = if up { "Larger" } else { "Smaller" };
                let (r, resp) = ui.allocate_exact_size(vec2(32.0, 30.0), if on { Sense::click() } else { Sense::hover() });
                let resp = widgets::named(resp, name).on_hover_text(name);
                let h = if on && resp.hovered() { 1.0 } else { 0.0 };
                if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
                    crate::glass::paint(ui, resp.id.with("glass"), r, crate::glass::control(8.0, theme::light(), h, if on { 1.0 } else { 0.55 }));
                } else {
                    ui.painter().rect_filled(r, 8.0, theme::glass(0.08 + 0.08 * h));
                }
                let stroke = egui::Stroke::new(1.6, theme::with_alpha(theme::text(), if on { 1.0 } else { 0.35 }));
                ui.painter().hline((r.center().x - 5.0)..=(r.center().x + 5.0), r.center().y, stroke);
                if up {
                    ui.painter().vline(r.center().x, (r.center().y - 5.0)..=(r.center().y + 5.0), stroke);
                }
                resp.clicked()
            };
            ui.spacing_mut().item_spacing.x = 6.0;
            if step(ui, true, z < *super::shell::ZOOM_RANGE.end() - 0.001) {
                super::shell::zoom_step(&ctx, true);
            }
            let (r, resp) = ui.allocate_exact_size(vec2(52.0, 30.0), Sense::click());
            let resp = widgets::named(resp, "Reset size").on_hover_text("Back to 100%").on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("{:.0}%", z * 100.0), theme::semibold(13.5), theme::text());
            if resp.clicked() {
                ctx.set_zoom_factor(1.0);
            }
            if step(ui, false, z > *super::shell::ZOOM_RANGE.start() + 0.001) {
                super::shell::zoom_step(&ctx, false);
            }
        });
        divider(ui);
        setting_row(ui, "Home gradient", Some("Show a soft ambient artwork gradient behind the Home page."), |ui| {
            toggle(ui, &mut app.home_gradient, "Home gradient");
        });
    });
}

/// Settings › Notifications: what a song change says on the desktop.
fn settings_notifications(app: &mut App, ui: &mut Ui) {
    section_label(ui, "Notifications", None);
    settings_card(ui, |ui| {
        setting_row(ui, "Desktop notifications", Some("Show a macOS notification when a new song starts while the window isn't in front."), |ui| {
            toggle(ui, &mut app.notify_songs, "Desktop notifications");
        });
    });
}

/// Settings › Window.
fn settings_window(app: &mut App, ui: &mut Ui) {
    section_label(ui, "Window", None);
    settings_card(ui, |ui| {
        let api = crate::renderer::api_name();
        let title = format!("Draw with {api}");
        let desc = format!("Use {api} instead of OpenGL. Turn this on if graphics glitch or display incorrectly.");
        setting_row(ui, &title, Some(&desc), |ui| {
            if toggle(ui, &mut app.wgpu_renderer, &title).changed() {
                use crate::renderer::Renderer;
                let r = if app.wgpu_renderer { Renderer::Wgpu } else { Renderer::OpenGl };
                if let Err(e) = crate::renderer::save(app.backend.data_dir(), r) {
                    log::warn!("renderer: couldn't save the setting: {e}");
                }
            }
        });
        if app.wgpu_renderer != (crate::renderer::active() == crate::renderer::Renderer::Wgpu) {
            divider(ui);
            let mut restart = false;
            setting_row(ui, "Restart to apply", Some("The app draws with the new renderer from the next start."), |ui| {
                restart = pill(ui, "Restart now", None, true).clicked();
            });
            if restart {
                app.restart(ui.ctx());
            }
        }
    });
}

/// Settings › Connections.
fn settings_connections(app: &mut App, ui: &mut Ui) {
    let mut discord_enabled = app.discord_enabled;
    section_label(ui, "Connections", None);
    settings_card(ui, |ui| {
        setting_row(ui, "Discord", Some("Show what you're playing on your Discord profile."), |ui| {
            toggle(ui, &mut discord_enabled, "Discord");
        });
        divider(ui);
        let busy = app.lastfm_busy.load(Ordering::Relaxed);
        let desc = match &app.lastfm_user {
            Some(name) => format!("Scrobbling what you play to {name}'s profile."),
            None if busy => "Allow Dawn FM on the Last.fm page that opened in your browser.".to_owned(),
            None => "Scrobble what you play to your Last.fm profile.".to_owned(),
        };
        let (mut connect, mut disconnect) = (false, false);
        setting_row(ui, "Last.fm", Some(&desc), |ui| {
            if app.lastfm_user.is_some() {
                disconnect = pill(ui, "Disconnect", None, false).clicked();
            } else if busy {
                ui.spinner();
            } else {
                connect = pill(ui, "Connect", None, false).clicked();
            }
        });
        if connect {
            app.lastfm_sign_in();
        }
        if disconnect {
            app.lastfm_sign_out();
        }
    });
    if discord_enabled != app.discord_enabled {
        app.set_discord_enabled(discord_enabled);
    }
}

/// Settings › Keyboard shortcuts.
fn settings_shortcuts(ui: &mut Ui) {
    let open = section_fold(ui, "Keyboard shortcuts");
    fold_body(ui, fold_state(ui, "Keyboard shortcuts").0, open, |ui| settings_card(ui, |ui| {
        let keys: &[(&str, &[&str])] = &[
            ("Play / pause", &["Space"]),
            ("Next / previous song", &["⌘", "→ / ←"]),
            ("Volume up / down", &["⌘", "↑ / ↓"]),
            ("Search", &["⌘", "F"]),
            ("Lyrics panel", &["⌘", "L"]),
            ("Settings", &["⌘", ","]),
            ("Larger / smaller", &["⌘", "+ / −"]),
            ("Actual size", &["⌘", "0"]),
            ("Mini player", &["⌥", "⌘", "M"]),
            ("Lyrics in now playing", &["L"]),
            ("Queue", &["Q"]),
            ("Full-screen lyrics (in now playing)", &["F"]),
            ("Back / forward", &["⌥", "← / →"]),
            ("Close now playing", &["Esc"]),
        ];
        for (i, (what, combo)) in keys.iter().enumerate() {
            if i > 0 {
                divider(ui);
            }
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
            ui.painter().text(pos2(rect.left(), rect.center().y), Align2::LEFT_CENTER, *what, theme::regular(14.0), theme::text());
            let mut x = rect.right();
            for (j, k) in combo.iter().enumerate().rev() {
                x = keycap(ui, pos2(x, rect.center().y), k) - 6.0;
                if j > 0 {
                    let plus = ui.painter().text(pos2(x, rect.center().y), Align2::RIGHT_CENTER, "+", theme::regular(13.0), theme::text_dim());
                    x = plus.left() - 6.0;
                }
            }
        }
    }));
}

/// Settings › Network: collapsible section for proxy configuration and connection testing.
fn settings_network(app: &mut App, ui: &mut Ui) {
    let open = section_fold(ui, "Network");
    fold_body(ui, fold_state(ui, "Network").0, open, |ui| settings_card(ui, |ui| network(app, ui)));
}

/// Settings › Network: the proxy everything goes through, a connection test, and a
/// restart when the setting changed.
fn network(app: &mut App, ui: &mut Ui) {
    use crate::proxy::{self, Setting};
    let same = |a: &Setting, b: &Setting| std::mem::discriminant(a) == std::mem::discriminant(b);
    let mut pick = None;
    setting_row(ui, "Proxy", Some("Send everything Dawn FM fetches through a proxy: songs, lyrics, sign-in and Listen together."), |ui| {
        let btn = dropdown_button(ui, app.proxy_mode.label(), 220.0, "Proxy");
        widgets::popup_menu(&btn, None, |ui| {
            ui.set_width(220.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.add_space(4.0);
            for s in [Setting::System, Setting::Off, Setting::Custom(String::new())] {
                if option_row(ui, s.label(), same(&s, &app.proxy_mode)).clicked() {
                    pick = Some(s);
                    ui.close();
                }
            }
            ui.add_space(4.0);
        });
    });
    if let Some(s) = pick {
        app.proxy_error = None;
        *app.proxy_test.lock_unpoisoned() = None;
        app.proxy_mode = s.clone();
        // System and Off take effect as picked; a custom address once it's saved.
        if !matches!(s, Setting::Custom(_)) {
            match proxy::save(app.backend.data_dir(), &s) {
                Ok(()) => app.proxy_saved = s,
                Err(e) => app.proxy_error = Some(format!("{e:#}")),
            }
        }
    }
    match &app.proxy_mode {
        Setting::System => {
            divider(ui);
            let desc = match proxy::system() {
                Some(p) => format!("Using {}.", proxy::redact(&p)),
                None => "Dawn FM uses the HTTPS_PROXY or HTTP_PROXY environment variable when one is set, and otherwise connects directly.".to_owned(),
            };
            setting_row(ui, "System proxy", Some(&desc), |_| {});
        }
        Setting::Custom(_) => {
            divider(ui);
            ui.add_space(12.0);
            super::text_field(ui, "Address", &mut app.proxy_url, false, "http://127.0.0.1:8080 or socks5://user:pass@host:1080");
            ui.add_space(10.0);
            let custom = Setting::Custom(app.proxy_url.trim().to_owned());
            let saved = app.proxy_saved == custom;
            let mut save = false;
            let save_label = if saved { t!("saved") } else { t!("save") };
            ui.horizontal(|ui| {
                save = pill(ui, &save_label, None, !saved).clicked() && !saved;
                ui.add_space(8.0);
                ui.label(RichText::new("The address, password included, is kept in your macOS Keychain.").font(theme::regular(12.5)).color(theme::text_dim()));
            });
            if save {
                match proxy::save(app.backend.data_dir(), &custom) {
                    Ok(()) => {
                        app.proxy_saved = custom;
                        app.proxy_error = None;
                    }
                    Err(e) => app.proxy_error = Some(format!("{e:#}")),
                }
            }
            ui.add_space(12.0);
        }
        Setting::Off => {}
    }
    if let Some(e) = &app.proxy_error {
        ui.label(RichText::new(e).font(theme::regular(12.5)).color(theme::RED));
        ui.add_space(8.0);
    }
    divider(ui);
    let result = app.proxy_test.lock_unpoisoned().clone();
    let desc = match &result {
        Some(Some((_, msg))) => msg.clone(),
        Some(None) => "Checking…".to_owned(),
        None => "Checks that YouTube Music answers through this setting.".to_owned(),
    };
    setting_row(ui, "Test connection", Some(&desc), |ui| {
        if matches!(result, Some(None)) {
            ui.spinner();
        } else if pill(ui, "Test", None, false).clicked() {
            let s = match &app.proxy_mode {
                Setting::Custom(_) => Setting::Custom(app.proxy_url.trim().to_owned()),
                other => other.clone(),
            };
            let (slot, b) = (app.proxy_test.clone(), app.backend.clone());
            *slot.lock_unpoisoned() = Some(None);
            app.backend.spawn(async move {
                let out = match proxy::test(&s).await {
                    Ok(t) => (true, format!("Working: YouTube Music answered in {} ms.", t.as_millis())),
                    Err(e) => (false, format!("Didn't connect: {e:#}")),
                };
                *slot.lock_unpoisoned() = Some(Some(out));
                b.wake();
            });
        }
    });
    if app.proxy_saved != *proxy::active() {
        divider(ui);
        let mut restart = false;
        setting_row(ui, "Restart to apply", Some("The new proxy setting is used from the next start."), |ui| {
            restart = pill(ui, "Restart now", None, true).clicked();
        });
        if restart {
            app.restart(ui.ctx());
        }
    }
}

const SYSTEM_DEFAULT: &str = "System default";

fn output_device_picker(app: &mut App, ui: &mut Ui) {
    let current = match &app.output_device {
        None => SYSTEM_DEFAULT.to_owned(),
        Some((id, name)) => match app.output_devices.iter().find(|d| d.id == *id) {
            Some(d) => d.name.clone(),
            None if app.output_devices.is_empty() => name.clone(),
            None => format!("{name} (not connected)"),
        },
    };
    let btn = dropdown_button(ui, &current, 210.0, "Output device");
    // Listed when opened, so a device that just connected shows up.
    if btn.clicked() || app.output_devices.is_empty() {
        app.output_devices = crate::audio::output_devices();
    }
    let mut pick: Option<Option<(String, String)>> = None;
    widgets::popup_menu(&btn, None, |ui| {
        ui.set_width(340.0);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            ui.add_space(4.0);
            if option_row(ui, SYSTEM_DEFAULT, app.output_device.is_none()).clicked() {
                pick = Some(None);
                ui.close();
            }
            for d in &app.output_devices {
                if option_row(ui, &d.name, app.output_device.as_ref().is_some_and(|(id, _)| *id == d.id)).clicked() {
                    pick = Some(Some((d.id.clone(), d.name.clone())));
                    ui.close();
                }
            }
            ui.add_space(4.0);
        });
    });
    if let Some(pick) = pick {
        app.audio.send(crate::audio::AudioCmd::SetDevice(pick.as_ref().map(|(id, _)| id.clone())));
        app.output_device = pick;
    }
}

/// A dropdown's closed state: the chosen value and a chevron on a glass pill.
fn dropdown_button(ui: &mut Ui, value: &str, width: f32, name: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click());
    let resp = widgets::named(resp, name).on_hover_cursor(egui::CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let h = if resp.hovered() || open { 1.0 } else { 0.0 };
    if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
        crate::glass::paint(ui, resp.id.with("glass"), rect, crate::glass::control(10.0, theme::light(), h, 1.0));
    } else {
        ui.painter().rect_filled(rect, 10.0, theme::glass(0.08 + 0.05 * h));
    }
    widgets::text_at(ui, pos2(rect.left() + 14.0, rect.center().y - 9.0), value, theme::regular(14.0), theme::text(), rect.width() - 44.0);
    let glyph = if open { theme::icon::CHEVRON_UP } else { theme::icon::CHEVRON_DOWN };
    widgets::icon_at(ui, pos2(rect.right() - 18.0, rect.center().y), glyph, 15.0, theme::lerp_color(theme::text_dim(), theme::text(), h));
    resp
}

/// A dropdown option: macOS menu item style with theme accent hover and leading check.
fn option_row(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 21.0), Sense::click());
    let resp = widgets::named(resp, label);
    let over = resp.hovered();
    if over {
        ui.painter().rect_filled(rect, 4.0, app_accent(ui));
    }
    let fg = if over {
        Color32::WHITE
    } else if theme::light() {
        Color32::from_rgb(32, 32, 32)
    } else {
        Color32::from_rgb(235, 235, 235)
    };
    if selected {
        widgets::icon_at(ui, pos2(rect.left() + 12.0, rect.center().y), theme::icon::CHECK, 12.5, fg);
    }
    widgets::text_at(ui, pos2(rect.left() + 25.0, rect.center().y - 8.5), label, theme::regular(13.0), fg, rect.width() - 32.0);
    resp
}

/// Small heading above a settings card, with an optional value on the right.
pub(super) fn section_label(ui: &mut Ui, title: &str, value: Option<&str>) {
    ui.add_space(30.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    ui.painter().text(pos2(rect.left() + 2.0, rect.center().y), Align2::LEFT_CENTER, title, theme::bold(20.0), theme::text());
    if let Some(v) = value {
        ui.painter().text(pos2(rect.right() - 2.0, rect.center().y), Align2::RIGHT_CENTER, v, theme::semibold(13.0), theme::text_dim());
    }
    ui.add_space(10.0);
}

/// Whether a folding part of Settings is open (remembered; closed at first).
fn fold_state(ui: &Ui, key: &str) -> (egui::Id, bool) {
    let id = egui::Id::unique(("settings-fold", key));
    (id, ui.data_mut(|d| *d.get_persisted_mut_or_default::<bool>(id)))
}

/// A chevron that turns as `open` animates.
fn chevron(ui: &Ui, at: egui::Pos2, id: egui::Id, open: bool, color: Color32) {
    let k = theme::anim_bool(ui.ctx(), id.with("turn"), open, 0.2);
    // Drawn, not the font's two glyphs: it turns through the quarter between them.
    let (sin, cos) = (k * std::f32::consts::FRAC_PI_2).sin_cos();
    let turn = |x: f32, y: f32| at + vec2(x * cos - y * sin, x * sin + y * cos);
    let points = vec![turn(-2.0, -4.5), turn(2.5, 0.0), turn(-2.0, 4.5)];
    ui.painter().add(egui::Shape::line(points, egui::Stroke::new(1.7, color)));
}

/// A fold's body, opening and closing over a fifth of a second rather than all at once:
/// laid out at its full height and clipped to the part that's open, fading as it goes.
/// The height is the one measured the frame before.
fn fold_body(ui: &mut Ui, id: egui::Id, open: bool, body: impl FnOnce(&mut Ui)) {
    let t = theme::anim_bool(ui.ctx(), id.with("body"), open, 0.22);
    if t <= 0.0 {
        return;
    }
    let height_id = id.with("body-h");
    let full: f32 = ui.data(|d| d.get_temp(height_id)).unwrap_or(0.0);
    let at = ui.cursor().min;
    let w = ui.available_width();
    let shown = Rect::from_min_size(at, vec2(w, if t >= 1.0 { full } else { full * t }));
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(height_id).max_rect(Rect::from_min_size(at, vec2(w, full.max(1.0)))));
    if t < 1.0 {
        child.set_clip_rect(shown.intersect(ui.clip_rect()));
        child.multiply_opacity(t);
        super::animate(ui.ctx());
    }
    body(&mut child);
    let measured = child.min_rect().height();
    if (measured - full).abs() > 0.5 {
        // First shown, or its content changed: the room it takes follows next frame.
        ui.data_mut(|d| d.insert_temp(height_id, measured));
        ui.ctx().request_repaint();
    }
    ui.allocate_rect(Rect::from_min_size(at, vec2(w, if t >= 1.0 { measured } else { full * t })), Sense::hover());
}

/// A section heading that folds its section away; a click toggles it. True when open.
fn section_fold(ui: &mut Ui, title: &str) -> bool {
    let (id, mut open) = fold_state(ui, title);
    ui.add_space(30.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    let resp = named(resp, title).on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_persisted(id, open));
    }
    let h = if resp.hovered() { 1.0 } else { 0.0 };
    ui.painter().text(pos2(rect.left() + 2.0, rect.center().y), Align2::LEFT_CENTER, title, theme::bold(20.0), theme::text());
    chevron(ui, pos2(rect.right() - 10.0, rect.center().y), id, open, theme::lerp_color(theme::text_dim(), theme::text(), h));
    ui.add_space(10.0);
    open
}

pub fn settings_card(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(2, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
}

/// A settings row: title and description on the left, the control on the right.
pub(super) fn setting_row(ui: &mut Ui, title: &str, desc: Option<&str>, control: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    let text_w = (w - 230.0).max(200.0);
    let title_g = widgets::galley(ui, title, theme::semibold(15.5), theme::text(), text_w);
    let desc_g = desc.map(|d| widgets::galley_wrapped(ui, d, theme::regular(11.0), theme::text_dim(), text_w, 3));
    let text_h = title_g.size().y + desc_g.as_ref().map_or(0.0, |g| g.size().y + 3.0);
    let h = (text_h + 26.0).max(58.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let fade = if ui.is_enabled() { 1.0 } else { 0.45 };
    let top = rect.center().y - text_h / 2.0;
    ui.painter().galley(pos2(rect.left(), top), title_g.clone(), theme::with_alpha(theme::text(), fade));
    if let Some(g) = desc_g {
        ui.painter().galley(pos2(rect.left(), top + title_g.size().y + 3.0), g, theme::with_alpha(theme::text_dim(), fade));
    }
    let ctl = Rect::from_min_max(pos2(rect.left() + text_w + 20.0, rect.top()), rect.max);
    let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(ctl).layout(egui::Layout::right_to_left(egui::Align::Center)));
    control(&mut cui);
}

pub(super) fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::glass(0.07));
}

/// macOS 26/27 Liquid Glass switch (click or drag across the track).
pub(super) fn toggle(ui: &mut Ui, on: &mut bool, name: &str) -> egui::Response {
    let (track, _) = ui.allocate_exact_size(vec2(41.0, 18.0), Sense::hover());
    let accent = app_accent(ui);
    let resp = crate::glass::liquid_toggle(ui, track, on, accent, 0.70, ui.scope_id().with(("toggle", name)), false);
    let value = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), value, name));
    resp
}

/// Whole seconds on a track like the volume slider's, with the ends labelled and the
/// value beside it: click or drag to set (scrolling over it scrolls the page).
fn seconds_slider(ui: &mut Ui, v: &mut u32, range: std::ops::RangeInclusive<u32>, name: &str) -> egui::Response {
    let (lo, hi) = (*range.start(), *range.end());
    let label = |ui: &mut Ui, text: String, w: f32, font: egui::FontId, color: Color32| {
        let (rect, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
        let g = widgets::galley(ui, &text, font, color, w);
        ui.painter().galley(pos2(rect.right() - g.size().x, rect.center().y - g.size().y / 2.0), g, color);
    };
    label(ui, format!("{v} s"), 34.0, theme::semibold(13.0), theme::text());
    ui.add_space(6.0);
    label(ui, format!("{hi} s"), 26.0, theme::regular(12.0), theme::text_dim());
    let (rect, mut resp) = ui.allocate_exact_size(vec2(170.0, 26.0), Sense::click_and_drag());
    label(ui, format!("{lo} s"), 22.0, theme::regular(12.0), theme::text_dim());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 16.0, 4.0));
    let old = *v;
    if let Some(p) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked())
    {
        let t = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0);
        *v = (lo as f32 + t * (hi - lo) as f32).round() as u32;
    }
    if *v != old {
        resp.mark_changed();
    }
    let value = *v;
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), value as f64, name));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let h = if resp.hovered() || resp.dragged() { 1.0 } else { 0.0 };
    let accent = ui.ctx().data(|d| d.get_temp::<Color32>(egui::Id::unique("accent"))).unwrap_or(theme::BLUE);
    let t = (value - lo) as f32 / (hi - lo) as f32;
    let knob = pos2(egui::lerp(track.left()..=track.right(), t), track.center().y);
    ui.painter().rect_filled(track, 2.0, theme::glass(0.15));
    ui.painter().rect_filled(Rect::from_min_max(track.min, pos2(knob.x, track.max.y)), 2.0, accent);
    ui.painter().circle_filled(knob + vec2(0.0, 1.0), 7.0 + h, theme::shade(0.25));
    ui.painter().circle_filled(knob, 7.0 + h, Color32::WHITE);
    resp
}

/// Draw a key cap whose right edge is at `right`; returns its left edge.
fn keycap(ui: &Ui, right: egui::Pos2, key: &str) -> f32 {
    let g = widgets::galley(ui, key, theme::semibold(12.5), theme::text(), 200.0);
    let size = vec2((g.size().x + 18.0).max(30.0), 26.0);
    let rect = Rect::from_min_size(pos2(right.x - size.x, right.y - size.y / 2.0), size);
    if crate::renderer::active() == crate::renderer::Renderer::OpenGl {
        crate::glass::paint(ui, ui.scope_id().with(("keycap", key)), rect, crate::glass::control(7.0, theme::light(), 0.0, 1.0));
    } else {
        ui.painter().rect_filled(rect, 7.0, theme::glass(0.08));
        ui.painter().rect_filled(Rect::from_min_max(pos2(rect.left() + 2.0, rect.bottom() - 2.0), pos2(rect.right() - 2.0, rect.bottom())), 2.0, theme::shade(0.25));
    }
    ui.painter().galley(rect.center() - g.size() / 2.0, g, theme::text());
    rect.left()
}


#[cfg(test)]
mod tests {
    use crate::backend::Collection;

    #[test]
    fn played_artists_are_newest_first_and_once() {
        use crate::model::{Link, Track};
        let row = |artists: Vec<(&str, Option<&str>)>| Track {
            id: String::new(),
            title: String::new(),
            artists: artists.into_iter().map(|(n, id)| Link { name: n.into(), id: id.map(str::to_owned) }).collect(),
            album: None,
            duration: None,
            thumbs: vec![],
            track_nr: None,
            plays: None,
            music_video: false,
            explicit: false,
        };
        let history = vec![row(vec![("A", Some("id-a")), ("Unlinked", None)]), row(vec![("B", Some("id-b")), ("A", Some("id-a"))])];
        let names: Vec<_> = super::played_artists(&history).into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["A", "B"]);
    }

    #[test]
    fn home_subtitles_lose_their_counts() {
        assert_eq!(super::without_counts("Song • The Weeknd • 1.2B plays"), "Song • The Weeknd");
        assert_eq!(super::without_counts("Playlist • YouTube Music • 30K views"), "Playlist • YouTube Music");
        assert_eq!(super::without_counts("Album • 2Pac"), "Album • 2Pac");
    }

    #[test]
    fn test_collection_genre_detection() {
        let make_coll = |title: &str, subtitle: &str, desc: Option<&str>, is_album: bool| Collection {
            id: "test_coll".into(),
            is_album,
            save_id: None,
            title: title.into(),
            subtitle: subtitle.into(),
            artists: Vec::new(),
            description: desc.map(str::to_owned),
            thumbs: Vec::new(),
            tracks: Vec::new(),
            set_ids: Vec::new(),
            count: None,
            owned: false,
        };

        // Chromeo Adult Contemporary -> ELECTRONIC
        let c1 = make_coll("Adult Contemporary", "Chromeo • 2024", Some("Electro-funk duo Chromeo"), true);
        assert_eq!(super::collection_genre(&c1), "ELECTRONIC");

        // Hip-hop
        let c2 = make_coll("Good Kid, M.A.A.D City", "Kendrick Lamar • 2012", Some("West coast rap and hip hop"), true);
        assert_eq!(super::collection_genre(&c2), "HIP-HOP");

        // Rock
        let c3 = make_coll("Nevermind", "Nirvana • 1991", Some("Grunge rock album"), true);
        assert_eq!(super::collection_genre(&c3), "ROCK");

        // R&B
        let c4 = make_coll("Blonde", "Frank Ocean • 2016", Some("Contemporary R&B and soul"), true);
        assert_eq!(super::collection_genre(&c4), "R&B");

        // Pop
        let c5 = make_coll("1989", "Taylor Swift • 2014", Some("Synth pop album"), true);
        assert_eq!(super::collection_genre(&c5), "ELECTRONIC"); // "synth" keyword -> ELECTRONIC

        let c5b = make_coll("Teenage Dream", "Katy Perry • 2010", Some("Mainstream pop record"), true);
        assert_eq!(super::collection_genre(&c5b), "POP");

        // Default album
        let c6 = make_coll("Unknown Album", "2024", None, true);
        assert_eq!(super::collection_genre(&c6), "ELECTRONIC");

        // Playlist
        let c7 = make_coll("Chill Mix", "Various Artists", None, false);
        assert_eq!(super::collection_genre(&c7), "PLAYLIST");
    }

    #[test]
    fn test_peach_color_constants() {
        use egui::Color32;
        assert_eq!(super::PEACH, Color32::from_rgb(0xE0, 0x7A, 0x5F));
        assert_eq!(super::PEACH_TEXT, Color32::from_rgb(0x1a, 0x14, 0x12));
    }
}
