//! Favorite model routes shared by the `/model` popup and the model browser.
//!
//! One process-wide store: a star toggled in any panel appears in every
//! other panel's picker. Tests and offline screenshots never touch the real
//! `~/.jcode/desktop-model-favorites.json`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::model_favorites as store;

struct Store {
    /// Bumped on every toggle; suggestion caches key on it.
    revision: u64,
    specs: Rc<[String]>,
    hidden: Rc<[String]>,
    loaded: bool,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store {
        revision: 0,
        specs: Rc::from(Vec::new()),
        hidden: Rc::from(Vec::new()),
        loaded: false,
    });
}

fn persisted() -> bool {
    !(cfg!(test) || crate::harness::screenshot_mode())
}

impl Store {
    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let env_list = |name| -> Vec<String> {
            std::env::var(name)
                .map(|raw| {
                    raw.split(',')
                        .map(str::trim)
                        .filter(|entry| !entry.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        if crate::harness::screenshot_mode() {
            // Offline fixtures: comma-separated spec/group-key lists.
            self.specs =
                Rc::from(env_list("JCODE_DESKTOP_SCREENSHOT_MODEL_FAVORITES"));
            self.hidden =
                Rc::from(env_list("JCODE_DESKTOP_SCREENSHOT_MODEL_HIDDEN"));
        } else if persisted()
            && let Some(path) = store::path()
        {
            let lists = store::load(&path);
            self.specs = Rc::from(lists.favorites);
            self.hidden = Rc::from(lists.hidden_groups);
        }
    }

    fn persist(&self) {
        if persisted()
            && let Some(path) = store::path()
        {
            let lists = store::Lists {
                favorites: self.specs.to_vec(),
                hidden_groups: self.hidden.to_vec(),
                hidden_models: store::load(&path).hidden_models,
            };
            if let Err(error) = store::save(&path, &lists) {
                eprintln!("[model-favorites] could not save: {error}");
            }
        }
    }
}

/// The favorite specs plus the revision that changes on every toggle.
pub(crate) fn favorites() -> (u64, Rc<[String]>) {
    STORE.with(|store| {
        let mut store = store.borrow_mut();
        store.ensure_loaded();
        (store.revision, store.specs.clone())
    })
}

pub(crate) fn is_favorite(specs: &[String], spec: &str) -> bool {
    specs.iter().any(|entry| entry == spec)
}

/// Toggle `spec`; returns the new favorited state.
pub(crate) fn toggle(spec: &str) -> bool {
    STORE.with(|store| {
        let mut store = store.borrow_mut();
        store.ensure_loaded();
        let mut specs = store.specs.to_vec();
        let favorited = store::toggle(&mut specs, spec);
        store.specs = Rc::from(specs);
        store.revision += 1;
        store.persist();
        favorited
    })
}

/// Hidden provider/channel group keys plus the shared revision.
pub(crate) fn hidden_groups() -> (u64, Rc<[String]>) {
    STORE.with(|store| {
        let mut store = store.borrow_mut();
        store.ensure_loaded();
        (store.revision, store.hidden.clone())
    })
}

pub(crate) fn is_hidden(hidden: &[String], group_key: &str) -> bool {
    hidden.iter().any(|entry| entry == group_key)
}

/// Toggle `group_key`; returns the new hidden state.
pub(crate) fn toggle_hidden(group_key: &str) -> bool {
    STORE.with(|store| {
        let mut store = store.borrow_mut();
        store.ensure_loaded();
        let mut hidden = store.hidden.to_vec();
        let now_hidden = store::toggle(&mut hidden, group_key);
        store.hidden = Rc::from(hidden);
        store.revision += 1;
        store.persist();
        now_hidden
    })
}

/// Unhide every group. No-op when nothing is hidden.
pub(crate) fn show_all() {
    STORE.with(|store| {
        let mut store = store.borrow_mut();
        store.ensure_loaded();
        if store.hidden.is_empty() {
            return;
        }
        store.hidden = Rc::from(Vec::new());
        store.revision += 1;
        store.persist();
    })
}

/// Drop in-memory state so tests cannot leak it into each other.
#[cfg(test)]
pub(crate) fn reset() {
    STORE.with(|store| {
        *store.borrow_mut() = Store {
            revision: 0,
            specs: Rc::from(Vec::new()),
            hidden: Rc::from(Vec::new()),
            loaded: false,
        };
    });
}
