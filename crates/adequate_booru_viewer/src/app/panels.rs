use super::*;

impl Bayonet {
    fn autocomplete(&mut self, ui: &mut egui::Ui) {
        let Some(prefix) = active_prefix(&self.tag_entry) else {
            self.suggest_memo = None;
            return;
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
            if let Err(err) = self.worker.send(Command::Suggest {
                serial: self.suggest_serial,
                prefix: prefix.body.clone(),
            }) {
                self.status = format!("{err:#}");
            }
        }
        let Some((_, suggestions)) = &self.suggest_memo else {
            return;
        };
        if suggestions.is_empty() {
            return;
        }
        let mut picked = None;
        let _row = ui.horizontal_wrapped(|ui| {
            let _label = ui.label("complete");
            for suggestion in suggestions {
                if ui
                    .small_button(tag_chroma::text(
                        format!("{} ({})", suggestion.tag, suggestion.posts),
                        suggestion.kind,
                    ))
                    .clicked()
                {
                    picked = Some(suggestion.clone());
                }
            }
        });
        if let Some(suggestion) = picked {
            self.complete_active(&suggestion, prefix.negative);
        }
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
        chrome::section(ui, "filter-library", "filter library", true, |ui| {
            self.filter_library_panel(ui);
        });
        chrome::section(ui, "active-filter", "active filter", true, |ui| {
            self.active_filter_panel(ui);
        });
        chrome::section(ui, "reference-query", "reference query", true, |ui| {
            self.query_panel(ui);
        });
        chrome::section(ui, "gallery-controls", "gallery", false, |ui| {
            self.gallery_panel(ui);
        });
        chrome::section(ui, "index-status", "index status", false, |ui| {
            self.index_status_panel(ui);
        });
        chrome::section(ui, "ui-controls", "ui", false, |ui| {
            self.ui_panel(ui);
        });
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
        let entry = ui.add_sized(
            [ui.available_width(), 20.0],
            egui::TextEdit::singleline(&mut self.tag_entry).hint_text("add tag to selected group…"),
        );
        if let Some(wake) = chrome::text_wake(ui, &entry, &before, &self.tag_entry) {
            self.text_plunge(wake);
        }
        let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
        if enter && (entry.has_focus() || entry.lost_focus()) {
            self.commit_tag_entry();
        }
        self.autocomplete(ui);
        ui.add_space(5.0);
        if query.is_empty() {
            let _empty = ui.label(chrome::muted("neutral query"));
        }
        render_query_tree(ui, query.root(), &active_group, &mut actions, &mut |atom| {
            self.atom_kind(atom)
        });
        ui.add_space(5.0);
        let _active = ui.horizontal_wrapped(|ui| {
            if chrome::icon(ui, "✚").on_hover_text("add group").clicked() {
                actions.push(QueryAction::AddGroup { op: BoolOp::And });
            }
        });
        self.apply_query_actions(actions);
    }

    fn gallery_panel(&mut self, ui: &mut egui::Ui) {
        let _sort = ui.horizontal_wrapped(|ui| {
            let _label = ui.label(chrome::eyebrow("SORT"));
            for sort in Sort::ALL {
                if chrome::glyph(ui, sort.label(), self.sort == sort).clicked() {
                    self.sort = sort;
                    self.save_config();
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
        let wet = self.water_ui.wet();
        let _wet = ui.horizontal_wrapped(|ui| {
            if chrome::glyph(ui, "DRY", !wet)
                .on_hover_text("disable the water shader entirely")
                .clicked()
                && wet
            {
                self.water_ui = WaterUi::Dry;
                changed = true;
            }
            if chrome::glyph(ui, "WET", wet)
                .on_hover_text("enable water, refraction, and veil shaders")
                .clicked()
                && !wet
            {
                self.water_ui = WaterUi::Wet(Definition::Sd);
                changed = true;
            }
        });
        let wet = self.water_ui.wet();
        let sd = self.water_ui.is(Definition::Sd);
        let hd = self.water_ui.is(Definition::Hd);
        let _quality = ui.add_enabled_ui(wet, |ui| {
            let _row = ui.horizontal_wrapped(|ui| {
                if chrome::glyph(ui, "SD", sd)
                    .on_hover_text("2 px water cells")
                    .clicked()
                    && !sd
                {
                    self.water_ui = WaterUi::Wet(Definition::Sd);
                    changed = true;
                }
                if chrome::glyph(ui, "HD", hd)
                    .on_hover_text("1 px water cells")
                    .clicked()
                    && !hd
                {
                    self.water_ui = WaterUi::Wet(Definition::Hd);
                    changed = true;
                }
            });
        });
        if changed {
            self.save_config();
            ui.ctx().request_repaint();
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
            QueryAction::Select { path } => {
                self.active_group = self.query.clamp_group_path(&path);
                self.sync_active_filter();
                self.save_config();
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
        }
    }
}
