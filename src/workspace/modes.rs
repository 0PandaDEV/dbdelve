//! What the mode prompt does when it is answered, and the one place a
//! connection's mode changes.

use super::*;
use crate::session::{PendingRun, Resume};
use crate::sql::Stop;

impl Workspace {
    /// The only place a connection's mode changes. Single, because a later
    /// caller (the grid's own cache, the titlebar picker) is one more thing
    /// that would have to be told separately if this were not the one door.
    pub(crate) fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        let Some(profile) = self.profile_mut() else {
            return;
        };
        profile.mode = mode;
        self.remember_profiles(cx);
        cx.notify();
    }

    pub(crate) fn cancel_pending_run(&mut self, cx: &mut Context<Self>) {
        if let Some(profile) = self.profile_mut() {
            profile.session.pending_run = None;
        }
        cx.notify();
    }

    pub(crate) fn toggle_dont_ask(&mut self, cx: &mut Context<Self>) {
        if let Some(profile) = self.profile_mut()
            && let Some(pending) = &mut profile.session.pending_run
        {
            pending.dont_ask = !pending.dont_ask;
        }
        cx.notify();
    }

    pub(crate) fn approve_pending_run(&mut self, cx: &mut Context<Self>) {
        let Some(profile) = self.profile_mut() else {
            return;
        };
        let Some(PendingRun {
            resume,
            verdict,
            dont_ask,
        }) = profile.session.pending_run.take()
        else {
            return;
        };
        let Some(stop) = sql::gate(verdict, profile.mode, &profile.confirmed) else {
            return;
        };

        match stop {
            Stop::Upgrade(mode) => {
                self.set_mode(mode, cx);
                // Re-checked rather than run outright: raising Read-only to
                // Full for a DROP answers "may this connection do this at
                // all" and leaves "did you mean this table", which is a
                // second dialog on purpose. `execute_and_then` is the only
                // arm here allowed to re-enter the gate -- it is the one
                // that just changed the mode the gate reads.
                let Some(resume) = resume else {
                    return;
                };
                self.execute_and_then(
                    resume.sql,
                    resume.tab,
                    resume.refresh,
                    resume.keep_rows,
                    resume.explain,
                    cx,
                );
            }
            // Confirm and RunOnce run *unchecked*. Sending either back
            // through `execute_and_then` would re-raise the prompt just
            // answered, forever.
            Stop::Confirm(kind) => {
                if dont_ask
                    && kind.suppressible()
                    && let Some(profile) = self.profile_mut()
                    && !profile.confirmed.contains(&kind)
                {
                    profile.confirmed.push(kind);
                    self.remember_profiles(cx);
                }
                self.run_resume(resume, cx);
            }
            Stop::RunOnce => self.run_resume(resume, cx),
        }
    }

    fn run_resume(&mut self, resume: Option<Resume>, cx: &mut Context<Self>) {
        let Some(resume) = resume else {
            return;
        };
        self.execute_unchecked(
            resume.sql,
            resume.tab,
            resume.refresh,
            resume.keep_rows,
            resume.explain,
            cx,
        );
    }
}
