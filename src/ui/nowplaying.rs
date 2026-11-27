//! Now-playing overlay: big art + synced lyrics / up next, over the full-strength
//! animated background. The lyrics themselves are drawn by `karaoke` (a port of the
//! better-lyrics renderer); this adds their toolbar and the plain-text fallback.

use egui::{Color32, Rect, Sense, Ui, Vec2, pos2, vec2};

use super::widgets::{self, icon_at, named, text_at};
use super::{Action, App, NpTab, Repeat, karaoke};
use crate::i18n::t;
use crate::lyrics::{Lyrics, Sync};
use crate::shader::Params;
use crate::theme::{self, icon};

pub fn overlay(app: &mut App, ctx: &egui::Context, now: f64, screen: Rect) {
    use crate::preset::BgMode;
    let t = theme::anim_bool(ctx, "np-open", app.np_open && app.current().is_some(), 0.42);
    if t <= 0.001 {
        return;
    }
    // The player brings its own transport, so it takes the whole window: the capsule
    // under it belongs to the pages it covers.
    let full = if ctx.viewport_rect().is_positive() { ctx.viewport_rect() } else { screen };
    let rect = full.translate(vec2(0.0, full.height() * (1.0 - t)));
    let look = app.look();
    let b = &look.background;
    egui::Area::new(egui::Id::unique("now-playing"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            // Swallow input so the page underneath doesn't react.
            ui.allocate_rect(rect, Sense::click_and_drag());
            // It covers the window's top too: that still moves the window.
            if !app.fullscreen {
                super::titlebar::drag_area(ui, Rect::from_min_size(rect.min, vec2(rect.width(), super::titlebar::HEIGHT)), "now-playing");
            }
            let painter = ui.painter_at(rect);
            let art = matches!(b.mode, BgMode::Kawarp | BgMode::CoverStatic | BgMode::CoverFullbleed);
            if let Some(bg) = app.bg.as_ref().filter(|_| art) {
                let blend = theme::ease_in_out(((now - app.art_changed) / 1.4) as f32);
                if blend < 1.0 || (b.mode != BgMode::CoverStatic && app.playing()) {
                    super::animate(ctx);
                }
                let level = if app.reactive_bg && app.playing() { app.audio.status.level() * b.reactive } else { 0.0 };
                bg.paint(
                    &painter,
                    rect,
                    Params {
                        time: app.bg_time as f32,
                        blend,
                        intensity: if b.mode == BgMode::CoverStatic { 0.0 } else { b.intensity },
                        saturation: b.saturation,
                        dim: b.np_dim,
                        scale: b.scale + level.min(0.5) * 0.12,
                    },
                );
            } else if b.mode == BgMode::Solid {
                painter.rect_filled(rect, 0.0, look.solid());
            } else {
                let dom = app.dominant_color().to_array();
                let ch = |i: usize| theme::anim(ctx, ("np-dom-shade", i), f32::from(dom[i]), 0.9).round() as u8;
                let solid = Color32::from_rgb(ch(0), ch(1), ch(2));
                painter.rect_filled(rect, 0.0, solid);
            }
            let k = b.scrim / 0.3;
            if app.bg_is_light() {
                // Gentle scrim on light backgrounds so text stays readable and gradients are soft
                super::shell::vgradient(ui.painter(), rect, Color32::from_black_alpha(10), Color32::from_black_alpha(30));
            } else {
                super::shell::vgradient(ui.painter(), rect, theme::shade((0.06 * k).min(0.5)), theme::shade((0.16 * k).min(0.5)));
            }
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            body(app, &mut child, now);
        });
}

fn paint_np_capsule(ui: &Ui, id_salt: &str, rect: Rect, bg_light: bool) {
    let tint = if bg_light {
        Color32::from_white_alpha(48)
    } else {
        Color32::from_rgba_unmultiplied(24, 24, 28, 42)
    };
    let style = crate::glass::Style::clear(crate::glass::Corner::Capsule, Some(tint), if bg_light { 0.10 } else { 0.88 }, 1.0);
    crate::glass::paint(ui, ui.scope_id().with(id_salt), rect, style);
}

fn body(app: &mut App, ui: &mut Ui, now: f64) {
    use crate::preset::{Layout, Side};
    // An Automix handoff keeps the outgoing song here until the next one is the louder
    // of the two (see `MixFrom`): its track, seek line and lyrics lag the queue's switch.
    let hold = app.mix_hold().map(|h| (h.total, h.mix, app.mix_hold_pos(h)));
    let Some(track) = app.current().cloned() else { return };
    let track = match &hold {
        Some(_) => app.shown().cloned().unwrap_or(track),
        None => track,
    };
    let heard = if hold.is_some() { track.clone() } else { app.playing_track().cloned().unwrap_or_else(|| track.clone()) };
    let look = app.look();
    let cs = &look.cover;
    let area = ui.max_rect();
    let no_lyrics = app.no_lyrics();
    // The content is this area less its margins: a cover column beside the panel needs
    // 860 points, and below that the cover gives way to the panel.
    let narrow = area.width() - 64.0 < 860.0;
    // Header: on the window buttons' line, when the app draws them.
    let is_fullscreen = app.fullscreen || ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
    let header_top = area.top() + if is_fullscreen { 16.0 } else { 6.0 };
    let header = Rect::from_min_size(pos2(area.left() + 20.0, header_top), vec2(area.width() - 40.0, 42.0));

    // Fullscreen auto-hide after 5.0s of mouse inactivity: top pills, bottom-right pill, and controls disappear together
    let controls_rect: Rect = ui.ctx().data(|d| d.get_temp(egui::Id::unique("np-controls-rect"))).unwrap_or(Rect::NOTHING);
    let tl_rect: Rect = ui.ctx().data(|d| d.get_temp(egui::Id::unique("np-top-left-rect"))).unwrap_or(Rect::NOTHING);
    let tr_rect: Rect = ui.ctx().data(|d| d.get_temp(egui::Id::unique("np-top-right-rect"))).unwrap_or(Rect::NOTHING);
    let br_rect: Rect = ui.ctx().data(|d| d.get_temp(egui::Id::unique("np-bottom-right-rect"))).unwrap_or(Rect::NOTHING);
    let controls_hovered = ui.rect_contains_pointer(controls_rect)
        || ui.rect_contains_pointer(tl_rect)
        || ui.rect_contains_pointer(tr_rect)
        || ui.rect_contains_pointer(br_rect);
    let busy = app.seek_drag.is_some() || egui::Popup::is_any_open(ui.ctx());
    let idle = now - app.mouse_last_moved >= 5.0;
    let narrow_panel = narrow && (app.np_tab == NpTab::UpNext || (app.np_tab == NpTab::Lyrics && !no_lyrics));
    let auto_hide = is_fullscreen || narrow_panel;
    let active = !auto_hide || !idle || busy || controls_hovered;
    let controls_alpha = if auto_hide {
        if active && !busy && !controls_hovered && !idle {
            let remaining = (5.0 - (now - app.mouse_last_moved)).max(0.05);
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(remaining));
        }
        theme::anim_bool(ui.ctx(), "np-controls-visible", active, 0.25)
    } else {
        1.0
    };
    if controls_alpha > 0.0 && controls_alpha < 1.0 {
        super::animate(ui.ctx());
    }

    // The side panel (lyrics / up next) slides in and out when its chip is toggled.
    let last_id = egui::Id::unique("np-last-panel");
    let panel_tab = if app.np_tab == NpTab::Art {
        ui.ctx().data(|d| d.get_temp(last_id)).unwrap_or(if no_lyrics { NpTab::UpNext } else { NpTab::Lyrics })
    } else {
        ui.ctx().data_mut(|d| d.insert_temp(last_id, app.np_tab));
        app.np_tab
    };
    let fullbleed_lyrics_view = !no_lyrics && cs.layout == Layout::LyricsOnly && look.background.mode == crate::preset::BgMode::CoverFullbleed;
    let split_queue = !narrow && !fullbleed_lyrics_view && (app.np_tab == NpTab::UpNext || panel_tab == NpTab::UpNext);
    let beside = !narrow && cs.layout == Layout::LyricsOnly && !split_queue && look.background.mode == crate::preset::BgMode::CoverFullbleed;
    let cover_hidden = (narrow || (cs.layout == Layout::LyricsOnly && !split_queue)) && !beside && app.np_tab != NpTab::Art && !no_lyrics;

    let mut close = false;

    let content = Rect::from_min_max(pos2(area.left() + 32.0, header.bottom() + 12.0), pos2(area.right() - 32.0, area.bottom() - 24.0));
    // Linear 0..1 over the transition; each part eases its own slice of it.
    let shown = ui.ctx().animate_bool_with_time(egui::Id::unique("np-panel"), app.np_tab != NpTab::Art, 0.6 * theme::SPEED);
    if shown > 0.0 && shown < 1.0 {
        // Every frame while sliding, not the now-playing view's usual 30 fps.
        super::animate(ui.ctx());
    }
    let ppp = ui.ctx().pixels_per_point();
    let p = if panel_tab == NpTab::Lyrics && no_lyrics { 0.0 } else { shown };
    // Whichever is leaving slides and fades out over the first 45%, then the other
    // comes in over the last 70%, both easing out, in either direction.
    let (panel_vis, art_vis) = if app.np_tab != NpTab::Art {
        (theme::ease_out((p - 0.3) / 0.7), 1.0 - theme::ease_out(p / 0.45))
    } else {
        let q = 1.0 - p;
        (1.0 - theme::ease_out(q / 0.45), theme::ease_out((q - 0.3) / 0.7))
    };
    // The cover gliding between its two places (wide layouts): a gentle S-curve.
    let glide = p * p * (3.0 - 2.0 * p);
    let lyrics_only = narrow || (cs.layout == Layout::LyricsOnly && !split_queue);
    // reCover: the backdrop cover sits on the left, so the lyrics take the right side.
    let beside_cover = !narrow && cs.layout == Layout::LyricsOnly && !split_queue && look.background.mode == crate::preset::BgMode::CoverFullbleed;
    let centered = lyrics_only && !beside_cover;
    // Artwork alone: centered.
    let col_art = Rect::from_center_size(content.center(), vec2(content.width().min(content.height() * 0.9), content.height()));
    let slide = 72.0;
    let right = if split_queue {
        // When showing queue: keep album artwork and controllers at the exact same size,
        // sliding smoothly to the left, and give the queue manager a compact, elegant width on the right.
        let side = art_side(cs, col_art);
        let max_queue_w = (content.width() - side - 48.0).max(280.0);
        let queue_w = ((content.width() * 0.36).clamp(320.0, 460.0)).min(max_queue_w);
        let right_rect = Rect::from_min_size(
            pos2(content.right() - queue_w, content.top()),
            vec2(queue_w, content.height()),
        );
        let avail_left = (right_rect.left() - 28.0 - content.left()).max(side);
        let left_center_x = content.left() + avail_left / 2.0;
        let slide_dist = (content.center().x - left_center_x).max(0.0);
        let col = col_art.translate(vec2(-slide_dist * glide, 0.0));
        let px = (art_side(cs, col_art) * ppp) as u32;
        cover_column(app, ui, col, &track, no_lyrics, now, px, controls_alpha);
        right_rect
    } else if lyrics_only {
        // No cover column next to the panel: the artwork fades in from the left as the panel leaves.
        let px = (art_side(cs, col_art) * ppp) as u32;
        if art_vis > 0.001 {
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(content));
            cui.multiply_opacity(art_vis);
            cover_column(app, &mut cui, col_art.translate(vec2(-slide * (1.0 - art_vis), 0.0)), &track, no_lyrics, now, px, controls_alpha);
        } else {
            // Hidden behind the lyrics: have it loaded for when they're turned off.
            widgets::preload_cover(ui.ctx(), &track.thumbs, px);
        }
        if beside_cover && art_vis < 0.999 {
            // The sharp cover itself, filling the window's height from its top left and
            // melting into the blurred backdrop before the lyrics (reCover, and the app's
            // own look).
            let side = area.height();
            let art = Rect::from_min_size(area.left_top(), Vec2::splat(side));
            let lyrics_left = content.left() + content.width() * 0.46;
            let fade_to = lyrics_left.min(art.right());
            let fade_from = (fade_to - art.left()).max(0.0) / side * 0.72;
            let tint = Color32::from_gray((look.background.np_dim.clamp(0.0, 1.0) * 255.0) as u8);
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(art));
            cui.multiply_opacity(1.0 - art_vis);
            widgets::paint_cover_fade(&cui, egui::Id::unique("np-art-fade"), art, &track.thumbs, (side * ppp) as u32, fade_from, tint);
            // A soft shade under the sleeve, so the controls stay legible on any cover:
            // dark scrim on dark/mid covers behind white text; soft light scrim on light covers behind black text.
            let bg_light_cover = app.bg_is_light();
            let scrim = Rect::from_min_max(pos2(art.left(), area.bottom() - 280.0), pos2((lyrics_left + 24.0).min(art.right()), area.bottom()));
            let mut shade = egui::Mesh::default();
            let white = egui::epaint::WHITE_UV;
            let scrim_col = if bg_light_cover { Color32::from_white_alpha(150) } else { Color32::from_black_alpha(130) };
            shade.vertices.push(egui::epaint::Vertex { pos: scrim.left_bottom(), uv: white, color: scrim_col });
            shade.vertices.push(egui::epaint::Vertex { pos: scrim.right_bottom(), uv: white, color: Color32::TRANSPARENT });
            shade.vertices.push(egui::epaint::Vertex { pos: scrim.left_top(), uv: white, color: Color32::TRANSPARENT });
            shade.vertices.push(egui::epaint::Vertex { pos: scrim.right_top(), uv: white, color: Color32::TRANSPARENT });
            shade.indices.extend_from_slice(&[0, 1, 2, 2, 1, 3]);
            cui.painter().add(egui::Shape::mesh(shade));

            // Minimal controls floating directly over the background without any glass card
            let w = (content.width() * 0.38).clamp(300.0, 440.0);
            let cw = w + 40.0;
            let left = (content.left() + 8.0).min(lyrics_left - 12.0 - cw).max(content.left());
            let h = 196.0;
            let card = Rect::from_min_size(pos2(left, content.bottom() - h), vec2(cw, h));
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(card));
            cui.multiply_opacity(1.0 - art_vis);
            sleeve(app, &mut cui, card.left() + 20.0, w, card.top() + 16.0, &track, no_lyrics, now, controls_alpha, false);
        }
        if beside_cover { Rect::from_min_max(pos2(content.left() + content.width() * 0.46, content.top()), content.max) } else { content }
    } else {
        // Cover column, on the preset's side; it glides to the center as the panel leaves.
        let frac = if cs.layout == Layout::CoverHero { 0.5 } else { 0.42 };
        let col_w = content.width() * frac;
        let col_panel = if cs.side == Side::Right {
            Rect::from_min_size(pos2(content.right() - col_w, content.top()), vec2(col_w, content.height()))
        } else {
            Rect::from_min_size(content.min, vec2(col_w, content.height()))
        };
        let col = Rect::from_min_max(col_art.min.lerp(col_panel.min, glide), col_art.max.lerp(col_panel.max, glide));
        // One image size for the whole glide, the bigger of its two ends.
        let px = (art_side(cs, col_art).max(art_side(cs, col_panel)) * ppp) as u32;
        cover_column(app, ui, col, &track, no_lyrics, now, px, controls_alpha);
        if cs.side == Side::Right {
            Rect::from_min_max(content.min, pos2(col_panel.left() - 28.0, content.bottom()))
        } else {
            Rect::from_min_max(pos2(col_panel.right() + 28.0, content.top()), content.max)
        }
    };
    if panel_vis > 0.001 {
        let toward = if !lyrics_only && cs.side == Side::Right { -slide } else { slide };
        let rect = right.translate(vec2(toward * (1.0 - panel_vis), 0.0));
        // Switching between Lyrics and Up next: the old one fades out over the first 55%,
        // the new one fades in and rises into place from a third of the way.
        for (tab, key) in [(NpTab::Lyrics, "np-tab-lyrics"), (NpTab::UpNext, "np-tab-upnext")] {
            let w = ui.ctx().animate_bool_with_time(egui::Id::unique(key), panel_tab == tab, 0.45 * theme::SPEED);
            if w > 0.0 && w < 1.0 {
                super::animate(ui.ctx());
            }
            let vis = if panel_tab == tab {
                theme::ease_out((w - 0.35) / 0.65)
            } else {
                let x = ((w - 0.45) / 0.55).clamp(0.0, 1.0);
                x * x * (3.0 - 2.0 * x)
            };
            if vis <= 0.001 || (tab == NpTab::Lyrics && no_lyrics) {
                continue;
            }
            let rise = if panel_tab == tab { 18.0 * (1.0 - vis) } else { 0.0 };
            let r = rect.translate(vec2(0.0, rise));
            let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(r));
            rui.set_clip_rect(content.intersect(ui.clip_rect()));
            rui.multiply_opacity(panel_vis * vis);
            match tab {
                NpTab::Lyrics => lyrics_panel(app, &mut rui, r, now, centered, app.bg_is_light()),
                _ => super::shell::queue_list(app, &mut rui, "np"),
            }
        }
    }

    // Narrow / iPad-style panel mode: overlay player controls at the bottom over lyrics/queue
    // only while the pointer is active (fades out after inactivity).
    if narrow && !beside_cover && panel_vis > 0.001 && controls_alpha > 0.001 {
        let overlay_alpha = panel_vis * controls_alpha;
        let bg_light = app.bg_is_light();
        let scrim_top = (area.bottom() - 280.0).max(area.top());
        let scrim = Rect::from_min_max(pos2(area.left(), scrim_top), area.max);
        let mut shade = egui::Mesh::default();
        let white = egui::epaint::WHITE_UV;
        let bot_col = if bg_light {
            let bg = app.page_color(ui.ctx());
            Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), (215.0 * overlay_alpha) as u8)
        } else {
            Color32::from_black_alpha((175.0 * overlay_alpha) as u8)
        };
        shade.vertices.push(egui::epaint::Vertex { pos: scrim.left_bottom(), uv: white, color: bot_col });
        shade.vertices.push(egui::epaint::Vertex { pos: scrim.right_bottom(), uv: white, color: bot_col });
        shade.vertices.push(egui::epaint::Vertex { pos: scrim.left_top(), uv: white, color: Color32::TRANSPARENT });
        shade.vertices.push(egui::epaint::Vertex { pos: scrim.right_top(), uv: white, color: Color32::TRANSPARENT });
        shade.indices.extend_from_slice(&[0, 1, 2, 2, 1, 3]);
        ui.painter().add(egui::Shape::mesh(shade));

        let ov_w = (content.width() - 16.0).clamp(240.0, 400.0);
        let ov_x = (area.center().x - ov_w * 0.5).max(area.left() + 20.0);
        let clears_br = ov_x + ov_w <= area.right() - 108.0;
        let ov_y = if clears_br { area.bottom() - 190.0 } else { area.bottom() - 220.0 };
        let card = Rect::from_min_size(pos2(ov_x - 12.0, ov_y - 12.0), vec2(ov_w + 24.0, 196.0));
        let mut ov_ui = ui.new_child(egui::UiBuilder::new().id_salt("np-narrow-overlay").max_rect(card));
        sleeve(app, &mut ov_ui, ov_x, ov_w, ov_y, &track, no_lyrics, now, overlay_alpha, true);
    }

    // ------------------------------------------------------------------ Apple Music Floating Corner Pills
    if controls_alpha > 0.001 {
        let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(area));
        cui.multiply_opacity(controls_alpha);
        let ui = &mut cui;
        let interactive = controls_alpha > 0.05;
        let bg_light = app.bg_is_light();
        let accent = app.accent_color();
        let fg_base = if bg_light { Color32::from_rgb(0x18, 0x14, 0x1c) } else { Color32::WHITE };

        // 1. Top-left pill beside macOS traffic lights: [ Close (X)  Fullscreen ] (single undivided 72×32 capsule)
        let tl_x = area.left() + if is_fullscreen || cfg!(windows) { 20.0 } else { 86.0 };
        let tl_w = if cfg!(windows) { 32.0 } else { 72.0 };
        let tl_rect = Rect::from_min_size(pos2(tl_x, header_top), vec2(tl_w, 32.0));
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("np-top-left-rect"), tl_rect));
        let tl_slot = ui.painter().add(egui::Shape::Noop);

        let close_rect = if cfg!(windows) {
            tl_rect
        } else {
            Rect::from_min_max(tl_rect.min, pos2(tl_rect.center().x, tl_rect.max.y))
        };
        let close_sense = if interactive { Sense::click() } else { Sense::hover() };
        let close_resp = named(ui.interact(close_rect, ui.scope_id().with("np-close"), close_sense), &t!("minimise_now_playing"))
            .on_hover_text(&t!("minimise_now_playing"));
        let close_resp = if interactive { close_resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { close_resp };

        let fs_opt = if !cfg!(windows) {
            let fs_rect = Rect::from_min_max(pos2(tl_rect.center().x, tl_rect.min.y), tl_rect.max);
            let fs_label = if is_fullscreen { t!("exit_full_screen") } else { t!("full_screen") };
            let fs_sense = if interactive { Sense::click() } else { Sense::hover() };
            let fs_resp = named(ui.interact(fs_rect, ui.scope_id().with("np-fs"), fs_sense), &fs_label).on_hover_text(&fs_label);
            let fs_resp = if interactive { fs_resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { fs_resp };
            Some((fs_rect, fs_resp))
        } else {
            None
        };

        let p_close = crate::glass::press(ui, &close_resp, tl_rect, if interactive { 0.75 } else { 0.0 });
        let p_fs = fs_opt.as_ref().map(|(_, r)| crate::glass::press(ui, r, tl_rect, if interactive { 0.75 } else { 0.0 }));
        let (tl_draw, tl_press) = match p_fs {
            Some(pf) if pf.amount.abs() > p_close.amount.abs() => (pf.rect, pf),
            _ => (p_close.rect, p_close),
        };
        let tl_tint = if bg_light { Color32::from_white_alpha(48) } else { Color32::from_rgba_unmultiplied(24, 24, 28, 42) };
        let mut tl_style = crate::glass::Style::clear(crate::glass::Corner::Capsule, Some(tl_tint), if bg_light { 0.10 } else { 0.88 }, 1.0);
        tl_press.apply(&mut tl_style);
        crate::glass::paint_at(ui, tl_slot, ui.scope_id().with("np-tl-glass"), tl_draw, tl_style);

        let close_active = interactive && (close_resp.hovered() || close_resp.is_pointer_button_down_on());
        let fs_active = interactive && fs_opt.as_ref().is_some_and(|(_, r)| r.hovered() || r.is_pointer_button_down_on());
        let tl_target_cx = if fs_active { fs_opt.as_ref().unwrap().0.center().x } else { close_rect.center().x };
        let tl_held = close_resp.is_pointer_button_down_on() || fs_opt.as_ref().is_some_and(|(_, r)| r.is_pointer_button_down_on());
        widgets::capsule_indicator(
            ui,
            ui.scope_id().with("np-tl-hover"),
            tl_rect,
            tl_draw,
            tl_target_cx,
            close_active || fs_active,
            tl_held,
            25.0,
            27.0,
            |vis| if bg_light { Color32::from_black_alpha((24.0 * vis) as u8) } else { Color32::from_white_alpha((36.0 * vis) as u8) },
            Some(|vis: f32| if bg_light { Color32::from_white_alpha((80.0 * vis) as u8) } else { Color32::from_white_alpha((28.0 * vis) as u8) }),
        );

        let tl_shift = tl_draw.center() - tl_rect.center();
        icon_at(ui, close_rect.center() + tl_shift, icon::X, 14.5 * p_close.scale, theme::with_alpha(fg_base, 0.88 + 0.12 * p_close.hover));
        if interactive && close_resp.clicked() {
            close = true;
        }
        if let Some((fs_rect, fs_resp)) = fs_opt {
            let pf = p_fs.unwrap();
            let fs_col = if is_fullscreen { accent } else { theme::with_alpha(fg_base, 0.88 + 0.12 * pf.hover) };
            icon_at(ui, fs_rect.center() + tl_shift, icon::MAXIMIZE_2, 14.0 * pf.scale, fs_col);
            if interactive && fs_resp.clicked() {
                app.set_fullscreen(ui.ctx(), !is_fullscreen);
            }
        }

        // 2. Top-right Sound Level / Volume pill: [ ───●  🔊 ]
        let r_gap = super::titlebar::right_gap(ui.ctx(), header, 0.0);
        let tr_right = area.right() - 20.0 - r_gap;
        let tr_rect = Rect::from_min_size(pos2(tr_right - 144.0, header_top), vec2(144.0, 32.0));
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("np-top-right-rect"), tr_rect));
        paint_np_capsule(ui, "np-tr-glass", tr_rect, bg_light);

        let slider_rect = Rect::from_min_max(pos2(tr_rect.left() + 14.0, tr_rect.top()), pos2(tr_rect.right() - 34.0, tr_rect.bottom()));
        let vol = app.audio.volume();
        let vsense = if interactive { Sense::click_and_drag() } else { Sense::hover() };
        let vresp = named(ui.interact(slider_rect, ui.scope_id().with("np-vol-slider"), vsense), "Volume");
        let vresp = if interactive { vresp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { vresp };
        vresp.widget_info(|| egui::WidgetInfo::slider(interactive, vol as f64, "Volume"));
        let vh = theme::anim_bool(ui.ctx(), ui.scope_id().with("np-vol-h"), interactive && (vresp.hovered() || vresp.dragged()), 0.15);
        let track_r = Rect::from_center_size(slider_rect.center(), vec2(slider_rect.width(), 4.0));
        ui.painter().rect_filled(track_r, 2.0, theme::glass(0.22));
        let fill_r = Rect::from_min_size(track_r.min, vec2(track_r.width() * vol, 4.0));
        ui.painter().rect_filled(fill_r, 2.0, fg_base);
        ui.painter().circle_filled(pos2(fill_r.right(), track_r.center().y), 5.0 + 1.5 * vh, fg_base);
        if interactive
            && let Some(p) = vresp.interact_pointer_pos()
            && (vresp.dragged() || vresp.clicked())
        {
            app.audio.set_volume(((p.x - track_r.left()) / track_r.width()).clamp(0.0, 1.0));
        }

        let icon_rect = Rect::from_min_max(pos2(tr_rect.right() - 32.0, tr_rect.top()), pos2(tr_rect.right() - 4.0, tr_rect.bottom()));
        let vol_btn = named(ui.interact(icon_rect, ui.scope_id().with("np-vol-mute"), if interactive { Sense::click() } else { Sense::hover() }), "Volume");
        let vol_btn = if interactive { vol_btn.on_hover_cursor(egui::CursorIcon::PointingHand) } else { vol_btn };
        let step = super::glyphs::volume_step(vol);
        super::glyphs::draw(ui, icon_rect.center(), 15.0, super::glyphs::VOLUME[step], theme::with_alpha(fg_base, if vol_btn.hovered() { 1.0 } else { 0.88 }));
        if interactive && vol_btn.clicked() {
            let stash = super::shell::unmute_id();
            if vol > 0.0 {
                ui.data_mut(|d| d.insert_temp(stash, vol));
                app.audio.set_volume(0.0);
            } else {
                app.audio.set_volume(ui.data(|d| d.get_temp::<f32>(stash)).unwrap_or(0.6));
            }
        }
        if interactive && ui.rect_contains_pointer(tr_rect) {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                app.audio.set_volume((vol + scroll / 600.0).clamp(0.0, 1.0));
            }
        }

        if cover_hidden {
            let opt_r = Rect::from_center_size(pos2(tr_rect.left() - 24.0, tr_rect.center().y), Vec2::splat(32.0));
            if interactive {
                options_menu(app, ui, opt_r, &heard, app.np_tab == NpTab::Lyrics && !no_lyrics, None);
            }
        }

        // 3. Bottom-right capsule: [ Lyrics  Queue ] with fluid sliding active circle & fluid hover circle
        let br_rect = Rect::from_min_size(pos2(area.right() - 24.0 - 72.0, area.bottom() - 18.0 - 32.0), vec2(72.0, 32.0));
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("np-bottom-right-rect"), br_rect));
        let br_slot = ui.painter().add(egui::Shape::Noop);

        let lyr_rect = Rect::from_min_max(br_rect.min, pos2(br_rect.center().x, br_rect.max.y));
        let q_rect = Rect::from_min_max(pos2(br_rect.center().x, br_rect.min.y), br_rect.max);

        let lyr_active = app.np_tab == NpTab::Lyrics && !no_lyrics;
        let q_active = app.np_tab == NpTab::UpNext;
        let inv_fg = if bg_light { Color32::WHITE } else { Color32::from_rgb(0x18, 0x14, 0x1c) };

        let lyr_sense = if interactive && !no_lyrics { Sense::click() } else { Sense::hover() };
        let lyr_resp = named(ui.interact(lyr_rect, ui.scope_id().with("np-br-lyrics"), lyr_sense), &t!("lyrics"))
            .on_hover_text(&t!("lyrics"));
        let lyr_resp = if interactive && !no_lyrics { lyr_resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { lyr_resp };

        let q_sense = if interactive { Sense::click() } else { Sense::hover() };
        let q_resp = named(ui.interact(q_rect, ui.scope_id().with("np-br-queue"), q_sense), &t!("view_queue"))
            .on_hover_text(&t!("view_queue"));
        let q_resp = if interactive { q_resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { q_resp };

        let p_lyr = crate::glass::press(ui, &lyr_resp, br_rect, if interactive && !no_lyrics { 0.75 } else { 0.0 });
        let p_q = crate::glass::press(ui, &q_resp, br_rect, if interactive { 0.75 } else { 0.0 });
        let (br_draw, br_press) = if p_q.amount.abs() > p_lyr.amount.abs() { (p_q.rect, p_q) } else { (p_lyr.rect, p_lyr) };
        let br_tint = if bg_light { Color32::from_white_alpha(48) } else { Color32::from_rgba_unmultiplied(24, 24, 28, 42) };
        let mut br_style = crate::glass::Style::clear(crate::glass::Corner::Capsule, Some(br_tint), if bg_light { 0.10 } else { 0.88 }, 1.0);
        br_press.apply(&mut br_style);
        crate::glass::paint_at(ui, br_slot, ui.scope_id().with("np-br-glass"), br_draw, br_style);

        let br_held = lyr_resp.is_pointer_button_down_on() || q_resp.is_pointer_button_down_on();
        let active_cx = if q_active { q_rect.center().x } else { lyr_rect.center().x };
        widgets::capsule_indicator(
            ui,
            ui.scope_id().with("np-br-active"),
            br_rect,
            br_draw,
            active_cx,
            lyr_active || q_active,
            br_held,
            25.0,
            27.0,
            |vis| theme::with_alpha(fg_base, vis),
            None::<fn(f32) -> Color32>,
        );

        // Fluid hover circle for whichever side is hovered while inactive
        let lyr_hover = interactive && !no_lyrics && !lyr_active && lyr_resp.hovered();
        let q_hover = interactive && !q_active && q_resp.hovered();
        let hover_cx = if q_hover { q_rect.center().x } else { lyr_rect.center().x };
        widgets::capsule_indicator(
            ui,
            ui.scope_id().with("np-br-hover"),
            br_rect,
            br_draw,
            hover_cx,
            lyr_hover || q_hover,
            false,
            25.0,
            27.0,
            |vis| if bg_light { Color32::from_black_alpha((24.0 * vis) as u8) } else { Color32::from_white_alpha((34.0 * vis) as u8) },
            None::<fn(f32) -> Color32>,
        );

        let br_shift = br_draw.center() - br_rect.center();
        let lyr_k = crate::glass::spring_bool(ui.ctx(), ui.scope_id().with("np-br-lyr-k"), lyr_active, crate::glass::Tuning::SNAP);
        let lyr_col = if no_lyrics {
            theme::with_alpha(fg_base, 0.35)
        } else {
            theme::lerp_color(theme::with_alpha(fg_base, 0.85 + 0.15 * p_lyr.hover), inv_fg, lyr_k)
        };
        super::glyphs::draw(ui, lyr_rect.center() + br_shift, 15.0 * p_lyr.scale, super::glyphs::LYRICS, lyr_col);
        if interactive && !no_lyrics && lyr_resp.clicked() {
            app.np_tab = if lyr_active { NpTab::Art } else { NpTab::Lyrics };
        }

        let q_k = crate::glass::spring_bool(ui.ctx(), ui.scope_id().with("np-br-q-k"), q_active, crate::glass::Tuning::SNAP);
        let q_col = theme::lerp_color(theme::with_alpha(fg_base, 0.85 + 0.15 * p_q.hover), inv_fg, q_k);
        super::glyphs::draw(ui, q_rect.center() + br_shift, 15.0 * p_q.scale, super::glyphs::QUEUE, q_col);
        if interactive && q_resp.clicked() {
            app.np_tab = if q_active {
                let prev = ui.ctx().data(|d| d.get_temp(egui::Id::unique("np-last-tab-prior"))).unwrap_or(if no_lyrics { NpTab::Art } else { NpTab::Lyrics });
                if prev == NpTab::UpNext {
                    if no_lyrics { NpTab::Art } else { NpTab::Lyrics }
                } else {
                    prev
                }
            } else {
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("np-last-tab-prior"), app.np_tab));
                NpTab::UpNext
            };
        }
    }
    if close {
        app.np_open = false;
        if app.fullscreen || ui.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
            app.set_fullscreen(ui.ctx(), false);
        }
    }
}

/// Size of the artwork laid out in `col`: what is left once the sleeve's information,
/// its bar and the transport underneath it have taken their room.
fn art_side(cs: &crate::preset::CoverStyle, col: Rect) -> f32 {
    (col.width() * 0.82 * cs.size.clamp(0.5, 1.35)).min(col.height() - 190.0).max(160.0)
}

/// The sleeve: art, title and artists, the seek line with its two times, then the
/// transport — the order the reference keeps them in. `px`: the size to fetch the image
/// at (fixed while the column glides).
#[allow(clippy::too_many_arguments)]
fn cover_column(app: &mut App, ui: &mut Ui, col: Rect, track: &crate::model::Track, no_lyrics: bool, now: f64, px: u32, controls_alpha: f32) {
    use crate::preset::Shape;
    let look = app.look();
    let cs = &look.cover;
    let side = art_side(cs, col);
    let art = Rect::from_center_size(pos2(col.center().x, col.top() + side / 2.0 + (col.height() - side - 190.0).max(0.0) * 0.42), Vec2::splat(side));
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("np-art-rect"), art));
    let round = cs.shape != Shape::Rounded;
    let radius = if round { side / 2.0 } else { cs.radius.clamp(0.0, side / 2.0) };
    let cr = egui::CornerRadius::same(radius.min(255.0) as u8);
    if cs.glow > 0.0 {
        let s = egui::epaint::Shadow { offset: [0, 16], blur: 90, spread: 6, color: theme::with_alpha(app.accent_color(), cs.glow.min(1.0) * 0.7) };
        ui.painter().add(s.as_shape(art, cr));
    }
    if cs.shadow > 0.0 {
        let shadow_val = cs.shadow.clamp(0.0, 1.0);
        let bg_light = app.bg_is_light();
        let (blur, alpha) = if bg_light {
            (26.0, (shadow_val * 0.18).clamp(0.05, 0.14))
        } else {
            (40.0, (shadow_val * 0.36).clamp(0.10, 0.25))
        };
        widgets::soft_shadow(ui, art, radius.min(255.0), blur, alpha);
    }
    let tint = Color32::from_gray((cs.brightness.clamp(0.1, 1.0) * 255.0) as u8);
    if round {
        if cs.shape == Shape::Record && app.playing() {
            let dt = ui.input(|i| i.stable_dt).min(0.1) as f64;
            app.record_angle = (app.record_angle + dt * std::f64::consts::TAU / cs.spin_secs.max(4.0) as f64) % std::f64::consts::TAU;
        }
        let angle = if cs.shape == Shape::Record { app.record_angle as f32 } else { 0.0 };
        widgets::paint_disc(ui, egui::Id::unique("np-art"), art.center(), side / 2.0, &track.thumbs, angle, tint, px);
        if cs.shape == Shape::Record {
            // Vinyl: faint grooves and a spindle hole.
            let p = ui.painter();
            for i in 1..6 {
                p.circle_stroke(art.center(), side / 2.0 * (0.42 + i as f32 * 0.1), egui::Stroke::new(1.0, theme::shade(0.18)));
            }
            p.circle_filled(art.center(), side * 0.09, theme::shade(0.35));
            p.circle_filled(art.center(), side * 0.022, Color32::from_rgb(10, 10, 12));
        }
    } else {
        widgets::paint_cover_xfade_px(ui, egui::Id::unique("np-art"), art, &track.thumbs, radius, 1.0, px);
        // Apple's motion sleeve, over the still until its first frame arrives.
        if let Some(album) = track.album.as_ref().map(|a| a.name.clone()).filter(|a| !a.is_empty()) {
            let artist = track.artists.first().map(|a| a.name.clone()).unwrap_or_else(|| track.artist_line());
            app.apple_cover(&album, &artist, Some(&track.title));
            if let Some(s) = app.apple_sprite(ui.ctx(), &album, &artist, px) {
                widgets::paint_motion(ui, art, radius, &s);
            }
        }
        if cs.brightness < 0.99 {
            ui.painter().rect_filled(art, cr, theme::shade(1.0 - cs.brightness));
        }
    }
    sleeve(app, ui, art.left(), side, art.bottom() + 22.0, track, no_lyrics, now, controls_alpha, false);
}

/// The sleeve's information under the art (title, artists, star, ⋯), the seek line
/// with its times and quality mark, then the transport, in a column `w` wide from `x0`,
/// starting at `info_y`. About 175 points tall. Shared by the cover column and the
/// card over a full-bleed cover. `controls`: fullscreen opacity multiplier.
#[allow(clippy::too_many_arguments)]
fn sleeve(app: &mut App, ui: &mut Ui, x0: f32, w: f32, info_y: f32, track: &crate::model::Track, no_lyrics: bool, now: f64, controls: f32, force_expand: bool) {
    let hold = app.mix_hold().map(|h| (h.total, h.mix, app.mix_hold_pos(h)));
    let heard = if hold.is_some() { track.clone() } else { app.playing_track().cloned().unwrap_or_else(|| track.clone()) };

    // Record the controls bounding box for inactivity hover detection
    let controls_rect = Rect::from_min_size(pos2(x0, info_y), vec2(w, 196.0));
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("np-controls-rect"), controls_rect));

    // In the song-only player with no lyrics (when not auto-hiding), controls are kept permanently visible and interactive.
    let no_lyrics_view = !force_expand && (no_lyrics || app.np_tab != NpTab::Lyrics);
    let playing = app.playing() || app.buffering;
    let hovered = ui.rect_contains_pointer(controls_rect.expand(16.0));
    let busy = app.seek_drag.is_some() || egui::Popup::is_any_open(ui.ctx());
    let expanded = no_lyrics_view || !playing || hovered || busy || ((app.fullscreen || force_expand) && controls > 0.01);
    let expand_alpha = if no_lyrics_view || force_expand {
        1.0
    } else {
        theme::anim_bool(ui.ctx(), "np-sleeve-expand", expanded, 0.25)
    };
    if expand_alpha > 0.0 && expand_alpha < 1.0 {
        super::animate(ui.ctx());
    }

    let effective_alpha = if no_lyrics_view { 1.0 } else { expand_alpha * controls };
    if effective_alpha <= 0.001 {
        return;
    }
    let mut cui = ui.new_child(egui::UiBuilder::new().id_salt("np-controls").max_rect(ui.max_rect()));
    cui.multiply_opacity(effective_alpha);
    let ui = &mut cui;
    let interactive = if no_lyrics_view { true } else { effective_alpha > 0.2 };

    let bg_light = app.bg_is_light();

    let text_w = w - 76.0;
    let mut line = track.artist_line();
    if let Some(album) = &track.album
        && !album.name.is_empty()
    {
        line = format!("{line} — {}", album.name);
    }

    let title_color = if bg_light {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    let artist_color = if bg_light {
        Color32::from_black_alpha(225)
    } else {
        Color32::from_white_alpha(225)
    };

    let text_shadow_color = if bg_light {
        Color32::from_black_alpha(25)
    } else {
        Color32::from_black_alpha(135)
    };

    let title_font = theme::semibold(17.0);
    let shadow_w = if track.explicit { (text_w - 19.0).max(20.0) } else { text_w };
    let gt_shadow = widgets::galley(ui, &track.title, title_font.clone(), text_shadow_color, shadow_w);
    ui.painter().galley(pos2(x0, info_y + 1.2), gt_shadow, text_shadow_color);
    widgets::title_with_explicit(ui, pos2(x0, info_y), &track.title, title_font, title_color, text_w, track.explicit);

    let line_w = text_w;
    let line_font = theme::regular(15.0);
    let gl_shadow = widgets::galley(ui, &line, line_font.clone(), text_shadow_color, line_w);
    ui.painter().galley(pos2(x0, info_y + 22.2), gl_shadow, text_shadow_color);
    let line_rect = widgets::linked_text(ui, pos2(x0, info_y + 21.0), &line, &track.artists, line_font, artist_color, line_w, &mut app.actions);
    // The album after the em dash opens its page too, underlined like the artists.
    if let Some(album) = track.album.as_ref().filter(|a| !a.name.is_empty())
        && let Some(id) = album.id.clone()
    {
        let g = widgets::galley(ui, &line, theme::regular(15.0), artist_color, line_w);
        if let Some(at) = g.job.text.rfind(album.name.as_str()) {
            let c0 = g.job.text[..at].chars().count();
            let c1 = c0 + album.name.chars().count();
            let a0 = g.pos_from_cursor(egui::text::CCursor::new(c0)).min.x;
            let a1 = g.pos_from_cursor(egui::text::CCursor::new(c1)).min.x;
            if a1 - a0 >= 2.0 {
                let r = Rect::from_min_max(pos2(line_rect.left() + a0, line_rect.top()), pos2(line_rect.left() + a1, line_rect.bottom()));
                let resp = ui.interact(r, ui.scope_id().with(("album-link", &album.name)), if interactive { Sense::click() } else { Sense::hover() });
                let resp = named(resp, &album.name);
                let resp = if interactive { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
                if resp.hovered() {
                    let hover_color = if bg_light { Color32::BLACK } else { Color32::WHITE };
                    let hg = widgets::galley(ui, &album.name, theme::regular(15.0), hover_color, line_w);
                    ui.painter().galley(r.min, hg, hover_color);
                }
                if interactive && resp.clicked() {
                    app.actions.push(Action::Open(super::Target::Album(id)));
                }
            }
        }
    }
    // The star: a 30 px disc that washes under the pointer and fills in the accent
    // once liked. The ⋯ menu sits at the line's end, as on music.youtube.com.
    let liked = app.liked.contains(&heard.id);
    let star = Rect::from_center_size(pos2((x0 + w) - 55.0, info_y + 14.0), Vec2::splat(30.0));
    let sr = ui.interact(star, ui.scope_id().with(("np-like", &heard.id)), if interactive { Sense::click() } else { Sense::hover() });
    let like_label = if liked { t!("remove_from_favourites") } else { t!("add_to_favourites") };
    let sr = named(sr, &like_label);
    let sr = if interactive { sr.on_hover_cursor(egui::CursorIcon::PointingHand) } else { sr };
    let (sh, ss) = if interactive {
        widgets::paint_icon_touch(ui, &sr, star, bg_light)
    } else {
        (0.0, 1.0)
    };
    let pop = theme::anim_bool(ui.ctx(), ("liked", &heard.id), liked, 0.3);
    let base_star = if bg_light { Color32::BLACK } else { Color32::WHITE };
    let dim = 0.92 + 0.08 * sh;
    let tint = theme::lerp_color(theme::with_alpha(base_star, dim), app.accent_color(), pop);
    // It swells a quarter on the way, as the mini player's does.
    let size = 19.0 * ss * (1.0 + 0.25 * (pop * std::f32::consts::PI).sin());
    if !bg_light {
        super::glyphs::draw(ui, star.center() + vec2(0.0, 1.2), size, super::glyphs::STAR, Color32::from_black_alpha(90));
    }
    super::glyphs::draw(ui, star.center(), size, super::glyphs::STAR, tint);
    if pop > 0.0 {
        super::glyphs::draw(ui, star.center(), size, super::glyphs::STAR_FILLED, theme::with_alpha(app.accent_color(), pop));
    }
    if interactive && sr.clicked() {
        app.actions.push(Action::Like(heard.clone(), !liked));
    }
    // The song's options, beside the star.
    let options = Rect::from_center_size(pos2((x0 + w) - 15.0, info_y + 14.0), Vec2::splat(34.0));
    if interactive {
        options_menu(app, ui, options, &heard, app.np_tab == NpTab::Lyrics && !no_lyrics, None);
    }

    // Seek: a 4 px line that grows under the pointer, its times below.
    let status = app.audio.status.clone();
    let held = hold.is_some();
    let (total, mix) = match &hold {
        Some((total, mix, _)) => (*total, *mix),
        None => {
            let total = status
                .duration()
                .map(|d| d.as_secs_f32())
                .or(track.duration.map(|d| d as f32))
                .unwrap_or(0.0);
            let next = app.queue.upcoming().is_some() || app.autoplay;
            let mix = match (total > 0.0, next, app.queue.repeat) {
                (true, true, repeat) if repeat != Repeat::One => {
                    status.mix_secs(status.duration().or_else(|| track.duration.map(|s| std::time::Duration::from_secs(s as u64)))).min(total * 0.5)
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
    let hit = Rect::from_min_size(pos2(x0, info_y + 55.0), vec2(w, 16.0));
    // Held for the handoff's first half: its last stretch is where the pointer is,
    // and seeking the queue's song from here would cut the mix.
    let sresp = ui.interact(hit, ui.scope_id().with("np-seek"), if held || !interactive { Sense::hover() } else { Sense::click_and_drag() });
    let sresp = if total > 0.0 && !held && interactive { sresp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { sresp };
    sresp.widget_info(|| egui::WidgetInfo::slider(total > 0.0 && !held && interactive, pos as f64, "Seek"));
    let focus = theme::anim_bool(ui.ctx(), "np-seek-h", total > 0.0 && !held && interactive && (sresp.hovered() || sresp.dragged()), 0.18);
    let thick = 4.0 + 4.0 * focus;
    let bar = Rect::from_min_max(pos2(x0, hit.center().y - thick / 2.0), pos2(x0 + w, hit.center().y + thick / 2.0));
    let seek_bg = if bg_light { Color32::from_black_alpha(45) } else { Color32::from_white_alpha(50) };
    ui.painter().rect_filled(bar, thick / 2.0, seek_bg);
    // Automix: the handoff window tints to the song's colour with its start ticked, so
    // the blend has a place it visibly begins at.
    if mix > 0.5 {
        let edge = x0 + w * ((total - mix) / total);
        let band = Rect::from_min_max(pos2(edge, bar.top()), bar.max);
        ui.painter().rect_filled(band, thick / 2.0, theme::with_alpha(app.accent_color(), 0.32));
        let tick = Rect::from_min_size(pos2(edge - 1.0, bar.top()), vec2(2.0, thick));
        ui.painter().rect_filled(tick, 1.0, theme::with_alpha(app.accent_color(), 0.9));
    }
    let frac = if total > 0.0 { (pos / total).clamp(0.0, 1.0) } else { 0.0 };
    if frac > 0.0 {
        let fill = if bg_light {
            theme::lerp_color(Color32::from_black_alpha(230), Color32::BLACK, focus)
        } else {
            theme::lerp_color(Color32::from_white_alpha(235), Color32::WHITE, focus)
        };
        ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac, thick)), thick / 2.0, fill);
    }
    let at = |p: egui::Pos2| (((p.x - x0) / w.max(1.0)).clamp(0.0, 1.0) * total).max(0.0);
    let tc = if bg_light { Color32::from_black_alpha(225) } else { Color32::from_white_alpha(225) };
    let tc_shadow = if bg_light { Color32::from_black_alpha(20) } else { Color32::from_black_alpha(110) };
    let time_font = theme::semibold(11.0);
    let pos_str = super::fmt_time(pos as u32);
    let g_pos_s = widgets::galley(ui, &pos_str, time_font.clone(), tc_shadow, 80.0);
    ui.painter().galley(pos2(x0, hit.bottom() + 3.0), g_pos_s, tc_shadow);
    let g = widgets::galley(ui, &pos_str, time_font.clone(), tc, 80.0);
    ui.painter().galley(pos2(x0, hit.bottom() + 2.0), g, tc);

    let left = super::fmt_time((total - pos).max(0.0) as u32);
    let left_str = format!("-{left}");
    let g_left_s = widgets::galley(ui, &left_str, time_font.clone(), tc_shadow, 80.0);
    ui.painter().galley(pos2(x0 + w - g_left_s.size().x, hit.bottom() + 3.0), g_left_s, tc_shadow);
    let g = widgets::galley(ui, &left_str, time_font, tc, 80.0);
    ui.painter().galley(pos2(x0 + w - g.size().x, hit.bottom() + 2.0), g, tc);
    // The quality mark sits between the two times, as the reference draws it.
    let badge_w = audio_badge_width(ui, app, false);
    if badge_w > 0.0 {
        audio_badge(app, ui, pos2(x0 + (w - badge_w) / 2.0, hit.bottom() + 1.0), false);
    }
    // While the two songs blend, its name sits between the two times.
    let mixed = theme::anim_bool(ui.ctx(), "np-automix-on", app.mixing(), 0.3);
    if mixed > 0.01 && badge_w == 0.0 {
        let accent = theme::with_alpha(app.accent_color(), mixed);
        let g = widgets::galley(ui, "Mixing", theme::semibold(10.5), accent, 80.0);
        ui.painter().galley(pos2(x0 + (w - g.size().x) / 2.0, hit.bottom() + 3.0), g, accent);
    }
    if total > 0.0 && interactive
        && let Some(p) = sresp.interact_pointer_pos()
    {
        if sresp.dragged() {
            app.seek_drag = Some(at(p));
        }
        if sresp.drag_stopped() || sresp.clicked() {
            app.seek_drag = None;
            app.actions.push(Action::Seek(at(p)));
        }
    }

    // Transport: the reference's row — shuffle, previous, play, next, repeat — 52 tall,
    // no wider than 315, its buttons 36/48/56/48/36 with icons of 30/40/48/40/30.
    let ty = info_y + 99.0 + 26.0;
    let row_w = w.min(315.0);
    let sizes = [36.0f32, 48.0, 56.0, 48.0, 36.0];
    let icons = [30.0f32, 40.0, 48.0, 40.0, 30.0];
    let gap = ((row_w - sizes.iter().sum::<f32>()) / 4.0).max(4.0);
    let mut xs = [0.0f32; 5];
    let mut cx = (x0 + w / 2.0) - row_w / 2.0;
    for (i, s) in sizes.iter().enumerate() {
        xs[i] = cx + s / 2.0;
        cx += s + gap;
    }
    // The marks are primary white, an off shuffle or repeat tertiary; a pressed button
    // shrinks to 90%, as the reference's does.
    let ink = |h: f32, base: f32, on: f32| {
        let col = if bg_light { Color32::BLACK } else { Color32::WHITE };
        theme::with_alpha(theme::lerp_color(theme::with_alpha(col, base), col, h), on)
    };
    let enabled = app.current().is_some();
    let dim = if enabled { 1.0 } else { 0.4 };
    let btn = |ui: &mut Ui, i: usize, key: &str, name: &str, live: bool| -> (egui::Response, f32, f32) {
        let live = live && interactive;
        let r = Rect::from_center_size(pos2(xs[i], ty), Vec2::splat(sizes[i]));
        let resp = ui.interact(r, ui.scope_id().with(("np-t", key)), if live { Sense::click() } else { Sense::hover() });
        let resp = named(resp, name);
        let resp = if live { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
        let (h, p_scale) = if live {
            widgets::paint_icon_touch(ui, &resp, r, bg_light)
        } else {
            (0.0, 1.0)
        };
        (resp, h, 1.0 - p_scale)
    };
    let draw_np_glyph = |ui: &mut Ui, center: egui::Pos2, size: f32, glyph: &'static [u8], tint: Color32| {
        if !bg_light {
            super::glyphs::draw(ui, center + vec2(0.0, 1.2), size, glyph, Color32::from_black_alpha(85));
        }
        super::glyphs::draw(ui, center, size, glyph, tint);
    };
    let (r, h, p) = btn(ui, 0, "shuffle", &t!("shuffle_up_next"), true);
    draw_np_glyph(ui, r.rect.center(), icons[0] * (1.0 - 0.1 * p), super::glyphs::SHUFFLE, ink(h, 0.90, 1.0));
    if interactive && r.clicked() {
        app.shuffle_upcoming();
    }
    let (r, h, p) = btn(ui, 1, "prev", &t!("previous"), enabled);
    draw_np_glyph(ui, r.rect.center(), icons[1] * (1.0 - 0.1 * p), super::glyphs::PREVIOUS, ink(h, 1.0, dim));
    if interactive && r.clicked() && enabled {
        app.prev();
    }
    let play_label = if app.playing() || app.buffering { t!("pause") } else { t!("play") };
    let (r, h, p) = btn(ui, 2, "play", &play_label, enabled);
    let c = r.rect.center();
    let size = icons[2] * (1.0 - 0.1 * p);
    let mark = ink(h, 1.0, dim);
    let morph = theme::anim_bool(ui.ctx(), "np-play-morph", app.playing() || app.buffering, 0.18);
    if app.buffering {
        let a = now as f32 * 6.0;
        let pts: Vec<egui::Pos2> = (0..16).map(|i| c + vec2((a + i as f32 * 0.24).cos(), (a + i as f32 * 0.24).sin()) * 11.0).collect();
        ui.painter().add(egui::Shape::Path(egui::epaint::PathShape::line(pts, egui::epaint::PathStroke::new(2.4, mark))));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    } else {
        // The two marks cross-fade: the reference keeps its pause up through a buffer,
        // and a hard cut between them flickers.
        if morph < 1.0 {
            draw_np_glyph(ui, c, size, super::glyphs::PLAY, theme::with_alpha(mark, 1.0 - morph));
        }
        if morph > 0.0 {
            draw_np_glyph(ui, c, size, super::glyphs::PAUSE, theme::with_alpha(mark, morph));
        }
    }
    if interactive && r.clicked() && enabled {
        app.toggle();
    }
    let (r, h, p) = btn(ui, 3, "next", &t!("next"), enabled);
    draw_np_glyph(ui, r.rect.center(), icons[3] * (1.0 - 0.1 * p), super::glyphs::NEXT, ink(h, 1.0, dim));
    if interactive && r.clicked() && enabled {
        app.next();
    }
    let (glyph, on) = match app.queue.repeat {
        Repeat::Off => (super::glyphs::REPEAT, false),
        Repeat::All => (super::glyphs::REPEAT, true),
        Repeat::One => (super::glyphs::REPEAT_ONE, true),
    };
    let (r, h, p) = btn(ui, 4, "repeat", &t!("repeat"), true);
    draw_np_glyph(ui, r.rect.center(), icons[4] * (1.0 - 0.1 * p), glyph, if on { app.accent_color() } else { ink(h, 0.90, 1.0) });
    if interactive && r.clicked() {
        app.queue.repeat = match app.queue.repeat {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        };
    }

    if no_lyrics && app.np_tab == NpTab::Lyrics {
        // A brief note, faded out after a few seconds.
        let since = ui.ctx().memory_mut(|m| *m.data.get_temp_mut_or_insert_with(egui::Id::unique(("no-lyrics", &track.id)), || now));
        let a = (1.0 - ((now - since - 3.0) / 1.0) as f32).clamp(0.0, 1.0);
        if a > 0.0 {
            let base = if bg_light { Color32::BLACK } else { Color32::WHITE };
            let c = theme::with_alpha(base, a * 0.90);
            text_at(ui, pos2(x0, ty + 40.0), &t!("no_lyrics_for_this_song"), theme::regular(14.0), c, w);
            // Frames only for the fade-out.
            let wait = (since + 3.0 - now).max(0.0);
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(if wait > 0.0 { wait } else { 0.033 }));
        }
    }
}

// ------------------------------------------------------------------ lyrics

/// The full-height frosted lyrics panel on the right: synced lyrics without a header bar.
pub(super) fn lyrics_drawer(app: &mut App, ui: &mut Ui, now: f64) {
    let rect = ui.max_rect();
    if app.current().is_none() {
        ui.add_space(rect.height() * 0.4);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(t!("play_song_to_see_lyrics")).font(theme::semibold(16.0)).color(theme::text_dim()));
        });
        return;
    }
    ui.set_clip_rect(rect);
    let bg_light = theme::light();
    if !lyrics_wait(app, ui, rect, now, bg_light) {
        lyrics_panel(app, ui, rect, now, false, bg_light);
    }
}

/// A song whose lyrics nobody asked for (restored, or picked up from another device)
/// or whose answer never came: ask immediately, then stop spinning after timeout.
/// False when there are lyrics for the song to draw.
fn lyrics_wait(app: &mut App, ui: &mut Ui, rect: Rect, now: f64, bg_light: bool) -> bool {
    let Some(shown) = app.shown().cloned() else { return false };
    if app.lyrics.as_ref().is_some_and(|(id, _)| *id == shown.id) {
        return false;
    }
    let key = egui::Id::unique("lyrics-drawer-wait");
    let (since, mut asked) = ui.ctx().data(|d| d.get_temp::<(String, f64, bool)>(key)).filter(|(id, ..)| *id == shown.id).map_or((now, false), |(_, t, a)| (t, a));
    if !asked {
        app.backend.request(crate::backend::Req::Lyrics(Box::new(shown.clone())));
        asked = true;
    }
    ui.ctx().data_mut(|d| d.insert_temp(key, (shown.id.clone(), since, asked)));
    ui.add_space(rect.height() * 0.35);
    let msg_col = if bg_light { Color32::from_black_alpha(160) } else { Color32::from_white_alpha(160) };
    ui.vertical_centered(|ui| {
        if now - since < 20.0 {
            ui.spinner();
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        } else {
            ui.label(egui::RichText::new(t!("no_lyrics_yet")).font(theme::semibold(16.0)).color(msg_col));
        }
    });
    true
}

fn lyrics_panel(app: &mut App, ui: &mut Ui, rect: Rect, now: f64, centered: bool, bg_light: bool) {
    let Some(shown) = app.shown().cloned() else { return };
    let msg_col = if bg_light { Color32::from_black_alpha(160) } else { Color32::from_white_alpha(160) };
    if !app.lyrics.as_ref().is_some_and(|(id, _)| *id == shown.id) {
        let key = egui::Id::unique("lyrics-panel-wait");
        let (since, mut asked) = ui.ctx().data(|d| d.get_temp::<(String, f64, bool)>(key)).filter(|(id, ..)| *id == shown.id).map_or((now, false), |(_, t, a)| (t, a));
        if !asked {
            app.backend.request(crate::backend::Req::Lyrics(Box::new(shown.clone())));
            asked = true;
        }
        ui.ctx().data_mut(|d| d.insert_temp(key, (shown.id.clone(), since, asked)));
        ui.add_space(rect.height() * 0.35);
        ui.vertical_centered(|ui| {
            if now - since < 20.0 {
                ui.spinner();
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            } else {
                ui.label(egui::RichText::new(t!("no_lyrics_yet")).font(theme::semibold(16.0)).color(msg_col));
            }
        });
        return;
    }

    // Lent out for the frame rather than cloned (the drawing needs `app` mutably).
    let Some((id, set)) = app.lyrics.take() else { return };
    if let Some(lyrics) = app.lyric_prefs.choose(&id, &set).and_then(|p| set.get(p)) {
        let panel = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - 4.0));
        if lyrics.sync == Sync::None {
            plain_lyrics(app, ui, panel, lyrics, bg_light);
        } else {
            synced(app, ui, panel, &id, lyrics, now, centered, bg_light);
        }
    } else {
        ui.add_space(rect.height() * 0.4);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(t!("no_lyrics_found")).font(theme::semibold(20.0)).color(msg_col));
        });
    }
    app.lyrics = Some((id, set));
}

/// Romanization and translation for each line, the way the extension picks them: what
/// the lyrics ship (Apple-style TTML) first, then Unison/Google.
fn decorations<'a>(
    show_roman: bool,
    show_trans: bool,
    target: &'a str,
    extras: Option<&'a (String, String, String, crate::translate::Extras)>,
    id: &str,
    lyrics: &'a Lyrics,
) -> (Vec<Option<karaoke::Roman<'a>>>, Vec<Option<&'a str>>) {
    let extras = extras.filter(|(v, s, t, _)| v == id && s == lyrics.provider.key() && t == target).map(|(_, _, _, e)| e);
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    let roman = lyrics
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if !show_roman || l.instrumental {
                return None;
            }
            if let Some(r) = l.romanization.as_deref().filter(|r| !same(r, &l.text)) {
                return Some(if l.timed_roman.is_empty() { karaoke::Roman::Text(r) } else { karaoke::Roman::Timed(&l.timed_roman, r) });
            }
            let r = extras?.roman.get(i)?.as_deref()?;
            (!same(r, &l.text)).then_some(karaoke::Roman::Text(r))
        })
        .collect();
    let trans = lyrics
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if !show_trans || l.instrumental {
                return None;
            }
            let t = super::lyric_prefs::shipped_translation(l, target).or_else(|| extras?.trans.get(i)?.as_deref())?;
            (!same(t, &l.text)).then_some(t)
        })
        .collect();
    (roman, trans)
}

fn synced(app: &mut App, ui: &mut Ui, rect: Rect, id: &str, lyrics: &Lyrics, now: f64, narrow: bool, bg_light: bool) {
    let style = &app.look().lyrics;
    let accent = app.accent_color();
    let centered = narrow || style.align == crate::preset::Align::Center;
    let playing = if app.mix_hold().is_some() { app.audio.status.outgoing_playing() } else { app.playing() };
    // Audible position: the device plays what the player hands it a buffer later. A
    // handoff's first half is still the outgoing song's tail, which lines up with it.
    let pos = match app.mix_hold() {
        Some(h) => app.mix_hold_pos(h) as f64 - app.audio.status.latency().as_secs_f64(),
        None => app.audio.status.position_smooth().as_secs_f64() - app.audio.status.latency().as_secs_f64(),
    };
    let pos = app.lyric_clock.tick(pos, playing, now);
    let t = pos - app.lyric_prefs.offset(id, lyrics.provider, lyrics.sync == Sync::Word);
    // A Sinhala song as written: neither how it's said nor what it means under it.
    let extras = !lyrics.lines.iter().any(|l| crate::translate::sinhala(&l.text));
    let (roman, trans) = decorations(app.show_roman && extras, app.show_trans && extras, &app.lyric_prefs.target, app.lyrics_extras.as_ref(), id, lyrics);
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut h);
        lyrics.provider.key().hash(&mut h);
        app.theme_id.hash(&mut h);
        for r in &roman {
            match r {
                Some(karaoke::Roman::Timed(_, s)) | Some(karaoke::Roman::Text(s)) => s.hash(&mut h),
                None => 0u8.hash(&mut h),
            }
        }
        trans.hash(&mut h);
        h.finish()
    };
    let scale = app.lyric_prefs.text_scale;
    let frame = karaoke::Frame { lyrics, key, roman, trans, t, playing, now, rect, style, accent, centered, scale, bg_light };
    // The view lives in `app`; the frame borrows `app` for decorations, so swap it out.
    let mut view = std::mem::take(&mut app.lyric_view);
    let out = if karaoke::dev_off("view") { karaoke::Out { seek: None, animating: true, user_scrolling: false } } else { view.show(ui, frame) };
    app.lyric_view = view;
    if let Some(s) = out.seek {
        app.actions.push(Action::Seek(s as f32));
    }
    if out.animating {
        super::animate(ui.ctx());
    }
    if out.user_scrolling {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
    }
    // Sleek, minimal floating capsule to resume autoscroll when reader scrolled away.
    // If not clicked, autoscroll automatically resumes in 5s.
    let shown = theme::anim_bool(ui.ctx(), "lyr-resume", out.user_scrolling, 0.22);
    if shown > 0.0 {
        let font = theme::semibold(12.0);
        let resume_label = t!("resume_autoscroll");
        let text_color = if bg_light { Color32::from_rgb(0x18, 0x14, 0x1c) } else { Color32::from_white_alpha(225) };
        let g = widgets::galley(ui, &resume_label, font, text_color, rect.width() - 40.0);
        let h = 26.0;
        let w = g.size().x + 24.0;
        let size = vec2(w, h);
        let y = rect.top() + 10.0 + egui::lerp(-28.0..=0.0, theme::ease_out(shown));
        let rest_r = Rect::from_min_size(pos2(rect.center().x - size.x / 2.0, y), size);
        let resp = ui.interact(rest_r, ui.scope_id().with("lyr-resume-btn"), if out.user_scrolling { Sense::click() } else { Sense::hover() });
        let resp = named(resp, &resume_label).on_hover_cursor(egui::CursorIcon::PointingHand);
        let press = crate::glass::press(ui, &resp, rest_r, if out.user_scrolling { 1.0 } else { 0.0 });
        let r = press.rect;
        let hover = press.hover;
        let clip = rect.expand2(vec2(0.0, 12.0));
        let (tint, fg) = if bg_light {
            (Color32::from_white_alpha(64), Color32::from_rgb(0x18, 0x14, 0x1c))
        } else {
            let fg = theme::lerp_color(Color32::from_white_alpha((225.0 * shown) as u8), Color32::WHITE, hover);
            (Color32::from_rgba_unmultiplied(24, 24, 28, (48.0 + 24.0 * hover) as u8), fg)
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(r.expand(6.0)), |ui| {
            ui.set_clip_rect(clip);
            let mut style = crate::glass::Style::clear(crate::glass::Corner::Capsule, Some(tint), if bg_light { 0.10 } else { 0.88 }, shown);
            press.apply(&mut style);
            crate::glass::paint(ui, egui::Id::unique("lyr-resume-liquid-glass"), r, style);
        });
        let p = ui.painter().with_clip_rect(clip);
        let at = r.center() - g.size() / 2.0;
        p.galley_with_override_text_color(at, g, fg);

        if shown < 1.0 {
            super::animate(ui.ctx());
        }
        if resp.clicked() {
            app.lyric_view.resume();
        }
    }
}

fn plain_lyrics(app: &mut App, ui: &mut Ui, rect: Rect, lyrics: &Lyrics, bg_light: bool) {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let default_text = if bg_light { Color32::BLACK } else { Color32::WHITE };
    let faint_text = if bg_light { Color32::from_black_alpha(210) } else { Color32::from_white_alpha(210) };
    egui::ScrollArea::vertical().id_salt("plain-lyrics").auto_shrink(false).show(&mut child, |ui| {
        ui.label(egui::RichText::new(t!("lyrics_not_synced")).font(theme::regular(13.0)).color(faint_text));
        ui.add_space(14.0);
        let style = &app.look().lyrics;
        let size = (32.0 * style.size.clamp(0.6, 1.6)).min(rect.width() / 12.5).max(18.0);
        for l in &lyrics.lines {
            let font = theme::lyric_font_for(&style.font, style.weight, size, &l.text);
            let color = theme::with_alpha(default_text, 0.85);
            if crate::bidi::has_rtl(&l.text) {
                // Wrapped in reading order, each row in display order; RTL lines start on the right.
                let width = ui.available_width();
                let g = super::widgets::galley_wrapped(ui, &l.text, font, color, width, usize::MAX);
                let (r, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
                let x = if crate::bidi::rtl_base(&l.text) { r.right() - g.size().x } else { r.left() };
                ui.painter().galley(pos2(x, r.top()), g, color);
            } else {
                ui.label(egui::RichText::new(&l.text).font(font).color(color));
            }
            ui.add_space(size * 0.25);
        }
        ui.add_space(80.0);
    });
}

/// The ⋯ menu: the song's actions, and with lyrics showing, translation and
/// romanization toggles and this song's timing offset above them.
pub(crate) fn options_menu(
    app: &mut App,
    ui: &mut Ui,
    rect: Rect,
    track: &crate::model::Track,
    lyrics: bool,
    tint: Option<&dyn Fn(f32) -> Color32>,
) {
    let resp = ui.interact(rect, ui.scope_id().with("np-options"), Sense::click());
    let resp = named(resp, &t!("options")).on_hover_cursor(egui::CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let light_bg = if tint.is_some() { theme::light() } else { app.bg_is_light() };
    let (touch_h, touch_s) = widgets::paint_icon_touch(ui, &resp, rect, light_bg);
    let h = touch_h.max(if open { 1.0 } else { 0.0 });

    let dot_col = if let Some(tint_fn) = tint {
        tint_fn(h)
    } else {
        let bg_light = app.bg_is_light();
        let base = if bg_light { Color32::BLACK } else { Color32::WHITE };
        theme::with_alpha(base, 0.90 + 0.10 * h)
    };
    if !app.bg_is_light() {
        for dx in [-5.0, 0.0, 5.0] {
            ui.painter().circle_filled(rect.center() + vec2(dx * touch_s, 1.2), 1.8 * touch_s, Color32::from_black_alpha(85));
        }
    }
    for dx in [-5.0, 0.0, 5.0] {
        ui.painter().circle_filled(rect.center() + vec2(dx * touch_s, 0.0), 1.8 * touch_s, dot_col);
    }
    widgets::popup_menu(&resp, Some(egui::PopupCloseBehavior::CloseOnClickOutside), |ui| {
        ui.set_width(240.0);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.add_space(2.0);
        // Lyrics first: the source, translation and timing are what the menu is for
        // while the lyrics are up.
        let liked = app.liked.contains(&track.id);
        if lyrics {
            lyric_options(app, ui);
            menu_divider(ui);
        }
        widgets::song_actions(ui, track, liked, None, None, &mut app.actions);
        menu_divider(ui);
        if widgets::menu_action(ui, Some(icon::LIST_PLUS), "Play next") {
            app.actions.push(Action::PlayNext(track.clone()));
        }
        if widgets::menu_action(ui, None, "Add to queue") {
            app.actions.push(Action::Enqueue(track.clone()));
        }
        let queue = app.np_tab == NpTab::UpNext;
        if widgets::menu_action(ui, None, if queue { "View lyrics" } else { "View queue" }) {
            app.np_tab = if queue { NpTab::Lyrics } else { NpTab::UpNext };
        }
        menu_divider(ui);
        sleep_options(app, ui);
        // Quality toggle at the bottom: revert when hi-res is playing, upgrade when not.
        let hires_playing = app.audio_info.as_ref().is_some_and(|(_, i)| i.lossless() || i.atmos);
        if hires_playing || !app.hires_sources.is_empty() {
            menu_divider(ui);
            if hires_playing {
                if widgets::menu_action(ui, None, "Revert quality") {
                    app.actions.push(Action::RevertQuality);
                }
            } else if widgets::menu_action(ui, None, "Upgrade quality") {
                app.actions.push(Action::UpgradeQuality);
            }
        }
        ui.add_space(4.0);
    });
}

fn lyric_options(app: &mut App, ui: &mut Ui) {
    let Some((id, set)) = app.lyrics.as_ref() else { return };
    let id = id.clone();
    let Some(shown) = app.lyric_prefs.choose(&id, set) else { return };
    let synced = set.get(shown).is_some_and(|l| l.sync != Sync::None);
    let tip = format!("Translation · {}", lang_name(&app.lyric_prefs.target));
    switch_row(ui, &tip, &mut app.show_trans);
    switch_row(ui, &t!("romanization"), &mut app.show_roman);
    if synced {
        // This song's timing: ‹ +0.0s ›
        let off = app.lyric_prefs.song_offset(&id, shown);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        ui.painter().text(pos2(row.left() + 10.0, row.center().y), egui::Align2::LEFT_CENTER, t!("timing"), theme::regular(13.0), theme::text());
        let mut sub = ui.new_child(egui::UiBuilder::new().max_rect(row.shrink2(vec2(6.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        sub.spacing_mut().item_spacing.x = 2.0;
        if mini_icon(&mut sub, icon::CHEVRON_RIGHT, "Lyrics later (+0.1 s)", true).clicked() {
            app.lyric_prefs.set_song_offset(&id, shown, off + 0.1);
        }
        let label = format!("{off:+.1}s").replace("+0.0", "0.0");
        if mini_text(&mut sub, &label, false, "This song's lyrics timing. Double-click to reset.").double_clicked() {
            app.lyric_prefs.set_song_offset(&id, shown, 0.0);
        }
        if mini_icon(&mut sub, icon::CHEVRON_LEFT, "Lyrics earlier (−0.1 s)", true).clicked() {
            app.lyric_prefs.set_song_offset(&id, shown, off - 0.1);
        }
    }
}

/// Sleep timer: a row showing what's set (and the time left), opening in place onto
/// the lengths to pick from.
fn sleep_options(app: &mut App, ui: &mut Ui) {
    use super::sleep::{MINUTES, Sleep};
    let now = std::time::Instant::now();
    let open_id = egui::Id::unique("np-sleep-open");
    let mut open = ui.data(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let resp = named(resp, &t!("sleep_timer")).on_hover_cursor(egui::CursorIcon::PointingHand);
    let over = resp.hovered();
    if over {
        let accent = ui.ctx().data(|d| d.get_temp::<Color32>(egui::Id::unique("accent"))).unwrap_or(theme::BLUE);
        ui.painter().rect_filled(rect, 4.0, accent);
    }
    let color = if over {
        Color32::WHITE
    } else if theme::light() {
        Color32::from_rgb(32, 32, 32)
    } else {
        Color32::from_rgb(235, 235, 235)
    };
    widgets::icon_at(ui, pos2(rect.right() - 15.0, rect.center().y), if open { icon::CHEVRON_UP } else { icon::CHEVRON_DOWN }, 14.0, if over { Color32::WHITE } else { theme::text_dim() });
    ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), egui::Align2::LEFT_CENTER, t!("sleep_timer"), theme::regular(13.0), color);
    let on = app.sleep.active();
    let value = if over { Color32::WHITE } else if on { theme::text() } else { theme::text_faint() };
    ui.painter().text(pos2(rect.right() - 32.0, rect.center().y), egui::Align2::RIGHT_CENTER, app.sleep.label(now), theme::semibold(12.0), value);
    if matches!(app.sleep, Sleep::At(_)) {
        // The time left counts down while the menu is up.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(open_id, open));
    }
    if !open {
        return;
    }
    let mut pick = None;
    for m in MINUTES {
        if sleep_row(ui, &format!("{m} minutes"), false) {
            pick = Some(Sleep::after(m, now));
        }
    }
    if sleep_row(ui, &t!("end_of_this_song"), app.sleep == Sleep::EndOfSong) {
        pick = Some(Sleep::EndOfSong);
    }
    if on && sleep_row(ui, &t!("turn_off_timer"), false) {
        pick = Some(Sleep::Off);
    }
    if let Some(s) = pick {
        app.sleep = s;
        ui.data_mut(|d| d.insert_temp(open_id, false));
        app.toast(match s {
            Sleep::Off => t!("sleep_timer_off").to_string(),
            Sleep::At(_) => format!("Pausing in {}", s.label(now)),
            Sleep::EndOfSong => "Pausing at the end of this song".to_owned(),
        });
    }
}

/// One of the sleep timer's choices, indented under its row; picking it closes the menu.
fn sleep_row(ui: &mut Ui, label: &str, current: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 21.0), Sense::click());
    let resp = named(resp, label).on_hover_cursor(egui::CursorIcon::PointingHand);
    let over = resp.hovered();
    if over {
        let accent = ui.ctx().data(|d| d.get_temp::<Color32>(egui::Id::unique("accent"))).unwrap_or(theme::BLUE);
        ui.painter().rect_filled(rect, 4.0, accent);
    }
    let color = if over {
        Color32::WHITE
    } else if theme::light() {
        Color32::from_rgb(32, 32, 32)
    } else {
        Color32::from_rgb(235, 235, 235)
    };
    ui.painter().text(pos2(rect.left() + 20.0, rect.center().y), egui::Align2::LEFT_CENTER, label, theme::regular(13.0), color);
    if current {
        widgets::icon_at(ui, pos2(rect.right() - 15.0, rect.center().y), icon::CHECK, 13.0, color);
    }
    if resp.clicked() {
        ui.close();
    }
    resp.clicked()
}

fn menu_caption(ui: &mut Ui, text: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 10.0, r.center().y), egui::Align2::LEFT_CENTER, text, theme::semibold(10.5), theme::text_faint());
}

fn menu_divider(ui: &mut Ui) {
    widgets::menu_separator(ui);
}

// ------------------------------------------------------------------ audio quality

fn badge_font(small: bool) -> egui::FontId {
    theme::semibold(if small { 9.0 } else { 11.0 })
}

fn glyph_side(small: bool) -> f32 {
    if small { 11.0 } else { 13.0 }
}

/// Whether the mark leads with an icon: Apple's lossless wave, or Dolby's double-D.
fn has_quality_glyph(info: &crate::hires::Info) -> bool {
    info.atmos || info.lossless()
}

use crate::ui::widgets::{paint_quality_icon, quality_icon_aspect};

/// How wide the quality mark is, so a line can leave room for it.
pub(super) fn audio_badge_width(ui: &Ui, app: &App, small: bool) -> f32 {
    let Some((_, info)) = app.audio_info.as_ref() else { return 0.0 };
    let label = if app.mixing() { "Mixing" } else { info.badge() };
    let text = widgets::galley(ui, label, badge_font(small), Color32::WHITE, 200.0).size().x;
    if has_quality_glyph(info) { text + glyph_side(small) * quality_icon_aspect(info.atmos) + 5.0 } else { text }
}

/// The quality mark — a glyph and the stream's name, tinted with the theme — drawn at
/// `at`; a click opens its details. Returns how much room it took.
pub(super) fn audio_badge(app: &App, ui: &mut Ui, at: egui::Pos2, small: bool) -> f32 {
    let Some((_, info)) = app.audio_info.as_ref() else { return 0.0 };
    let font = badge_font(small);
    // A mix in progress wears its own name over the quality mark; the details keep the truth.
    let g = widgets::galley(ui, if app.mixing() { "Mixing" } else { info.badge() }, font, Color32::WHITE, 200.0);
    let gh = glyph_side(small);
    let glyph = has_quality_glyph(info);
    let w = audio_badge_width(ui, app, small);
    let h = g.size().y.max(if glyph { gh } else { 0.0 });
    let rect = Rect::from_min_size(at, vec2(w, h));
    let resp = ui.interact(rect, ui.scope_id().with("audio-badge"), Sense::click());
    let resp = named(resp, "Audio quality").on_hover_cursor(egui::CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hover = theme::anim_bool(ui.ctx(), resp.id.with("h"), resp.hovered() || open, 0.12);
    // Lossless and Atmos wear the song's colour; anything lossy stays quiet.
    let (ink, mark) = if info.lossless() || info.atmos {
        let accent = app.accent_color();
        let toward = if app.bg_is_light() { Color32::BLACK } else { Color32::WHITE };
        (theme::lerp_color(accent, toward, 0.10 + 0.30 * hover), theme::with_alpha(accent, 0.85 + 0.15 * hover))
    } else {
        let base = if app.bg_is_light() { Color32::BLACK } else { Color32::WHITE };
        (theme::with_alpha(base, 0.88 + 0.12 * hover), theme::with_alpha(base, 0.88))
    };
    let mut x = at.x;
    if glyph {
        let icon_w = gh * quality_icon_aspect(info.atmos);
        paint_quality_icon(ui, pos2(x + icon_w / 2.0, at.y + h / 2.0), gh, info.atmos, mark);
        x += icon_w + 5.0;
    }
    ui.painter().galley(pos2(x, at.y + (h - g.size().y) / 2.0), g, ink);
    widgets::popup_menu(&resp, Some(egui::PopupCloseBehavior::CloseOnClickOutside), |ui| audio_info_menu(ui, info));
    w
}

/// Just the quality glyph, centred on `center`, for the mini player's artist line.
pub(super) fn audio_badge_icon_width(app: &App, small: bool) -> f32 {
    let Some((_, info)) = app.audio_info.as_ref().filter(|(_, i)| has_quality_glyph(i)) else { return 0.0 };
    glyph_side(small) * quality_icon_aspect(info.atmos)
}

pub(super) fn audio_badge_icon(app: &App, ui: &mut Ui, center: egui::Pos2, small: bool) {
    let Some((_, info)) = app.audio_info.as_ref().filter(|(_, i)| has_quality_glyph(i)) else { return };
    let gh = glyph_side(small);
    let rect = Rect::from_center_size(center, vec2(gh * quality_icon_aspect(info.atmos), gh));
    let resp = ui.interact(rect, ui.scope_id().with("audio-badge"), Sense::click());
    let resp = named(resp, "Audio quality").on_hover_cursor(egui::CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hover = theme::anim_bool(ui.ctx(), resp.id.with("h"), resp.hovered() || open, 0.12);
    let accent = app.accent_color();
    let mark = theme::with_alpha(accent, 0.85 + 0.15 * hover);
    paint_quality_icon(ui, center, gh, info.atmos, mark);
    widgets::popup_menu(&resp, Some(egui::PopupCloseBehavior::CloseOnClickOutside), |ui| audio_info_menu(ui, info));
}

/// What the stream is: its source, format and the numbers under those.
fn audio_info_menu(ui: &mut Ui, info: &crate::hires::Info) {
    ui.set_width(300.0);
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    ui.add_space(4.0);
    menu_caption(ui, "AUDIO QUALITY");
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    let g = widgets::galley(ui, info.badge(), theme::semibold(17.0), theme::text(), 200.0);
    let mut x = r.left() + 12.0;
    if has_quality_glyph(info) {
        let ih = 17.0;
        let icon_w = ih * quality_icon_aspect(info.atmos);
        paint_quality_icon(ui, pos2(x + icon_w / 2.0, r.center().y), ih, info.atmos, theme::text());
        x += icon_w + 8.0;
    }
    ui.painter().galley(pos2(x, r.center().y - g.size().y / 2.0), g.clone(), theme::text());
    menu_divider(ui);
    info_row(ui, "Source", if info.source.is_empty() { &info.host } else { &info.source });
    if !info.codec.is_empty() {
        info_row(ui, "Codec", &info.codec.to_ascii_uppercase());
    }
    if let Some(kbps) = bitrate(info) {
        info_row(ui, "Bitrate", &format!("{} kbps", thousands(kbps)));
    }
    if let Some(hz) = info.sample_rate {
        info_row(ui, "Sample rate", &rate_text(hz));
    }
    if let Some(bits) = info.bit_depth {
        info_row(ui, "Bit depth", &format!("{bits}-bit"));
    }
    if let Some(n) = info.channels {
        let text = match n {
            1 => "Mono".to_owned(),
            2 => "Stereo".to_owned(),
            _ => format!("{n} channels"),
        };
        info_row(ui, "Channels", &text);
    }
    if info.size > 0 {
        info_row(ui, "Size", &size_text(info.size));
    }
    if info.duration_ms > 0 {
        info_row(ui, "Duration", &super::fmt_time((info.duration_ms / 1000) as u32));
    }
    ui.add_space(6.0);
}

fn info_row(ui: &mut Ui, label: &str, value: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 12.0, r.center().y), egui::Align2::LEFT_CENTER, label, theme::regular(13.0), theme::text_dim());
    let g = widgets::galley(ui, value, theme::semibold(13.0), theme::text(), 190.0);
    ui.painter().galley(pos2(r.right() - 12.0 - g.size().x, r.center().y - g.size().y / 2.0), g, theme::text());
}

/// The stated bitrate, or the one the file's size and length imply.
fn bitrate(info: &crate::hires::Info) -> Option<u32> {
    info.bitrate_kbps
        .or_else(|| (info.duration_ms > 0 && info.size > 0).then(|| (info.size * 8 / info.duration_ms) as u32))
        .filter(|k| *k > 0)
}

fn thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn rate_text(hz: u32) -> String {
    if hz.is_multiple_of(1000) { format!("{} kHz", hz / 1000) } else { format!("{:.1} kHz", hz as f64 / 1000.0) }
}

fn size_text(bytes: u64) -> String {
    if bytes >= 1024 * 1024 { format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)) } else { format!("{:.0} KB", bytes as f64 / 1024.0) }
}

/// A label with an on/off switch; the whole row toggles.
fn switch_row(ui: &mut Ui, label: &str, on: &mut bool) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    let resp = named(resp, label).on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        *on = !*on;
    }
    let accent = ui.ctx().data(|d| d.get_temp::<Color32>(egui::Id::unique("accent"))).unwrap_or(theme::BLUE);
    let fg = if theme::light() {
        Color32::from_rgb(32, 32, 32)
    } else {
        Color32::from_rgb(235, 235, 235)
    };
    ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), egui::Align2::LEFT_CENTER, label, theme::regular(13.0), fg);
    let track = Rect::from_center_size(pos2(rect.right() - 27.5, rect.center().y), vec2(41.0, 18.0));
    crate::glass::liquid_toggle(ui, track, on, accent, 0.70, resp.id.with("switch"), resp.clicked());
}

/// A small icon button (24 × 24).
fn mini_icon(ui: &mut Ui, glyph: char, tip: &str, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    let resp = named(resp, tip).on_hover_text(tip);
    let resp = if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    let h = theme::anim_bool(ui.ctx(), resp.id.with("h"), resp.hovered() && enabled, 0.12);
    if h > 0.0 {
        ui.painter().rect_filled(rect, 7.0, theme::glass(0.12 * h));
    }
    let a = if enabled { 0.6 + 0.35 * h } else { 0.25 };
    widgets::icon_at(ui, rect.center(), glyph, 14.0, theme::with_alpha(Color32::WHITE, a));
    resp
}

/// A small text toggle or label (24 px tall), highlighted when `on`.
fn mini_text(ui: &mut Ui, text: &str, on: bool, tip: &str) -> egui::Response {
    let font = theme::semibold(12.0);
    let g = ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, Color32::WHITE));
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 12.0, 24.0), Sense::click());
    let resp = named(resp, tip).on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand);
    let h = theme::anim_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), 0.12);
    let gold = Color32::from_rgb(255, 214, 120);
    let fill = if on { theme::with_alpha(Color32::from_rgb(255, 196, 64), 0.16 + 0.06 * h) } else { theme::glass(0.12 * h) };
    if fill.a() > 0 {
        ui.painter().rect_filled(rect, 7.0, fill);
    }
    let color = if on { gold } else { theme::with_alpha(Color32::WHITE, 0.6 + 0.35 * h) };
    ui.painter().galley_with_override_text_color(rect.center() - g.size() / 2.0, g, color);
    resp
}

fn lang_name(code: &str) -> &str {
    super::lyric_prefs::LANGUAGES.iter().find(|(c, _)| *c == code).map_or(code, |(_, n)| n)
}
