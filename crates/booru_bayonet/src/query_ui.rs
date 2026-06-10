use eframe::egui;

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
    let frame = egui::Frame::group(ui.style())
        .fill(group_fill(depth))
        .stroke(group_stroke(depth, active_here));
    let frame = frame.show(ui, |ui| {
        let _header = ui.horizontal_wrapped(|ui| {
            if ui
                .add(chrome::glyph_button(
                    group_label(path, active_here),
                    active_here,
                ))
                .on_hover_text("click to select this group for new tags")
                .clicked()
            {
                actions.push(QueryAction::Select { path: path.clone() });
            }
            if ui.add(chrome::glyph_button("¬", negated)).clicked() {
                actions.push(QueryAction::ToggleNot { path: path.clone() });
            }
            for op in BoolOp::ALL {
                if ui
                    .add(chrome::glyph_button(op.label(), group.op == op))
                    .clicked()
                {
                    actions.push(QueryAction::SetOp {
                        path: path.clone(),
                        op,
                    });
                }
            }
            if let Some((parent, child)) = parent.as_ref()
                && ui.small_button("×").clicked()
            {
                actions.push(QueryAction::RemoveChild {
                    parent: parent.clone(),
                    child: *child,
                });
            }
        });
        if group.children.is_empty() {
            let _empty = ui.label("empty");
        }
        for (child, expr) in group.children.iter().enumerate() {
            let parent_path = path.clone();
            path.push(child);
            render_query_expr(
                ui,
                expr,
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
        if let Some((parent, child)) = parent
            && ui.small_button("×").clicked()
        {
            actions.push(QueryAction::RemoveChild { parent, child });
        }
        let _label = ui.label(tag_chroma::atom(atom, tag_kind(atom), negated));
    });
}

fn group_title(path: &[usize]) -> String {
    if path.is_empty() {
        "root".to_owned()
    } else {
        format!(
            "g{}",
            path.iter()
                .map(|slot| (slot + 1).to_string())
                .collect::<Vec<_>>()
                .join(".")
        )
    }
}

fn group_label(path: &[usize], active: bool) -> String {
    if active {
        format!("◆ {}", group_title(path))
    } else {
        format!("◇ {}", group_title(path))
    }
}

fn group_fill(depth: usize) -> egui::Color32 {
    const PALETTE: [(u8, u8, u8); 6] = [
        (86, 105, 143),
        (107, 128, 104),
        (135, 111, 83),
        (116, 93, 132),
        (84, 128, 133),
        (137, 91, 98),
    ];
    let (r, g, b) = PALETTE[depth % PALETTE.len()];
    let alpha = if depth == 0 { 40 } else { 30 };
    egui::Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

fn group_stroke(depth: usize, active: bool) -> egui::Stroke {
    const PALETTE: [(u8, u8, u8); 6] = [
        (135, 161, 214),
        (155, 188, 148),
        (198, 164, 118),
        (174, 145, 202),
        (125, 190, 198),
        (203, 136, 145),
    ];
    let (r, g, b) = PALETTE[depth % PALETTE.len()];
    if active {
        egui::Stroke::new(2.0, chrome::HOT)
    } else {
        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(r, g, b, 96))
    }
}
