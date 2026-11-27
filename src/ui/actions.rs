//! What clicks and keys ask for: the actions pages queue up, and keyboard shortcuts.

use std::time::Duration;

use crate::backend::Req;
use crate::model::{Card, Target, Track};

use super::queue::QueueFill;
use super::{Action, App, Dialog, LibTab, NpTab, Page, jam, transfer};

impl App {
    pub(super) fn apply(&mut self, a: Action) {
        // In someone else's jam, playback changes become requests to the host.
        let a = if self.jam.guest() {
            match jam::guest_action(self, a) {
                Some(a) => a,
                None => return,
            }
        } else {
            a
        };
        // The first result opened after a search is what Recently Searched remembers.
        self.remember_search_click(&a);
        match a {
            // Clicking the song that's already playing pauses/resumes it, not restart it.
            Action::Play(tracks, i) if tracks.get(i).is_some_and(|t| self.current_id() == Some(t.id.as_str())) && self.resume_at.is_none() => {
                self.toggle()
            }
            Action::Play(tracks, i) => {
                // Play buttons and rows live on the page they play from.
                self.queue_source = match &self.page {
                    Page::Album(id) => Some(Target::Album(id.clone())),
                    Page::Playlist(id) => Some(Target::Playlist(id.clone())),
                    Page::Library(LibTab::Songs) => Some(Target::Playlist(self.active_liked_id().into())),
                    Page::Artist(id) => Some(Target::Artist(id.clone())),
                    _ => None,
                };
                // All of a playlist that's still coming in, in order or shuffled: the rest follows.
                let fill = self.page_list().filter(|id| self.receiving(id)).and_then(|id| {
                    let o = self.opened(id).filter(|o| o.c.tracks.len() == tracks.len())?;
                    Some(QueueFill { id: id.to_owned(), load: o.load, shuffled: o.c.tracks.iter().zip(&tracks).any(|(a, b)| a.id != b.id), ran_out: false })
                });
                self.play_list_filling(tracks, i, fill);
            }
            Action::TogglePlay => self.toggle(),
            Action::TransferPick(t) => transfer::use_pick(self, t),
            // Kept already: at once. A playlist that isn't starts with its first page.
            Action::PlayCollection(Target::Album(id) | Target::Playlist(id), shuffle) if self.opened(&id).is_some() => self.play_opened(id, shuffle),
            Action::PlayCollection(target, shuffle) => {
                self.toast("Starting playback…");
                self.play_pending = Some((target.clone(), shuffle));
                match target {
                    Target::Playlist(id) => self.load_list(&id, false, false),
                    target => self.backend.request(Req::PlayCollection { target }),
                }
            }
            Action::Open(Target::Song(t)) | Action::Radio(t) => {
                self.queue_source = None;
                self.play_list(vec![t.clone()], 0);
            }
            Action::Open(Target::Album(id)) => self.go(Page::Album(id)),
            Action::Open(Target::Playlist(id)) => self.go(Page::Playlist(id)),
            Action::Open(Target::Artist(id)) => self.go(Page::Artist(id)),
            Action::PlayNext(t) => {
                let start = self.queue.play_next(t);
                self.toast("Playing next");
                if start {
                    self.play_current();
                } else {
                    self.prefetch_next();
                }
            }
            Action::PlayNextMany(tracks) => {
                if tracks.is_empty() {
                    return;
                }
                let start = self.queue.play_next_many(tracks);
                self.toast("Playing next");
                if start {
                    self.play_current();
                } else {
                    self.prefetch_next();
                }
            }
            Action::Enqueue(t) => {
                let start = self.queue.enqueue(t);
                self.toast("Added to queue");
                if start {
                    self.play_current();
                }
            }
            Action::Go(p) => self.go(p),
            Action::QueueJump(i) => {
                self.play_pending = None;
                self.queue.index = Some(i);
                self.play_current();
            }
            Action::QueueRemove(i) => self.queue.remove(i),
            Action::QueueMove(from, to) => {
                self.queue.move_track(from, to);
            }
            Action::ViewQueue => {
                if self.np_open {
                    self.np_tab = NpTab::UpNext;
                } else {
                    self.queue_open = true;
                }
            }
            Action::Seek(secs) => {
                self.seek_to(Duration::from_secs_f32(secs.max(0.0)));
                self.lyric_view.resume();
            }
            Action::Like(t, like) => {
                if !self.signed_in_for(&t.id) {
                    self.toast(self.sign_in_prompt_for(&t.id, "like songs", "like this song"));
                    return;
                }
                // Optimistic: update the like state and the cached Liked songs list now.
                self.set_liked(std::slice::from_ref(&t), like);
                self.backend.request(Req::Like { video_id: t.id, like });
            }
            Action::LikeMany(tracks, like) => {
                if !self.logged_in {
                    self.toast("Sign in to like songs");
                    return;
                }
                let tracks: Vec<Track> = tracks.into_iter().filter(|t| self.liked.contains(&t.id) != like).collect();
                if tracks.is_empty() { return; }
                if like {
                    self.set_liked(&tracks, true);
                } else {
                    // Unlikes go a few a second (YouTube Music drops faster ones): each song
                    // leaves the list when its unlike goes through, not all at once.
                    self.unliking.extend(tracks.iter().map(|t| (t.id.clone(), t.clone())));
                }
                if tracks.len() > 10 {
                    // One at a time, then checked: a few hundred take a few minutes.
                    self.toast(format!("{} {} songs…", if like { "Liking" } else { "Removing likes from" }, tracks.len()));
                }
                self.backend.request(Req::LikeMany { video_ids: tracks.into_iter().map(|t| t.id).collect(), like });
            }
            Action::Dislike(t) => {
                if !self.logged_in { self.toast("Sign in to rate songs"); return; }
                self.set_liked(std::slice::from_ref(&t), false);
                self.backend.request(Req::Dislike(t.id));
            }
            Action::Share(target) => {
                self.share_pending = Some(share_url(&target));
                self.toast("Link copied");
            }
            Action::ShareAt(target, rect) => {
                self.share_sheet_pending = Some((share_url(&target), rect));
            }
            Action::Download(track) => {
                self.toast(format!("Downloading \u{201c}{}\u{201d}", track.title));
                // The song version playing for a music video: that video plays the file too.
                let for_video = if self.playing_as().is_some_and(|s| s.id == track.id) { self.current_id().map(str::to_owned) } else { None };
                self.backend.request(Req::Download { track: Box::new(track), for_video });
            }
            Action::DownloadMany(tracks) => {
                let missing: Vec<Track> = tracks.into_iter().filter(|t| crate::downloads::path(&t.id).is_none()).collect();
                if missing.is_empty() {
                    self.toast("Already downloaded");
                    return;
                }
                if missing.len() == 1 {
                    let track = missing.into_iter().next().unwrap();
                    self.toast(format!("Downloading \u{201c}{}\u{201d}", track.title));
                    let for_video = if self.playing_as().is_some_and(|s| s.id == track.id) { self.current_id().map(str::to_owned) } else { None };
                    self.backend.request(Req::Download { track: Box::new(track), for_video });
                } else {
                    self.toast(format!("Downloading {} songs…", missing.len()));
                    let playing_as_id = self.playing_as().map(|s| s.id.clone());
                    let cur_id = self.current_id().map(str::to_owned);
                    for track in missing {
                        let for_video = if playing_as_id.as_deref() == Some(track.id.as_str()) { cur_id.clone() } else { None };
                        self.backend.request(Req::Download { track: Box::new(track), for_video });
                    }
                }
            }
            Action::AddToPlaylist(tracks) => {
                if self.logged_in { self.dialog = Some(Dialog::PickPlaylist(tracks)); }
                else { self.toast("Sign in to edit playlists"); }
            }
            Action::RemoveFromPlaylist(mut rows) => {
                let Some(c) = self.current_collection().filter(|c| c.owned && !c.is_album) else { return };
                rows.sort_unstable();
                rows.dedup();
                rows.retain(|&i| c.set_ids.get(i).is_some_and(Option::is_some));
                if rows.is_empty() {
                    self.toast("YouTube Music didn't say where these songs are in the playlist. Reload it and try again.");
                    return;
                }
                let items: Vec<(String, String)> = rows.iter().map(|&i| (c.tracks[i].id.clone(), c.set_ids[i].clone().unwrap_or_default())).collect();
                let playlist = c.id.clone();
                // Optimistic: drop the rows now; the playlist reloads once YouTube confirms.
                c.remove_rows(&rows);
                log::debug!("list {playlist}: removing {} entries", items.len());
                self.lists.edits.remove(&playlist, items.iter().map(|(_, set)| set.clone()));
                self.selection.clear();
                self.backend.request(Req::RemoveFromPlaylist { playlist_id: playlist, items });
            }
            Action::NewPlaylist(tracks) => {
                if self.logged_in { self.dialog = Some(Dialog::NewPlaylist { title: String::new(), tracks }); }
                else { self.toast("Sign in to create playlists"); }
            }
            Action::EditPlaylist => {
                if let Some(c) = self.current_collection().filter(|c| !c.is_album) {
                    self.dialog = Some(Dialog::EditPlaylist {
                        id: c.id.clone(), title: c.title.clone(), description: c.description.clone().unwrap_or_default(), privacy: None,
                    });
                }
            }
            Action::Collaborate => {
                if let Some(id) = self.current_collection().filter(|c| !c.is_album && c.owned).map(|c| c.id.clone()) {
                    self.backend.request(Req::Collaboration(id.clone()));
                    self.dialog = Some(Dialog::Collaborate { id, info: None, busy: true, confirm_off: false });
                }
            }
            Action::DeletePlaylist => {
                if let Some(c) = self.current_collection().filter(|c| !c.is_album) {
                    self.dialog = Some(Dialog::ConfirmDelete { id: c.id.clone(), title: c.title.clone() });
                }
            }
            Action::SaveCollection(id, save) => {
                if self.signed_in_for(&id) {
                    // Flip the library tick now: the save's round trip, and the library
                    // refresh after it, are far too slow to leave the button stuck.
                    let card = self.lists.get(&id).map(|o| {
                        let c = &o.c;
                        Card {
                            title: c.title.clone(),
                            subtitle: c.byline(),
                            thumbs: c.thumbs.clone(),
                            target: if c.is_album { Target::Album(id.clone()) } else { Target::Playlist(id.clone()) },
                            round: false,
                            links: c.artists.clone(),
                        }
                    });
                    if let Some(card) = card {
                        self.set_saved(&card, save);
                        self.optimistic_save = Some((card, save));
                    }
                    self.backend.request(Req::SaveCollection { id, save });
                } else {
                    self.toast(self.sign_in_prompt_for(&id, "save to library", "save this to library"));
                }
            }
            Action::TogglePin(card) => {
                if let Some(pos) = self.pinned.iter().position(|c| c.target == card.target) {
                    self.pinned.remove(pos);
                    self.toast("Unpinned");
                } else {
                    self.pinned.insert(0, card);
                    self.toast("Pinned to sidebar");
                }
            }
            Action::ToggleFavouriteCollection(id) => {
                if self.favourite_collections.remove(&id) {
                    self.toast("Removed from favourites");
                } else {
                    self.favourite_collections.insert(id);
                    self.toast("Added to favourites");
                }
            }
            Action::SuggestLess(id) => {
                if self.suggest_less.remove(&id) {
                    self.toast("Removed from suggest less");
                } else {
                    self.suggest_less.insert(id);
                    self.toast("We'll suggest less like this");
                }
            }
            // The account's YouTube subscription to the artist, flipped at once (the
            // round trip is slow) and put back if YouTube Music refuses it.
            Action::SetSubscribed(subscribe) => {
                let channel = self.artist.as_ref().and_then(|a| a.following.as_ref()).map(|s| s.channel.clone());
                if let Some(ch) = channel.as_deref()
                    && !self.signed_in_for(ch)
                {
                    self.toast(self.sign_in_prompt_for(ch, "subscribe", "subscribe"));
                } else if channel.is_none() && !self.active_logged_in() {
                    self.toast("Sign in to subscribe");
                } else if let Some(sub) = self.artist.as_mut().and_then(|a| a.following.as_mut()) {
                    self.optimistic_sub = Some((sub.channel.clone(), sub.subscribed));
                    sub.subscribed = subscribe;
                    for (cached, _) in self.artists_cache.values_mut() {
                        if let Some(s) = cached.following.as_mut().filter(|s| s.channel == sub.channel) {
                            s.subscribed = subscribe;
                        }
                    }
                    let req = Req::SetSubscribed {
                        channel: sub.channel.clone(),
                        subscribe,
                        params: sub.params.clone(),
                    };
                    self.backend.request(req);
                }
            }
            Action::HomeChip(params) => {
                self.home_loading = true;
                if let Some(h) = &mut self.home {
                    h.shelves.clear();
                    for c in &mut h.chips {
                        c.selected = c.params == params && params.is_some();
                    }
                }
                self.backend.request(Req::Home(params));
            }
            Action::OpenNowPlaying(open) => {
                if self.current().is_some() {
                    self.np_open = open;
                }
            }
            Action::Mini(on) => self.mini.request(on),
            Action::RevertQuality => {
                // Mark the playing stream's id (a song version differs from the listed id)
                // so resolve skips hi-res for it, then replay at the same position.
                let listed = self.current_id().map(str::to_owned);
                let playing = self.playing_track().map(|t| t.id.clone()).or(listed.clone());
                let Some(id) = playing else { return };
                let at = self.audio.status.position();
                self.backend.revert_quality(&id);
                if let Some(listed) = listed.filter(|l| l != &id) {
                    self.backend.revert_quality(&listed);
                }
                self.play_current_at(at, false);
            }
            Action::UpgradeQuality => {
                let listed = self.current_id().map(str::to_owned);
                let playing = self.playing_track().map(|t| t.id.clone()).or(listed.clone());
                let Some(id) = playing else { return };
                let at = self.audio.status.position();
                self.backend.upgrade_quality(&id);
                if let Some(listed) = listed.filter(|l| l != &id) {
                    self.backend.upgrade_quality(&listed);
                }
                self.play_current_at(at, false);
            }
        }
    }

    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        // In a text box, ⌘ with the arrows moves the cursor to line ends (and the text's
        // ends) instead; on a focused button or slider they still skip and change the volume.
        let editing = ctx.text_edit_focused();
        let i = ctx.input(|i| {
            // Plain keys only: ⌘L and ⌘F are their own shortcuts, ⌘Q quits.
            let k = |key| !typing && !i.modifiers.command && i.key_pressed(key);
            // ⌘ on macOS (`command` is Ctrl elsewhere).
            let cmd = |key| !editing && i.modifiers.command && i.key_pressed(key);
            (
                k(egui::Key::Space),
                i.pointer.button_pressed(egui::PointerButton::Extra1)
                    || (i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft)),
                i.pointer.button_pressed(egui::PointerButton::Extra2)
                    || (i.modifiers.alt && i.key_pressed(egui::Key::ArrowRight)),
                cmd(egui::Key::ArrowRight),
                cmd(egui::Key::ArrowLeft),
                k(egui::Key::L),
                k(egui::Key::Q),
                k(egui::Key::F),
                i.key_pressed(egui::Key::Escape),
                cmd(egui::Key::ArrowUp),
                cmd(egui::Key::ArrowDown),
                // Once per press: holding it doesn't flip back and forth.
                i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::M, pressed: true, repeat: false, modifiers, .. } if modifiers.command && modifiers.alt)),
                cmd(egui::Key::L),
                i.modifiers.command && i.key_pressed(egui::Key::Comma),
                cmd(egui::Key::W),
                cmd(egui::Key::F),
            )
        });
        let (space, back, fwd, next, prev, lyr, queue, full, esc, vol_up, vol_down, mini, lyr_panel, settings, close_w, find) = i;
        if find {
            self.np_open = false;
            self.search_refocus = true;
            if !matches!(self.page, Page::Search(_)) {
                self.go(Page::Search(self.search_text.clone()));
            }
        }
        if mini {
            self.actions.push(Action::Mini(!self.mini.target()));
        }
        // The mini player has no pages or now playing to go to, nor does a window on
        // its way back until it's there.
        let busy = self.mini.busy();
        let (back, fwd, lyr, queue, full, esc, lyr_panel, settings) =
            if busy { Default::default() } else { (back, fwd, lyr, queue, full, esc, lyr_panel, settings) };
        if space {
            self.toggle();
        }
        if back {
            self.nav_back();
        }
        if fwd {
            self.nav_forward();
        }
        if next {
            self.next();
        }
        if prev {
            self.prev();
        }
        if lyr && self.current().is_some() {
            self.np_open = true;
            self.np_tab = NpTab::Lyrics;
        }
        if lyr_panel {
            self.toggle_lyrics();
        }
        if settings {
            self.np_open = false;
            self.go(Page::Settings);
        }
        if queue && self.current().is_some() {
            self.np_open = true;
            self.np_tab = NpTab::UpNext;
        }
        let is_fullscreen = self.fullscreen || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let fs_toggled = ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique("fullscreen-toggled"))).unwrap_or(false);
        if (full && self.np_open) || fs_toggled {
            self.set_fullscreen(ctx, !is_fullscreen);
        }
        let in_search = self.search_focused || !self.search_text.is_empty();
        if esc {
            if in_search {
                self.search_focused = false;
                self.search_text.clear();
                self.search_synced.clear();
                self.live_search_due = None;
                self.suggestions = Default::default();
                self.suggest_sel = None;
                if matches!(self.page, Page::Search(_)) {
                    self.page = Page::Search(String::new());
                }
            } else if is_fullscreen {
                self.set_fullscreen(ctx, false);
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            } else {
                self.np_open = false;
            }
        }
        if close_w {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.window_hidden = true;
        }
    if vol_up || vol_down {
            let v = (self.audio.volume() + if vol_up { 0.05 } else { -0.05 }).clamp(0.0, 1.0);
            self.audio.set_volume(v);
        }
    }

    /// The player bar's lyrics button (and ⌘L): now playing's lyrics while it's up,
    /// otherwise the lyrics panel beside the pages, in Up next's place.
    pub(super) fn toggle_lyrics(&mut self) {
        let on = if self.np_open { self.np_tab == NpTab::Lyrics && !self.no_lyrics() } else { self.lyrics_drawer && !self.queue_open };
        if self.np_open {
            if on {
                self.np_open = false;
            } else {
                self.np_tab = NpTab::Lyrics;
            }
        } else {
            self.lyrics_drawer = !on;
            self.queue_open = false;
        }
    }

    /// Into the mini player or back. Full screen ends first, a frame ahead.
    pub(super) fn set_mini(&mut self, ctx: &egui::Context, on: bool, now: f64) {
        if on == self.mini.active() {
            return;
        }
        if on && self.fullscreen {
            self.set_fullscreen(ctx, false);
            self.mini.request(true);
            return;
        }
        self.mini.toggle(ctx, now);
    }

    pub(super) fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.fullscreen = on;
        super::titlebar::set_fullscreen(ctx, on);
        if !on && self.mouse_hidden {
            self.mouse_hidden = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        }
        // Windows: in one step, without winit's unthemed-frame flash.
        if !crate::fullscreen::set(on) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
        } else if !on {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        ctx.request_repaint();
    }
}

fn share_url(target: &Target) -> String {
    match target {
        Target::Song(t) => format!("https://music.youtube.com/watch?v={}", t.id),
        Target::Playlist(id) => format!("https://music.youtube.com/playlist?list={id}"),
        Target::Album(id) => format!("https://music.youtube.com/browse/{id}"),
        Target::Artist(id) => format!("https://music.youtube.com/channel/{id}"),
    }
}

#[cfg(test)]
mod share_tests {
    use super::*;

    #[test]
    fn shares_youtube_music_urls() {
        let song = Track { id: "abc123".into(), title: String::new(), artists: vec![], album: None, duration: None, thumbs: vec![], track_nr: None, plays: None, music_video: false, explicit: false };
        assert_eq!(share_url(&Target::Song(song)), "https://music.youtube.com/watch?v=abc123");
        assert_eq!(share_url(&Target::Playlist("PL123".into())), "https://music.youtube.com/playlist?list=PL123");
        assert_eq!(share_url(&Target::Album("MPRE123".into())), "https://music.youtube.com/browse/MPRE123");
        assert_eq!(share_url(&Target::Artist("UC123".into())), "https://music.youtube.com/channel/UC123");
    }
}
