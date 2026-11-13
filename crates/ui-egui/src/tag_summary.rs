//! The read-only Tags Summary pane. Query scope and note identity belong to the engine.

use crate::theme::Tokens;
use egui::Ui;
use notecraft_engine::{Output, Params, Session};
use notecraft_store::search::TagHit;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Scope {
    Page,
    Section,
    Notebook,
    #[default]
    All,
}

impl Scope {
    fn id(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Section => "section",
            Self::Notebook => "notebook",
            Self::All => "all",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Page => crate::tl_id!("ui.tags_scope_page"),
            Self::Section => crate::tl_id!("ui.tags_scope_section"),
            Self::Notebook => crate::tl_id!("ui.tags_scope_notebook"),
            Self::All => crate::tl_id!("ui.tags_scope_all"),
        }
    }
}

#[derive(Default)]
pub struct Summary {
    pub scope: Scope,
    incomplete: bool,
    pub page: Option<PathBuf>,
    hits: Vec<TagHit>,
    error: Option<String>,
}

impl Summary {
    pub fn fail(&mut self, error: String) {
        self.hits.clear();
        self.error = Some(error);
    }

    pub fn follows_page(&self) -> bool {
        self.scope != Scope::All
    }

    pub fn refresh(&mut self, session: &mut Session, page: Option<&Path>) {
        self.page = page.map(Path::to_path_buf);
        let mut params = Params::from([("scope".into(), self.scope.id().into()), ("incomplete".into(), self.incomplete.to_string())]);
        if let Some(path) = page {
            params.insert("path".into(), path.to_string_lossy().into_owned());
        } else if self.scope != Scope::All {
            self.hits.clear();
            self.error = Some(crate::tl_id!("ui.open_a_page_first").to_owned());
            return;
        }
        match session.run("tag.find", &params) {
            Ok(Output::Tags(hits)) => {
                self.hits = hits;
                self.error = None;
            }
            Ok(_) => {
                self.hits.clear();
                self.error = Some(crate::tl_id!("ui.tags_query_failed").to_owned());
            }
            Err(error) => {
                self.hits.clear();
                self.error = Some(error.to_string());
            }
        }
    }
}

#[derive(Default)]
pub struct Action {
    pub close: bool,
    pub refresh: bool,
    pub target: Option<TagHit>,
}

pub fn show(ui: &mut Ui, summary: &mut Summary, tokens: &Tokens) -> Action {
    let mut action = Action::default();
    egui::Panel::right("tags-summary").resizable(true).default_size(340.0).size_range(260.0..=600.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(crate::tl_id!("ui.tags_summary"));
            action.close = ui.button(crate::tl_id!("ui.close_tags_summary")).clicked();
        });
        let before = summary.scope;
        egui::ComboBox::from_label(crate::tl_id!("ui.find_tags_in")).selected_text(summary.scope.label()).show_ui(ui, |ui| {
            for scope in [Scope::Page, Scope::Section, Scope::Notebook, Scope::All] {
                ui.selectable_value(&mut summary.scope, scope, scope.label());
            }
        });
        action.refresh |= summary.scope != before;
        action.refresh |= ui.checkbox(&mut summary.incomplete, crate::tl_id!("ui.only_incomplete_todo")).changed();
        action.refresh |= ui.button(crate::tl_id!("ui.refresh_tags")).clicked();
        ui.separator();
        if let Some(error) = &summary.error {
            ui.label(egui::RichText::new(error).color(tokens.warn));
            return;
        }
        ui.label(crate::i18n::format_message_id("ui.tagged_notes_count", &[("count", &summary.hits.len().to_string())]));
        if summary.hits.is_empty() {
            ui.weak(crate::tl_id!("ui.no_tagged_notes"));
        }
        egui::ScrollArea::vertical().id_salt("tag-results").show(ui, |ui| {
            for hit in &summary.hits {
                ui.push_id((&hit.path, hit.block, hit.tag.id()), |ui| {
                    let label = format!("{}: {}", crate::tl!(hit.tag.label()), hit.text);
                    if ui.add(egui::Button::new(label).wrap()).clicked() {
                        action.target = Some(hit.clone());
                    }
                    ui.label(egui::RichText::new(format!("{} / {} / {}", hit.notebook, hit.section, hit.title)).small().color(tokens.muted));
                    if let Some(done) = hit.done {
                        ui.weak(if done { crate::tl_id!("ui.tag_task_completed") } else { crate::tl_id!("ui.tag_task_incomplete") });
                    }
                    ui.separator();
                });
            }
        });
    });
    action
}
