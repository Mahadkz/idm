//! Virtualized task table model: the daemon owns sorting, filtering and the full
//! list (`ListPage`); the GUI holds only the pages the table has rendered.
//!
//! `row_data` is called by the list view for visible rows only. A row on a page
//! that is not cached yields a placeholder and asks the owner to fetch that page,
//! so scrolling through 20,000 tasks never transfers more than a few pages.

use slint::{Model, ModelNotify, ModelTracker};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use xdm_ipc::TaskFilter;

/// Rows per `ListPage` request (the daemon allows up to 2000).
pub const PAGE: usize = 200;

/// Sort and filter state pushed to the daemon; a change invalidates every page.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Query {
    pub sort: String,
    pub filter: TaskFilter,
}

/// Called (on the UI thread) when a rendered row lives on an uncached page:
/// page index, model epoch and the query to fetch it with.
pub type FetchPage = Rc<dyn Fn(usize, u64, Query)>;

struct State<T> {
    total: usize,
    pages: HashMap<usize, Vec<T>>,
    touched: HashSet<usize>,
    pending: HashSet<usize>,
    query: Query,
    epoch: u64,
}

pub struct PagedModel<T: Clone + PartialEq + 'static> {
    notify: ModelNotify,
    state: RefCell<State<T>>,
    placeholder: T,
    fetch: RefCell<Option<FetchPage>>,
}

impl<T: Clone + PartialEq + 'static> PagedModel<T> {
    pub fn new(placeholder: T) -> Self {
        Self {
            notify: ModelNotify::default(),
            state: RefCell::new(State {
                total: 0,
                pages: HashMap::new(),
                touched: HashSet::new(),
                pending: HashSet::new(),
                query: Query::default(),
                epoch: 0,
            }),
            placeholder,
            fetch: RefCell::new(None),
        }
    }

    pub fn set_fetch(&self, fetch: FetchPage) {
        *self.fetch.borrow_mut() = Some(fetch);
    }

    #[cfg(test)]
    pub fn total(&self) -> usize {
        self.state.borrow().total
    }

    pub fn epoch(&self) -> u64 {
        self.state.borrow().epoch
    }

    /// Adopts `query`; when it differs from the current one every cached page is
    /// dropped and the view resets. Returns whether it changed.
    pub fn begin_query(&self, query: &Query) -> bool {
        let old_total = {
            let mut state = self.state.borrow_mut();
            if &state.query == query {
                return false;
            }
            state.query = query.clone();
            state.epoch += 1;
            state.pages.clear();
            state.pending.clear();
            state.touched.clear();
            std::mem::take(&mut state.total)
        };
        if old_total > 0 {
            self.notify.row_removed(0, old_total);
        }
        true
    }

    /// Pages rendered since the last call; the next refresh re-fetches exactly these.
    pub fn take_touched(&self) -> Vec<usize> {
        let mut state = self.state.borrow_mut();
        let mut pages: Vec<usize> = state.touched.drain().collect();
        pages.sort_unstable();
        pages
    }

    /// Installs fetched `pages` (`replace_all` drops cached pages that were not
    /// refreshed and are not currently rendered). A stale `epoch` is ignored.
    pub fn apply(&self, epoch: u64, total: usize, pages: Vec<(usize, Vec<T>)>, replace_all: bool) -> bool {
        let mut changed_rows: Vec<usize> = Vec::new();
        let (old_total, new_total) = {
            let mut state = self.state.borrow_mut();
            if state.epoch != epoch {
                return false;
            }
            let old_total = state.total;
            state.total = total;
            let fetched: HashSet<usize> = pages.iter().map(|(page, _)| *page).collect();
            for page in &fetched {
                state.pending.remove(page);
            }
            if replace_all {
                let keep: HashSet<usize> = state.touched.union(&fetched).copied().collect();
                state.pages.retain(|page, _| keep.contains(page));
            }
            for (page, rows) in pages {
                let old = state.pages.get(&page);
                for (i, row) in rows.iter().enumerate() {
                    let index = page * PAGE + i;
                    if index < old_total.min(total) && old.and_then(|o| o.get(i)) != Some(row) {
                        changed_rows.push(index);
                    }
                }
                state.pages.insert(page, rows);
            }
            (old_total, total)
        };
        for index in changed_rows {
            self.notify.row_changed(index);
        }
        if new_total > old_total {
            self.notify.row_added(old_total, new_total - old_total);
        } else if new_total < old_total {
            self.notify.row_removed(new_total, old_total - new_total);
        }
        true
    }

    /// A page fetch failed: allow it to be requested again.
    pub fn fetch_failed(&self, epoch: u64, page: usize) {
        let mut state = self.state.borrow_mut();
        if state.epoch == epoch {
            state.pending.remove(&page);
        }
    }

    /// Calls `edit` on every cached row; rows it reports as changed are notified.
    pub fn update_cached(&self, mut edit: impl FnMut(&mut T) -> bool) {
        let mut changed = Vec::new();
        {
            let mut state = self.state.borrow_mut();
            for (page, rows) in state.pages.iter_mut() {
                for (i, row) in rows.iter_mut().enumerate() {
                    if edit(row) {
                        changed.push(page * PAGE + i);
                    }
                }
            }
        }
        for index in changed {
            self.notify.row_changed(index);
        }
    }

    /// Row ids by index (0 where the page is not loaded); `id_of` reads a row's id.
    pub fn ids(&self, id_of: impl Fn(&T) -> i64) -> Vec<i64> {
        let state = self.state.borrow();
        let mut ids = vec![0i64; state.total];
        for (page, rows) in &state.pages {
            for (i, row) in rows.iter().enumerate() {
                if let Some(slot) = ids.get_mut(page * PAGE + i) {
                    *slot = id_of(row);
                }
            }
        }
        ids
    }
}

impl<T: Clone + PartialEq + 'static> Model for PagedModel<T> {
    type Data = T;

    fn row_count(&self) -> usize {
        self.state.borrow().total
    }

    fn row_data(&self, row: usize) -> Option<T> {
        let request = {
            let mut state = self.state.borrow_mut();
            if row >= state.total {
                return None;
            }
            let page = row / PAGE;
            state.touched.insert(page);
            if let Some(found) = state.pages.get(&page).and_then(|rows| rows.get(row % PAGE)) {
                return Some(found.clone());
            }
            if state.pending.insert(page) {
                Some((page, state.epoch, state.query.clone()))
            } else {
                None
            }
        };
        if let Some((page, epoch, query)) = request {
            let fetch = self.fetch.borrow().clone();
            if let Some(fetch) = fetch {
                fetch(page, epoch, query);
            }
        }
        Some(self.placeholder.clone())
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::ModelRc;

    #[test]
    fn placeholder_then_page_and_bounded_cache() {
        let model = Rc::new(PagedModel::<i64>::new(-1));
        let requested = Rc::new(RefCell::new(Vec::new()));
        let log = requested.clone();
        model.set_fetch(Rc::new(move |page, _, _| log.borrow_mut().push(page)));
        let handle: ModelRc<i64> = ModelRc::from(model.clone());
        assert!(model.apply(0, 20_000, Vec::new(), true));
        assert_eq!(handle.row_count(), 20_000);
        assert_eq!(handle.row_data(450), Some(-1));
        assert_eq!(handle.row_data(451), Some(-1));
        assert_eq!(*requested.borrow(), vec![2], "one fetch per missing page");
        let rows: Vec<i64> = (400..600).collect();
        assert!(model.apply(0, 20_000, vec![(2, rows)], false));
        assert_eq!(handle.row_data(450), Some(450));
        assert_eq!(model.ids(|v| *v)[450], 450);
        assert_eq!(model.ids(|v| *v)[0], 0);
        // A refresh that does not include page 2 and has not touched it drops it.
        model.take_touched();
        assert!(model.apply(0, 20_000, vec![(0, (0..200).collect())], true));
        assert_eq!(model.ids(|v| *v)[450], 0);
    }

    #[test]
    fn query_change_resets_and_stale_epoch_is_ignored() {
        let model = PagedModel::<i64>::new(-1);
        let first = Query { sort: "-id".into(), filter: TaskFilter::default() };
        assert!(model.begin_query(&first));
        let epoch = model.epoch();
        assert!(model.apply(epoch, 5, vec![(0, vec![1, 2, 3, 4, 5])], true));
        assert!(!model.begin_query(&first));
        let second = Query { sort: "id".into(), filter: TaskFilter { search: Some("x".into()), ..Default::default() } };
        assert!(model.begin_query(&second));
        assert_eq!(model.total(), 0);
        assert!(!model.apply(epoch, 5, vec![(0, vec![9])], true), "old epoch must not install");
    }
}
