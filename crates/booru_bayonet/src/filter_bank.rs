use crate::{
    config::{FilterName, SavedFilter},
    model::Query,
};

pub fn sorted(mut filters: Vec<SavedFilter>) -> Vec<SavedFilter> {
    filters.sort_by(|a, b| a.name.cmp(&b.name));
    filters.dedup_by(|a, b| a.name == b.name);
    filters
}

pub fn active(active: Option<FilterName>, filters: &[SavedFilter]) -> Option<FilterName> {
    active.filter(|active| get(active, filters).is_some())
}

pub fn get<'a>(name: &FilterName, filters: &'a [SavedFilter]) -> Option<&'a SavedFilter> {
    filters
        .binary_search_by(|filter| filter.name.cmp(name))
        .ok()
        .map(|slot| &filters[slot])
}

pub fn spare(query: &Query, filters: &[SavedFilter]) -> FilterName {
    let base = FilterName::forge(&stem(query)).unwrap_or_else(FilterName::neutral);
    spare_named(&base, filters)
}

pub fn spare_named(base: &FilterName, filters: &[SavedFilter]) -> FilterName {
    if !taken(base, filters) {
        return base.clone();
    }
    let mut suffix = 2_u64;
    loop {
        let raw = format!("{} {suffix}", base.as_str());
        if let Some(candidate) = FilterName::forge(&raw)
            && !taken(&candidate, filters)
        {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
}

fn stem(query: &Query) -> String {
    let text = query.to_text();
    let text = if text.is_empty() {
        "neutral".to_owned()
    } else {
        text
    };
    clip(&text, 48)
}

fn clip(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut out = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

fn taken(name: &FilterName, filters: &[SavedFilter]) -> bool {
    filters
        .binary_search_by(|filter| filter.name.cmp(name))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use anyhow::{Context as _, Result};

    use super::*;

    #[test]
    fn active_filter_must_exist() -> Result<()> {
        let filters = vec![filter("pose")?];
        assert_eq!(
            active(FilterName::forge("pose"), &filters)
                .as_ref()
                .map(FilterName::as_str),
            Some("pose")
        );
        assert!(active(FilterName::forge("lost"), &filters).is_none());
        Ok(())
    }

    #[test]
    fn clone_names_take_the_next_free_suffix() -> Result<()> {
        let filters = vec![filter("pose")?, filter("pose 2")?];
        let base = FilterName::forge("pose").context("base filter name")?;
        assert_eq!(spare_named(&base, &filters).as_str(), "pose 3");
        Ok(())
    }

    fn filter(name: &str) -> Result<SavedFilter> {
        Ok(SavedFilter::new(
            FilterName::forge(name).context("filter name")?,
            Query::default(),
            Vec::new(),
        ))
    }
}
