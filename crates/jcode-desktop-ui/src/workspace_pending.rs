//! Session creation never holds up the native editor. Draft IDs also
//! identify replies after concurrent creates, closure, and hot reload.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn next_draft_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // Time distinguishes reloaded cdylibs, whose statics start over, while the
    // counter distinguishes requests even on a coarse system clock.
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "startup://draft/{epoch}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

pub(super) fn next_remote_draft_id(host: &str) -> String {
    next_draft_id().replacen(
        "startup://draft/",
        &format!("startup://draft/remote/{host}/"),
        1,
    )
}

/// The destination travels with the draft through reloads and default-machine
/// changes. Valid SSH destinations cannot contain '/', so no escaping is needed.
pub(super) fn remote_draft_host(id: &str) -> Option<&str> {
    let (host, request) = id
        .strip_prefix("startup://draft/remote/")?
        .split_once('/')?;
    (!request.is_empty() && crate::remote_targets::validate_host(host).is_ok()).then_some(host)
}

pub(super) fn draft_session_info(
    session_id: String,
    title: Option<String>,
    working_dir: Option<String>,
) -> jcode_sdk::SessionInfo {
    jcode_sdk::SessionInfo {
        user_connected: false,
        session_id,
        title,
        working_dir,
        status: "starting".into(),
        transcript_bytes: None,
        saved: false,
        updated_at_ms: None,
        last_active_at_ms: None,
        archived: false,
        archived_at_ms: None,
        save_label: None,
        parent_session_id: None,
        agent_label: None,
        swarm_status: None,
        edit_stats: None,
    }
}

impl Workspace {
    pub(super) fn activate_or_open_project_directory(
        &mut self,
        directory: String,
        cx: &mut Context<Self>,
    ) {
        let key = self.project_key_for_directory(&directory);
        self.sidebar_projects.collapsed.insert(key, false);
        let replace = self.slots.get(self.active).filter(|slot| {
            let panel = slot.panel.read(cx);
            !slot.closing && panel.is_untouched_empty_chat()
                && !self.sessions.iter().any(|session| {
                    session.session_id == panel.session_id
                        && (session.saved || session.transcript_bytes.is_some_and(|bytes| bytes > 0))
                })
        });
        let Some(slot) = replace else {
            self.open_local_draft(Some(directory), cx);
            return;
        };
        let old = slot.panel.read(cx);
        if old.working_dir.as_deref() == Some(directory.as_str()) {
            self.set_active(self.active, cx);
            self.focus_pending = true;
            return;
        }
        let draft = old.input.read(cx).snapshot();
        let old_id = old.session_id.clone();
        if !Panel::is_pending_session_id(&old_id) {
            self.bridge.send(Command::Unwatch { session_id: old_id.clone() });
            self.sessions.retain(|session| session.session_id != old_id);
        }

        let request_id = next_draft_id();
        let panel = self.new_session_panel(request_id.clone(), Some("New session".into()), Some(directory.clone()), cx);
        panel.read(cx).input.clone().update(cx, |input, cx| input.restore(draft, cx));
        self.slots[self.active].panel = panel;
        for (pending, _) in &mut self.pending_collection_drafts {
            if *pending == old_id {
                pending.clone_from(&request_id);
            }
        }
        self.set_active(self.active, cx);
        self.focus_pending = true;
        self.bridge.send(Command::CreateSession {
            working_dir: Some(directory),
            request_id: Some(request_id),
        });
    }

    pub(super) fn open_default_draft(&mut self, directory: Option<String>, cx: &mut Context<Self>) {
        if let Some(host) = self.remotes.default_host.clone() {
            self.open_remote_draft(host, cx);
        } else {
            self.open_local_draft(directory, cx);
        }
    }

    pub(super) fn open_local_draft(&mut self, directory: Option<String>, cx: &mut Context<Self>) {
        self.open_local_draft_kind(directory, false, cx);
    }

    pub(super) fn open_local_draft_kind(
        &mut self,
        directory: Option<String>,
        help: bool,
        cx: &mut Context<Self>,
    ) {
        let mut request_id = next_draft_id();
        if help {
            request_id = request_id.replacen("startup://draft/", "startup://draft/help/", 1);
        }
        self.mount_session_draft(
            request_id.clone(),
            directory.clone(),
            "New session".into(),
            cx,
        );
        self.bridge.send(Command::CreateSession {
            working_dir: directory,
            request_id: Some(request_id),
        });
    }

    pub(super) fn open_remote_draft(&mut self, host: String, cx: &mut Context<Self>) {
        // The old Cloud button stored a personal AWS alias. New drafts must
        // never turn that legacy preference into an implicit AWS operation.
        // Existing live SSH sessions keep their original destination.
        let host = if host == "jcode-cloud-alpha" {
            if self.remotes.default_host.as_deref() == Some("jcode-cloud-alpha") {
                self.remotes.default_host = Some(remotes::MANAGED_CLOUD_HOST.into());
            }
            remotes::MANAGED_CLOUD_HOST.to_owned()
        } else {
            host
        };
        let request_id = next_remote_draft_id(&host);
        self.remotes.failed = false;
        self.remotes.status = Some(format!("Preparing connection to {host}…"));
        self.mount_session_draft(
            request_id.clone(),
            None,
            if host == remotes::MANAGED_CLOUD_HOST {
                "Jcode Cloud unavailable".into()
            } else {
                format!("Connecting to {host}…")
            },
            cx,
        );
        let panel = self.slots[self.active].panel.clone();
        panel.update(cx, |panel, cx| {
            panel.status = format!("Preparing connection to {host}…");
            cx.notify();
        });
        self.create_remote_draft_session(host, request_id, cx);
    }

    pub(super) fn mount_session_draft(
        &mut self,
        request_id: String,
        directory: Option<String>,
        title: String,
        cx: &mut Context<Self>,
    ) {
        let inserted = self.open_session(draft_session_info(request_id, Some(title), directory), cx);
        // The editor must be hit-testable and visible in the very next frame,
        // rather than waiting for either backend creation or a width tween.
        // Snap sibling demotions too. Otherwise the old full-width panel can
        // push this full-width editor beyond the clip until its tween finishes.
        for slot in self
            .slots
            .iter_mut()
            .filter(|slot| slot.row == self.active_row)
        {
            slot.animated_width = AnimatedValue::new(
                slot.width_fraction,
                transition::policy(Transition::PanelOpen).duration,
            );
        }
        // Keep a spatial entrance without shrinking the native editor into a
        // sliver. Only offset the newcomer, so rapid spawns preserve earlier
        // entrances. The existing order tween also drives animation frames and
        // hit testing, and respects the reduced-motion PanelOpen policy.
        let slot = &mut self.slots[inserted];
        slot.order_offset =
            AnimatedValue::new(0.12, transition::policy(Transition::PanelOpen).duration);
        slot.order_offset.set(0.0, Instant::now());
        self.set_active(inserted, cx);
        // The viewport width is only known in render_strip. Resolve and snap
        // its camera there instead of interpolating an offscreen editor in.
        self.camera_snap_pending[self.active_row] = true;
        self.focus_pending = true;
        cx.notify();
    }
}
