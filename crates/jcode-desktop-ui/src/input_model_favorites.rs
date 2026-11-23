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
    loaded: bool,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store {
        revision: 0,
        specs: Rc::from(Vec::new()),
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
        if crate::harness::screenshot_mode() {
            // Offline fixture: a comma-separated spec list.
            if let Ok(raw) = std::env::var("JCODE_DESKTOP_SCREENSHOT_MODEL_FAVORITES") {
                self.specs = Rc::from(
                    raw.split(',')
                        .map(str::trim)
                        .filter(|spec| !spec.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                );
            }
        } else if persisted()
            && let Some(path) = store::path()
        {
            self.specs = Rc::from(store::load(&path));
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
        if persisted()
            && let Some(path) = store::path()
            && let Err(error) = store::save(&path, &store.specs)
        {
            eprintln!("[model-favorites] could not save favorites: {error}");
        }
        favorited
    })
}

/// Drop in-memory favorites so tests cannot leak state into each other.
#[cfg(test)]
pub(crate) fn reset() {
    STORE.with(|store| {
        *store.borrow_mut() = Store {
            revision: 0,
            specs: Rc::from(Vec::new()),
            loaded: false,
        };
    });
}
