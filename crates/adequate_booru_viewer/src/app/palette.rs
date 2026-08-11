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
        let water = &mut self.water;
        let mut strikes = Vec::new();
        let mut hovered_definition = None;
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
                        water,
                        TAG_MENU_HEIGHT,
                        &mut strikes,
                        None,
                        &mut hovered_definition,
                    );
                });
            });
        self.tag_menu_rect = Some(area.response.rect);
        if let Some(cuts) = &mut self.menu_cuts {
            cuts.1 = area.response.rect;
        }
        for strike in strikes {
            self.apply_tag_strike(strike);
        }
    }

    pub(super) fn viewer_tag_drawer(&mut self, ui: &mut egui::Ui, post: &PostRecord, height: f32) {
        let groups = self.cached_viewer_groups(post);
        let mut strikes = Vec::new();
        let mut hovered_definition = None;
        let definitions = self
            .worker
            .has_tag_definitions()
            .then_some(&self.tag_definitions);
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
                        palette_body(
                            ui,
                            &groups,
                            &self.query,
                            &mut self.water,
                            height,
                            &mut strikes,
                            definitions,
                            &mut hovered_definition,
                        );
                    });
            },
        );
        if let Some(tag) = hovered_definition {
            self.request_tag_definition(tag);
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

    fn request_tag_definition(&mut self, tag: Tag) {
        let should_fetch = match self.tag_definitions.get(&tag) {
            None => true,
            Some(TagDefinitionMemo::Fault { born, .. }) => born.elapsed() >= TAG_DEFINITION_RETRY,
            Some(TagDefinitionMemo::Pending(_) | TagDefinitionMemo::Ready(_)) => false,
        };
        if !should_fetch {
            return;
        }
        self.tag_definition_serial = self.tag_definition_serial.saturating_add(1);
        let serial = self.tag_definition_serial;
        let _old = self
            .tag_definitions
            .insert(tag.clone(), TagDefinitionMemo::Pending(serial));
        if let Err(err) = self.worker.send(Command::TagDefinition {
            serial,
            tag: tag.clone(),
        }) {
            let _old = self.tag_definitions.insert(
                tag,
                TagDefinitionMemo::Fault {
                    message: format!("{err:#}"),
                    born: Instant::now(),
                },
            );
        }
    }

    pub(super) fn absorb_tag_menu_wheel(&mut self, ctx: &egui::Context) {
        if self.pointer_in_tag_menu(ctx) {
            consume_wheel(ctx);
        }
    }

    pub(super) fn retain_tag_menu(&mut self, ctx: &egui::Context, menu_opened: bool) {
        if matches!(self.tag_menu, TagMenu::Closed)
            || self.guide.is_open()
            || ctx.memory(|memory| memory.top_modal_layer().is_some())
        {
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
    water: &mut crate::water::Surface,
    max_height: f32,
    strikes: &mut Vec<TagStrike>,
    definitions: Option<&HashMap<Tag, TagDefinitionMemo>>,
    hovered_definition: &mut Option<Tag>,
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
                        if let Some(remove) = tag_action(
                            ui,
                            water,
                            active.is_some().then_some(chrome::Symbol::Remove),
                            "remove from query",
                        ) {
                            crate::probe_anchor!(
                                ui,
                                format!("tagrow:{}:remove", tag.as_str()),
                                remove.interact_rect
                            );
                            if remove.clicked() {
                                strikes.push(TagStrike::Remove(tag.clone()));
                            }
                        }
                        let require =
                            tag_action(ui, water, Some(chrome::Symbol::Add), "require tag");
                        if require.as_ref().is_some_and(|response| {
                            crate::probe_anchor!(
                                ui,
                                format!("tagrow:{}:require", tag.as_str()),
                                response.interact_rect
                            );
                            response.clicked()
                        }) {
                            strikes.push(TagStrike::Require(tag.clone()));
                        }
                        let exclude =
                            tag_action(ui, water, Some(chrome::Symbol::Decrement), "exclude tag");
                        if exclude.as_ref().is_some_and(|response| {
                            crate::probe_anchor!(
                                ui,
                                format!("tagrow:{}:exclude", tag.as_str()),
                                response.interact_rect
                            );
                            response.clicked()
                        }) {
                            strikes.push(TagStrike::Exclude(tag.clone()));
                        }
                        ui.add_space(4.0);
                        let response = ui.add(
                            egui::Label::new(tag_chroma::text(tag.as_str(), *kind)).truncate(),
                        );
                        crate::probe_anchor!(
                            ui,
                            format!("tagrow:{}:definition", tag.as_str()),
                            response.interact_rect
                        );
                        let _tag = if let Some(definitions) = definitions {
                            if response.hovered() {
                                *hovered_definition = Some(tag.clone());
                            }
                            response.on_hover_ui(|ui| {
                                definition_tooltip(ui, tag, *kind, definitions.get(tag));
                            })
                        } else {
                            response.on_hover_text(tag.as_str())
                        };
                    });
                }
            }
        });
}

fn definition_tooltip(
    ui: &mut egui::Ui,
    tag: &Tag,
    kind: TagKind,
    memo: Option<&TagDefinitionMemo>,
) {
    const WIDTH: f32 = 420.0;
    const HEIGHT: f32 = 360.0;
    ui.set_max_width(WIDTH);
    let title = match memo {
        Some(TagDefinitionMemo::Ready(Some(definition))) => definition.title.as_str(),
        _ => tag.as_str(),
    };
    let _title = ui.label(tag_chroma::text(title, kind).strong().size(14.0));
    let _rule = ui.separator();
    match memo {
        None | Some(TagDefinitionMemo::Pending(_)) => {
            let _loading = ui.label(
                egui::RichText::new("fetching definition…")
                    .italics()
                    .color(chrome::MUTED),
            );
        }
        Some(TagDefinitionMemo::Ready(None)) => {
            let _missing = ui.label(
                egui::RichText::new("no definition")
                    .italics()
                    .color(chrome::MUTED),
            );
        }
        Some(TagDefinitionMemo::Fault { message, .. }) => {
            let _fault = ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("definition unavailable\n{message}"))
                        .italics()
                        .color(chrome::MUTED),
                )
                .wrap(),
            );
        }
        Some(TagDefinitionMemo::Ready(Some(definition))) => {
            let _body = egui::ScrollArea::vertical()
                .max_height(HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_min_width(WIDTH);
                    for block in &definition.blocks {
                        match block {
                            crate::booru::DefinitionBlock::Heading(text) => {
                                ui.add_space(5.0);
                                let _heading = ui
                                    .label(egui::RichText::new(text).strong().color(chrome::TEXT));
                            }
                            crate::booru::DefinitionBlock::Paragraph(text) => {
                                let _paragraph = ui.add(egui::Label::new(text).wrap());
                                ui.add_space(4.0);
                            }
                            crate::booru::DefinitionBlock::Bullet(text) => {
                                let _bullet = ui.horizontal_top(|ui| {
                                    let _mark = ui.label("•");
                                    let _text = ui.add(egui::Label::new(text).wrap());
                                });
                            }
                        }
                    }
                });
        }
    }
}

fn tag_action(
    ui: &mut egui::Ui,
    water: &mut crate::water::Surface,
    symbol: Option<chrome::Symbol>,
    hover: &'static str,
) -> Option<chrome::MonoglyphResponse> {
    let size = egui::Vec2::splat(chrome::MechanismSize::Small.side());
    let Some(symbol) = symbol else {
        let _blank = ui.allocate_space(size);
        return None;
    };
    let response = chrome::Monoglyph::symbol(symbol)
        .size(chrome::MechanismSize::Small)
        .show(ui)
        .on_hover_text(hover);
    water.monoglyph(&response);
    Some(response)
}
