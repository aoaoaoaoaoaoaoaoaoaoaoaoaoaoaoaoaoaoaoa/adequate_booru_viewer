use super::*;

impl Bayonet {
    fn autocomplete(&mut self, ui: &mut egui::Ui, focused: bool) -> bool {
        let Some(prefix) = active_prefix(&self.tag_entry) else {
            self.suggest_memo = None;
            self.suggest_pick = 0;
            return false;
        };
        // Suggestion lookups walk chunked bitmap ranges — far too expensive
        // for the UI thread. A keystroke requests them from the refresh
        // worker; results land via `Event::Suggested` and render from here.
        let stale = self
            .suggest_memo
            .as_ref()
            .is_none_or(|(memo, _)| memo != &prefix.body);
        if stale {
            self.suggest_serial = self.suggest_serial.saturating_add(1);
            let kept = self
                .suggest_memo
                .take()
                .map(|(_, hits)| hits)
                .unwrap_or_default();
            // Keep the previous hits visible while the worker catches up.
            self.suggest_memo = Some((prefix.body.clone(), kept));
            self.suggest_pick = 0;
            if let Err(err) = self.worker.send(Command::Suggest {
                serial: self.suggest_serial,
                prefix: prefix.body.clone(),
            }) {
                self.status = format!("{err:#}");
            }
        }
        let Some((_, suggestions)) = &self.suggest_memo else {
            return false;
        };
        if suggestions.is_empty() {
            return false;
        }
        self.suggest_pick = self.suggest_pick.min(suggestions.len().saturating_sub(1));
        let picked_by_key = focused
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
        if picked_by_key {
            self.suggest_pick = (self.suggest_pick + 1) % suggestions.len();
        }
        let accepted_by_key = focused
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        if accepted_by_key {
            let suggestion = suggestions[self.suggest_pick].clone();
            self.complete_active(&suggestion, prefix.negative);
            return true;
        }
        let mut picked = None;
        let _row = ui.horizontal_wrapped(|ui| {
            let _label = ui.label("complete");
            for (slot, suggestion) in suggestions.iter().enumerate() {
                let selected = slot == self.suggest_pick;
                let cursor = if selected { "▸ " } else { "" };
                if chrome::complete_chip(
                    ui,
                    tag_chroma::text(
                        format!("{cursor}{} ({})", suggestion.tag, suggestion.posts),
                        suggestion.kind,
                    ),
                    selected,
                )
                .clicked()
                {
                    picked = Some(suggestion.clone());
                    self.suggest_pick = slot;
                }
            }
        });
        if let Some(suggestion) = picked {
            self.complete_active(&suggestion, prefix.negative);
            return true;
        }
        false
    }

    fn complete_active(&mut self, suggestion: &TagSuggestion, negative: bool) {
        let polarity = if negative {
            TagPolarity::Negative
        } else {
            TagPolarity::Positive
        };
        if let Some(tag) = Tag::forge(&suggestion.tag) {
            self.add_atom(QueryAtom::Tag(tag), polarity);
        }
        self.tag_entry.clear();
    }

    pub(super) fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.set_width(ui.available_width());
        self.panel_section(ui, "filter-library", "filter library", true, |this, ui| {
            this.filter_library_panel(ui);
        });
        self.panel_section(ui, "active-filter", "active filter", true, |this, ui| {
            this.active_filter_panel(ui);
        });
        self.panel_section(
            ui,
            "reference-query",
            "reference query",
            true,
            |this, ui| {
                this.query_panel(ui);
            },
        );
        self.panel_section(ui, "gallery-controls", "gallery", false, |this, ui| {
            this.gallery_panel(ui);
        });
        self.panel_section(ui, "ui-controls", "ui", false, |this, ui| {
            this.ui_panel(ui);
        });
        self.panel_section(ui, "help", "help", false, |_, ui| {
            Self::help_panel(ui);
        });
        self.panel_section(ui, "index-status", "index status", false, |this, ui| {
            this.index_status_panel(ui);
        });
    }

    fn panel_section(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        title: &'static str,
        default_open: bool,
        add: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        let wake = chrome::section(ui, id, title, default_open, |ui| add(self, ui));
        self.fold_plunge(wake);
    }

    fn active_filter_panel(&mut self, ui: &mut egui::Ui) {
        let mut edit = self.name_edit;
        let actions = saved_filter_ui::active_card(
            ui,
            &mut self.filter_name_entry,
            &mut edit,
            self.active_filter.as_ref(),
        );
        self.name_edit = edit;
        self.apply_saved_filter_actions(actions);
    }

    fn query_panel(&mut self, ui: &mut egui::Ui) {
        let query = self.query.clone();
        let active_group = self.active_group.clone();
        let mut actions = Vec::new();
        let before = self.tag_entry.clone();
        let focus_entry = !ui.ctx().text_edit_focused()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Slash));
        let entry_id = ui.make_persistent_id("tag-entry");
        if focus_entry {
            discard_text(ui, "/");
        }
        let seeded_entry = !focus_entry && self.seed_tag_entry(ui);
        if focus_entry || seeded_entry {
            ui.memory_mut(|mem| mem.request_focus(entry_id));
        }
        let entry = ui.add_sized(
            [ui.available_width(), 20.0],
            egui::TextEdit::singleline(&mut self.tag_entry)
                .id(entry_id)
                .hint_text("add tag to selected group…"),
        );
        if let Some(wake) = chrome::text_wake(ui, &entry, &before, &self.tag_entry) {
            self.text_plunge(wake);
        }
        let accepted_completion = self.autocomplete(ui, entry.has_focus());
        let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
        if !accepted_completion && enter && (entry.has_focus() || entry.lost_focus()) {
            self.commit_tag_entry();
        }
        ui.add_space(5.0);
        if query.is_empty() {
            let _empty = ui.label(chrome::muted("neutral query"));
        }
        render_query_tree(ui, query.root(), &active_group, &mut actions, &mut |atom| {
            self.atom_kind(atom)
        });
        ui.add_space(5.0);
        let _active = ui.horizontal_wrapped(|ui| {
            let add = chrome::icon_still(ui, "✚").on_hover_text("add group");
            if chrome::hover_started(ui, &add) {
                actions.push(QueryAction::Pulse(add.rect));
            }
            if add.clicked() {
                actions.push(QueryAction::AddGroup { op: BoolOp::And });
            }
        });
        self.apply_query_actions(actions);
    }

    fn seed_tag_entry(&mut self, ui: &mut egui::Ui) -> bool {
        if self.zoom.is_some() || self.tag_menu.is_open() || ui.ctx().text_edit_focused() {
            return false;
        }
        let seed = ui.input_mut(|input| {
            if input.modifiers.ctrl || input.modifiers.command || input.modifiers.alt {
                return None;
            }
            let index = input.events.iter().position(
                |event| matches!(event, egui::Event::Text(text) if tag_seed(text).is_some()),
            )?;
            match input.events.remove(index) {
                egui::Event::Text(text) => tag_seed(&text),
                _ => None,
            }
        });
        let Some(seed) = seed else {
            return false;
        };
        self.tag_entry.push_str(&seed);
        true
    }

    fn gallery_panel(&mut self, ui: &mut egui::Ui) {
        let _sort = ui.horizontal_wrapped(|ui| {
            let _label = ui.label(chrome::eyebrow("SORT"));
            for sort in Sort::ALL {
                if chrome::glyph(ui, sort.label(), self.sort == sort).clicked() && self.sort != sort
                {
                    self.sort = sort;
                    self.save_config();
                    self.clear_hit();
                    self.strike(true, AUTO_WARM_PAGES);
                }
            }
        });
        let _row = ui.horizontal(|ui| {
            let _label = ui.label(chrome::eyebrow("IMAGES/ROW"));
            let _value = ui.label(chrome::muted(format!("{}", self.images_per_row)));
        });
        if chrome::rail_u16(
            ui,
            &mut self.images_per_row,
            MIN_IMAGES_PER_ROW..=MAX_IMAGES_PER_ROW,
        )
        .changed()
        {
            self.advance_thumb_epoch();
            self.save_config();
        }
        if ui
            .checkbox(&mut self.prefetch_on_hover, "prefetch on hover")
            .on_hover_text("warm the disk cache with the full image while hovering")
            .changed()
        {
            self.save_config();
        }
    }

    fn filter_library_panel(&mut self, ui: &mut egui::Ui) {
        let mut shelf_edit = self.shelf_edit.take();
        let actions = saved_filter_ui::library(
            ui,
            self.active_filter.as_ref(),
            &self.filters,
            &mut shelf_edit,
        );
        self.shelf_edit = shelf_edit;
        self.apply_saved_filter_actions(actions);
    }

    fn index_status_panel(&mut self, ui: &mut egui::Ui) {
        for line in [
            format!("status: {}", self.status),
            format!("cache: {}", self.cache_status),
            format!("warm: {}", self.warm_status),
            format!("crawl: {}", self.crawl_status),
            format!("build: {}", env!("CARGO_PKG_VERSION")),
            format!("data: {}", self.lair.data.display()),
            format!("index: {}", self.lair.index_path().display()),
        ] {
            let _line = chrome::note(ui, line);
        }
    }

    fn ui_panel(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let dry = self.water_mode == WaterMode::Dry;
        let wet = self.water_mode == WaterMode::Wet;
        let really = self.water_mode == WaterMode::ReallyWet;
        let _wet = ui.horizontal_wrapped(|ui| {
            if chrome::glyph(ui, "DRY", dry)
                .on_hover_text("disable the water shader entirely")
                .clicked()
                && !dry
            {
                self.water_mode = WaterMode::Dry;
                changed = true;
            }
            if chrome::glyph(ui, "WET", wet)
                .on_hover_text("enable water, refraction, and veil shaders")
                .clicked()
                && !wet
            {
                self.water_mode = WaterMode::Wet;
                changed = true;
            }
            if chrome::glyph(ui, "REALLY WET", really)
                .on_hover_text("stronger, slower, wetter water")
                .clicked()
                && !really
            {
                self.water_mode = WaterMode::ReallyWet;
                changed = true;
            }
        });
        if changed {
            self.save_config();
            ui.ctx().request_repaint();
        }
    }

    fn help_panel(ui: &mut egui::Ui) {
        for line in [
            "enter: add typed tag(s) to highlighted group",
            "/: focus tag field",
            "tab in tag field: cycle completions",
            "tab / shift-tab: cycle reference-query groups",
            "-tag: add a negative tag atom",
            "right-click thumbnail: inspect tags",
            "thumbnail tag menu: + require, - exclude, × remove",
            "click thumbnail: open full viewer",
            "viewer tab / tags: toggle image tags",
            "viewer ← / →: previous / next result",
            "viewer click image: touch water",
            "viewer right-click / esc: close",
            "viewer copy / save: export full image",
            "ctrl-wheel gallery: images per row",
            "f10: dump water debug state",
            "shift-f10: purge water debug dumps",
            "f12: water physics bench",
        ] {
            let _line = chrome::note(ui, line);
        }
    }

    fn apply_saved_filter_actions(&mut self, actions: Vec<SavedFilterAction>) {
        for action in actions {
            match action {
                SavedFilterAction::New => self.new_filter(),
                SavedFilterAction::Save => self.save_current_filter(),
                SavedFilterAction::BeginNameEdit => self.begin_name_edit(),
                SavedFilterAction::Rename => self.rename_filter(),
                SavedFilterAction::Load(filter) => self.load_filter(filter),
                SavedFilterAction::Clone(name) => self.clone_filter(&name),
                SavedFilterAction::Delete(name) => self.delete_filter(&name),
                SavedFilterAction::Moor { name, berth } => {
                    self.filters.moor(&name, &berth);
                    self.save_config();
                }
                SavedFilterAction::NewShelf => {
                    self.filters.add_shelf();
                    self.save_config();
                }
                SavedFilterAction::ToggleShelf(shelf) => {
                    self.filters.toggle_shelf(shelf);
                    self.save_config();
                }
                SavedFilterAction::TypeWake(wake) => self.text_plunge(wake),
                SavedFilterAction::Pulse(rect) => self.bump_plunge(rect),
                SavedFilterAction::ScuttleShelf(shelf) => {
                    self.filters.scuttle_shelf(shelf);
                    self.shelf_edit = None;
                    self.save_config();
                }
                SavedFilterAction::BeginShelfRename(shelf) => {
                    let name = self
                        .filters
                        .shelves
                        .get(shelf)
                        .map(|rack| rack.name.clone())
                        .unwrap_or_default();
                    self.shelf_edit = Some(ShelfEdit {
                        shelf,
                        name,
                        focus: true,
                    });
                }
                SavedFilterAction::CommitShelfRename => {
                    if let Some(edit) = self.shelf_edit.take() {
                        self.filters.rename_shelf(edit.shelf, &edit.name);
                        self.save_config();
                    }
                }
            }
        }
    }

    fn commit_tag_entry(&mut self) {
        let terms = Query::parse_terms(&self.tag_entry);
        if terms.is_empty() {
            return;
        }
        let mut query = self.query.clone();
        for term in terms {
            let _inserted = query.push_atom(&self.active_group, term.atom, term.polarity);
        }
        self.tag_entry.clear();
        self.install_query(query);
    }

    fn apply_query_actions(&mut self, actions: Vec<QueryAction>) {
        for action in actions {
            self.apply_query_action(action);
        }
    }

    fn apply_query_action(&mut self, action: QueryAction) {
        match action {
            QueryAction::Select { path, rect } => {
                let path = self.query.clamp_group_path(&path);
                if self.active_group != path {
                    self.active_group = path;
                    self.group_plunge(rect);
                    self.sync_active_filter();
                    self.save_config();
                }
            }
            QueryAction::SetOp { path, op } => {
                let mut query = self.query.clone();
                if query.set_group_op(&path, op) {
                    self.install_query(query);
                }
            }
            QueryAction::ToggleNot { path } => {
                let mut query = self.query.clone();
                if query.toggle_not(&path) {
                    self.install_query(query);
                }
            }
            QueryAction::RemoveChild { parent, child } => {
                let mut query = self.query.clone();
                if query.remove_child(&parent, child) {
                    self.install_query_at(query, parent);
                }
            }
            QueryAction::MoveAtom {
                parent,
                child,
                target,
                rect,
            } => {
                let mut query = self.query.clone();
                if let Some(target) = query.move_atom(&parent, child, &target) {
                    self.bump_plunge(rect);
                    self.install_query_at(query, target);
                }
            }
            QueryAction::AddGroup { op } => {
                if self.active_group.len() >= MAX_GROUP_DEPTH {
                    self.status = format!("group nesting is capped at {MAX_GROUP_DEPTH}");
                    return;
                }
                let mut query = self.query.clone();
                if let Some(path) = query.push_group(&self.active_group, op) {
                    self.install_query_at(query, path);
                }
            }
            QueryAction::Pulse(rect) => self.bump_plunge(rect),
        }
    }
}

fn discard_text(ui: &mut egui::Ui, text: &str) {
    ui.input_mut(|input| {
        if let Some(index) = input
            .events
            .iter()
            .position(|event| matches!(event, egui::Event::Text(found) if found == text))
        {
            let _discarded = input.events.remove(index);
        }
    });
}

fn tag_seed(text: &str) -> Option<String> {
    let seed = text
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>();
    seed.chars().any(|ch| !ch.is_whitespace()).then_some(seed)
}

#[cfg(test)]
mod tests {
    use super::tag_seed;

    #[test]
    fn tag_seed_accepts_printable_query_glyphs() {
        assert_eq!(tag_seed("b"), Some("b".to_owned()));
        assert_eq!(tag_seed("-rating:g"), Some("-rating:g".to_owned()));
    }

    #[test]
    fn tag_seed_rejects_blank_and_control_text() {
        assert_eq!(tag_seed(" "), None);
        assert_eq!(tag_seed("\n"), None);
    }
}
