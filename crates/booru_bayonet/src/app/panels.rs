use super::*;

impl Bayonet {
    fn autocomplete(&mut self, ui: &mut egui::Ui) {
        let Some(prefix) = active_prefix(&self.tag_entry) else {
            return;
        };
        let suggestions = match self.index.tag_suggestions(&prefix.body, SUGGESTIONS) {
            Ok(suggestions) => suggestions,
            Err(err) => {
                self.status = format!("{err:#}");
                return;
            }
        };
        if suggestions.is_empty() {
            return;
        }
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
                    self.complete_active(&suggestion, prefix.negative);
                }
            }
        });
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
        chrome::section(ui, "active-filter", "active filter", true, |ui| {
            self.active_filter_panel(ui);
        });
        chrome::section(ui, "filter-library", "filter library", false, |ui| {
            self.filter_library_panel(ui);
        });
        chrome::section(ui, "reference-query", "reference query", true, |ui| {
            self.query_panel(ui);
        });
        chrome::section(ui, "embedding-magnets", "image magnets", true, |ui| {
            self.embedding_panel(ui);
        });
        chrome::section(ui, "gallery-controls", "gallery", false, |ui| {
            self.gallery_panel(ui);
        });
        chrome::section(ui, "index-status", "index status", false, |ui| {
            self.index_status_panel(ui);
        });
    }

    fn active_filter_panel(&mut self, ui: &mut egui::Ui) {
        let actions = saved_filter_ui::active_card(
            ui,
            &mut self.filter_name_entry,
            &mut self.filter_name_focus,
            self.active_filter.as_ref(),
        );
        self.apply_saved_filter_actions(actions);
    }

    fn query_panel(&mut self, ui: &mut egui::Ui) {
        let query = self.query.clone();
        let active_group = self.active_group.clone();
        let mut actions = Vec::new();
        let entry = ui.add_sized(
            [ui.available_width(), 20.0],
            egui::TextEdit::singleline(&mut self.tag_entry).hint_text("add tag to selected group…"),
        );
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
            if ui
                .add(chrome::icon_button("✚"))
                .on_hover_text("add group")
                .clicked()
            {
                actions.push(QueryAction::AddGroup { op: BoolOp::And });
            }
        });
        self.apply_query_actions(actions);
    }

    fn embedding_panel(&mut self, ui: &mut egui::Ui) {
        let _row = ui.horizontal(|ui| {
            let _label = ui.label(chrome::eyebrow("α"));
            let _value = ui.label(chrome::muted(format!("{:.2}", self.rank_alpha)));
        });
        if chrome::rail_f32(ui, &mut self.rank_alpha, 0.0..=2.0).changed() {
            self.save_config();
            self.request_refresh();
        }
        if self.rank_magnets.is_empty() {
            return;
        }
        let mut actions = Vec::new();
        for (slot, magnet) in self.rank_magnets.iter_mut().enumerate() {
            let _row = ui.horizontal(|ui| {
                let _id = ui.label(format!("#{}", magnet.post.id));
                let mut weight = magnet.weight;
                if chrome::rail_i8_sized(
                    ui,
                    &mut weight,
                    MagnetConfig::MIN_WEIGHT..=MagnetConfig::MAX_WEIGHT,
                    82.0,
                )
                .changed()
                {
                    magnet.weight = weight;
                    actions.push(MagnetAction::Changed);
                }
                let _w = ui.label(magnet_marks(magnet.weight));
                if ui.add(chrome::icon_button("−")).clicked() {
                    actions.push(MagnetAction::Repel(magnet.post.id));
                }
                if ui.add(chrome::icon_button("×")).clicked() {
                    actions.push(MagnetAction::Remove(magnet.post.id));
                }
                if slot == 0 {
                    let _prime = ui.label(chrome::eyebrow("prime"));
                }
            });
        }
        if ui
            .add(chrome::icon_button("⌫"))
            .on_hover_text("clear magnets")
            .clicked()
        {
            actions.push(MagnetAction::Clear);
        }
        self.apply_magnet_actions(actions);
    }

    fn gallery_panel(&mut self, ui: &mut egui::Ui) {
        let _sort = ui.horizontal_wrapped(|ui| {
            let _label = ui.label(chrome::eyebrow("SORT"));
            for sort in Sort::ALL {
                if ui
                    .add(chrome::glyph_button(sort.label(), self.sort == sort))
                    .clicked()
                {
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
    }

    fn filter_library_panel(&mut self, ui: &mut egui::Ui) {
        self.apply_saved_filter_actions(saved_filter_ui::library(
            ui,
            self.active_filter.as_ref(),
            &self.saved_filters,
        ));
    }

    fn index_status_panel(&mut self, ui: &mut egui::Ui) {
        for line in [
            format!("status: {}", self.status),
            format!("cache: {}", self.cache_status),
            format!("warm: {}", self.warm_status),
            format!("crawl: {}", self.crawl_status),
            format!("build: {}", env!("CARGO_PKG_VERSION")),
            format!("data: {}", compact_path(&self.lair.data)),
            format!("index: {}", compact_path(&self.lair.index_path())),
        ] {
            let _line = chrome::note(ui, line);
        }
    }

    fn apply_saved_filter_actions(&mut self, actions: Vec<SavedFilterAction>) {
        for action in actions {
            match action {
                SavedFilterAction::New => self.new_filter(),
                SavedFilterAction::Save => self.save_current_filter(),
                SavedFilterAction::BeginRename(name) => self.begin_rename_filter(&name),
                SavedFilterAction::Rename => self.rename_filter(),
                SavedFilterAction::Load(filter) => self.load_filter(filter),
                SavedFilterAction::Clone(name) => self.clone_filter(&name),
                SavedFilterAction::Delete(name) => self.delete_filter(&name),
            }
        }
    }

    fn apply_magnet_actions(&mut self, actions: Vec<MagnetAction>) {
        if actions.is_empty() {
            return;
        }
        let mut changed = false;
        for action in actions {
            match action {
                MagnetAction::Changed => changed = true,
                MagnetAction::Repel(id) => {
                    if let Some(post) = self
                        .rank_magnets
                        .iter()
                        .find(|magnet| magnet.post.id == id)
                        .map(|magnet| magnet.post.clone())
                    {
                        self.repel_magnet(&post);
                    }
                    changed = false;
                }
                MagnetAction::Remove(id) => {
                    self.remove_magnet(id);
                    changed = false;
                }
                MagnetAction::Clear => {
                    self.rank_magnets.clear();
                    changed = true;
                }
            }
        }
        if changed {
            self.rank_magnets.retain(|magnet| magnet.weight != 0);
            self.save_config();
            self.request_refresh();
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
                let mut query = self.query.clone();
                if let Some(path) = query.push_group(&self.active_group, op) {
                    self.install_query_at(query, path);
                }
            }
        }
    }
}

fn magnet_marks(weight: i8) -> egui::RichText {
    let magnitude = weight.unsigned_abs();
    if magnitude == 0 {
        return chrome::muted("0");
    }
    egui::RichText::new(MAGNET_GLYPH.repeat(usize::from(magnitude)))
        .color(magnet_ink(weight))
        .strong()
}
