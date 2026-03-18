use gpui::{
    AppContext, AsyncWindowContext, ClipboardItem, Context, Entity, IntoElement, ParentElement, Render,
    Styled, Subscription, VisualContext, WeakEntity, Window, div, img, px, SharedString,
};
use gpui_component::{
    ActiveTheme,
    h_flex, v_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    menu::{ContextMenuExt, PopupMenuItem},
    resizable::{h_resizable, resizable_panel, ResizableState},
    tree::{TreeItem, TreeState, tree},
    IconName,
};
use std::path::PathBuf;

use crate::i18n::tr;
use crate::utils::deb_files::{DebFileEntry, DebFileKind, extract_previewable_files};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub enum FilesLoadState {
    Idle,
    Loading,
    Loaded(Vec<DebFileEntry>),
    Error,
}

pub struct FilesPreviewView {
    load_state: FilesLoadState,
    /// Last deb path we triggered a load for (used to avoid duplicate loads)
    last_loaded_path: Option<PathBuf>,
    tree_state: Entity<TreeState>,
    /// Resizable panel state for the left/right split
    resizable_state: Entity<ResizableState>,
    /// Code editor for text file preview (recreated per selected file)
    editor_state: Entity<InputState>,
    /// The currently selected file (None = nothing selected)
    selected: Option<DebFileEntry>,
    /// Search input state for filtering tree
    search_state: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl FilesPreviewView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree_state = cx.new(|cx| TreeState::new(cx));
        let resizable_state = cx.new(|_cx| ResizableState::default());
        // Create a default code-editor InputState; we recreate it on each file selection
        let editor_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("text")
                .line_number(true)
        });

        // Search Input State
        let search_state = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("files_preview.select_first"))
        });

        let mut view = Self {
            load_state: FilesLoadState::Idle,
            last_loaded_path: None,
            tree_state,
            resizable_state,
            editor_state,
            selected: None,
            search_state: search_state.clone(),
            _subscriptions: Vec::new(),
        };

        view._subscriptions.push(cx.subscribe(
            &search_state,
            |view: &mut Self, _emitter, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    view.update_tree_filter(cx);
                }
            },
        ));

        view
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    /// Called from AppView when a new .deb is selected.
    /// Avoids duplicate loads for the same path.
    pub fn trigger_load(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.last_loaded_path.as_ref() == Some(&path) {
            return; // already loaded this file
        }
        self.last_loaded_path = Some(path.clone());
        self.load_state = FilesLoadState::Loading;
        self.selected = None;

        // Reset search
        self.search_state.update(cx, |s, cx| {
            s.set_value(String::new(), window, cx);
            s.set_placeholder(tr("files_preview.loading"), window, cx);
        });

        // Clear tree
        self.tree_state.update(cx, |state, cx| state.set_items(vec![], cx));

        // Reset editor
        self.editor_state.update(cx, |s, cx| {
            s.set_value(String::new(), window, cx);
        });

        cx.notify();

        cx.spawn_in(window, async move |weak, cx| {
            load_files_async(path, weak, cx).await;
        })
        .detach();
    }

    /// Called when a leaf tree item is clicked.
    fn select_file(&mut self, entry_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let entries = match &self.load_state {
            FilesLoadState::Loaded(e) => e,
            _ => return,
        };

        let found = entries.iter().find(|e| e.path == entry_id).cloned();
        let Some(file) = found else { return };

        // Load text content into the editor
        if let DebFileKind::Text(text) = &file.kind {
            let lang = detect_language(entry_id);
            let text = text.clone();
            // Create a fresh InputState with the correct language
            let new_editor = cx.new(|cx| {
                InputState::new(window, cx)
                    .code_editor(lang)
                    .line_number(true)
                    .default_value(text)
            });
            self.editor_state = new_editor;
        }

        self.selected = Some(file);
        cx.notify();
    }

    fn update_tree_filter(&mut self, cx: &mut Context<Self>) {
        let search_text = self.search_state.read(cx).text().to_string().to_lowercase();
        let entries = match &self.load_state {
            FilesLoadState::Loaded(e) => e,
            _ => return,
        };

        let items = if search_text.is_empty() {
            build_tree_items_clean(entries)
        } else {
            let filtered: Vec<DebFileEntry> = entries
                .iter()
                .filter(|e| e.path.to_lowercase().contains(&search_text))
                .cloned()
                .collect();
            build_tree_items_clean(&filtered)
        };

        self.tree_state.update(cx, |state, cx| {
            state.set_items(items, cx);
        });
        cx.notify();
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

impl Render for FilesPreviewView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree_state = &self.tree_state;
        let view: Entity<FilesPreviewView> = cx.entity();

        div()
            .size_full()
            .child(h_resizable("files-preview-split")
            .with_state(&self.resizable_state)
            // ── Left panel: file tree ─────────────────────────────────────
            .child(
                resizable_panel()
                    .size(px(240.))
                    .size_range(px(120.)..px(480.))
                    .child(
                        v_flex()
                            .size_full()
                            .border_r_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().sidebar)
                            // Search Box
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .flex_col()
                                    .px_2()
                                    .py_2()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .child(
                                        Input::new(&self.search_state)
                                            .w_full()
                                            .cleanable(true)
                                    )
                            )
                            // Tree
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .child(tree(tree_state, {
                                        move |ix, entry, selected, _window, _cx| {
                                            let item = entry.item();
                                            let depth = entry.depth();
                                            let is_folder = entry.is_folder();
                                            let is_expanded = entry.is_expanded();
                                            let item_id = item.id.clone();
                                            let full_path = item_id.to_string();

                                            let icon = if is_folder {
                                                if is_expanded {
                                                    IconName::FolderOpen
                                                } else {
                                                    IconName::Folder
                                                }
                                            } else {
                                                IconName::File
                                            };

                                            let indent = px(12.) + px(16.) * depth as f32;

                                            ListItem::new(ix)
                                                .selected(selected)
                                                .pl(indent)
                                                .child(
                                                    h_flex()
                                                        .gap_1()
                                                        .items_center()
                                                        .child(icon)
                                                        .child(item.label.clone())
                                                        .context_menu({
                                                            let path = full_path.clone();
                                                            move |menu, _window, _cx| {
                                                                let path = path.clone();
                                                                menu.item(
                                                                    PopupMenuItem::new(tr("files_preview.copy_path"))
                                                                        .on_click(move |_: &gpui::ClickEvent, _window, cx| {
                                                                            cx.write_to_clipboard(
                                                                                ClipboardItem::new_string(path.clone()),
                                                                            );
                                                                        })
                                                                )
                                                            }
                                                        }),
                                                )
                                                .on_click({
                                                    let view = view.clone();
                                                    let id = item_id.to_string();
                                                    move |_, window, cx| {
                                                        if !is_folder {
                                                            view.update(cx, |this, cx| {
                                                                this.select_file(&id, window, cx);
                                                            });
                                                        }
                                                    }
                                                })
                                        }
                                    })),
                            ),
                    ),
            )
            // ── Right panel: preview ──────────────────────────────────────
            .child(
                resizable_panel()
                    .size_range(px(200.)..px(f32::MAX))
                    .child(
                        v_flex()
                            .size_full()
                            .overflow_hidden()
                            .child(self.render_preview(cx)),
                    ),
            )
            )
    }
}

impl FilesPreviewView {
    fn render_preview(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        match &self.selected {
            None => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child(tr("files_preview.select_left"))
                .into_any_element(),

            Some(file) => match &file.kind {
                DebFileKind::Text(_) => div()
                    .flex_1()
                    .size_full()
                    .child(
                        Input::new(&self.editor_state)
                            .h_full()
                            .disabled(true)
                            .appearance(false)
                            .bg(cx.theme().background)
                    )
                    .into_any_element(),

                DebFileKind::Image(bytes) => render_image_preview(&file.path, bytes, cx),

                DebFileKind::Unsupported => {
                    let path = file.path.clone();
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("files_preview.unsupported_preview")),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(path),
                        )
                        .into_any_element()
                }
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Async helper
// ---------------------------------------------------------------------------

async fn load_files_async(
    path: PathBuf,
    weak: WeakEntity<FilesPreviewView>,
    cx: &mut AsyncWindowContext,
) {
    let result = cx
        .background_executor()
        .spawn(async move { extract_previewable_files(&path) })
        .await;

    match result {
        Ok(entries) => {
            let search_state = weak.read_with(cx, |v, _| v.search_state.clone()).ok();
            let count = entries.len();

            weak.update(cx, |view, cx| {
                view.load_state = FilesLoadState::Loaded(entries);
                view.selected = None;
                view.update_tree_filter(cx);
            })
            .ok();

            if let Some(state) = search_state {
                cx.update_window_entity(&state, |s: &mut InputState, w, c| {
                    s.set_placeholder(SharedString::from(rust_i18n::t!("files_preview.search_files", count = count).to_string()), w, c)
                }).ok();
            }
        }
        Err(e) => {
            let search_state = weak.read_with(cx, |v, _| v.search_state.clone()).ok();
            let err_msg = e.to_string();

            weak.update(cx, |view, cx| {
                view.load_state = FilesLoadState::Error;
                cx.notify();
            })
            .ok();

            if let Some(state) = search_state {
                cx.update_window_entity(&state, |s: &mut InputState, w, c| {
                    s.set_placeholder(SharedString::from(rust_i18n::t!("files_preview.error", err = err_msg.as_str()).to_string()), w, c)
                }).ok();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tree building (two-pass, parent-before-child order)
// ---------------------------------------------------------------------------

fn build_tree_items_clean(entries: &[DebFileEntry]) -> Vec<TreeItem> {
    use std::collections::{BTreeMap, BTreeSet};

    // Collect all implied directory paths
    let mut all_dirs: BTreeSet<String> = BTreeSet::new();
    all_dirs.insert(String::new()); // root

    for entry in entries {
        let clean = entry.path.trim_start_matches("./");
        let parts: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
        let mut cum = String::new();
        for (i, p) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                break;
            }
            if !cum.is_empty() {
                cum.push('/');
            }
            cum.push_str(p);
            all_dirs.insert(cum.clone());
        }
    }

    // child_map: parent_clean_path → Vec<(label, is_dir, id)>
    let mut child_map: BTreeMap<String, Vec<(String, bool, String)>> = BTreeMap::new();

    // Directory entries
    for dir in &all_dirs {
        if dir.is_empty() {
            continue;
        }
        let parent = match dir.rfind('/') {
            Some(p) => dir[..p].to_string(),
            None => String::new(),
        };
        let name = match dir.rfind('/') {
            Some(p) => dir[p + 1..].to_string(),
            None => dir.clone(),
        };
        child_map
            .entry(parent)
            .or_default()
            .push((name, true, dir.clone()));
    }

    // File entries
    for entry in entries {
        let clean = entry.path.trim_start_matches("./").to_string();
        let parts: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }
        let name = parts.last().unwrap().to_string();
        let parent = if parts.len() == 1 {
            String::new()
        } else {
            parts[..parts.len() - 1].join("/")
        };
        // Use the original path (with ./) as the id so select_file can look it up
        child_map
            .entry(parent)
            .or_default()
            .push((name, false, entry.path.clone()));
    }

    // Sort: dirs first, then alphabetically by name
    for children in child_map.values_mut() {
        children.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    }

    // Recursively build TreeItems
    fn build(
        parent: &str,
        child_map: &BTreeMap<String, Vec<(String, bool, String)>>,
    ) -> Vec<TreeItem> {
        let Some(children) = child_map.get(parent) else {
            return vec![];
        };
        children
            .iter()
            .map(|(name, is_dir, full_path)| {
                if *is_dir {
                    let mut current_name = name.clone();
                    let mut current_path = full_path.clone();

                    // Compact folder logic: if a directory has exactly one child AND that child is also a directory, merge them
                    while let Some(sub_children) = child_map.get(&current_path) {
                        if sub_children.len() == 1 && sub_children[0].1 {
                            current_name = format!("{}/{}", current_name, sub_children[0].0);
                            current_path = sub_children[0].2.clone();
                        } else {
                            break;
                        }
                    }

                    let sub = build(&current_path, child_map);
                    TreeItem::new(current_path.clone(), current_name)
                        .children(sub)
                        .expanded(true)
                } else {
                    TreeItem::new(full_path.clone(), name.clone())
                }
            })
            .collect()
    }

    build("", &child_map)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn render_image_preview(path: &str, bytes: &[u8], cx: &mut Context<FilesPreviewView>) -> gpui::AnyElement {
    use std::io::Write;

    // Use a unique filename in the temp directory based on the file path to avoid conflicts
    let hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        hasher.finish()
    };
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("tmp");

    let tmp = std::env::temp_dir().join(format!("gpui_deb_preview_{:x}.{}", hash, extension));

    match std::fs::File::create(&tmp).and_then(|mut f| f.write_all(bytes)) {
        Ok(_) => div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .child(
                img(tmp)
                    .max_w_full()
                    .max_h_full(),
            )
            .into_any_element(),
        Err(e) => div()
            .text_color(cx.theme().danger)
            .child(rust_i18n::t!("files_preview.render_image_failed", err = e.to_string()).to_string())
            .into_any_element(),
    }
}

fn detect_language(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "sh" | "bash" | "zsh" => "bash",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp",
        "go" => "go",
        "java" => "java",
        "xml" | "html" | "htm" | "xhtml" => "html",
        "css" | "scss" | "sass" => "css",
        "md" | "markdown" => "markdown",
        "sql" => "sql",
        "lua" => "lua",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "r" => "r",
        "ini" | "cfg" | "conf" | "properties" => "ini",
        "dockerfile" => "dockerfile",
        "makefile" | "mk" => "makefile",
        _ => "text",
    }
}
