use super::*;

#[derive(Debug, Default)]
pub(super) struct TagCompletion {
    memo: Option<(String, Vec<TagSuggestion>)>,
    pick: usize,
    ticket: u64,
}

impl TagCompletion {
    pub(super) fn clear(&mut self) {
        self.memo = None;
        self.pick = 0;
    }

    pub(super) fn demand(&mut self, prefix: &str, serial: &mut u64, worker: &Worker) -> Result<()> {
        if self.memo.as_ref().is_some_and(|(memo, _)| memo == prefix) {
            return Ok(());
        }
        *serial = serial.saturating_add(1);
        self.ticket = *serial;
        let kept = self.memo.take().map(|(_, hits)| hits).unwrap_or_default();
        self.memo = Some((prefix.to_owned(), kept));
        self.pick = 0;
        worker.send(Command::Suggest {
            serial: self.ticket,
            prefix: prefix.to_owned(),
        })
    }

    pub(super) fn absorb(&mut self, ticket: u64, hits: &[TagSuggestion]) -> bool {
        if ticket != self.ticket {
            return false;
        }
        let Some((_, memo)) = &mut self.memo else {
            return false;
        };
        hits.clone_into(memo);
        self.pick = self.pick.min(memo.len().saturating_sub(1));
        true
    }

    pub(super) fn take_cycle(
        &self,
        ui: &mut egui::Ui,
        owns_keys: bool,
        admit: impl Fn(&TagSuggestion) -> bool,
    ) -> Option<GroupCycle> {
        (owns_keys && self.has_choices(admit))
            .then(|| ui.input_mut(take_completion_cycle))
            .flatten()
    }

    pub(super) fn has_choices(&self, admit: impl Fn(&TagSuggestion) -> bool) -> bool {
        self.memo
            .as_ref()
            .is_some_and(|(_, suggestions)| suggestions.iter().any(admit))
    }

    pub(super) fn choose(
        &mut self,
        ui: &mut egui::Ui,
        cycle: Option<GroupCycle>,
        accept: bool,
        _anchor: &'static str,
        admit: impl Fn(&TagSuggestion) -> bool,
    ) -> Option<TagSuggestion> {
        let (_, suggestions) = self.memo.as_ref()?;
        let admitted = suggestions
            .iter()
            .filter(|suggestion| admit(suggestion))
            .count();
        if admitted == 0 {
            return None;
        }
        self.pick = self.pick.min(admitted - 1);
        if let Some(cycle) = cycle {
            self.pick = match cycle {
                GroupCycle::Forward => (self.pick + 1) % admitted,
                GroupCycle::Backward => self.pick.checked_sub(1).unwrap_or(admitted - 1),
            };
        }
        if accept {
            return suggestions
                .iter()
                .filter(|suggestion| admit(suggestion))
                .nth(self.pick)
                .cloned();
        }

        let mut chosen = None;
        let _row = ui.horizontal_wrapped(|ui| {
            let _label = ui.label("complete");
            for (slot, suggestion) in suggestions
                .iter()
                .filter(|suggestion| admit(suggestion))
                .enumerate()
            {
                let selected = slot == self.pick;
                let cursor = if selected { "▸ " } else { "" };
                let chip = chrome::complete_chip(
                    ui,
                    tag_chroma::text(
                        format!("{cursor}{} ({})", suggestion.tag, suggestion.posts),
                        suggestion.kind,
                    ),
                    selected,
                );
                crate::probe_anchor!(
                    ui,
                    format!("{_anchor}:{}", suggestion.tag),
                    chip.interact_rect
                );
                if chip.clicked() {
                    chosen = Some(suggestion.clone());
                    self.pick = slot;
                }
            }
        });
        chosen
    }
}
