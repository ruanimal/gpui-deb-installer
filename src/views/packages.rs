use chrono::{DateTime, Local};
use gpui_kit::component::{
    ActiveTheme, Sizable, Size,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    table::{Column, ColumnSort, DataTable, TableDelegate, TableState},
    v_flex,
};
use gpui_kit::{
    App, AppContext, Context, Edges, Entity, IntoElement, ParentElement, Render, Styled,
    Subscription, WeakEntity, Window, div, prelude::FluentBuilder, px,
};
use std::sync::Arc;

use crate::i18n::tr;
use crate::models::package::InstalledPackage;
use crate::utils::apt;
use crate::views::install::InstallView;

const TIME_FORMAT: &str = "%Y-%m-%d %H:%M";

/// Widths of the fixed columns; whatever is left over from the table viewport
/// goes to the package name column, which is the one that needs the room.
const VERSION_WIDTH: f32 = 175.;
const TIME_WIDTH: f32 = 130.;
/// Left/right padding of the action column: the left value keeps the header
/// text on the same inset as the other columns, the right one keeps the
/// uninstall button clear of the table's scrollbar (which overlays the last
/// 13px of the table).
const ACTION_PADDING_LEFT: f32 = 8.;
const ACTION_PADDING_RIGHT: f32 = 22.;
/// Wide enough for the widest action label ("Uninstall") plus both paddings.
const ACTION_WIDTH: f32 = 102.;
/// Floor for the name column, so it stays readable in a very narrow window.
const NAME_MIN_WIDTH: f32 = 150.;
/// Window padding (`p_4`), the table border, and a little slack so the columns
/// never overflow into a horizontal scrollbar.
const TABLE_INSET: f32 = 36.;

/// Field the table is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Installed,
    Updated,
}

/// View model behind the Installed table.
///
/// It owns the loaded packages and the active filter/sort, so the table can
/// sort itself without reaching back into the view.
pub struct PackagesTableDelegate {
    /// Every package reported by `apt list`.
    all: Vec<InstalledPackage>,
    /// `all` after the search filter and sort order — the rendered rows.
    visible: Vec<InstalledPackage>,
    /// Lower-cased search text.
    query: String,
    sort_key: SortKey,
    sort: ColumnSort,
    loading: bool,
    /// Width available to the table, tracked from the window so the name
    /// column can absorb the leftover space.
    table_width: f32,
    /// Weak handle used by row actions to reach the view.
    view: WeakEntity<PackagesView>,
}

impl PackagesTableDelegate {
    fn new(view: WeakEntity<PackagesView>, table_width: f32) -> Self {
        Self {
            all: Vec::new(),
            visible: Vec::new(),
            query: String::new(),
            sort_key: SortKey::Name,
            sort: ColumnSort::Default,
            loading: true,
            table_width,
            view,
        }
    }

    fn is_loading(&self) -> bool {
        self.loading
    }

    fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    /// Records a new table viewport width; returns whether it changed.
    fn set_table_width(&mut self, width: f32) -> bool {
        if (self.table_width - width).abs() < 1.0 {
            return false;
        }
        self.table_width = width;
        true
    }

    /// Width of the package name column: everything the fixed columns do not
    /// use. Without this the table would leave a dead strip on the right, which
    /// reads as a very wide action column.
    fn name_width(&self) -> f32 {
        let fixed = VERSION_WIDTH + TIME_WIDTH * 2. + ACTION_WIDTH;
        (self.table_width - fixed).max(NAME_MIN_WIDTH)
    }

    /// Replaces the loaded rows and clears the loading state.
    fn set_packages(&mut self, packages: Vec<InstalledPackage>) {
        self.all = packages;
        self.loading = false;
        self.refilter();
    }

    /// Applies the search text.
    fn set_query(&mut self, query: &str) {
        self.query = query.trim().to_lowercase();
        self.refilter();
    }

    /// Column index the current sort key lives in.
    fn column_of(&self, key: SortKey) -> usize {
        match key {
            SortKey::Name => 0,
            SortKey::Installed => 2,
            SortKey::Updated => 3,
        }
    }

    /// Rebuilds `visible` from `all` using the current search text and sort.
    ///
    /// A column in its `Default` state means "no explicit sort", which falls
    /// back to the package name.
    fn refilter(&mut self) {
        let mut rows: Vec<InstalledPackage> = self
            .all
            .iter()
            .filter(|package| matches_query(package, &self.query))
            .cloned()
            .collect();

        let ascending = !matches!(self.sort, ColumnSort::Descending);
        let key = match self.sort {
            ColumnSort::Default => SortKey::Name,
            _ => self.sort_key,
        };

        rows.sort_by(|a, b| {
            let primary = match key {
                SortKey::Name => order_by_name(a, b, ascending),
                SortKey::Installed => order_by_time(a.install_date, b.install_date, ascending),
                SortKey::Updated => order_by_time(a.update_date, b.update_date, ascending),
            };
            primary.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        self.visible = rows;
    }
}

impl TableDelegate for PackagesTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        5
    }

    fn rows_count(&self, _: &App) -> usize {
        self.visible.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        // The table re-reads the columns on every refresh, so the active sort
        // has to be encoded here to keep its header indicator in sync.
        let sorted = |column: Column| {
            if col_ix == self.column_of(self.sort_key) {
                column.sort(self.sort)
            } else {
                column
            }
        };

        match col_ix {
            0 => sorted(
                Column::new("name", tr("packages.header.package"))
                    .width(px(self.name_width()))
                    .min_width(px(NAME_MIN_WIDTH))
                    .sortable(),
            ),
            1 => Column::new("version", tr("packages.header.version"))
                .width(px(VERSION_WIDTH))
                .min_width(px(120.)),
            2 => sorted(
                Column::new("installed", tr("packages.header.installed_time"))
                    .width(px(TIME_WIDTH))
                    .min_width(px(115.))
                    .sortable(),
            ),
            3 => sorted(
                Column::new("updated", tr("packages.header.updated_time"))
                    .width(px(TIME_WIDTH))
                    .min_width(px(115.))
                    .sortable(),
            ),
            // The action column only holds a small button, so it drops most of
            // the default padding.
            _ => Column::new("action", tr("packages.header.action"))
                .width(px(ACTION_WIDTH))
                .min_width(px(ACTION_WIDTH))
                // Custom paddings replace the default cell padding wholesale, so
                // the vertical values repeat the default (4px for the medium
                // table size). Leaving them at 0 lifts this header text a few
                // pixels above the other columns.
                .paddings(Edges {
                    top: px(4.),
                    right: px(ACTION_PADDING_RIGHT),
                    bottom: px(4.),
                    left: px(ACTION_PADDING_LEFT),
                })
                .resizable(false)
                .movable(false)
                .selectable(false),
        }
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        self.sort_key = match col_ix {
            0 => SortKey::Name,
            2 => SortKey::Installed,
            3 => SortKey::Updated,
            _ => return,
        };
        self.sort = sort;
        self.refilter();
        cx.notify();
    }

    fn loading(&self, _: &App) -> bool {
        self.is_loading()
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let message = if self.query.is_empty() {
            tr("packages.empty")
        } else {
            tr("packages.no_match")
        };

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .child(message)
            .into_any_element()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(package) = self.visible.get(row_ix) else {
            return div().into_any_element();
        };

        match col_ix {
            0 => div()
                .overflow_hidden()
                .truncate()
                .child(package.name.clone())
                .into_any_element(),
            1 => div()
                .overflow_hidden()
                .truncate()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(package.version.clone())
                .into_any_element(),
            2 => div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(format_time(package.install_date))
                .into_any_element(),
            3 => div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(format_time(package.update_date))
                .into_any_element(),
            _ => {
                let view = self.view.clone();
                let name = package.name.clone();
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_end()
                    .child(
                        Button::new(("uninstall", row_ix))
                            .danger()
                            .with_size(Size::Small)
                            .label(tr("packages.uninstall"))
                            .on_click(move |_ev, _window, cx| {
                                let name = name.clone();
                                view.update(cx, |view, cx| view.start_uninstall(name, cx))
                                    .ok();
                            }),
                    )
                    .into_any_element()
            }
        }
    }
}

pub struct PackagesView {
    table_state: Entity<TableState<PackagesTableDelegate>>,
    query: Entity<InputState>,
    /// Whether packages that still exist in a repository are listed too.
    include_repo: bool,
    error: Option<String>,
    /// Package name and purge choice pending uninstall confirmation.
    confirm_target: Option<(String, bool)>,
    /// Weak handle to InstallView, used to delegate the uninstall operation.
    pub install_view: Option<WeakEntity<InstallView>>,
    /// Called before uninstall starts — used by AppView to switch to the Install tab.
    pub on_tab_switch: Option<Arc<dyn Fn(&mut App) + 'static>>,
    /// Whether the window-bounds observer has been registered yet.
    tracks_window_bounds: bool,
    _subscriptions: Vec<Subscription>,
}

impl PackagesView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("packages.search_placeholder")));

        let view_handle = cx.weak_entity();
        let table_width = table_width_for(window);
        let table_state = cx.new(|cx| {
            TableState::new(
                PackagesTableDelegate::new(view_handle, table_width),
                window,
                cx,
            )
            .sortable(true)
            .col_resizable(true)
            .col_movable(false)
            .row_selectable(false)
            .col_selectable(false)
        });

        let subscriptions = vec![
            // Re-filter as the user types.
            cx.subscribe_in(
                &query,
                window,
                |view, _input, event: &InputEvent, _window, cx| {
                    if matches!(event, InputEvent::Change) {
                        let text = view.query.read(cx).value().to_string();
                        view.table_state.update(cx, |state, cx| {
                            state.delegate_mut().set_query(&text);
                            state.refresh(cx);
                            cx.notify();
                        });
                    }
                },
            ),
            // Keep the toolbar counters in sync with the table's rows.
            cx.observe(&table_state, |_view, _state, cx| cx.notify()),
        ];

        let mut view = Self {
            table_state,
            query,
            include_repo: false,
            error: None,
            confirm_target: None,
            install_view: None,
            on_tab_switch: None,
            tracks_window_bounds: false,
            _subscriptions: subscriptions,
        };
        view.reload(cx);
        view
    }

    /// Keeps the table's column widths in step with the window size.
    ///
    /// The table columns are fixed-width, so a resized window would otherwise
    /// leave a dead strip on the right instead of widening the package name.
    fn apply_table_width(&mut self, window: &Window, cx: &mut Context<Self>) {
        let width = table_width_for(window);
        self.table_state.update(cx, |state, cx| {
            if state.delegate_mut().set_table_width(width) {
                state.refresh(cx);
                cx.notify();
            }
        });
    }

    /// Reloads the package list from apt on a background thread.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        let include_repo = self.include_repo;

        self.table_state.update(cx, |state, cx| {
            state.delegate_mut().set_loading(true);
            state.refresh(cx);
            cx.notify();
        });

        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { apt::list_installed(include_repo) })
                .await;

            weak.update(cx, |view, cx| {
                let error = result.as_ref().err().map(|err| err.to_string());
                let packages = result.unwrap_or_default();

                view.table_state.update(cx, |state, cx| {
                    state.delegate_mut().set_packages(packages);
                    state.refresh(cx);
                    cx.notify();
                });
                view.error = error;
                view.confirm_target = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_uninstall(&mut self, name: String, cx: &mut Context<Self>) {
        self.confirm_target = Some((name, false));
        cx.notify();
    }

    fn confirm_uninstall(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (name, purge) = match self.confirm_target.take() {
            Some(target) => target,
            None => return,
        };
        cx.notify();

        // Switch to the Install tab so the user sees the streaming log.
        if let Some(ref f) = self.on_tab_switch {
            f(cx);
        }

        // Delegate to InstallView's streaming uninstall logic.
        if let Some(ref install_weak) = self.install_view {
            install_weak
                .update(cx, |install, install_cx| {
                    install.start_uninstall_by_name(name, purge, window, install_cx);
                })
                .ok();
        }
    }

    fn cancel_confirm(&mut self, cx: &mut Context<Self>) {
        self.confirm_target = None;
        cx.notify();
    }
}

impl Render for PackagesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Registered here rather than in `new`, because the observer fires
        // immediately and the entity does not exist yet at that point.
        if !self.tracks_window_bounds {
            self.tracks_window_bounds = true;
            let subscription = cx.observe_window_bounds(window, |view, window, cx| {
                view.apply_table_width(window, cx);
            });
            self._subscriptions.push(subscription);
        }

        let error = self.error.clone();
        let confirm_target = self.confirm_target.clone();
        let include_repo = self.include_repo;
        let (shown, total) = {
            let state = self.table_state.read(cx);
            let delegate = state.delegate();
            (delegate.visible.len(), delegate.all.len())
        };
        let count_label = rust_i18n::t!("packages.count", shown = shown, total = total).to_string();

        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            // Toolbar: search box, refresh, repository toggle, and the counter
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .w(px(230.))
                            .flex_shrink_0()
                            .child(Input::new(&self.query).cleanable(true)),
                    )
                    .child(
                        Button::new("refresh")
                            .with_size(Size::Small)
                            .label(tr("packages.refresh"))
                            .on_click(cx.listener(|view, _ev, _window, cx| view.reload(cx))),
                    )
                    .child(
                        Checkbox::new("include-repo")
                            .label(tr("packages.include_repository"))
                            .checked(include_repo)
                            .on_click(cx.listener(|view, checked: &bool, _window, cx| {
                                view.include_repo = *checked;
                                view.reload(cx);
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(count_label),
                    ),
            )
            // Error banner
            .when_some(error, |el, message| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(cx.theme().danger.opacity(0.1))
                        .border_1()
                        .border_color(cx.theme().danger)
                        .text_color(cx.theme().foreground)
                        .child(
                            rust_i18n::t!("packages.load_failed", err = message.as_str())
                                .to_string(),
                        ),
                )
            })
            // Confirm dialog (inline overlay)
            .when_some(confirm_target, |el, (name, purge)| {
                el.child(
                    h_flex()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(cx.theme().danger.opacity(0.1))
                        .border_1()
                        .border_color(cx.theme().danger)
                        .child(
                            v_flex().flex_1().min_w_0().gap_2()
                                .child(
                                    div().text_color(cx.theme().foreground).child(
                                        rust_i18n::t!(
                                            "packages.remove_package_confirm",
                                            pkg = name.as_str()
                                        )
                                        .to_string(),
                                    ),
                                )
                                .child(
                                    Checkbox::new("purge-config")
                                        .label(tr("common.purge_config"))
                                        .checked(purge)
                                        .on_click(cx.listener(|view, checked: &bool, _window, cx| {
                                            if let Some((_, purge)) = &mut view.confirm_target {
                                                *purge = *checked;
                                            }
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(
                            Button::new("confirm-yes")
                                .danger()
                                .with_size(Size::Small)
                                .label(tr("packages.yes_remove"))
                                .on_click(cx.listener(|view, _ev, window, cx| {
                                    view.confirm_uninstall(window, cx);
                                })),
                        )
                        .child(
                            Button::new("confirm-no")
                                .with_size(Size::Small)
                                .label(tr("packages.cancel"))
                                .on_click(cx.listener(|view, _ev, _window, cx| {
                                    view.cancel_confirm(cx);
                                })),
                        ),
                )
            })
            .child(
                div().flex_1().overflow_hidden().child(
                    DataTable::new(&self.table_state)
                        .stripe(true)
                        .bordered(true),
                ),
            )
    }
}

/// Renders a timestamp, or a dash when dpkg no longer knows it.
fn format_time(time: Option<DateTime<Local>>) -> String {
    time.map(|time| time.format(TIME_FORMAT).to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Width available to the table inside a window of this size.
fn table_width_for(window: &Window) -> f32 {
    (f32::from(window.bounds().size.width) - TABLE_INSET).max(0.)
}

/// Whether a package matches the (already lower-cased) search text.
fn matches_query(package: &InstalledPackage, query: &str) -> bool {
    query.is_empty()
        || package.name.to_lowercase().contains(query)
        || package.version.to_lowercase().contains(query)
}

/// Ascending/descending comparison of package names.
fn order_by_name(
    a: &InstalledPackage,
    b: &InstalledPackage,
    ascending: bool,
) -> std::cmp::Ordering {
    let order = a.name.to_lowercase().cmp(&b.name.to_lowercase());
    if ascending { order } else { order.reverse() }
}

/// Comparison of install/update times; packages with an unknown time sort last
/// in both directions.
fn order_by_time(
    a: Option<DateTime<Local>>,
    b: Option<DateTime<Local>>,
    ascending: bool,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Some(a), Some(b)) => {
            if ascending {
                a.cmp(&b)
            } else {
                b.cmp(&a)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::cmp::Ordering;

    fn package(name: &str, version: &str, install: i64, update: i64) -> InstalledPackage {
        InstalledPackage {
            name: name.to_string(),
            version: version.to_string(),
            architecture: "amd64".to_string(),
            install_date: Some(Local.timestamp_opt(install, 0).unwrap()),
            update_date: Some(Local.timestamp_opt(update, 0).unwrap()),
        }
    }

    fn unknown_time(name: &str) -> InstalledPackage {
        InstalledPackage {
            name: name.to_string(),
            version: "1".to_string(),
            architecture: "all".to_string(),
            install_date: None,
            update_date: None,
        }
    }

    #[test]
    fn search_matches_name_and_version() {
        let package = package("docker-ce", "5:28.3.2", 100, 100);
        assert!(matches_query(&package, ""));
        assert!(matches_query(&package, "docker"));
        assert!(matches_query(&package, "28.3"));
        assert!(!matches_query(&package, "podman"));
    }

    #[test]
    fn names_sort_case_insensitively() {
        let docker = package("Docker", "1", 0, 0);
        let eudic = package("eudic", "1", 0, 0);
        assert_eq!(order_by_name(&docker, &eudic, true), Ordering::Less);
        assert_eq!(order_by_name(&docker, &eudic, false), Ordering::Greater);
    }

    #[test]
    fn times_sort_with_unknown_values_last() {
        let older = package("a", "1", 100, 100);
        let newer = package("b", "1", 200, 200);
        let unknown = unknown_time("c");

        assert_eq!(
            order_by_time(older.install_date, newer.install_date, true),
            Ordering::Less
        );
        assert_eq!(
            order_by_time(older.install_date, newer.install_date, false),
            Ordering::Greater
        );
        // A missing timestamp stays at the bottom in both directions.
        assert_eq!(
            order_by_time(older.install_date, unknown.install_date, true),
            Ordering::Less
        );
        assert_eq!(
            order_by_time(older.install_date, unknown.install_date, false),
            Ordering::Less
        );
    }
}
