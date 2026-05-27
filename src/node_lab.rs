use egui::{Color32, RichText};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct NodeParam {
    pub name: String,
    pub value: f32,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NodeDef {
    pub id: u64,
    pub title: String,
    pub x: f32,
    pub y: f32,
    pub params: Vec<NodeParam>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct NodeGraph {
    pub nodes: Vec<NodeDef>,
}

#[derive(Default)]
pub struct NodeLabState {
    pub open: bool,
    pub graph: NodeGraph,
    next_id: u64,
    selected_id: Option<u64>,
    new_param_name: String,
}

impl NodeLabState {
    pub fn ensure_seed_nodes(&mut self) {
        if !self.graph.nodes.is_empty() {
            return;
        }
        self.next_id = 3;
        self.graph.nodes.push(NodeDef {
            id: 1,
            title: "Noise".to_owned(),
            x: 32.0,
            y: 48.0,
            params: vec![
                NodeParam { name: "Scale".to_owned(), value: 1.0 },
                NodeParam { name: "Seed".to_owned(), value: 42.0 },
            ],
        });
        self.graph.nodes.push(NodeDef {
            id: 2,
            title: "Colorize".to_owned(),
            x: 280.0,
            y: 120.0,
            params: vec![
                NodeParam { name: "Hue".to_owned(), value: 0.5 },
                NodeParam { name: "Contrast".to_owned(), value: 1.0 },
            ],
        });
        self.selected_id = Some(1);
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        self.ensure_seed_nodes();

        egui::Window::new("Node Lab (Texture/Image Generator)")
            .open(&mut self.open)
            .resizable(true)
            .min_size([560.0, 360.0])
            .default_size([820.0, 620.0])
            .max_size([1800.0, 1400.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("+ Add node").clicked() {
                        let id = self.next_id.max(1);
                        self.next_id = id + 1;
                        self.graph.nodes.push(NodeDef {
                            id,
                            title: format!("Node {id}"),
                            x: 80.0,
                            y: 80.0,
                            params: vec![NodeParam { name: "Value".to_owned(), value: 1.0 }],
                        });
                        self.selected_id = Some(id);
                    }

                    if ui.button("Delete selected").clicked() {
                        if let Some(id) = self.selected_id {
                            self.graph.nodes.retain(|n| n.id != id);
                            self.selected_id = self.graph.nodes.first().map(|n| n.id);
                        }
                    }

                    if ui.button("Auto layout").clicked() {
                        for (i, n) in self.graph.nodes.iter_mut().enumerate() {
                            let col = (i % 3) as f32;
                            let row = (i / 3) as f32;
                            n.x = 48.0 + col * 220.0;
                            n.y = 54.0 + row * 130.0;
                        }
                    }

                    ui.label(
                        RichText::new("Editable graph scaffold: select, rename, reposition, and parameterize nodes.")
                            .color(Color32::from_rgb(130, 130, 130))
                            .small(),
                    );
                });

                ui.separator();
                ui.columns(2, |cols| {
                    cols[0].heading("Nodes");
                    egui::ScrollArea::vertical()
                        .id_salt("node_lab_nodes_list")
                        .max_height((cols[0].available_height() - 8.0).max(80.0))
                        .show(&mut cols[0], |ui| {
                            for n in &self.graph.nodes {
                                let sel = self.selected_id == Some(n.id);
                                let label = format!("#{}  {}", n.id, n.title);
                                if ui.selectable_label(sel, label).clicked() {
                                    self.selected_id = Some(n.id);
                                }
                            }
                        });

                    cols[1].heading("Inspector");
                    if let Some(id) = self.selected_id {
                        if let Some(node_idx) = self.graph.nodes.iter().position(|n| n.id == id) {
                            let node = &mut self.graph.nodes[node_idx];

                            cols[1].horizontal(|ui| {
                                ui.label("Title");
                                ui.text_edit_singleline(&mut node.title);
                            });
                            cols[1].horizontal(|ui| {
                                ui.label("x");
                                ui.add(egui::DragValue::new(&mut node.x).speed(0.5));
                                ui.label("y");
                                ui.add(egui::DragValue::new(&mut node.y).speed(0.5));
                            });
                            cols[1].separator();
                            cols[1].label(RichText::new("Parameters").strong());

                            egui::ScrollArea::vertical()
                                .id_salt(("node_lab_params", node.id))
                                .max_height((cols[1].available_height() - 34.0).max(64.0))
                                .show(&mut cols[1], |ui| {
                                    let mut remove_idx: Option<usize> = None;
                                    for (i, p) in node.params.iter_mut().enumerate() {
                                        ui.horizontal(|ui| {
                                            ui.text_edit_singleline(&mut p.name);
                                            ui.add(egui::Slider::new(&mut p.value, -10.0..=10.0));
                                            if ui.small_button("x").clicked() {
                                                remove_idx = Some(i);
                                            }
                                        });
                                    }
                                    if let Some(i) = remove_idx {
                                        node.params.remove(i);
                                    }
                                });

                            cols[1].horizontal(|ui| {
                                ui.label("New param");
                                ui.text_edit_singleline(&mut self.new_param_name);
                                if ui.button("+ Add").clicked() {
                                    let nm = self.new_param_name.trim();
                                    if !nm.is_empty() {
                                        node.params.push(NodeParam { name: nm.to_owned(), value: 0.0 });
                                        self.new_param_name.clear();
                                    }
                                }
                            });
                        } else {
                            self.selected_id = self.graph.nodes.first().map(|n| n.id);
                            cols[1].label("Selected node no longer exists.");
                        }
                    } else {
                        cols[1].label("Select a node to edit its fields and parameters.");
                    }
                });
            });
    }
}
