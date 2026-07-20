use super::*;

enum TagStrike {
    Require(Tag),
    Exclude(Tag),
    Remove(Tag),
}

impl Bayonet {
    pub(super) fn tag_palette_overlay(&mut self, ctx: &egui::Context) {
        let Some((post, anchor, groups)) = self.tag_menu.view() else {
            self.tag_menu_rect = None;
            return;
        };
        let pos = tag_menu_pos(anchor, ctx.content_rect());
        let query = &self.query;
        let mut strikes = Vec::new();
        let mut pulses = Vec::new();
        // Per-post area id: egui remembers area sizes by id and never shrinks
        // them, so a shared id would inherit the widest menu ever shown.
        let area = egui::Area::new(egui::Id::new(("tag-palette", post.id.0)))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                let _frame = egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(TAG_MENU_WIDTH);
                    palette_body(
                        ui,
                        groups,
                        query,
                        TAG_MENU_HEIGHT,
                        &mut strikes,
                        &mut pulses,
                    );
                });
            });
        self.tag_menu_rect = Some(area.response.rect);
        if let Some(cuts) = &mut self.menu_cuts {
            cuts.1 = area.response.rect;
        }
        for rect in pulses {
            self.water.bump(rect);
        }
        for strike in strikes {
            self.apply_tag_strike(strike);
        }
    }

    pub(super) fn viewer_tag_drawer(&mut self, ui: &mut egui::Ui, post: &PostRecord, height: f32) {
        let groups = self.cached_viewer_groups(post);
        let mut strikes = Vec::new();
        let mut pulses = Vec::new();
        let _slot = ui.allocate_ui_with_layout(
            egui::vec2(TAG_MENU_WIDTH, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_size(egui::vec2(TAG_MENU_WIDTH, height));
                let _frame = egui::Frame::new()
                    .fill(chrome::SURFACE)
                    .stroke(egui::Stroke::new(1.0_f32, chrome::EDGE))
                    .inner_margin(egui::Margin::symmetric(7, 6))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        palette_body(ui, &groups, &self.query, height, &mut strikes, &mut pulses);
                    });
            },
        );
        for rect in pulses {
            self.water.bump(rect);
        }
        for strike in strikes {
            self.apply_tag_strike(strike);
        }
    }

    fn cached_viewer_groups(&mut self, post: &PostRecord) -> TagGroups {
        if let Some((id, groups)) = &self.viewer_tag_groups
            && *id == post.id
        {
            return groups.clone();
        }
        let groups = self.learn_tag_groups(post);
        self.viewer_tag_groups = Some((post.id, groups.clone()));
        groups
    }

    fn apply_tag_strike(&mut self, strike: TagStrike) {
        match strike {
            TagStrike::Require(tag) => {
                self.add_atom(QueryAtom::Tag(tag), TagPolarity::Positive);
            }
            TagStrike::Exclude(tag) => {
                self.add_atom(QueryAtom::Tag(tag), TagPolarity::Negative);
            }
            TagStrike::Remove(tag) => self.remove_atom(&QueryAtom::Tag(tag)),
        }
    }

    pub(super) fn absorb_tag_menu_wheel(&mut self, ctx: &egui::Context) {
        if self.pointer_in_tag_menu(ctx) {
            consume_wheel(ctx);
        }
    }

    pub(super) fn retain_tag_menu(&mut self, ctx: &egui::Context, menu_opened: bool) {
        if matches!(self.tag_menu, TagMenu::Closed) {
            return;
        }
        let escaped = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let inside = self.pointer_in_tag_menu(ctx);
        let outside_click =
            ctx.input(|input| input.pointer.primary_clicked()) && !inside && !menu_opened;
        // Any right-click that didn't just open or switch a menu dismisses it
        // — including one landing on the menu itself (the cursor sits on the
        // fresh menu's corner, so "right-click the same image" arrives here).
        let secondary = ctx.input(|input| input.pointer.secondary_clicked()) && !menu_opened;
        if escaped || outside_click || secondary {
            self.close_tag_menu();
        }
    }

    fn pointer_in_tag_menu(&self, ctx: &egui::Context) -> bool {
        let Some(rect) = self.tag_menu_rect else {
            return false;
        };
        ctx.pointer_latest_pos()
            .is_some_and(|pos| rect.expand(2.0).contains(pos))
    }
}

fn palette_body(
    ui: &mut egui::Ui,
    groups: &[(TagKind, Vec<Tag>)],
    query: &Query,
    max_height: f32,
    strikes: &mut Vec<TagStrike>,
    pulses: &mut Vec<egui::Rect>,
) {
    let _scroll = egui::ScrollArea::vertical()
        .max_height(max_height)
        // Never shrink horizontally: the rows' intrinsic widths vary, and a
        // shrunk scroll area strands the scrollbar mid-popup.
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (kind, tags) in groups {
                let _kind = ui.label(tag_chroma::text(kind.label(), *kind).strong());
                for tag in tags {
                    let active = query.polarity(tag);
                    let _row = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if let Some(remove) =
                            tag_action(ui, active.is_some().then_some("×"), "remove from query")
                        {
                            crate::probe_anchor!(
                                ui,
                                format!("tagrow:{}:remove", tag.as_str()),
                                remove.interact_rect
                            );
                            if chrome::hover_started(ui, &remove) {
                                pulses.push(remove.rect);
                            }
                            if remove.clicked() {
                                strikes.push(TagStrike::Remove(tag.clone()));
                            }
                        }
                        let require = tag_action(ui, Some("+"), "require tag");
                        if require.as_ref().is_some_and(|response| {
                            crate::probe_anchor!(
                                ui,
                                format!("tagrow:{}:require", tag.as_str()),
                                response.interact_rect
                            );
                            if chrome::hover_started(ui, response) {
                                pulses.push(response.rect);
                            }
                            response.clicked()
                        }) {
                            strikes.push(TagStrike::Require(tag.clone()));
                        }
                        let exclude = tag_action(ui, Some("-"), "exclude tag");
                        if exclude.as_ref().is_some_and(|response| {
                            crate::probe_anchor!(
                                ui,
                                format!("tagrow:{}:exclude", tag.as_str()),
                                response.interact_rect
                            );
                            if chrome::hover_started(ui, response) {
                                pulses.push(response.rect);
                            }
                            response.clicked()
                        }) {
                            strikes.push(TagStrike::Exclude(tag.clone()));
                        }
                        ui.add_space(4.0);
                        let _tag = ui
                            .add(egui::Label::new(tag_chroma::text(tag.as_str(), *kind)).truncate())
                            .on_hover_text(tag.as_str());
                    });
                }
            }
        });
}

fn tag_action(
    ui: &mut egui::Ui,
    glyph: Option<&'static str>,
    hover: &'static str,
) -> Option<egui::Response> {
    const ACTION: f32 = 22.0;
    let size = egui::vec2(ACTION, ui.spacing().interact_size.y);
    let Some(glyph) = glyph else {
        let _blank = ui.allocate_space(size);
        return None;
    };
    Some(
        ui.add_sized(
            size,
            egui::Button::new(egui::RichText::new(glyph).monospace()).small(),
        )
        .on_hover_text(hover),
    )
}
