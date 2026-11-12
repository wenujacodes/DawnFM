//! The Google and Spotify sign-in windows.

use std::sync::atomic::Ordering;

use crate::backend::Req;

use super::App;

impl App {
    pub(super) fn start_spotify_sign_in(&self, ctx: &egui::Context) {
        let xfer = self.transfer.clone();
        if xfer.signing_in.swap(true, Ordering::Relaxed) {
            return;
        }
        let (backend, ctx) = (self.backend.clone(), ctx.clone());
        crate::login::spawn(crate::login::Site::Spotify, xfer.spotify_profile(), move |outcome| {
            match outcome {
                crate::login::Outcome::Cookie(c) => xfer.set_cookie(Some(c), &backend),
                crate::login::Outcome::Cancelled => {}
                crate::login::Outcome::Failed(e) => backend.report_error(format!("Spotify sign-in window failed: {e}")),
            }
            xfer.signing_in.store(false, Ordering::Relaxed);
            ctx.request_repaint();
        });
    }

    pub(super) fn start_sign_in(&self, ctx: &egui::Context) {
        if self.signing_in.swap(true, Ordering::Relaxed) {
            return;
        }
        let (backend, busy, ctx) = (self.backend.clone(), self.signing_in.clone(), ctx.clone());
        crate::login::spawn(crate::login::Site::Google, self.backend.login_profile(), move |outcome| {
            match outcome {
                crate::login::Outcome::Cookie(c) => backend.request(Req::SetCookie(c)),
                crate::login::Outcome::Cancelled => {}
                crate::login::Outcome::Failed(e) => backend.report_error(format!("Sign-in window failed: {e}")),
            }
            busy.store(false, Ordering::Relaxed);
            ctx.request_repaint();
        });
    }
}

