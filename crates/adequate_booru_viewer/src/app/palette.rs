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
                    palette_body(ui, groups, query, &mut strikes, &mut pulses);
                });
            });
        self.tag_menu_rect = Some(area.response.rect);
        if let Some(cuts) = &mut self.menu_cuts {
            cuts.1 = area.response.rect;
        }
        for rect in pulses {
            self.bump_plunge(rect);
        }
        for strike in strikes {
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
    strikes: &mut Vec<TagStrike>,
    pulses: &mut Vec<egui::Rect>,
) {
    let _scroll = egui::ScrollArea::vertical()
        .max_height(TAG_MENU_HEIGHT)
        // Never shrink horizontally: the rows' intrinsic widths vary, and a
        // shrunk scroll area strands the scrollbar mid-popup.
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (kind, tags) in groups {
                let _kind = ui.label(tag_chroma::text(kind.label(), *kind).strong());
                for tag in tags {
                    let active = query.polarity(tag);
                    let _row = ui.horizontal(|ui| {
                        let require = chrome::small_still(ui, "+").on_hover_text("require tag");
                        if chrome::hover_started(ui, &require) {
                            pulses.push(require.rect);
                        }
                        if require.clicked() {
                            strikes.push(TagStrike::Require(tag.clone()));
                        }
                        let exclude = chrome::small_still(ui, "-").on_hover_text("exclude tag");
                        if chrome::hover_started(ui, &exclude) {
                            pulses.push(exclude.rect);
                        }
                        if exclude.clicked() {
                            strikes.push(TagStrike::Exclude(tag.clone()));
                        }
                        if active.is_some() {
                            let remove =
                                chrome::small_still(ui, "×").on_hover_text("remove from query");
                            if chrome::hover_started(ui, &remove) {
                                pulses.push(remove.rect);
                            }
                            if remove.clicked() {
                                strikes.push(TagStrike::Remove(tag.clone()));
                            }
                        } else {
                            ui.add_space(18.0);
                        }
                        let _tag = ui
                            .add(egui::Label::new(tag_chroma::text(tag.as_str(), *kind)).truncate())
                            .on_hover_text(tag.as_str());
                    });
                }
            }
        });
}
