use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use gtk::glib::Object;
use gtk::gio::prelude::ListModelExt;
use itertools::Itertools;
use serde::{Serialize, Deserialize};

use crate::capture::{CaptureReader, ItemSource};
use crate::model::GenericModel;
use crate::row_data::ToGenericRowData;

#[derive(Serialize, Deserialize)]
pub enum UiAction {
    Open(PathBuf),
    Update(u64),
    SetExpanded(String, u32, bool),
}

impl std::fmt::Display for UiAction {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use UiAction::*;
        match self {
            Open(path) =>
                write!(f, "Opening file {}", path.display()),
            Update(count) =>
                write!(f, "Updating after {} packets decoded", count),
            SetExpanded(name, position, true) =>
                write!(f, "Expanding {} view, row {}", name, position),
            SetExpanded(name, position, false) =>
                write!(f, "Collapsing {} view, row {}", name, position),
        }
    }
}

pub struct Recording {
    capture: CaptureReader,
    packet_count: u64,
    #[cfg(feature="record-ui-test")]
    action_log: File,
    #[cfg(feature="record-ui-test")]
    output_log: File,
    #[cfg(feature="test-ui-replay")]
    output_log: Option<File>,
    view_items: HashMap<String, Vec<String>>,
}

impl Recording {
    pub fn new(capture: CaptureReader) -> Recording {
        Recording {
            capture,
            packet_count: 0,
            #[cfg(feature="record-ui-test")]
            action_log: File::options()
                .write(true)
                .create(true)
                .truncate(true)
                .open("actions.json")
                .expect("Failed to open UI action log file"),
            #[cfg(feature="record-ui-test")]
            output_log: File::options()
                .write(true)
                .create(true)
                .truncate(true)
                .open("output.txt")
                .expect("Failed to open UI output log file"),
            #[cfg(feature="test-ui-replay")]
            output_log: None,
            view_items: HashMap::new(),
        }
    }

    #[cfg(feature="test-ui-replay")]
    pub fn set_output(&mut self, file: File) {
        self.output_log = Some(file)
    }

    fn log_action(&mut self, action: UiAction) {
        #[cfg(feature="record-ui-test")]
        self.action_log
            .write_all(
                format!("{}\n",
                    serde_json::to_string(&action)
                        .expect("Failed to serialise UI action")
                ).as_bytes())
            .expect("Failed to write to UI action log");

        if let UiAction::SetExpanded(ref name, position, _) = action {
            let summary = self.summary(&name, position);
            self.log_output(format!("{}: {}\n", action, summary));
        } else {
            self.log_output(format!("{}\n", action));
        }
    }

    fn log_output(&mut self, string: String) {
        #[cfg(feature="record-ui-test")]
        let output_log = &mut self.output_log;
        #[cfg(feature="test-ui-replay")]
        let output_log = self.output_log
            .as_mut()
            .expect("Recording has no output file set");

        output_log
            .write_all(string.as_bytes())
            .expect("Failed to write to UI output log");
    }

    pub fn log_open_file(&mut self,
                         path: &PathBuf, 
                         capture: &CaptureReader)
    {
        self.log_action(UiAction::Open(path.clone()));
        self.capture = capture.clone();
        self.packet_count = 0;
        self.view_items.clear()
    }

    pub fn log_update(&mut self, packet_count: u64) {
        if packet_count > self.packet_count {
            self.log_action(UiAction::Update(packet_count));
            self.packet_count = packet_count;
        }
    }

    pub fn log_item_expanded(
        &mut self,
        name: &str,
        position: u32,
        expanded: bool)
    {
        let name = name.to_string();
        self.log_action(UiAction::SetExpanded(name, position, expanded));
    }

    pub fn log_item_updated(
        &mut self,
        name: &str,
        position: u32,
        new_summary: String)
    {
        let old_summary = self.summary(name, position).to_string();
        if new_summary != old_summary {
            self.log_output(format!("At {} row {}:\n", name, position));
            self.log_output(format!("- {}\n", old_summary));
            self.log_output(format!("+ {}\n", new_summary));
        }
    }

    pub fn log_items_changed<Model, Item>(
        &mut self,
        name: &str,
        model: &Model,
        position: u32,
        removed: u32,
        added: u32)
    where
        Model: ListModelExt + GenericModel<Item>,
        CaptureReader: ItemSource<Item>,
        Object: ToGenericRowData<Item>,
        Item: Copy
    {
        if (removed, added) == (0, 0) {
            return;
        }
        let added_range = position..(position + added);
        let position = position as usize;
        let removed = removed as usize;
        let removed_range = position..(position + removed);
        let added_items: Vec<String> = added_range
            .clone()
            .map(|i| self.item_text(model, i))
            .collect();
        let removed_items: Vec<String> = self.view_items
            .entry(name.to_string())
            .or_insert_with(Vec::new)
            .splice(removed_range, added_items.clone())
            .collect();
        self.log_output(format!("At {} row {}:\n", name, position));
        for (n, string) in removed_items.iter().dedup_with_count() {
            if n == 1 {
                self.log_output(format!("- {}\n", string));
            } else {
                self.log_output(format!("- {} times: {}\n", n, string));
            }
        }
        for (n, string) in added_items.iter().dedup_with_count() {
            if n == 1 {
                self.log_output(format!("+ {}\n", string));
            } else {
                self.log_output(format!("+ {} times: {}\n", n, string));
            }
        }
    }

    fn item_text<Model, Item>(&mut self,
                              model: &Model,
                              position: u32) -> String
        where Model: ListModelExt + GenericModel<Item>,
              CaptureReader: ItemSource<Item>,
              Object: ToGenericRowData<Item>,
              Item: Copy
    {
        let item = model
            .item(position)
            .expect("Failed to retrieve row data")
            .to_generic_row_data()
            .node()
            .expect("Failed to fetch item node from row data")
            .borrow()
            .item;
        self.capture
            .summary(&item)
            .expect("Failed to generate item summary")
    }

    fn summary(&self, name: &str, position: u32) -> &str {
        &self.view_items
            .get(name)
            .expect("Recording has no items for model")
            .get(position as usize)
            .expect("Recording has no summary for row")
    }
}
