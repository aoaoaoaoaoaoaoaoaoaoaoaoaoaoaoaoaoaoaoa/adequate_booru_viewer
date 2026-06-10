use super::*;

impl Bayonet {
    pub(super) fn tag_palette_overlay(&mut self, ctx: &egui::Context) {
        let Some((post, anchor)) = self.tag_menu.view() else {
            self.tag_menu_rect = None;
            return;
        };
        let post = post.clone();
        let pos = tag_menu_pos(anchor, ctx.content_rect());
        let area = egui::Area::new(egui::Id::new("tag-palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                let _frame = egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(TAG_MENU_WIDTH);
                    self.tag_palette(ui, &post);
                });
            });
        self.tag_menu_rect = Some(area.response.rect);
    }

    fn tag_palette(&mut self, ui: &mut egui::Ui, post: &PostRecord) {
        let query = self.query();
        let learned = match self.index.tag_kinds(&post.tags) {
            Ok(kinds) => {
                for (tag, kind) in &kinds {
                    if *kind != TagKind::General {
                        let _old = self.tag_kinds.insert(tag.clone(), *kind);
                    }
                }
                kinds
            }
            Err(err) => {
                self.status = format!("{err:#}");
                std::collections::BTreeMap::new()
            }
        };
        let groups = tag_palette::grouped(post, |tag| {
            learned.get(tag).copied().unwrap_or(TagKind::General)
        });
        let _heading = ui.label(format!(
            "#{}  score {}  fav {}",
            post.id, post.score, post.favs
        ));
        let _scroll = egui::ScrollArea::vertical()
            .max_height(TAG_MENU_HEIGHT)
            .show(ui, |ui| {
                for (kind, tags) in groups {
                    let _kind = ui.label(tag_chroma::text(kind.label(), kind).strong());
                    for tag in tags {
                        let active = query.polarity(&tag);
                        let _row = ui.horizontal(|ui| {
                            if ui.small_button("-").clicked() {
                                self.set_tag(tag.as_str(), TagPolarity::Negative);
                            }
                            if active.is_some() && ui.small_button("×").clicked() {
                                self.remove_tag(tag.as_str());
                            } else if active.is_none() {
                                ui.add_space(18.0);
                            }
                            let _tag = ui.label(tag_chroma::text(tag.as_str(), kind));
                            if ui.small_button("+").clicked() {
                                self.set_tag(tag.as_str(), TagPolarity::Positive);
                            }
                        });
                    }
                }
            });
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
        let inside = self.pointer_in_tag_menu(ctx);
        let outside_click =
            ctx.input(|input| input.pointer.primary_clicked()) && !inside && !menu_opened;
        if outside_click {
            self.tag_menu = TagMenu::Closed;
            self.tag_menu_rect = None;
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
