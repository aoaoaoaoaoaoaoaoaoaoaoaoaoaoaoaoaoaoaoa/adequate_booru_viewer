use std::collections::HashSet;

use crate::{
    config::{FilterName, SavedFilter, Shelf},
    model::Query,
};

/// Where a dragged filter lands. Name-anchored, so it cannot go stale while
/// the drag is in flight.
#[derive(Clone, Debug)]
pub enum Berth {
    /// Before or after the named filter, in that filter's container.
    Beside { anchor: FilterName, after: bool },
    /// Appended to a folder.
    Shelf(usize),
    /// Appended to the root list.
    Root,
}

/// The ordered filter library: loose filters plus one level of folders.
/// Order is user state; names are globally unique.
#[derive(Clone, Debug, Default)]
pub struct Bank {
    pub root: Vec<SavedFilter>,
    pub shelves: Vec<Shelf>,
}

impl Bank {
    /// Restores config order, dropping duplicate names (first wins).
    pub fn forge(root: Vec<SavedFilter>, shelves: Vec<Shelf>) -> Self {
        let mut seen = HashSet::new();
        let mut bank = Self {
            root: root
                .into_iter()
                .filter(|filter| seen.insert(filter.name.clone()))
                .collect(),
            shelves,
        };
        for shelf in &mut bank.shelves {
            shelf
                .filters
                .retain(|filter| seen.insert(filter.name.clone()));
        }
        bank
    }

    pub fn all(&self) -> impl Iterator<Item = &SavedFilter> {
        self.root
            .iter()
            .chain(self.shelves.iter().flat_map(|shelf| shelf.filters.iter()))
    }

    pub fn get(&self, name: &FilterName) -> Option<&SavedFilter> {
        self.all().find(|filter| &filter.name == name)
    }

    pub fn taken(&self, name: &FilterName) -> bool {
        self.get(name).is_some()
    }

    /// Replaces in place; new names land at the end of the root list.
    pub fn upsert(&mut self, filter: SavedFilter) {
        match self.find_mut(&filter.name) {
            Some(slot) => *slot = filter,
            None => self.root.push(filter),
        }
    }

    pub fn remove(&mut self, name: &FilterName) -> Option<SavedFilter> {
        if let Some(slot) = self.root.iter().position(|filter| &filter.name == name) {
            return Some(self.root.remove(slot));
        }
        for shelf in &mut self.shelves {
            if let Some(slot) = shelf.filters.iter().position(|filter| &filter.name == name) {
                return Some(shelf.filters.remove(slot));
            }
        }
        None
    }

    /// Renames in place, keeping the filter's position.
    pub fn rename(&mut self, old: &FilterName, new: FilterName) {
        if let Some(filter) = self.find_mut(old) {
            filter.name = new;
        }
    }

    /// Re-homes `name` at `berth`.
    pub fn moor(&mut self, name: &FilterName, berth: &Berth) {
        if let Berth::Beside { anchor, .. } = berth
            && anchor == name
        {
            return;
        }
        let Some(filter) = self.remove(name) else {
            return;
        };
        match berth {
            Berth::Beside { anchor, after } => {
                let slip = usize::from(*after);
                match self.berth_of(anchor) {
                    Some((None, slot)) => self.root.insert(slot + slip, filter),
                    Some((Some(shelf), slot)) => {
                        self.shelves[shelf].filters.insert(slot + slip, filter);
                    }
                    None => self.root.push(filter),
                }
            }
            Berth::Shelf(shelf) => match self.shelves.get_mut(*shelf) {
                Some(shelf) => shelf.filters.push(filter),
                None => self.root.push(filter),
            },
            Berth::Root => self.root.push(filter),
        }
    }

    pub fn add_shelf(&mut self) {
        let mut name = "folder".to_owned();
        let mut suffix = 2_u64;
        while self.shelves.iter().any(|shelf| shelf.name == name) {
            name = format!("folder {suffix}");
            suffix = suffix.saturating_add(1);
        }
        self.shelves.push(Shelf {
            name,
            open: true,
            filters: Vec::new(),
        });
    }

    pub fn toggle_shelf(&mut self, shelf: usize) {
        if let Some(shelf) = self.shelves.get_mut(shelf) {
            shelf.open = !shelf.open;
        }
    }

    /// Inserts a new filter immediately after `anchor`, in the same container
    /// — clones live next to their source.
    pub fn adopt_beside(&mut self, anchor: &FilterName, filter: SavedFilter) {
        match self.berth_of(anchor) {
            Some((None, slot)) => self.root.insert(slot + 1, filter),
            Some((Some(shelf), slot)) => self.shelves[shelf].filters.insert(slot + 1, filter),
            None => self.root.push(filter),
        }
    }

    /// Deletes a folder; its filters spill back to the root list.
    pub fn scuttle_shelf(&mut self, shelf: usize) {
        if shelf < self.shelves.len() {
            let shelf = self.shelves.remove(shelf);
            self.root.extend(shelf.filters);
        }
    }

    pub fn rename_shelf(&mut self, shelf: usize, name: &str) {
        let name = name.trim();
        if !name.is_empty()
            && let Some(shelf) = self.shelves.get_mut(shelf)
        {
            name.clone_into(&mut shelf.name);
        }
    }

    /// A free name derived from the query text.
    pub fn spare(&self, query: &Query) -> FilterName {
        let base = FilterName::forge(&stem(query)).unwrap_or_else(FilterName::neutral);
        self.spare_named(&base)
    }

    pub fn spare_named(&self, base: &FilterName) -> FilterName {
        if !self.taken(base) {
            return base.clone();
        }
        let mut suffix = 2_u64;
        loop {
            let raw = format!("{} {suffix}", base.as_str());
            if let Some(candidate) = FilterName::forge(&raw)
                && !self.taken(&candidate)
            {
                return candidate;
            }
            suffix = suffix.saturating_add(1);
        }
    }

    fn find_mut(&mut self, name: &FilterName) -> Option<&mut SavedFilter> {
        self.root
            .iter_mut()
            .chain(
                self.shelves
                    .iter_mut()
                    .flat_map(|shelf| shelf.filters.iter_mut()),
            )
            .find(|filter| &filter.name == name)
    }

    fn berth_of(&self, name: &FilterName) -> Option<(Option<usize>, usize)> {
        if let Some(slot) = self.root.iter().position(|filter| &filter.name == name) {
            return Some((None, slot));
        }
        self.shelves.iter().enumerate().find_map(|(shelf, rack)| {
            rack.filters
                .iter()
                .position(|filter| &filter.name == name)
                .map(|slot| (Some(shelf), slot))
        })
    }
}

fn stem(query: &Query) -> String {
    let text = query.to_text();
    let text = if text.is_empty() {
        "neutral".to_owned()
    } else {
        text
    };
    truncate(&text, 48)
}

fn truncate(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut out = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use anyhow::{Context as _, Result};

    use super::*;

    #[test]
    fn clone_names_take_the_next_free_suffix() -> Result<()> {
        let bank = Bank::forge(vec![filter("pose")?, filter("pose 2")?], Vec::new());
        let base = FilterName::forge("pose").context("base filter name")?;
        assert_eq!(bank.spare_named(&base).as_str(), "pose 3");
        Ok(())
    }

    #[test]
    fn mooring_rearranges_and_shelves() -> Result<()> {
        let mut bank = Bank::forge(vec![filter("a")?, filter("b")?, filter("c")?], Vec::new());
        bank.add_shelf();

        let c = FilterName::forge("c").context("c")?;
        bank.moor(
            &c,
            &Berth::Beside {
                anchor: FilterName::forge("a").context("a")?,
                after: false,
            },
        );
        assert_eq!(order(&bank.root), ["c", "a", "b"]);

        bank.moor(&c, &Berth::Shelf(0));
        assert_eq!(order(&bank.root), ["a", "b"]);
        assert_eq!(order(&bank.shelves[0].filters), ["c"]);

        // Clones land beside their source, inside the same container.
        bank.adopt_beside(&c, filter("c 2")?);
        assert_eq!(order(&bank.shelves[0].filters), ["c", "c 2"]);

        bank.scuttle_shelf(0);
        assert_eq!(order(&bank.root), ["a", "b", "c", "c 2"]);
        Ok(())
    }

    fn order(filters: &[SavedFilter]) -> Vec<&str> {
        filters.iter().map(|filter| filter.name.as_str()).collect()
    }

    fn filter(name: &str) -> Result<SavedFilter> {
        Ok(SavedFilter::new(
            FilterName::forge(name).context("filter name")?,
            Query::default(),
            Vec::new(),
        ))
    }
}
