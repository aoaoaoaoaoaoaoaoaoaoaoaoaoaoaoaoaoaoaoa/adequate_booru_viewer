use crate::{
    configuration::{FilterLibrary, FilterName, Folder, SavedFilter},
    model::Query,
};

pub type Bank = eternalist_apps::Cabinet<SavedFilter>;
pub type Berth = eternalist_apps::CabinetBerth<FilterName>;
pub type ShelfBerth = eternalist_apps::CabinetShelfBerth;

pub fn forge(config: &FilterLibrary) -> Bank {
    let shelves = config
        .folders
        .iter()
        .map(|folder| eternalist_apps::CabinetShelf {
            name: folder.name.clone(),
            open: true,
            entries: folder.filters.clone(),
        })
        .collect();
    Bank::forge(config.unfiled.clone(), shelves)
}

pub fn project(bank: &Bank) -> FilterLibrary {
    FilterLibrary {
        unfiled: bank.saved.clone(),
        folders: bank
            .shelves
            .iter()
            .map(|shelf| Folder {
                name: shelf.name.clone(),
                filters: shelf.entries.clone(),
            })
            .collect(),
    }
}

pub fn spare(bank: &Bank, query: &Query) -> FilterName {
    let base = FilterName::forge(&stem(query)).unwrap_or_else(FilterName::neutral);
    bank.spare_named(&base)
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
