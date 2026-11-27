//! Spotify `spclient.wg.spotify.com` catalog, search, library, playlist, radio, and lyrics client.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{StreamExt, future::join_all, stream};
use reqwest::Method;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::backend::{ArtistPage, ArtistSub, Collection, Explore, Library, SearchResults};
use crate::lyrics::{Line, Part};
use crate::model::{Card, Link, Shelf, ShelfKind, Target, Thumb, Track};
use crate::proxy;
use crate::ytm::HomePage;

use super::auth::{
    EmailCodeChallenge, EmailLoginStep, SpotifyAuth, random_uuid_v4,
};
use super::proto::{ProtoReader, canonical_uri, encode, gid_uri, hex_lower, unzigzag, uri_id};

const SPCLIENT_BASE: &str = "https://spclient.wg.spotify.com/";
pub const SPOTIFY_LIKED_ID: &str = "spotify:collection:tracks";
const LIKED_SONGS_COVER: &str = "https://misc.scdn.co/liked-songs/liked-songs-640.png";

const EXT_ARTIST_V4: i32 = 8;
const EXT_ALBUM_V4: i32 = 9;
const EXT_TRACK_V4: i32 = 10;
const EXT_POPULAR_RELEASES: i32 = 152;
const EXT_RELATED_CREATORS: i32 = 165;
const EXT_TOP_PLAYABLES: i32 = 169;
const EXT_REPUTATION: i32 = 185;

const COLL_MIME: &str = "application/vnd.collection-v2.spotify.proto";

pub struct SpotifyClient {
    pub auth: SpotifyAuth,
    liked_tracks: RwLock<HashSet<String>>,
    liked_order: RwLock<Vec<String>>,
    saved_albums: RwLock<HashSet<String>>,
    followed_artists: RwLock<HashSet<String>>,
    playlist_cover_cache: RwLock<HashMap<String, Vec<Thumb>>>,
    editorial_cache: RwLock<Option<Vec<Card>>>,
    user_names: RwLock<HashMap<String, String>>,
    collection_loaded: RwLock<bool>,
}

impl SpotifyClient {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            auth: SpotifyAuth::new(data_dir),
            liked_tracks: RwLock::new(HashSet::new()),
            liked_order: RwLock::new(Vec::new()),
            saved_albums: RwLock::new(HashSet::new()),
            followed_artists: RwLock::new(HashSet::new()),
            playlist_cover_cache: RwLock::new(HashMap::new()),
            editorial_cache: RwLock::new(None),
            user_names: RwLock::new(HashMap::new()),
            collection_loaded: RwLock::new(false),
        }
    }

    pub fn saved_display_name(&self) -> Option<String> {
        self.auth.saved_display_name()
    }

    pub async fn restore_if_needed(&self) -> Result<Option<String>> {
        let Some(username) = self.auth.restore_if_needed(&proxy::http()).await? else {
            return Ok(None);
        };
        Ok(Some(self.resolve_signed_in_display_name(&username).await))
    }

    pub async fn signed_in_username(&self) -> Option<String> {
        self.auth.signed_in_username().await
    }

    pub async fn is_signed_in(&self) -> bool {
        self.auth.is_signed_in().await
    }

    pub async fn start_email_login(&self, email: &str) -> Result<EmailLoginStep> {
        match self.auth.start_email_login(&proxy::http(), email).await? {
            EmailLoginStep::SignedIn(u) => {
                let _ = self.refresh_liked_tracks().await;
                Ok(EmailLoginStep::SignedIn(self.resolve_signed_in_display_name(&u).await))
            }
            EmailLoginStep::NeedsCode(ch) => Ok(EmailLoginStep::NeedsCode(ch)),
        }
    }

    pub async fn finish_email_login(
        &self,
        challenge: &EmailCodeChallenge,
        code: &str,
    ) -> Result<String> {
        let user = self
            .auth
            .finish_email_login(&proxy::http(), challenge, code)
            .await?;
        let _ = self.refresh_liked_tracks().await;
        Ok(self.resolve_signed_in_display_name(&user).await)
    }

    pub async fn login_password(&self, username: &str, password: &str) -> Result<String> {
        let user = self
            .auth
            .login_password(&proxy::http(), username, password)
            .await?;
        let _ = self.refresh_liked_tracks().await;
        Ok(self.resolve_signed_in_display_name(&user).await)
    }

    pub async fn login_sp_dc(&self, sp_dc: &str) -> Result<String> {
        let user = self.auth.login_sp_dc(&proxy::http(), sp_dc).await?;
        let _ = self.refresh_liked_tracks().await;
        Ok(self.resolve_signed_in_display_name(&user).await)
    }

    pub async fn logout(&self) {
        self.auth.logout().await;
        self.liked_tracks.write().await.clear();
        self.liked_order.write().await.clear();
        self.saved_albums.write().await.clear();
        self.followed_artists.write().await.clear();
        self.playlist_cover_cache.write().await.clear();
        *self.editorial_cache.write().await = None;
        self.user_names.write().await.clear();
        *self.collection_loaded.write().await = false;
    }

    pub async fn liked_uris(&self) -> Vec<String> {
        self.liked_tracks.read().await.iter().cloned().collect()
    }

    pub async fn home_page(&self) -> Result<HomePage> {
        let (lib, artists) = self.library().await?;
        let liked_uris: Vec<String> = self
            .liked_order
            .read()
            .await
            .iter()
            .take(16)
            .cloned()
            .collect();

        let first_playlist_id = lib
            .playlists
            .iter()
            .find_map(|c| match &c.target {
                Target::Playlist(id) if id != SPOTIFY_LIKED_ID => Some((id.clone(), c.title.clone())),
                _ => None,
            });
        let first_artist_id = artists.first().and_then(|c| match &c.target {
            Target::Artist(id) => Some((id.clone(), c.title.clone())),
            _ => None,
        });
        let artist_ids: Vec<String> = artists
            .iter()
            .take(8)
            .filter_map(|c| match &c.target {
                Target::Artist(id) => Some(id.clone()),
                _ => None,
            })
            .collect();

        let liked_fut = self.hydrate_tracks(&liked_uris);
        let pl_fut = async {
            match &first_playlist_id {
                Some((pid, _)) => self.playlist_page(pid, 0, 16).await.ok().map(|(c, _)| c.tracks),
                None => None,
            }
        };
        let releases_fut = self.latest_releases_for_artists(&artist_ids, 16);
        let editorial_fut = self.suggested_playlists();
        let artist_radio_fut = async {
            match &first_artist_id {
                Some((aid, _)) => self.radio(aid).await.ok(),
                None => None,
            }
        };

        let (liked_res, pl_tracks, recent_releases, editorial, artist_radio) =
            tokio::join!(liked_fut, pl_fut, releases_fut, editorial_fut, artist_radio_fut);
        let liked_tracks = liked_res.unwrap_or_default();

        // Seed quick picks from Liked Songs, or fall back to top playlist + artist radio.
        let mut quick_picks = liked_tracks.clone();
        if quick_picks.is_empty() {
            if let Some(ref tracks) = pl_tracks {
                quick_picks.extend(tracks.iter().take(12).cloned());
            }
            if let Some(ref radio_tracks) = artist_radio {
                for t in radio_tracks {
                    if !quick_picks.iter().any(|x| x.id == t.id) {
                        quick_picks.push(t.clone());
                    }
                    if quick_picks.len() >= 16 {
                        break;
                    }
                }
            }
        }
        quick_picks.truncate(16);

        let radio_seed = liked_tracks
            .first()
            .map(|t| (t.id.clone(), t.title.clone()))
            .or_else(|| pl_tracks.as_ref().and_then(|v| v.first()).map(|t| (t.id.clone(), t.title.clone())));
        let track_radio = match radio_seed {
            Some((ref sid, _)) => self.radio(sid).await.ok().unwrap_or_default(),
            None => Vec::new(),
        };

        let mut shelves = Vec::new();
        if !quick_picks.is_empty() {
            let title = if liked_tracks.is_empty() {
                "Quick picks".to_string()
            } else {
                "Liked songs".to_string()
            };
            shelves.push(Shelf {
                title,
                strapline: Some("SPOTIFY".to_string()),
                strap_thumb: None,
                kind: ShelfKind::Songs {
                    tracks: quick_picks,
                    rows: 4,
                },
            });
        }

        let visible_playlists: Vec<Card> = lib
            .playlists
            .into_iter()
            .filter(|c| match &c.target {
                Target::Playlist(id) => id != SPOTIFY_LIKED_ID || !liked_tracks.is_empty(),
                _ => true,
            })
            .take(20)
            .collect();
        if !visible_playlists.is_empty() {
            shelves.push(Shelf {
                title: "Your Spotify playlists".to_string(),
                strapline: None,
                strap_thumb: None,
                kind: ShelfKind::Cards(visible_playlists),
            });
        }

        if !recent_releases.is_empty() {
            shelves.push(Shelf {
                title: "New releases from artists you follow".to_string(),
                strapline: None,
                strap_thumb: None,
                kind: ShelfKind::Cards(recent_releases),
            });
        }

        if !editorial.is_empty() {
            shelves.push(Shelf {
                title: "Suggested playlists".to_string(),
                strapline: Some("DISCOVER".to_string()),
                strap_thumb: None,
                kind: ShelfKind::Cards(editorial),
            });
        }

        if let Some((ref sid, ref sname)) = radio_seed {
            let mix: Vec<Track> = track_radio
                .into_iter()
                .filter(|t| &t.id != sid)
                .take(16)
                .collect();
            if !mix.is_empty() {
                shelves.push(Shelf {
                    title: format!("Mix inspired by {sname}"),
                    strapline: Some("RADIO".to_string()),
                    strap_thumb: None,
                    kind: ShelfKind::Songs { tracks: mix, rows: 4 },
                });
            }
        } else if let (Some((_, aname)), Some(mut mix)) = (&first_artist_id, artist_radio) {
            mix.truncate(16);
            if !mix.is_empty() {
                shelves.push(Shelf {
                    title: format!("{aname} Radio"),
                    strapline: Some("RADIO".to_string()),
                    strap_thumb: None,
                    kind: ShelfKind::Songs { tracks: mix, rows: 4 },
                });
            }
        }

        if !lib.albums.is_empty() {
            shelves.push(Shelf {
                title: "Saved albums".to_string(),
                strapline: None,
                strap_thumb: None,
                kind: ShelfKind::Cards(lib.albums.into_iter().take(20).collect()),
            });
        }

        if !artists.is_empty() {
            shelves.push(Shelf {
                title: "Artists you follow".to_string(),
                strapline: None,
                strap_thumb: None,
                kind: ShelfKind::Cards(artists.into_iter().take(20).collect()),
            });
        }

        Ok(HomePage {
            chips: Vec::new(),
            shelves,
            continuation: None,
        })
    }

    pub async fn explore_page(&self) -> Result<Explore> {
        let (lib, artists) = self.library().await?;
        let artist_ids: Vec<String> = artists
            .iter()
            .take(10)
            .filter_map(|c| match &c.target {
                Target::Artist(id) => Some(id.clone()),
                _ => None,
            })
            .collect();
        let seed_uri = self
            .liked_order
            .read()
            .await
            .first()
            .cloned()
            .or_else(|| artist_ids.first().cloned())
            .or_else(|| {
                lib.playlists.iter().find_map(|c| match &c.target {
                    Target::Playlist(id) if id != SPOTIFY_LIKED_ID => Some(id.clone()),
                    _ => None,
                })
            });
        let (releases, editorial, top) = tokio::join!(
            self.latest_releases_for_artists(&artist_ids, 20),
            self.suggested_playlists(),
            async {
                match seed_uri {
                    Some(ref uri) => self.radio(uri).await.unwrap_or_default(),
                    None => Vec::new(),
                }
            }
        );
        let new_albums = if releases.is_empty() { lib.albums } else { releases };
        let mut charts: Vec<Card> = editorial;
        for c in lib.playlists {
            if matches!(&c.target, Target::Playlist(id) if id == SPOTIFY_LIKED_ID) {
                continue;
            }
            if !charts.iter().any(|x| x.target == c.target) {
                charts.push(c);
            }
        }
        Ok(Explore {
            new_albums,
            charts,
            top,
        })
    }

    async fn suggested_playlists(&self) -> Vec<Card> {
        if let Some(cached) = self.editorial_cache.read().await.clone() {
            return cached;
        }
        const EDITORIAL_IDS: &[&str] = &[
            "37i9dQZF1DXcBWIGoYBM5M", // Today's Top Hits
            "37i9dQZF1DX4JAvHpjipBk", // New Music Friday
            "37i9dQZEVXbMDoHDwVN2tF", // Top 50 - Global
            "37i9dQZEVXbLiRSasKsNU9", // Viral 50 - Global
            "37i9dQZF1DX4WYpdgoIcn6", // Chill Hits
            "37i9dQZF1DX0XUsuxWHRQd", // RapCaviar
            "37i9dQZF1DXcRXFNfZr7Tp", // Pop Rising
            "37i9dQZF1DX4o1oenSJRJd", // All Out 2000s
            "37i9dQZF1DWXRqgorJj26U", // Rock Classics
            "37i9dQZF1DX10zKzsJ2jva", // Viva Latino
        ];
        let futs = EDITORIAL_IDS.iter().map(|id| self.fetch_playlist_card(id));
        let cards: Vec<Card> = join_all(futs).await.into_iter().flatten().collect();
        if !cards.is_empty() {
            *self.editorial_cache.write().await = Some(cards.clone());
        }
        cards
    }

    async fn fetch_playlist_card(&self, base62: &str) -> Option<Card> {
        let uri = format!("spotify:playlist:{base62}");
        let url = format!(
            "{SPCLIENT_BASE}playlist/v2/playlist/{base62}?decorate=attributes,length,owner&from=0&length=0"
        );
        let bytes = self
            .sp_request(
                Method::GET,
                &url,
                None,
                "application/protobuf",
                Some(&[("x-accept-list-items", "audio-track")]),
            )
            .await
            .ok()?;
        let slc = ProtoReader::parse(&bytes).ok()?;
        let attrs = slc.message(3)?;
        let title = attrs.text(1).filter(|s| !s.is_empty())?;
        let thumbs = playlist_thumbs(&attrs);
        let owner = self
            .resolve_owner_label(slc.text(11).as_deref(), slc.text(16).as_deref())
            .await;
        if !thumbs.is_empty() {
            self.playlist_cover_cache
                .write()
                .await
                .insert(uri.clone(), thumbs.clone());
        }
        Some(Card {
            title,
            subtitle: if owner.is_empty() {
                "Spotify".to_string()
            } else {
                owner
            },
            thumbs,
            target: Target::Playlist(uri),
            round: false,
            links: Vec::new(),
        })
    }

    async fn latest_releases_for_artists(&self, artist_uris: &[String], limit: usize) -> Vec<Card> {
        if artist_uris.is_empty() {
            return Vec::new();
        }
        let queries: Vec<(&str, i32)> = artist_uris
            .iter()
            .map(|u| (u.as_str(), EXT_ARTIST_V4))
            .collect();
        let Ok(meta) = self.extended_metadata(&queries).await else {
            return Vec::new();
        };
        let mut album_uris = Vec::new();
        if let Some(map) = meta.get(&EXT_ARTIST_V4) {
            for u in artist_uris {
                if let Some(raw) = map.get(u) {
                    if let Ok(msg) = ProtoReader::parse(raw) {
                        for field in [5u32, 6] {
                            for grp in msg.messages(field) {
                                if let Some(alb_uri) = grp
                                    .messages(1)
                                    .into_iter()
                                    .find_map(|a| a.bytes(1).and_then(|g| gid_uri("album", g)))
                                {
                                    if !album_uris.contains(&alb_uri) {
                                        album_uris.push(alb_uri);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        album_uris.truncate(limit.saturating_mul(2));
        let mut cards = self.hydrate_albums_sorted(&album_uris).await.unwrap_or_default();
        cards.truncate(limit);
        cards
    }

    pub async fn search(&self, query: &str) -> Result<SearchResults> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(SearchResults {
                query: String::new(),
                tracks: Vec::new(),
                albums: Vec::new(),
                artists: Vec::new(),
                playlists: Vec::new(),
            });
        }
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let req_id = random_uuid_v4();
        let url = format!(
            "{SPCLIENT_BASE}searchview/v3/search?query={}&locale=en&entity_types=album,artist,playlist,track&limit=20&album_states=live,prerelease&request_id={req_id}&timestamp={ts}",
            url::form_urlencoded::byte_serialize(q.as_bytes()).collect::<String>()
        );
        let body = self
            .sp_request(Method::GET, &url, None, "application/protobuf", None)
            .await?;
        let root = ProtoReader::parse(&body)?;

        let mut track_uris = Vec::new();
        let mut album_uris = Vec::new();
        let mut artist_uris = Vec::new();
        let mut raw_playlists = Vec::new();
        let mut fallback_tracks: HashMap<String, Track> = HashMap::new();
        let mut fallback_albums: HashMap<String, Card> = HashMap::new();
        let mut fallback_artists: HashMap<String, Card> = HashMap::new();

        for hit in root.messages(1) {
            let Some(uri) = hit.text(1).and_then(|u| canonical_uri(&u)) else {
                continue;
            };
            let title = hit.text(2).unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let thumbs = hit
                .text(3)
                .map(|s| thumbs_from_url(&s))
                .unwrap_or_default();

            if uri.starts_with("spotify:track:") {
                let tr = hit.message(5);
                if tr.as_ref().and_then(|t| t.number(14)).is_some_and(|k| (3..=4).contains(&k)) {
                    continue;
                }
                if !track_uris.contains(&uri) {
                    track_uris.push(uri.clone());
                    let artists = tr
                        .as_ref()
                        .map(|t| parse_artist_links(t, 4))
                        .unwrap_or_default();
                    let duration = tr
                        .as_ref()
                        .and_then(|t| t.message(18))
                        .and_then(|d| d.number(1))
                        .map(|s| s.max(1) as u32);
                    fallback_tracks.insert(
                        uri.clone(),
                        Track {
                            id: uri,
                            title,
                            artists,
                            album: None,
                            duration,
                            thumbs,
                            track_nr: None,
                            plays: None,
                            music_video: false,
                            explicit: tr.as_ref().is_some_and(|t| t.number(9) == Some(1)),
                        },
                    );
                }
            } else if uri.starts_with("spotify:album:") {
                if !album_uris.contains(&uri) {
                    album_uris.push(uri.clone());
                    let artist_names = hit
                        .message(6)
                        .map(|a| a.texts(1))
                        .unwrap_or_default();
                    let subtitle = artist_names.join(", ");
                    fallback_albums.insert(
                        uri.clone(),
                        Card {
                            title,
                            subtitle,
                            thumbs,
                            target: Target::Album(uri),
                            round: false,
                            links: Vec::new(),
                        },
                    );
                }
            } else if uri.starts_with("spotify:artist:") {
                if !artist_uris.contains(&uri) {
                    artist_uris.push(uri.clone());
                    fallback_artists.insert(
                        uri.clone(),
                        Card {
                            title,
                            subtitle: "Artist".to_string(),
                            thumbs,
                            target: Target::Artist(uri),
                            round: true,
                            links: Vec::new(),
                        },
                    );
                }
            } else if uri.starts_with("spotify:playlist:") {
                let owner = hit
                    .message(7)
                    .and_then(|p| p.text(5))
                    .unwrap_or_default();
                let subtitle = if owner.is_empty() {
                    "Spotify Playlist".to_string()
                } else {
                    owner
                };
                raw_playlists.push(Card {
                    title,
                    subtitle,
                    thumbs,
                    target: Target::Playlist(uri),
                    round: false,
                    links: Vec::new(),
                });
            }
        }

        let owner_ids: Vec<String> = raw_playlists
            .iter()
            .filter_map(|c| {
                let raw = c.subtitle.trim();
                (looks_like_spotify_user_id(raw) || raw.eq_ignore_ascii_case("spotify"))
                    .then(|| raw.to_owned())
            })
            .collect();

        let (tracks_res, albums_res, artists_res, ()) = tokio::join!(
            self.hydrate_tracks(&track_uris),
            self.hydrate_albums(&album_uris),
            self.hydrate_artists(&artist_uris),
            self.resolve_user_names(&owner_ids),
        );
        for card in &mut raw_playlists {
            if looks_like_spotify_user_id(&card.subtitle) || card.subtitle.eq_ignore_ascii_case("spotify") {
                let resolved = self.cached_owner_name(&card.subtitle).await;
                if !resolved.is_empty() {
                    card.subtitle = resolved;
                }
            }
        }

        let hydrated_tracks: HashMap<String, Track> = tracks_res
            .unwrap_or_default()
            .into_iter()
            .map(|t| (t.id.clone(), t))
            .collect();
        let hydrated_albums: HashMap<String, Card> = albums_res
            .unwrap_or_default()
            .into_iter()
            .filter_map(|c| match &c.target {
                Target::Album(id) => Some((id.clone(), c)),
                _ => None,
            })
            .collect();
        let hydrated_artists: HashMap<String, Card> = artists_res
            .unwrap_or_default()
            .into_iter()
            .filter_map(|c| match &c.target {
                Target::Artist(id) => Some((id.clone(), c)),
                _ => None,
            })
            .collect();

        let tracks = track_uris
            .into_iter()
            .filter_map(|u| hydrated_tracks.get(&u).cloned().or_else(|| fallback_tracks.remove(&u)))
            .collect();
        let albums = album_uris
            .into_iter()
            .filter_map(|u| hydrated_albums.get(&u).cloned().or_else(|| fallback_albums.remove(&u)))
            .collect();
        let artists = artist_uris
            .into_iter()
            .filter_map(|u| hydrated_artists.get(&u).cloned().or_else(|| fallback_artists.remove(&u)))
            .collect();

        Ok(SearchResults {
            query: q.to_string(),
            tracks,
            albums,
            artists,
            playlists: raw_playlists,
        })
    }

    pub async fn artist_page(&self, id: &str) -> Result<ArtistPage> {
        let uri = canonical_uri(id).ok_or_else(|| anyhow!("Invalid Spotify artist ID"))?;
        let base62 = uri_id(&uri).unwrap_or(id);

        let meta_queries = [
            (uri.as_str(), EXT_ARTIST_V4),
            (uri.as_str(), EXT_TOP_PLAYABLES),
            (uri.as_str(), EXT_POPULAR_RELEASES),
            (uri.as_str(), EXT_RELATED_CREATORS),
            (uri.as_str(), EXT_REPUTATION),
        ];
        let meta_fut = self.extended_metadata(&meta_queries);
        let about_url = format!(
            "{SPCLIENT_BASE}creatorabout/v0/artist/{base62}/about?fields=artist,listenerCount,monthlyListenerRank,isVerified"
        );
        let about_fut = self.sp_request(Method::GET, &about_url, None, "application/json", None);
        let ensure_coll_fut = self.ensure_collection_loaded();

        let (meta_res, about_res, _) = tokio::join!(meta_fut, about_fut, ensure_coll_fut);
        let meta = meta_res?;

        let artist_raw = meta
            .get(&EXT_ARTIST_V4)
            .and_then(|m| m.get(&uri))
            .ok_or_else(|| anyhow!("Spotify artist not found"))?;
        let artist_msg = ProtoReader::parse(artist_raw)?;

        let name = artist_msg.text(2).unwrap_or_else(|| "Artist".to_string());
        let mut photo = artist_thumbs(&artist_msg);
        let mut banner = Vec::new();
        let mut audience = meta
            .get(&EXT_REPUTATION)
            .and_then(|m| m.get(&uri))
            .and_then(|b| ProtoReader::parse(b).ok())
            .and_then(|r| r.long(1))
            .filter(|&n| n > 0)
            .map(|n| format!("{} monthly listeners", format_count(n as u64)));
        let mut description = artist_msg
            .messages(12)
            .into_iter()
            .find_map(|b| b.text(1))
            .map(|s| strip_html(&s))
            .filter(|s| !s.is_empty());

        if let Ok(about_bytes) = about_res {
            if let Ok(json) = serde_json::from_slice::<Value>(&about_bytes) {
                if let Some(count) = json["monthlyListeners"]
                    .as_u64()
                    .or_else(|| json["listenerCount"].as_u64())
                    .filter(|&n| n > 0)
                {
                    audience = Some(format!("{} monthly listeners", format_count(count)));
                }
                if description.is_none() {
                    description = json["biography"]
                        .as_str()
                        .or_else(|| json["artist"]["profile"]["biography"]["text"].as_str())
                        .map(strip_html)
                        .filter(|s| !s.is_empty());
                }
                if let Some(hdr_uri) = json["headerImage"]["uri"].as_str().filter(|s| !s.is_empty()) {
                    banner = thumbs_from_url(hdr_uri);
                } else if let Some(hdr) = json["artist"]["visuals"]["headerImage"]["sources"].as_array() {
                    banner = thumbs_from_json_sources(hdr);
                }
                if photo.is_empty() {
                    if let Some(av) = json["artist"]["visuals"]["avatarImage"]["sources"].as_array() {
                        photo = thumbs_from_json_sources(av);
                    }
                }
            }
        }
        if banner.is_empty() {
            banner = photo.clone();
        }

        let mut top_uris = Vec::new();
        for country_group in artist_msg.messages(4) {
            for tr in country_group.messages(2) {
                if let Some(u) = tr.bytes(1).and_then(|g| gid_uri("track", g)) {
                    if !top_uris.contains(&u) {
                        top_uris.push(u);
                    }
                }
            }
            if !top_uris.is_empty() {
                break;
            }
        }
        if top_uris.is_empty() {
            if let Some(top_raw) = meta.get(&EXT_TOP_PLAYABLES).and_then(|m| m.get(&uri)) {
                if let Ok(top_msg) = ProtoReader::parse(top_raw) {
                    for sec in top_msg.messages(1) {
                        for item in sec.messages(1) {
                            if let Some(u) = item.text(1).and_then(|s| canonical_uri(&s)) {
                                if u.starts_with("spotify:track:") && !top_uris.contains(&u) {
                                    top_uris.push(u);
                                }
                            }
                        }
                    }
                }
            }
        }
        top_uris.truncate(20);

        let mut album_uris = Vec::new();
        if let Some(rel_raw) = meta.get(&EXT_POPULAR_RELEASES).and_then(|m| m.get(&uri)) {
            if let Ok(rel_msg) = ProtoReader::parse(rel_raw) {
                for item in rel_msg.messages(1) {
                    if let Some(u) = item.text(1).and_then(|s| canonical_uri(&s)) {
                        if u.starts_with("spotify:album:") && !album_uris.contains(&u) {
                            album_uris.push(u);
                        }
                    }
                }
            }
        }
        for field in [5u32, 6, 7] {
            for group in artist_msg.messages(field) {
                for alb in group.messages(1) {
                    if let Some(u) = alb.bytes(1).and_then(|g| gid_uri("album", g)) {
                        if !album_uris.contains(&u) {
                            album_uris.push(u);
                        }
                    }
                }
            }
        }
        album_uris.truncate(36);

        let mut sim_uris = Vec::new();
        for rel in artist_msg.messages(15) {
            if let Some(u) = rel.bytes(1).and_then(|g| gid_uri("artist", g)) {
                if !sim_uris.contains(&u) {
                    sim_uris.push(u);
                }
            }
        }
        if sim_uris.is_empty() {
            if let Some(sim_raw) = meta.get(&EXT_RELATED_CREATORS).and_then(|m| m.get(&uri)) {
                if let Ok(sim_msg) = ProtoReader::parse(sim_raw) {
                    for sec in sim_msg.messages(1) {
                        for item in sec.messages(1) {
                            if let Some(u) = item.text(1).and_then(|s| canonical_uri(&s)) {
                                if u.starts_with("spotify:artist:") && !sim_uris.contains(&u) {
                                    sim_uris.push(u);
                                }
                            }
                        }
                    }
                }
            }
        }
        sim_uris.truncate(20);

        let (top_res, albums_res, similar_res) = tokio::join!(
            self.hydrate_tracks(&top_uris),
            self.hydrate_albums_sorted(&album_uris),
            self.hydrate_artists(&sim_uris),
        );

        let subscribed = self.followed_artists.read().await.contains(&uri);

        Ok(ArtistPage {
            id: uri.clone(),
            name,
            banner,
            audience,
            subscribers: None,
            views: None,
            description,
            top: top_res.unwrap_or_default(),
            top_playlist: None,
            albums: albums_res.unwrap_or_default(),
            playlists: Vec::new(),
            similar: similar_res.unwrap_or_default(),
            photo,
            following: Some(ArtistSub {
                channel: uri,
                subscribed,
                params: None,
            }),
        })
    }

    pub async fn album_page(&self, id: &str) -> Result<Collection> {
        let uri = canonical_uri(id).ok_or_else(|| anyhow!("Invalid Spotify album ID"))?;
        let meta = self.extended_metadata(&[(&uri, EXT_ALBUM_V4)]).await?;
        let raw = meta
            .get(&EXT_ALBUM_V4)
            .and_then(|m| m.get(&uri))
            .ok_or_else(|| anyhow!("Spotify album not found"))?;
        let alb = ProtoReader::parse(raw)?;

        let title = alb.text(2).unwrap_or_else(|| "Album".to_string());
        let artists = parse_artist_links(&alb, 3);
        let kind = match alb.number(4) {
            Some(2) => "Single",
            Some(3) => "Compilation",
            Some(4) => "EP",
            _ => "Album",
        };
        let year = alb
            .message(6)
            .and_then(|d| d.number(1))
            .map(unzigzag)
            .filter(|&y| y > 0);
        let subtitle = match year {
            Some(y) => format!("{kind} • {y}"),
            None => kind.to_string(),
        };
        let thumbs = album_thumbs(&alb);

        let mut discs: Vec<(i32, Vec<String>)> = Vec::new();
        for (idx, disc) in alb.messages(11).into_iter().enumerate() {
            let dnum = disc.number(1).map(unzigzag).unwrap_or(idx as i32 + 1);
            let mut uris = Vec::new();
            for tr in disc.messages(3) {
                if let Some(u) = tr.bytes(1).and_then(|g| gid_uri("track", g)) {
                    uris.push(u);
                }
            }
            discs.push((dnum, uris));
        }
        discs.sort_by_key(|(dnum, _)| *dnum);
        let track_uris: Vec<String> = discs.into_iter().flat_map(|(_, u)| u).collect();

        let mut tracks = self.hydrate_tracks(&track_uris).await?;
        for tr in &mut tracks {
            if tr.thumbs.is_empty() {
                tr.thumbs = thumbs.clone();
            }
        }
        let count = Some(tracks.len() as u64);

        Ok(Collection {
            id: uri.clone(),
            is_album: true,
            save_id: Some(uri),
            title,
            subtitle,
            artists,
            description: None,
            thumbs,
            tracks,
            set_ids: Vec::new(),
            count,
            owned: false,
        })
    }

    pub async fn playlist_page(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(Collection, bool)> {
        if id == SPOTIFY_LIKED_ID {
            let items = self.collection_page_all("collection").await?;
            let uris: Vec<String> = items
                .into_iter()
                .filter(|u| u.starts_with("spotify:track:"))
                .collect();
            {
                let mut set = self.liked_tracks.write().await;
                set.clear();
                set.extend(uris.iter().cloned());
                *self.liked_order.write().await = uris.clone();
                *self.collection_loaded.write().await = true;
            }
            let total = uris.len();
            let end = (offset + limit).min(total);
            let slice = if offset < total { &uris[offset..end] } else { &[] };
            let tracks = self.hydrate_tracks(slice).await?;
            let set_ids = vec![None; tracks.len()];
            let more = end < total;
            return Ok((
                Collection {
                    id: SPOTIFY_LIKED_ID.to_string(),
                    is_album: false,
                    save_id: None,
                    title: "Liked Songs".to_string(),
                    subtitle: "Spotify".to_string(),
                    artists: Vec::new(),
                    description: Some("Songs you saved on Spotify".to_string()),
                    thumbs: vec![Thumb {
                        url: LIKED_SONGS_COVER.to_string(),
                        width: 640,
                        height: 640,
                    }],
                    tracks,
                    set_ids,
                    count: Some(total as u64),
                    owned: false,
                },
                more,
            ));
        }

        let uri = canonical_uri(id).ok_or_else(|| anyhow!("Invalid Spotify playlist ID"))?;
        let base62 = uri_id(&uri).ok_or_else(|| anyhow!("Invalid Spotify playlist ID"))?;
        let url = format!(
            "{SPCLIENT_BASE}playlist/v2/playlist/{base62}?decorate=revision,attributes,length,owner,capabilities,status_code&from={offset}&length={limit}"
        );
        let bytes = self
            .sp_request(
                Method::GET,
                &url,
                None,
                "application/protobuf",
                Some(&[("x-accept-list-items", "audio-track")]),
            )
            .await?;
        let slc = ProtoReader::parse(&bytes)?;
        let total_length = slc.number(2).unwrap_or(0).max(0) as usize;
        let attrs = slc.message(3);
        let title = attrs
            .as_ref()
            .and_then(|a| a.text(1))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Playlist".to_string());
        let description = attrs
            .as_ref()
            .and_then(|a| a.text(2))
            .map(|s| strip_html(&s))
            .filter(|s| !s.is_empty());
        let mut thumbs = attrs.as_ref().map(playlist_thumbs).unwrap_or_default();
        let owner_id = slc.text(11).unwrap_or_default();
        let owner_text = slc.text(16).unwrap_or_default();

        let mut item_uris = Vec::new();
        let mut raw_count = 0usize;
        let mut truncated = false;
        let mut pos = 0usize;
        if let Some(contents) = slc.message(5) {
            pos = contents.number(1).unwrap_or(0).max(0) as usize;
            truncated = contents.number(2) == Some(1);
            for item in contents.messages(3) {
                raw_count += 1;
                if let Some(u) = item.text(1).and_then(|s| canonical_uri(&s)) {
                    if u.starts_with("spotify:track:") {
                        item_uris.push(u);
                    }
                }
            }
        }
        let (tracks_res, owner) = tokio::join!(
            self.hydrate_tracks(&item_uris),
            self.resolve_owner_label(
                (!owner_id.is_empty()).then_some(owner_id.as_str()),
                (!owner_text.is_empty()).then_some(owner_text.as_str()),
            ),
        );
        let tracks = tracks_res?;
        if thumbs.is_empty() {
            thumbs = mosaic_from_tracks(&tracks);
        }
        if !thumbs.is_empty() {
            self.playlist_cover_cache
                .write()
                .await
                .insert(uri.clone(), thumbs.clone());
        }
        let set_ids = vec![None; tracks.len()];
        let me = self.signed_in_username().await.unwrap_or_default();
        let owned = !me.is_empty()
            && ((!owner_id.is_empty() && me.eq_ignore_ascii_case(&owner_id))
                || (!owner_text.is_empty() && me.eq_ignore_ascii_case(&owner_text)));
        let more = raw_count > 0 && (truncated || pos + raw_count < total_length);

        Ok((
            Collection {
                id: uri,
                is_album: false,
                save_id: None,
                title,
                subtitle: if owner.is_empty() {
                    "Spotify Playlist".to_string()
                } else {
                    owner
                },
                artists: Vec::new(),
                description,
                thumbs,
                tracks,
                set_ids,
                count: Some(total_length as u64),
                owned,
            },
            more,
        ))
    }

    pub async fn playlist_more(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<Track>, Vec<Option<String>>, bool)> {
        let (col, more) = self.playlist_page(id, offset, limit).await?;
        Ok((col.tracks, col.set_ids, more))
    }

    pub async fn library(&self) -> Result<(Library, Vec<Card>)> {
        let username = self
            .signed_in_username()
            .await
            .ok_or_else(|| anyhow!("Not signed in to Spotify"))?;

        let (collection_res, artist_res, rootlist_res) = tokio::join!(
            self.collection_page_all("collection"),
            self.collection_page_all("artist"),
            self.rootlist(&username),
        );

        let albums_failed = collection_res.is_err();
        let mut track_uris = Vec::new();
        let mut album_uris = Vec::new();
        match collection_res {
            Ok(all) => {
                for u in all {
                    if u.starts_with("spotify:track:") {
                        track_uris.push(u);
                    } else if u.starts_with("spotify:album:") {
                        album_uris.push(u);
                    }
                }
                {
                    let mut w = self.liked_tracks.write().await;
                    w.clear();
                    w.extend(track_uris.iter().cloned());
                    *self.liked_order.write().await = track_uris.clone();
                    let mut wa = self.saved_albums.write().await;
                    wa.clear();
                    wa.extend(album_uris.iter().cloned());
                    *self.collection_loaded.write().await = true;
                }
            }
            Err(ref e) => log::warn!("spotify collection/v2 paging failed: {e:#}"),
        }

        let artist_uris = match artist_res {
            Ok(list) => {
                let mut w = self.followed_artists.write().await;
                w.clear();
                w.extend(list.iter().cloned());
                list
            }
            Err(ref e) => {
                log::warn!("spotify followed artists failed: {e:#}");
                Vec::new()
            }
        };

        let playlists_failed = rootlist_res.is_err();
        let mut playlists = match rootlist_res {
            Ok(list) => list,
            Err(ref e) => {
                log::warn!("spotify rootlist failed: {e:#}");
                Vec::new()
            }
        };

        self.fill_missing_playlist_covers(&mut playlists).await;

        let (albums_res, artists_res) = tokio::join!(
            self.hydrate_albums(&album_uris),
            self.hydrate_artists(&artist_uris),
        );

        let mut all_playlists = Vec::with_capacity(playlists.len() + 1);
        all_playlists.push(Card {
            title: "Liked Songs".to_string(),
            subtitle: format!("{} songs", track_uris.len()),
            thumbs: vec![Thumb {
                url: LIKED_SONGS_COVER.to_string(),
                width: 640,
                height: 640,
            }],
            target: Target::Playlist(SPOTIFY_LIKED_ID.to_string()),
            round: false,
            links: Vec::new(),
        });
        all_playlists.append(&mut playlists);

        let albums = albums_res.unwrap_or_default();
        let artists = artists_res.unwrap_or_default();
        log::info!(
            "spotify library loaded: {} liked songs, {} playlists, {} albums, {} artists",
            track_uris.len(),
            all_playlists.len().saturating_sub(1),
            albums.len(),
            artists.len()
        );

        Ok((
            Library {
                playlists: all_playlists,
                albums,
                playlists_failed,
                albums_failed,
            },
            artists,
        ))
    }

    async fn fill_missing_playlist_covers(&self, playlists: &mut [Card]) {
        let mut need = Vec::new();
        {
            let cached = self.playlist_cover_cache.read().await;
            for (i, c) in playlists.iter_mut().enumerate() {
                if c.thumbs.is_empty() {
                    if let Target::Playlist(ref uri) = c.target {
                        if let Some(t) = cached.get(uri) {
                            c.thumbs = t.clone();
                        } else if need.len() < 16 {
                            need.push((i, uri.clone()));
                        }
                    }
                }
            }
        }
        if need.is_empty() {
            return;
        }
        let futs = need.iter().map(|&(idx, ref uri)| async move {
            let thumbs = match self.playlist_page(uri, 0, 8).await {
                Ok((col, _)) => col.thumbs,
                Err(_) => Vec::new(),
            };
            (idx, thumbs)
        });
        for (idx, thumbs) in join_all(futs).await {
            if !thumbs.is_empty() {
                playlists[idx].thumbs = thumbs;
            }
        }
    }

    async fn rootlist(&self, username: &str) -> Result<Vec<Card>> {
        let mut raw_cards: Vec<(String, String, i32, String, Vec<Thumb>)> = Vec::new();
        let mut seen = HashSet::new();
        let mut from = 0usize;
        let enc_user: String = url::form_urlencoded::byte_serialize(username.as_bytes()).collect();
        for _ in 0..10 {
            let url = format!(
                "{SPCLIENT_BASE}playlist/v2/user/{enc_user}/rootlist?decorate=revision,attributes,length,owner,capabilities,status_code&from={from}&length=300"
            );
            let bytes = match self
                .sp_request(
                    Method::GET,
                    &url,
                    None,
                    "application/protobuf",
                    Some(&[("x-accept-list-items", "audio-track")]),
                )
                .await
            {
                Ok(b) => b,
                Err(e) if from == 0 && self.auth.is_web().await => {
                    log::info!("spotify rootlist spclient fallback to Web API: {e:#}");
                    return self.web_playlists().await;
                }
                Err(e) => return Err(e),
            };
            let slc = ProtoReader::parse(&bytes)?;
            let total_length = slc.number(2).unwrap_or(0).max(0) as usize;
            let Some(contents) = slc.message(5) else {
                break;
            };
            let items = contents.messages(3);
            let metas = contents.messages(4);
            let pos = contents.number(1).unwrap_or(from as i32).max(0) as usize;
            let truncated = contents.number(2) == Some(1);
            let raw_count = items.len();
            for (idx, item) in items.iter().enumerate() {
                let Some(u) = item.text(1).and_then(|s| canonical_uri(&s)) else {
                    continue;
                };
                if !u.starts_with("spotify:playlist:") || !seen.insert(u.clone()) {
                    continue;
                }
                let info = metas.get(idx);
                let status = info.and_then(|m| m.number(9)).map(unzigzag).unwrap_or(0);
                if (400..=599).contains(&status) {
                    continue;
                }
                let attrs = info.and_then(|m| m.message(2));
                let title = attrs
                    .as_ref()
                    .and_then(|a| a.text(1))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Playlist".to_string());
                let count = info.and_then(|m| m.number(3)).unwrap_or(0).max(0);
                let owner = info.and_then(|m| m.text(5)).unwrap_or_default();
                let thumbs = attrs.as_ref().map(playlist_thumbs).unwrap_or_default();
                raw_cards.push((u, title, count, owner, thumbs));
            }
            from += raw_count;
            if raw_count == 0 || (!truncated && pos + raw_count >= total_length) {
                break;
            }
        }
        let owner_ids: Vec<String> = raw_cards.iter().map(|(_, _, _, o, _)| o.clone()).collect();
        self.resolve_user_names(&owner_ids).await;
        let mut cards = Vec::with_capacity(raw_cards.len());
        for (u, title, count, raw_owner, thumbs) in raw_cards {
            let owner = self.cached_owner_name(&raw_owner).await;
            let subtitle = if owner.is_empty() {
                format!("{count} songs")
            } else {
                format!("{owner} • {count} songs")
            };
            cards.push(Card {
                title,
                subtitle,
                thumbs,
                target: Target::Playlist(u),
                round: false,
                links: Vec::new(),
            });
        }
        Ok(cards)
    }

    pub async fn refresh_liked_tracks(&self) -> Result<()> {
        let uris = self.collection_page_all("collection").await?;
        let tracks: Vec<String> = uris
            .into_iter()
            .filter(|u| u.starts_with("spotify:track:"))
            .collect();
        let mut w = self.liked_tracks.write().await;
        w.clear();
        w.extend(tracks.iter().cloned());
        *self.liked_order.write().await = tracks;
        *self.collection_loaded.write().await = true;
        Ok(())
    }

    async fn ensure_collection_loaded(&self) {
        if *self.collection_loaded.read().await || !self.is_signed_in().await {
            return;
        }
        let _ = self.library().await;
    }

    pub async fn set_saved(&self, id: &str, save: bool) -> Result<()> {
        let uri = canonical_uri(id).ok_or_else(|| anyhow!("Invalid Spotify URI"))?;
        let set_name = if uri.starts_with("spotify:track:") || uri.starts_with("spotify:album:") {
            "collection"
        } else if uri.starts_with("spotify:artist:") {
            "artist"
        } else {
            bail!("Unsupported Spotify collection target: {uri}");
        };
        let username = self
            .signed_in_username()
            .await
            .ok_or_else(|| anyhow!("Not signed in to Spotify"))?;
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let req_id = random_uuid_v4();
        let payload = encode(|w| {
            w.write_str(1, &username);
            w.write_str(2, set_name);
            w.write_msg(3, |item| {
                item.write_str(1, &uri);
                item.write_int64(2, if save { now_secs } else { 0 });
                item.write_bool(3, !save);
            });
            w.write_str(4, &req_id);
        });
        let url = format!("{SPCLIENT_BASE}collection/v2/write");
        let _ = self
            .sp_request(
                Method::POST,
                &url,
                Some((payload, COLL_MIME)),
                COLL_MIME,
                None,
            )
            .await?;

        let cache = if uri.starts_with("spotify:track:") {
            &self.liked_tracks
        } else if uri.starts_with("spotify:album:") {
            &self.saved_albums
        } else {
            &self.followed_artists
        };
        let mut w = cache.write().await;
        if save {
            w.insert(uri);
        } else {
            w.remove(&uri);
        }
        Ok(())
    }

    pub async fn radio(&self, seed_id: &str) -> Result<Vec<Track>> {
        let seed_uri = canonical_uri(seed_id).ok_or_else(|| anyhow!("Invalid Spotify seed URI"))?;
        let encoded = url::form_urlencoded::byte_serialize(seed_uri.as_bytes()).collect::<String>();
        let url = format!("{SPCLIENT_BASE}inspiredby-mix/v2/seed_to_playlist/{encoded}?response-format=json");
        let bytes = self
            .sp_request(Method::GET, &url, None, "application/json", None)
            .await?;
        let json: Value = serde_json::from_slice(&bytes)?;
        let mut uris = Vec::new();
        for key in ["mediaItems", "tracks"] {
            if let Some(arr) = json[key].as_array() {
                for item in arr {
                    if let Some(u) = item["uri"].as_str().and_then(canonical_uri) {
                        if u.starts_with("spotify:track:") && !uris.contains(&u) {
                            uris.push(u);
                        }
                    }
                }
            }
        }
        uris.truncate(50);
        self.hydrate_tracks(&uris).await
    }

    pub async fn lyrics(&self, track_id: &str) -> Result<Option<(Vec<Line>, Option<String>)>> {
        if !self.is_signed_in().await {
            return Ok(None);
        }
        let uri = canonical_uri(track_id).ok_or_else(|| anyhow!("Invalid Spotify track ID"))?;
        let base62 = uri_id(&uri).ok_or_else(|| anyhow!("Invalid Spotify track ID"))?;
        let url = format!("{SPCLIENT_BASE}color-lyrics/v2/track/{base62}?format=json&vocalRemoval=false&market=from_token");
        let bytes = self
            .sp_request(Method::GET, &url, None, "application/json", None)
            .await?;
        let json: Value = serde_json::from_slice(&bytes)?;
        let lyrics = &json["lyrics"];
        let sync_type = lyrics["syncType"].as_str().unwrap_or("");
        let unsynced = sync_type == "UNSYNCED";
        let lang = lyrics["language"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let Some(raw_lines) = lyrics["lines"].as_array() else {
            return Ok(None);
        };
        let mut out: Vec<Line> = Vec::with_capacity(raw_lines.len());
        for row in raw_lines {
            let text = row["words"].as_str().unwrap_or("").trim().to_string();
            let start = if unsynced {
                0
            } else {
                row["startTimeMs"]
                    .as_str()
                    .and_then(|s| s.parse::<i32>().ok())
                    .or_else(|| row["startTimeMs"].as_i64().map(|n| n as i32))
                    .unwrap_or(0)
            };
            let mut parts = Vec::new();
            if !unsynced {
                if let Some(syllables) = row["syllables"].as_array() {
                    for s in syllables {
                        let part_text = s["chars"].as_str().unwrap_or("").to_string();
                        let s_start = s["startTimeMs"]
                            .as_str()
                            .and_then(|v| v.parse::<i32>().ok())
                            .or_else(|| s["startTimeMs"].as_i64().map(|n| n as i32))
                            .unwrap_or(start);
                        let s_end = s["endTimeMs"]
                            .as_str()
                            .and_then(|v| v.parse::<i32>().ok())
                            .or_else(|| s["endTimeMs"].as_i64().map(|n| n as i32))
                            .unwrap_or(s_start);
                        if !part_text.is_empty() {
                            parts.push(Part {
                                start: s_start,
                                dur: (s_end - s_start).max(0),
                                text: part_text,
                                bg: false,
                            });
                        }
                    }
                }
            }
            out.push(Line {
                start,
                dur: 0,
                text,
                parts,
                agent: None,
                instrumental: false,
                translations: Vec::new(),
                romanization: None,
                timed_roman: Vec::new(),
            });
        }
        for i in 0..out.len() {
            if !unsynced {
                let next_start = out.get(i + 1).map(|l| l.start).unwrap_or(out[i].start + 4000);
                out[i].dur = (next_start - out[i].start).max(0);
            }
        }
        out.retain(|l| !l.text.is_empty());
        if out.is_empty() {
            Ok(None)
        } else {
            Ok(Some((out, lang)))
        }
    }

    pub async fn hydrate_tracks(&self, uris: &[String]) -> Result<Vec<Track>> {
        if uris.is_empty() {
            return Ok(Vec::new());
        }
        let mut by_uri: HashMap<String, Track> = HashMap::new();
        for chunk in uris.chunks(40) {
            let queries: Vec<(&str, i32)> = chunk.iter().map(|u| (u.as_str(), EXT_TRACK_V4)).collect();
            let meta = self.extended_metadata(&queries).await?;
            if let Some(map) = meta.get(&EXT_TRACK_V4) {
                for (uri, raw) in map {
                    if let Ok(tr) = parse_track(uri, raw) {
                        by_uri.insert(uri.clone(), tr);
                    }
                }
            }
        }
        Ok(uris.iter().filter_map(|u| by_uri.remove(u)).collect())
    }

    async fn hydrate_albums(&self, uris: &[String]) -> Result<Vec<Card>> {
        if uris.is_empty() {
            return Ok(Vec::new());
        }
        let mut by_uri: HashMap<String, Card> = HashMap::new();
        for chunk in uris.chunks(40) {
            let queries: Vec<(&str, i32)> = chunk.iter().map(|u| (u.as_str(), EXT_ALBUM_V4)).collect();
            let meta = self.extended_metadata(&queries).await?;
            if let Some(map) = meta.get(&EXT_ALBUM_V4) {
                for (uri, raw) in map {
                    if let Ok((card, _)) = parse_album_card_with_year(uri, raw) {
                        by_uri.insert(uri.clone(), card);
                    }
                }
            }
        }
        Ok(uris.iter().filter_map(|u| by_uri.remove(u)).collect())
    }

    async fn hydrate_albums_sorted(&self, uris: &[String]) -> Result<Vec<Card>> {
        if uris.is_empty() {
            return Ok(Vec::new());
        }
        let mut by_uri: HashMap<String, (Card, i32)> = HashMap::new();
        for chunk in uris.chunks(40) {
            let queries: Vec<(&str, i32)> = chunk.iter().map(|u| (u.as_str(), EXT_ALBUM_V4)).collect();
            let meta = self.extended_metadata(&queries).await?;
            if let Some(map) = meta.get(&EXT_ALBUM_V4) {
                for (uri, raw) in map {
                    if let Ok(pair) = parse_album_card_with_year(uri, raw) {
                        by_uri.insert(uri.clone(), pair);
                    }
                }
            }
        }
        let mut list: Vec<(Card, i32)> = uris.iter().filter_map(|u| by_uri.remove(u)).collect();
        list.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(list.into_iter().map(|(c, _)| c).collect())
    }

    async fn hydrate_artists(&self, uris: &[String]) -> Result<Vec<Card>> {
        if uris.is_empty() {
            return Ok(Vec::new());
        }
        let mut by_uri: HashMap<String, Card> = HashMap::new();
        for chunk in uris.chunks(40) {
            let queries: Vec<(&str, i32)> = chunk.iter().map(|u| (u.as_str(), EXT_ARTIST_V4)).collect();
            let meta = self.extended_metadata(&queries).await?;
            if let Some(map) = meta.get(&EXT_ARTIST_V4) {
                for (uri, raw) in map {
                    if let Ok(card) = parse_artist_card(uri, raw) {
                        by_uri.insert(uri.clone(), card);
                    }
                }
            }
        }
        Ok(uris.iter().filter_map(|u| by_uri.remove(u)).collect())
    }

    async fn extended_metadata(
        &self,
        queries: &[(&str, i32)],
    ) -> Result<HashMap<i32, HashMap<String, Vec<u8>>>> {
        if queries.is_empty() {
            return Ok(HashMap::new());
        }
        let mut by_uri_kinds: Vec<(&str, Vec<i32>)> = Vec::new();
        for &(uri, kind) in queries {
            if let Some(entry) = by_uri_kinds.iter_mut().find(|(u, _)| *u == uri) {
                if !entry.1.contains(&kind) {
                    entry.1.push(kind);
                }
            } else {
                by_uri_kinds.push((uri, vec![kind]));
            }
        }
        let payload = encode(|w| {
            for (uri, kinds) in &by_uri_kinds {
                w.write_msg(2, |er| {
                    er.write_str(1, uri);
                    for &kind in kinds {
                        er.write_msg(2, |q| {
                            q.write_enum(1, kind);
                        });
                    }
                });
            }
        });
        let url = format!("{SPCLIENT_BASE}extended-metadata/v0/extended-metadata");
        let bytes = self
            .sp_request(
                Method::POST,
                &url,
                Some((payload, "application/x-protobuf")),
                "application/x-protobuf",
                None,
            )
            .await?;
        let resp = ProtoReader::parse(&bytes)?;
        let mut out: HashMap<i32, HashMap<String, Vec<u8>>> = HashMap::new();
        for ext_group in resp.messages(2) {
            let group_status = ext_group.message(1).and_then(|h| h.number(1)).unwrap_or(0);
            if group_status != 0 && group_status != 200 {
                continue;
            }
            let kind = ext_group.number(2).unwrap_or(0);
            let entry = out.entry(kind).or_default();
            for data in ext_group.messages(3) {
                let status = data
                    .message(1)
                    .and_then(|h| h.number(1))
                    .unwrap_or(0);
                if status != 0 && status != 200 {
                    continue;
                }
                if let Some(uri) = data.text(2).and_then(|s| canonical_uri(&s)) {
                    if let Some(any) = data.message(3) {
                        if let Some(val) = any.bytes(2) {
                            entry.insert(uri, val.to_vec());
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    async fn collection_page_all(&self, set_name: &str) -> Result<Vec<String>> {
        let username = self
            .signed_in_username()
            .await
            .ok_or_else(|| anyhow!("Not signed in to Spotify"))?;
        let mut records: HashMap<String, i32> = HashMap::new();
        let mut token = String::new();
        let mut seen = HashSet::new();
        for _ in 0..20 {
            if !seen.insert(token.clone()) {
                break;
            }
            let payload = encode(|w| {
                w.write_str(1, &username);
                w.write_str(2, set_name);
                w.write_str(3, &token);
                w.write_int32(4, 300);
            });
            let url = format!("{SPCLIENT_BASE}collection/v2/paging");
            let bytes = match self
                .sp_request(
                    Method::POST,
                    &url,
                    Some((payload, COLL_MIME)),
                    COLL_MIME,
                    None,
                )
                .await
            {
                Ok(b) => b,
                Err(e) if token.is_empty() && self.auth.is_web().await => {
                    log::info!("spotify collection {set_name} spclient fallback to Web API: {e:#}");
                    return self.web_collection(set_name).await;
                }
                Err(e) => return Err(e),
            };
            let resp = ProtoReader::parse(&bytes)?;
            for item in resp.messages(1) {
                let Some(u) = item.text(1).and_then(|s| canonical_uri(&s)) else {
                    continue;
                };
                if item.number(3) == Some(1) {
                    records.remove(&u);
                } else {
                    records.insert(u, item.number(2).unwrap_or(0));
                }
            }
            let next = resp.text(2).unwrap_or_default();
            if next.is_empty() {
                break;
            }
            token = next;
        }
        let mut sorted: Vec<(String, i32)> = records.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(sorted.into_iter().map(|(u, _)| u).collect())
    }

    async fn web_get_json(&self, url: &str) -> Result<Value> {
        let http = proxy::http();
        let tok = self.auth.tokens(&http).await?;
        Ok(http
            .get(url)
            .bearer_auth(&tok.access_token)
            .header("User-Agent", &tok.user_agent)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn web_playlists(&self) -> Result<Vec<Card>> {
        let mut raw_cards: Vec<(String, String, String, String, u64, Vec<Thumb>)> = Vec::new();
        let mut url = Some("https://api.spotify.com/v1/me/playlists?limit=50".to_string());
        for _ in 0..10 {
            let Some(u) = url.take() else { break };
            let page = self.web_get_json(&u).await?;
            for p in page["items"].as_array().into_iter().flatten() {
                let (Some(id), Some(title)) = (p["id"].as_str(), p["name"].as_str()) else {
                    continue;
                };
                let display = p["owner"]["display_name"].as_str().unwrap_or_default().trim().to_string();
                let owner_id = p["owner"]["id"].as_str().unwrap_or_default().trim().to_string();
                let count = p["tracks"]["total"].as_u64().or(p["items"]["total"].as_u64()).unwrap_or(0);
                let thumbs = p["images"]
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|img| img["url"].as_str())
                    .map(thumbs_from_url)
                    .unwrap_or_default();
                raw_cards.push((id.to_string(), title.to_string(), display, owner_id, count, thumbs));
            }
            url = page["next"].as_str().map(str::to_owned);
        }
        let need_resolve: Vec<String> = raw_cards
            .iter()
            .filter_map(|(_, _, display, owner_id, _, _)| {
                let needs = display.is_empty() || looks_like_spotify_user_id(display);
                needs.then(|| {
                    if !owner_id.is_empty() { owner_id.clone() } else { display.clone() }
                })
            })
            .filter(|s| !s.is_empty())
            .collect();
        self.resolve_user_names(&need_resolve).await;
        let mut cards = Vec::with_capacity(raw_cards.len());
        for (id, title, display, owner_id, count, thumbs) in raw_cards {
            let owner = if !display.is_empty() && !looks_like_spotify_user_id(&display) {
                display
            } else {
                let lookup = if !owner_id.is_empty() { owner_id } else { display };
                self.cached_owner_name(&lookup).await
            };
            let subtitle = if owner.is_empty() {
                format!("{count} songs")
            } else {
                format!("{owner} • {count} songs")
            };
            cards.push(Card {
                title,
                subtitle,
                thumbs,
                target: Target::Playlist(format!("spotify:playlist:{id}")),
                round: false,
                links: Vec::new(),
            });
        }
        Ok(cards)
    }

    async fn web_collection(&self, set_name: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        match set_name {
            "collection" => {
                let mut url = Some("https://api.spotify.com/v1/me/tracks?limit=50".to_string());
                for _ in 0..20 {
                    let Some(u) = url.take() else { break };
                    let page = self.web_get_json(&u).await?;
                    for it in page["items"].as_array().into_iter().flatten() {
                        if let Some(id) = it["track"]["id"].as_str() {
                            out.push(format!("spotify:track:{id}"));
                        }
                    }
                    url = page["next"].as_str().map(str::to_owned);
                }
                let mut url = Some("https://api.spotify.com/v1/me/albums?limit=50".to_string());
                for _ in 0..10 {
                    let Some(u) = url.take() else { break };
                    let page = self.web_get_json(&u).await?;
                    for it in page["items"].as_array().into_iter().flatten() {
                        if let Some(id) = it["album"]["id"].as_str() {
                            out.push(format!("spotify:album:{id}"));
                        }
                    }
                    url = page["next"].as_str().map(str::to_owned);
                }
            }
            "artist" => {
                let mut url = Some("https://api.spotify.com/v1/me/following?type=artist&limit=50".to_string());
                for _ in 0..10 {
                    let Some(u) = url.take() else { break };
                    let page = self.web_get_json(&u).await?;
                    let artists = &page["artists"];
                    for it in artists["items"].as_array().into_iter().flatten() {
                        if let Some(id) = it["id"].as_str() {
                            out.push(format!("spotify:artist:{id}"));
                        }
                    }
                    url = artists["next"].as_str().map(str::to_owned);
                }
            }
            _ => {}
        }
        Ok(out)
    }

    async fn sp_request(
        &self,
        method: Method,
        url: &str,
        body: Option<(Vec<u8>, &str)>,
        accept: &str,
        extra_headers: Option<&[(&str, &str)]>,
    ) -> Result<Vec<u8>> {
        let http = proxy::http();
        for attempt in 0..2 {
            let tok = self.auth.tokens(&http).await?;
            let platform = if tok.is_web { "WebPlayer" } else { "Android" };
            let mut req = http
                .request(method.clone(), url)
                .timeout(std::time::Duration::from_secs(15))
                .header("Authorization", format!("Bearer {}", tok.access_token))
                .header("client-token", &tok.client_token)
                .header("Spotify-App-Version", &tok.app_version)
                .header("X-Client-Id", &tok.client_id)
                .header("App-Platform", platform)
                .header("User-Agent", &tok.user_agent)
                .header("Accept-Language", "en")
                .header("Accept", accept);
            if let Some(headers) = extra_headers {
                for &(k, v) in headers {
                    req = req.header(k, v);
                }
            }
            if let Some((ref bytes, ctype)) = body {
                req = req.header("Content-Type", ctype).body(bytes.clone());
            }
            let resp = req
                .send()
                .await
                .with_context(|| format!("Spotify request failed: {url}"))?;
            let status = resp.status();
            let raw = resp.bytes().await?.to_vec();
            if status.as_u16() == 401 && attempt == 0 {
                self.auth.invalidate_access_token().await;
                continue;
            }
            if status.as_u16() == 403 && attempt == 0 {
                self.auth.invalidate_client_token().await;
                continue;
            }
            if !status.is_success() {
                bail!("Spotify HTTP {status}");
            }
            return Ok(raw);
        }
        bail!("Spotify authentication retry exhausted")
    }

    async fn fetch_user_display_name(&self, username: &str, is_self: bool) -> Option<String> {
        let clean = normalize_owner_id(username);
        if clean.is_empty() {
            return None;
        }
        if clean.eq_ignore_ascii_case("spotify") {
            return Some("Spotify".to_string());
        }
        let enc: String = url::form_urlencoded::byte_serialize(clean.as_bytes()).collect();
        let sp_url = format!("{SPCLIENT_BASE}user-profile-view/v3/profile/{enc}?market=from_token");
        if let Ok(bytes) = self.sp_request(Method::GET, &sp_url, None, "application/json", None).await
            && let Ok(val) = serde_json::from_slice::<Value>(&bytes)
            && let Some(name) = parse_profile_display_name(&val)
        {
            return Some(name);
        }
        let web_url = if is_self {
            "https://api.spotify.com/v1/me".to_string()
        } else {
            format!("https://api.spotify.com/v1/users/{enc}")
        };
        if let Ok(val) = self.web_get_json(&web_url).await {
            return parse_profile_display_name(&val);
        }
        None
    }

    async fn resolve_user_names(&self, usernames: &[String]) {
        let mut to_fetch = Vec::new();
        let mut seen = HashSet::new();
        let mut seeds = Vec::new();
        {
            let cache = self.user_names.read().await;
            for raw in usernames {
                let id = normalize_owner_id(raw);
                if id.is_empty() || !seen.insert(id.to_owned()) || cache.contains_key(id) {
                    continue;
                }
                if id.eq_ignore_ascii_case("spotify") {
                    seeds.push((id.to_owned(), "Spotify".to_owned()));
                } else {
                    to_fetch.push(id.to_owned());
                }
            }
        }
        if !seeds.is_empty() {
            let mut cache = self.user_names.write().await;
            for (k, v) in seeds {
                cache.insert(k, v);
            }
        }
        if to_fetch.is_empty() {
            return;
        }
        let me = self.signed_in_username().await.unwrap_or_default();
        let me_clean = normalize_owner_id(&me).to_owned();
        let resolved: Vec<(String, String)> = stream::iter(to_fetch)
            .map(|id| {
                let is_self = !me_clean.is_empty() && id.eq_ignore_ascii_case(&me_clean);
                async move {
                    let name = self
                        .fetch_user_display_name(&id, is_self)
                        .await
                        .unwrap_or_else(|| id.clone());
                    (id, name)
                }
            })
            .buffer_unordered(8)
            .collect()
            .await;
        let mut cache = self.user_names.write().await;
        for (id, name) in resolved {
            cache.insert(id, name);
        }
    }

    async fn cached_owner_name(&self, raw: &str) -> String {
        let id = normalize_owner_id(raw);
        if id.is_empty() {
            return String::new();
        }
        self.user_names
            .read()
            .await
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_owned())
    }

    async fn resolve_owner_label(&self, owner_id: Option<&str>, text_owner: Option<&str>) -> String {
        let id_clean = owner_id.map(normalize_owner_id).filter(|s| !s.is_empty());
        let text_clean = text_owner.map(str::trim).filter(|s| !s.is_empty());
        if let Some(txt) = text_clean {
            let same_as_id = id_clean.is_some_and(|id| txt.eq_ignore_ascii_case(id));
            if !same_as_id && !looks_like_spotify_user_id(txt) && !txt.eq_ignore_ascii_case("spotify") {
                return txt.to_owned();
            }
        }
        let Some(lookup) = id_clean.or_else(|| text_clean.map(normalize_owner_id)) else {
            return String::new();
        };
        let owned = [lookup.to_owned()];
        self.resolve_user_names(&owned).await;
        self.cached_owner_name(lookup).await
    }

    pub async fn resolve_signed_in_display_name(&self, username: &str) -> String {
        if let Some(stored) = self.auth.stored_display_name().await {
            self.user_names
                .write()
                .await
                .insert(normalize_owner_id(username).to_owned(), stored.clone());
            return stored;
        }
        if let Some(name) = self.fetch_user_display_name(username, true).await {
            self.auth.update_display_name(&name).await;
            self.user_names
                .write()
                .await
                .insert(normalize_owner_id(username).to_owned(), name.clone());
            return name;
        }
        username.to_owned()
    }
}

fn normalize_owner_id(raw: &str) -> &str {
    let trimmed = raw.trim();
    trimmed.strip_prefix("spotify:user:").unwrap_or(trimmed).trim()
}

fn parse_profile_display_name(val: &Value) -> Option<String> {
    val["name"]
        .as_str()
        .or_else(|| val["display_name"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn looks_like_spotify_user_id(raw: &str) -> bool {
    let clean = normalize_owner_id(raw);
    if clean.len() < 5 || !clean.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    clean.bytes().all(|b| b.is_ascii_digit())
        || (clean.len() >= 20 && clean.bytes().any(|b| b.is_ascii_digit()))
}

fn parse_track(uri: &str, raw: &[u8]) -> Result<Track> {
    let msg = ProtoReader::parse(raw)?;
    let canonical = msg
        .bytes(1)
        .and_then(|g| gid_uri("track", g))
        .unwrap_or_else(|| uri.to_string());
    let title = msg.text(2).unwrap_or_default();
    let artists = parse_artist_links(&msg, 4);
    let album_msg = msg.message(3);
    let album = album_msg.as_ref().and_then(|a| {
        let name = a.text(2).filter(|s| !s.is_empty())?;
        let id = a.bytes(1).and_then(|g| gid_uri("album", g));
        Some(Link { name, id })
    });
    let thumbs = album_msg.as_ref().map(album_thumbs).unwrap_or_default();
    let duration_ms = msg.number(7).map(unzigzag).unwrap_or(0).max(0) as u32;
    let duration = (duration_ms / 1000).max(1);
    let track_nr = msg
        .number(5)
        .map(unzigzag)
        .filter(|&n| n > 0)
        .map(|n| n as u16);

    Ok(Track {
        id: canonical,
        title,
        artists,
        album,
        duration: Some(duration),
        thumbs,
        track_nr,
        plays: None,
        music_video: false,
        explicit: msg.number(9) == Some(1),
    })
}

fn parse_album_card_with_year(uri: &str, raw: &[u8]) -> Result<(Card, i32)> {
    let msg = ProtoReader::parse(raw)?;
    let canonical = msg
        .bytes(1)
        .and_then(|g| gid_uri("album", g))
        .unwrap_or_else(|| uri.to_string());
    let title = msg.text(2).unwrap_or_else(|| "Album".to_string());
    let links = parse_artist_links(&msg, 3);
    let year = msg
        .message(6)
        .and_then(|d| d.number(1))
        .map(unzigzag)
        .filter(|&y| y > 0);
    let month = msg
        .message(6)
        .and_then(|d| d.number(2))
        .map(unzigzag)
        .unwrap_or(0);
    let day = msg
        .message(6)
        .and_then(|d| d.number(3))
        .map(unzigzag)
        .unwrap_or(0);
    let sort_key = year.unwrap_or(0) * 10000 + month.clamp(0, 12) * 100 + day.clamp(0, 31);
    let artist_line = links
        .iter()
        .map(|l| l.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let subtitle = match (artist_line.is_empty(), year) {
        (false, Some(y)) => format!("{artist_line} • {y}"),
        (false, None) => artist_line,
        (true, Some(y)) => y.to_string(),
        (true, None) => "Album".to_string(),
    };
    let thumbs = album_thumbs(&msg);

    Ok((
        Card {
            title,
            subtitle,
            thumbs,
            target: Target::Album(canonical),
            round: false,
            links,
        },
        sort_key,
    ))
}

fn parse_artist_card(uri: &str, raw: &[u8]) -> Result<Card> {
    let msg = ProtoReader::parse(raw)?;
    let canonical = msg
        .bytes(1)
        .and_then(|g| gid_uri("artist", g))
        .unwrap_or_else(|| uri.to_string());
    let title = msg.text(2).unwrap_or_else(|| "Artist".to_string());
    let thumbs = artist_thumbs(&msg);
    Ok(Card {
        title,
        subtitle: "Artist".to_string(),
        thumbs,
        target: Target::Artist(canonical),
        round: true,
        links: Vec::new(),
    })
}

fn parse_artist_links(msg: &ProtoReader, field: u32) -> Vec<Link> {
    let mut out = Vec::new();
    for a in msg.messages(field) {
        if let Some(name) = a.text(2).filter(|s| !s.is_empty()) {
            let id = a.bytes(1).and_then(|g| gid_uri("artist", g));
            out.push(Link { name, id });
        }
    }
    out
}

fn album_thumbs(msg: &ProtoReader) -> Vec<Thumb> {
    let mut imgs = Vec::new();
    if let Some(cg) = msg.message(17) {
        imgs.extend(cg.messages(1));
    }
    imgs.extend(msg.messages(9));
    thumbs_from_image_msgs(&imgs)
}

fn artist_thumbs(msg: &ProtoReader) -> Vec<Thumb> {
    let mut imgs = Vec::new();
    for pg in msg.messages(17) {
        imgs.extend(pg.messages(1));
    }
    if let Some(pg) = msg.message(10) {
        imgs.extend(pg.messages(1));
    }
    imgs.extend(msg.messages(11));
    thumbs_from_image_msgs(&imgs)
}

fn thumbs_from_image_msgs(imgs: &[ProtoReader]) -> Vec<Thumb> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for im in imgs {
        let Some(fid) = im.bytes(1).filter(|b| !b.is_empty()) else {
            continue;
        };
        let url = upgrade_image_url(&format!("https://i.scdn.co/image/{}", hex_lower(fid)));
        if !seen.insert(url.clone()) {
            continue;
        }
        let px = im
            .number(3)
            .map(unzigzag)
            .filter(|&w| w > 0)
            .map(|w| w as u32)
            .unwrap_or_else(|| match im.number(2) {
                Some(1) => 64,
                Some(2) | Some(3) => 640,
                _ => 300,
            });
        out.push(Thumb {
            url,
            width: px,
            height: px,
        });
    }
    out.sort_by_key(|t| t.width);
    out
}

fn playlist_thumbs(attrs: &ProtoReader) -> Vec<Thumb> {
    let mut out = Vec::new();
    for ps in attrs.messages(13).into_iter().chain(attrs.messages(11)) {
        if let Some(u) = ps.text(2).filter(|s| !s.is_empty()) {
            let size = match ps.text(1).as_deref() {
                Some("small") => 64,
                Some("large") | Some("xlarge") => 640,
                _ => 300,
            };
            out.push(Thumb {
                url: upgrade_image_url(&u),
                width: size,
                height: size,
            });
        }
    }
    if out.is_empty() {
        if let Some(pic) = attrs.bytes(3).filter(|b| !b.is_empty()) {
            let hex = hex_lower(pic);
            let url = if pic.len() == 20 {
                upgrade_image_url(&format!("https://i.scdn.co/image/{hex}"))
            } else if hex.len() > 40 && hex.len() % 40 == 0 {
                let take = (hex.len() / 40).min(4) * 40;
                format!("https://mosaic.scdn.co/640/{}", &hex[..take])
            } else if let Ok(s) = std::str::from_utf8(pic) {
                upgrade_image_url(s)
            } else {
                String::new()
            };
            if !url.is_empty() {
                out.push(Thumb {
                    url,
                    width: 640,
                    height: 640,
                });
            }
        }
    }
    out.sort_by_key(|t| t.width);
    out
}

fn mosaic_from_tracks(tracks: &[Track]) -> Vec<Thumb> {
    let mut ids = Vec::new();
    for t in tracks {
        if let Some(max_t) = t.thumbs.iter().max_by_key(|th| th.width) {
            if let Some(id) = max_t.url.strip_prefix("https://i.scdn.co/image/") {
                if id.len() == 40 && !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        if ids.len() >= 4 {
            break;
        }
    }
    if ids.len() >= 4 {
        vec![Thumb {
            url: format!("https://mosaic.scdn.co/640/{}", ids.concat()),
            width: 640,
            height: 640,
        }]
    } else if let Some(first) = ids.first() {
        vec![Thumb {
            url: format!("https://i.scdn.co/image/{first}"),
            width: 640,
            height: 640,
        }]
    } else {
        Vec::new()
    }
}

fn thumbs_from_url(url: &str) -> Vec<Thumb> {
    if url.trim().is_empty() {
        return Vec::new();
    }
    vec![Thumb {
        url: upgrade_image_url(url.trim()),
        width: 640,
        height: 640,
    }]
}

fn thumbs_from_json_sources(sources: &[Value]) -> Vec<Thumb> {
    let mut out: Vec<Thumb> = sources
        .iter()
        .filter_map(|s| {
            let url = s["url"].as_str().filter(|u| !u.is_empty())?;
            let width = s["width"].as_u64().unwrap_or(640) as u32;
            let height = s["height"].as_u64().unwrap_or(640) as u32;
            Some(Thumb {
                url: upgrade_image_url(url),
                width,
                height,
            })
        })
        .collect();
    out.sort_by_key(|t| t.width);
    out
}

fn upgrade_image_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("spotify:image:") {
        return format!("https://i.scdn.co/image/{rest}");
    }
    url.replace("ab67616d00004851", "ab67616d0000b273")
        .replace("ab67616d00001e02", "ab67616d0000b273")
        .replace("ab6761610000f178", "ab6761610000e5eb")
        .replace("ab67616100005174", "ab6761610000e5eb")
}

fn strip_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .trim()
        .to_string()
}

fn format_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_profile_display_names_from_spclient_and_web_api() {
        let sp = serde_json::json!({ "name": "Wenuja", "username": "31abc123" });
        assert_eq!(parse_profile_display_name(&sp).as_deref(), Some("Wenuja"));

        let web = serde_json::json!({ "id": "31abc123", "display_name": "Lofi Community" });
        assert_eq!(parse_profile_display_name(&web).as_deref(), Some("Lofi Community"));

        let empty = serde_json::json!({ "name": "   " });
        assert_eq!(parse_profile_display_name(&empty), None);
    }

    #[test]
    fn recognizes_numeric_and_generated_spotify_user_ids() {
        assert_eq!(normalize_owner_id("spotify:user:31xyz123"), "31xyz123");
        assert!(looks_like_spotify_user_id("1214567890"));
        assert!(looks_like_spotify_user_id("31abcdefghijklmnopqrstuvwxyz"));
        assert!(looks_like_spotify_user_id("spotify:user:3123456789abcdefghij"));
        assert!(!looks_like_spotify_user_id("Lofi Girl"));
        assert!(!looks_like_spotify_user_id("Spotify"));
    }
}
