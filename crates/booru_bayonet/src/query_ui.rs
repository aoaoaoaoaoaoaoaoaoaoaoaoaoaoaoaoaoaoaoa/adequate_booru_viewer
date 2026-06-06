use eframe::egui;

use crate::model::{BoolGroup, BoolOp, QueryAtom, QueryExpr};

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
) {
    let mut path = Vec::new();
    render_query_expr(ui, root, &mut path, None, active, 0, actions);
}

fn render_query_expr(
    ui: &mut egui::Ui,
    expr: &QueryExpr,
    path: &mut Vec<usize>,
    parent: Option<(Vec<usize>, usize)>,
    active: &[usize],
    depth: usize,
    actions: &mut Vec<QueryAction>,
) {
    let (negated, core) = expr.denote();
    match core {
        QueryExpr::Atom { atom } => render_atom(ui, atom, negated, parent, actions),
        QueryExpr::Group { group } => {
            render_group(ui, group, negated, path, parent, active, depth, actions);
        }
        QueryExpr::Not { child } => {
            render_query_expr(ui, child, path, parent, active, depth, actions);
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
) {
    let active_here = path.as_slice() == active;
    let frame = egui::Frame::group(ui.style())
        .fill(group_fill(depth))
        .stroke(group_stroke(depth, active_here));
    let _frame = frame.show(ui, |ui| {
        let _header = ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(active_here, group_title(path))
                .clicked()
            {
                actions.push(QueryAction::Select { path: path.clone() });
            }
            if ui.selectable_label(negated, "NOT").clicked() {
                actions.push(QueryAction::ToggleNot { path: path.clone() });
            }
            for op in BoolOp::ALL {
                if ui.selectable_label(group.op == op, op.label()).clicked() {
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
            );
            let _old = path.pop();
        }
    });
}

fn render_atom(
    ui: &mut egui::Ui,
    atom: &QueryAtom,
    negated: bool,
    parent: Option<(Vec<usize>, usize)>,
    actions: &mut Vec<QueryAction>,
) {
    let text = if negated {
        format!("¬ {atom}")
    } else {
        format!("+ {atom}")
    };
    let color = if negated {
        egui::Color32::from_rgb(218, 150, 146)
    } else {
        egui::Color32::from_rgb(156, 204, 176)
    };
    let _row = ui.horizontal(|ui| {
        if let Some((parent, child)) = parent
            && ui.small_button("×").clicked()
        {
            actions.push(QueryAction::RemoveChild { parent, child });
        }
        let _label = ui.label(egui::RichText::new(text).color(color));
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
    egui::Color32::from_rgba_unmultiplied(r, g, b, 28)
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
        egui::Stroke::new(2.0, egui::Color32::from_rgb(r, g, b))
    } else {
        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(r, g, b, 96))
    }
}
