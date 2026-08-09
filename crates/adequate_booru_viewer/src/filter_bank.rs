use crate::{
    config::{FilterConfig, FilterName, SavedFilter, Shelf},
    model::Query,
};

pub type Bank = eternalist_apps::Cabinet<SavedFilter>;
pub type Berth = eternalist_apps::CabinetBerth<FilterName>;
pub type ShelfBerth = eternalist_apps::CabinetShelfBerth;

pub fn forge(config: &FilterConfig) -> Bank {
    let shelves = config
        .shelves
        .iter()
        .map(|shelf| eternalist_apps::CabinetShelf {
            name: shelf.name.clone(),
            open: shelf.open,
            entries: shelf.filters.clone(),
        })
        .collect();
    Bank::forge(config.saved.clone(), shelves)
}

pub fn project(bank: &Bank) -> FilterConfig {
    FilterConfig {
        saved: bank.saved.clone(),
        shelves: bank
            .shelves
            .iter()
            .map(|shelf| Shelf {
                name: shelf.name.clone(),
                open: shelf.open,
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
