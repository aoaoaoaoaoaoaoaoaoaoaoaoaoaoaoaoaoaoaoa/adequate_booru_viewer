use crate::{
    chrome,
    model::{BoolGroup, BoolOp, QueryAtom, QueryExpr, TagKind},
    tag_chroma,
};

#[derive(Clone, Debug)]
pub enum QueryAction {
    Select { path: Vec<usize> },
    SetOp { path: Vec<usize>, op: BoolOp },
    ToggleNot { path: Vec<usize> },
    RemoveChild { parent: Vec<usize>, child: usize },
    AddGroup { op: BoolOp },
}

pub fn render_query_tree(
    ui: &mut egui::Ui,
    root: &QueryExpr,
    active: &[usize],
    actions: &mut Vec<QueryAction>,
    tag_kind: &mut impl FnMut(&QueryAtom) -> TagKind,
) {
    let mut path = Vec::new();
    render_query_expr(ui, root, &mut path, None, active, 0, actions, tag_kind);
}

fn render_query_expr(
    ui: &mut egui::Ui,
    expr: &QueryExpr,
    path: &mut Vec<usize>,
    parent: Option<(Vec<usize>, usize)>,
    active: &[usize],
    depth: usize,
    actions: &mut Vec<QueryAction>,
    tag_kind: &mut impl FnMut(&QueryAtom) -> TagKind,
) {
    let (negated, core) = expr.denote();
    match core {
        QueryExpr::Atom { atom } => render_atom(ui, atom, negated, parent, actions, tag_kind),
        QueryExpr::Group { group } => {
            render_group(
                ui, group, negated, path, parent, active, depth, actions, tag_kind,
            );
        }
        QueryExpr::Not { child } => {
            render_query_expr(ui, child, path, parent, active, depth, actions, tag_kind);
        }
    }
}

fn render_group(
    ui: &mut egui::Ui,
    group: &BoolGroup,
    negated: bool,
    path: &mut Vec<usize>,
    parent: Option<(Vec<usize>, usize)>,
    active: &[usize],
    depth: usize,
    actions: &mut Vec<QueryAction>,
    tag_kind: &mut impl FnMut(&QueryAtom) -> TagKind,
) {
    let active_here = path.as_slice() == active;
    let baseline = actions.len();
    // Lean margins: nesting reads from the depth hues, not from shrinkage, so
    // headers stay unwrapped down to the depth cap.
    let frame = egui::Frame::new()
        .fill(group_fill(depth))
        .stroke(group_stroke(depth, active_here))
        .corner_radius(2)
        .inner_margin(egui::Margin {
            left: 5,
            right: 2,
            top: 3,
            bottom: 3,
        });
    let frame = frame.show(ui, |ui| {
        // Fill the parent: the gutter around every group is then exactly the
        // frame margins, at the root no less than anywhere else.
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        let _header = ui.horizontal_wrapped(|ui| {
            if chrome::glyph(ui, group_label(path, active_here), active_here)
                .on_hover_text("click to select this group for new tags")
                .clicked()
            {
                actions.push(QueryAction::Select { path: path.clone() });
            }
            // The remove button rides directly after the title: when the
            // header wraps at nested depths, only self-evident op buttons may
            // land on the next line — never an orphaned ×.
            if let Some((parent, child)) = parent.as_ref()
                && chrome::small(ui, "×")
                    .on_hover_text("remove group")
                    .clicked()
            {
                actions.push(QueryAction::RemoveChild {
                    parent: parent.clone(),
                    child: *child,
                });
            }
            if chrome::glyph(ui, "¬", negated)
                .on_hover_text("negate this group")
                .clicked()
            {
                actions.push(QueryAction::ToggleNot { path: path.clone() });
            }
            for op in BoolOp::ALL {
                if chrome::glyph(ui, op.label(), group.op == op)
                    .on_hover_text(op_blurb(op))
                    .clicked()
                {
                    actions.push(QueryAction::SetOp {
                        path: path.clone(),
                        op,
                    });
                }
            }
        });
        if group.children.is_empty() {
            let _empty = ui.label("empty");
        }
        // Display order: atoms first, groups at the bottom. Pure view-side —
        // model indices stay untouched so held group paths can never shift.
        let mut order = (0..group.children.len()).collect::<Vec<_>>();
        order.sort_by_key(|&child| group.children[child].term().is_none());
        for child in order {
            let parent_path = path.clone();
            path.push(child);
            render_query_expr(
                ui,
                &group.children[child],
                path,
                Some((parent_path, child)),
                active,
                depth + 1,
                actions,
                tag_kind,
            );
            let _old = path.pop();
        }
    });
    if actions.len() == baseline && primary_click_inside(ui, frame.response.rect) {
        actions.push(QueryAction::Select { path: path.clone() });
    }
}

fn primary_click_inside(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.input(|input| {
        input.pointer.primary_clicked()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| rect.contains(pos))
    })
}

fn render_atom(
    ui: &mut egui::Ui,
    atom: &QueryAtom,
    negated: bool,
    parent: Option<(Vec<usize>, usize)>,
    actions: &mut Vec<QueryAction>,
    tag_kind: &mut impl FnMut(&QueryAtom) -> TagKind,
) {
    let _row = ui.horizontal(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        if let Some((parent, child)) = parent
            && ui
                .small_button("×")
                .on_hover_text("remove from query")
                .clicked()
        {
            actions.push(QueryAction::RemoveChild { parent, child });
        }
        let _label = ui
            .label(tag_chroma::atom(atom, tag_kind(atom), negated))
            .on_hover_text(atom.term());
    });
}

fn op_blurb(op: BoolOp) -> &'static str {
    match op {
        BoolOp::And => "all children must match",
        BoolOp::Or => "any child may match",
        BoolOp::Xor => "exactly one child must match",
    }
}

fn group_title(path: &[usize]) -> String {
    if path.is_empty() {
        return "root".to_owned();
    }
    let dotted = |slots: &[usize]| {
        slots
            .iter()
            .map(|slot| (slot + 1).to_string())
            .collect::<Vec<_>>()
            .join(".")
    };
    if path.len() <= 3 {
        format!("g{}", dotted(path))
    } else {
        // Deep paths keep the tail; the nesting frames carry the rest.
        format!("g…{}", dotted(&path[path.len() - 2..]))
    }
}

fn group_label(path: &[usize], active: bool) -> String {
    if active {
        format!("◆ {}", group_title(path))
    } else {
        format!("◇ {}", group_title(path))
    }
}

/// One hue per nesting depth; fill and stroke are the same hue at different alphas.
const GROUP_HUES: [(u8, u8, u8); 6] = [
    (135, 161, 214),
    (155, 188, 148),
    (198, 164, 118),
    (174, 145, 202),
    (125, 190, 198),
    (203, 136, 145),
];

fn group_hue(depth: usize) -> (u8, u8, u8) {
    GROUP_HUES[depth % GROUP_HUES.len()]
}

fn group_fill(depth: usize) -> egui::Color32 {
    let (r, g, b) = group_hue(depth);
    let alpha = if depth == 0 { 30 } else { 22 };
    egui::Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

fn group_stroke(depth: usize, active: bool) -> egui::Stroke {
    if active {
        return egui::Stroke::new(2.0, chrome::HOT);
    }
    let (r, g, b) = group_hue(depth);
    egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(r, g, b, 96))
}
