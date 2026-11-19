use super::*;

impl Workspace {
    /// The workspace column: the window's only tab surface.
    ///
    /// It runs the full height of the window, including the corner the bar would
    /// otherwise own, so the list opens onto the mark that toggles it. Width is
    /// the whole state — below `RAIL_THRESHOLD` the list draws as a rail of
    /// chips, which is how it collapses without a second control to learn.
    pub(super) fn render_sidebar(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = self.sidebar_width;
        let rail = width < RAIL_THRESHOLD;
        let wash = theme.muted;

        // The mark and the fold control. The mark is the app's identity and the
        // switch for this column in one object: it sits in the corner the column
        // opens onto, so the thing you press is next to the thing it opens.
        let mut top = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .h(px(TITLE_BAR_HEIGHT))
            .gap(px(6.0))
            .when(!rail, |this| this.pl(px(14.0)).pr(px(10.0)))
            .when(rail, |this| this.justify_center())
            .child(
                div()
                    .id("toggle-rail")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.0))
                    .rounded(px(9.0))
                    .bg(theme.ring.opacity(0.13))
                    .cursor_pointer()
                    .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                        workspace.toggle_rail(cx);
                    }))
                    .child(icon_data(APP_MARK, 15.0, theme.ring)),
            );
        if !rail {
            top = top
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("myterm"),
                )
                .child(div().flex_1())
                .child(
                    icon_button("sidebar-search", SEARCH_ICON, 26.0, theme, cx).on_click(
                        cx.listener(|workspace, _: &ClickEvent, window, cx| {
                            workspace.open_palette(window, cx);
                        }),
                    ),
                )
                .child(
                    icon_button("sidebar-new", PLUS_SMALL, 26.0, theme, cx).on_click(cx.listener(
                        |workspace, _: &ClickEvent, window, cx| {
                            workspace.new_tab(window, cx);
                        },
                    )),
                );
        }

        // Every workspace and pane, with enough state to jump to one.
        //
        // This is the whole model the list reads: it is rebuilt from the
        // workspace each frame rather than kept in step with it, so there is
        // nothing to keep in sync.
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .px(px(8.0))
            .pt(px(12.0))
            .pb(px(8.0))
            // A drag released over the empty part of the column still has to
            // end, or the insertion rule stays on screen until the next click.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseUpEvent, _, cx| {
                    workspace.end_tab_drag(cx);
                }),
            );
        // One answer for "which pane is current", shared with the grid.
        let current = self.effective_active();
        // Both are copied out before the loop: the rows need to know where a
        // drag would land, and holding a borrow of `tab_drag` across the
        // listeners would make the list impossible to build.
        let drag = self.tab_drag.as_ref().filter(|drag| drag.moved);
        let dragging_tab = drag.map(|drag| drag.from);
        let drop_target = drag.and_then(|drag| drag.target);

        for (tab_index, tab) in self.tabs.iter().enumerate() {
            let current_tab = tab_index == self.active_tab;

            // Workspaces are separated by air rather than a rule: the selected
            // fill already marks the current place, and a line drawn across the
            // column would be a second edge saying the same thing louder.
            if tab_index > 0 {
                list = list.child(div().h(px(12.0)));
            }

            // Where the workspace would land. A two-pixel rule in the accent
            // colour, placed between rows rather than drawn on one: an insertion
            // point is a gap in the list, not a property of a row.
            if drop_target == Some(tab_index) && dragging_tab != Some(tab_index) {
                list = list.child(div().h(px(2.0)).mx(px(8.0)).rounded(px(1.0)).bg(theme.ring));
            }

            let rename = self
                .rename
                .as_ref()
                .filter(|rename| rename.tab_id == tab.id);
            let folded = self.folded_tabs.contains(&tab.id) && rename.is_none();

            list = list.child(sidebar_group_row(
                tab_index,
                tab.id,
                tab.title.clone(),
                tab.panes.len(),
                current_tab,
                folded,
                tab.unseen,
                rename.map(|rename| rename.input.clone()),
                rename.is_some_and(|rename| rename.invalid),
                dragging_tab == Some(tab_index),
                theme,
                cx,
            ));

            if folded {
                continue;
            }

            let mut pane_rows = div().flex().flex_col().flex_1().min_w_0().gap(px(2.0));
            for (id, pane) in &tab.panes {
                let id = *id;
                let selected = current == Some((tab_index, id));
                let label = pane_row_title(pane);
                pane_rows = pane_rows.child(sidebar_pane_row(
                    tab_index,
                    id,
                    label,
                    pane_status(pane),
                    selected,
                    tab.unseen,
                    // Closing a workspace is the header's job: it is the row
                    // that stands for the workspace itself.
                    false,
                    dragging_tab == Some(tab_index),
                    theme,
                    cx,
                ));
            }
            if !tab.panes.is_empty() {
                list = list.child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .gap(px(2.0))
                        .pl(px(7.0))
                        .child(pane_rows),
                );
            }
        }

        if self.tabs.is_empty() {
            list = list.child(sidebar_empty_row(theme, cx));
        }

        let scroll = if rail {
            div()
                .id("side-rail")
                .w_full()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(self.render_rail(theme, cx))
        } else {
            div()
                .id("side-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(list)
        };

        // The foot holds the two things that are about the column rather than
        // about a row in it: adding another workspace, and opening settings.
        let mut foot = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.0))
            .p(px(8.0))
            .pt(px(12.0))
            .when(rail, |this| this.flex_col());

        let new_button = div()
            .id("new-session")
            .flex()
            .items_center()
            .gap(px(7.0))
            .h(px(30.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(px(11.5))
            .text_color(theme.muted_foreground)
            .hover(move |this| this.bg(wash).text_color(theme.foreground))
            .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| {
                workspace.new_tab(window, cx);
            }))
            .child(icon_data(PLUS_SMALL, 13.0, theme.muted_foreground));
        if rail {
            foot = foot.child(new_button.flex_shrink_0().justify_center().w(px(30.0)));
        }
        foot = foot.child(
            icon_button("side-settings", SETTINGS_ICON, 30.0, theme, cx).on_click(cx.listener(
                |workspace, _: &ClickEvent, _, cx| {
                    workspace.toggle_settings(cx);
                },
            )),
        );
        if !rail {
            foot = foot
                .child(
                    icon_button("side-theme", THEME_ICON, 30.0, theme, cx).on_click(
                        cx.listener(|workspace, _: &ClickEvent, _, cx| workspace.toggle_theme(cx)),
                    ),
                )
                .child(div().flex_1());
            let attention: Vec<_> = self
                .tabs
                .iter()
                .enumerate()
                .flat_map(|(index, tab)| {
                    tab.panes.iter().filter_map(move |(&id, pane)| {
                        ((tab.unseen && tab.active == id)
                            || matches!(pane_status(pane), PaneStatus::Exited))
                        .then_some((index, id))
                    })
                })
                .collect();
            if !attention.is_empty() {
                let next = attention
                    .iter()
                    .position(|&pane| Some(pane) == current)
                    .map_or(0, |i| (i + 1) % attention.len());
                let (index, id) = attention[next];
                let tab_id = self.tabs[index].id;
                foot = foot.child(
                    div()
                        .id("side-attention")
                        .px(px(7.0))
                        .h(px(28.0))
                        .flex()
                        .items_center()
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .text_size(px(11.0))
                        .text_color(theme.warning)
                        .hover(move |this| this.bg(wash))
                        .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                            workspace.folded_tabs.remove(&tab_id);
                            workspace.select_tab(index, cx);
                            workspace.focus_pane(id, cx);
                        }))
                        .child(format!("{} 待关注", attention.len())),
                );
            }
        }

        div()
            .w(px(width))
            .flex_shrink_0()
            .h_full()
            .relative()
            .flex()
            .flex_col()
            .child(top)
            .child(scroll)
            .child(foot)
            // The edge is the control: it is where a pointer already goes when
            // it wants to resize a column, and dragging it to the narrow end is
            // how the list collapses into the rail.
            .child(sidebar_handle(theme, self.sidebar_drag.is_some(), cx))
            .into_any_element()
    }

    /// The list with its names dropped: one block per workspace, one chip per
    /// shell.
    ///
    /// At this width the column is navigation, not content. A workspace gets a
    /// block — its own tint when it is the one on screen, and a heading carrying
    /// its first letter — so the grouping that the list shows with indent and
    /// air is carried here by a surface and a letter.
    ///
    /// The heading is drawn even for a one-shell workspace. Without it the block
    /// shows the shell's letter instead of the workspace's, and `日志` and
    /// `部署` both read `P` because both run `pwsh` — the column would be
    /// naming the program and never the place.
    ///
    /// The chip carries a character because a column of identical dots says how
    /// many places there are but not which is which. Names collide often — both
    /// halves of a split usually run the same shell — so a workspace with more
    /// than one pane numbers its chips instead, the same numbers the pane
    /// headers wear.
    pub(super) fn render_rail(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut rail = div()
            .w_full()
            .flex()
            .flex_col()
            .px(px(6.0))
            .pt(px(2.0))
            .pb(px(8.0));
        let current = self.effective_active();

        for (tab_index, tab) in self.tabs.iter().enumerate() {
            let grouped = tab.panes.len() > 1;
            let current_tab = tab_index == self.active_tab;
            let wash = theme.muted;

            // The block is the workspace. A tint says which one the window is
            // showing — it is the one fact this column exists to answer, and a
            // tint answers it for a whole group at once rather than for one chip.
            let mut block = div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .py(px(4.0))
                .rounded(px(10.0))
                .when(tab_index > 0, |this| this.mt(px(8.0)))
                // One step off the shell, not a card: the block is a bracket
                // around a workspace, and a white card would out-shout the chip
                // it is only there to group.
                .when(current_tab, |this| this.bg(theme.muted));

            {
                block = block.child(
                    div()
                        .id(ElementId::Name(format!("rail-tab-{tab_index}").into()))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(26.0))
                        .rounded(px(7.0))
                        .cursor_pointer()
                        .hover(move |this| this.bg(wash))
                        .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                            workspace.select_tab(tab_index, cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                workspace.open_tab_menu(tab_index, event.position, cx);
                            }),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(if current_tab {
                                    theme.ring
                                } else {
                                    theme.muted_foreground
                                })
                                .child(rail_initial(&tab.title)),
                        ),
                );
            }

            // `panes` is keyed by the pane id, so this iteration is the order
            // the panes were opened — the same order the layout grows in, and
            // the order `render_node` numbers the headers with. The chip and the
            // badge have to agree, or the rail points at a pane the user cannot
            // find.
            for (index, (id, pane)) in tab.panes.iter().enumerate() {
                let id = *id;
                let selected = current == Some((tab_index, id));
                let status = pane_status(pane);
                // A split numbers its chips, because the number is what the
                // pane headers wear. A lone shell keeps its own letter: the
                // heading above already carries the workspace's.
                let glyph = if grouped {
                    (index + 1).to_string()
                } else {
                    rail_initial(&pane_label(pane))
                };
                let ink = if selected {
                    theme.foreground
                } else {
                    status_color(status, theme)
                };

                block = block.child(
                    div()
                        // Pane ids restart at 1 in every workspace, so the chip
                        // has to carry the workspace too: without it two chips
                        // share an id and only one of them can be hit.
                        .id(ElementId::Name(
                            format!("rail-pane-{tab_index}-{id}").into(),
                        ))
                        .relative()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(30.0))
                        .rounded(px(8.0))
                        .cursor_pointer()
                        .when(selected, |this| this.bg(theme.popover))
                        .when(!selected, |this| this.hover(move |this| this.bg(wash)))
                        .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                            workspace.select_tab(tab_index, cx);
                            workspace.focus_pane(id, cx);
                        }))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(if selected {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .text_color(ink)
                                .child(glyph),
                        )
                        .when(matches!(status, PaneStatus::Working), |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .left(px(8.0))
                                    .right(px(8.0))
                                    .bottom_0()
                                    .h(px(2.0))
                                    .rounded(px(1.0))
                                    .bg(theme.primary),
                            )
                        })
                        .when(matches!(status, PaneStatus::Exited), |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .right(px(5.0))
                                    .bottom(px(5.0))
                                    .size(px(5.0))
                                    .rounded(px(1.0))
                                    .bg(theme.danger),
                            )
                        }),
                );
            }

            rail = rail.child(block);
        }

        rail.into_any_element()
    }

    /// Draws the bar across the top of the work area.
    ///
    /// It answers two questions and nothing else: which workspace this is — its
    /// name, where the focused shell is, the branch when the shell reports one —
    /// and how to reach everything else without knowing a chord, through the
    /// command entry and the menu. The workspace column owns the top-left
    /// corner, so this bar starts where the work starts.
    pub(super) fn render_top_bar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = gpui_kit::component::Theme::global(cx).clone();
        let maximized = window.is_maximized();
        let muted = theme.muted_foreground;

        let mut bar = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .h(px(TITLE_BAR_HEIGHT))
            .pl(px(16.0))
            .pr(px(6.0))
            .gap(px(12.0))
            .text_color(theme.foreground);

        if self.sidebar_width == 0.0 {
            bar = bar.child(
                icon_button("sidebar-reveal", CHEVRON_RIGHT, 26.0, &theme, cx).on_click(
                    cx.listener(|workspace, _: &ClickEvent, _, cx| workspace.toggle_rail(cx)),
                ),
            );
        }

        // The workspace, not the window: a name the user gave this place, and
        // the one line of context that matters while reading output — where the
        // shell thinks it is.
        let identity = self.active_tab().map(|tab| {
            let pane = tab
                .panes
                .get(&tab.active)
                .or_else(|| tab.panes.values().next());
            (
                tab.title.clone(),
                pane.and_then(pane_cwd),
                pane.map(|pane| pty_command::display_name(&pane.command)),
            )
        });
        let (name, detail, shell) = identity.unwrap_or_else(|| ("myterm".to_owned(), None, None));

        let mut crumbs = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .min_w_0()
            .h_full()
            // The crumbs are not controls, so the space they cover is drag
            // area: the bar keeps no strip reserved for moving the window, and
            // the part that carries text is as good a handle as any.
            .window_control_area(WindowControlArea::Drag)
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(px(200.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(name),
            );
        if let Some(shell) = shell {
            crumbs = crumbs.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .h(px(19.0))
                    .px(px(7.0))
                    .rounded(px(5.0))
                    .bg(theme.ring.opacity(0.13))
                    .text_size(px(10.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.ring)
                    .child(shell),
            );
        }
        if let Some(detail) = detail {
            crumbs = crumbs.child(
                div()
                    .min_w_0()
                    .max_w(px(240.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(11.0))
                    .text_color(muted)
                    .child(detail),
            );
        }
        // The branch, when the shell's directory is in a checkout. It is a pill
        // rather than more text because it answers a different question from the
        // path beside it, and the accent is what says "this one is a place in a
        // repository" without a second label.
        if let Some(branch) = self
            .active_pane()
            .and_then(pane_cwd)
            .and_then(|cwd| self.branches.get(&cwd).cloned().flatten())
        {
            crumbs = crumbs.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_shrink_0()
                    .gap(px(4.0))
                    .h(px(18.0))
                    .px(px(7.0))
                    .rounded(px(999.0))
                    .bg(theme.ring.opacity(0.13))
                    .text_size(px(10.0))
                    .text_color(theme.ring)
                    .child(icon_data(BRANCH_ICON, 10.0, theme.ring))
                    .child(branch.clone()),
            );
        }

        bar = bar.child(crumbs);

        // The rest of the bar's left half is drag area too, so the window can be
        // moved from the emptiest part of it as well as from the name.
        bar = bar.child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .window_control_area(WindowControlArea::Drag),
        );

        bar = bar
            .when(self.sidebar_width < RAIL_THRESHOLD, |this| {
                this.child(
                    icon_button("top-search", SEARCH_ICON, 30.0, &theme, cx).on_click(cx.listener(
                        |workspace, _: &ClickEvent, window, cx| {
                            workspace.open_palette(window, cx);
                        },
                    )),
                )
            })
            .when(self.sidebar_width == 0.0, |this| {
                this.child(
                    icon_button("top-new", PLUS_SMALL, 30.0, &theme, cx).on_click(cx.listener(
                        |workspace, _: &ClickEvent, window, cx| {
                            workspace.new_tab(window, cx);
                        },
                    )),
                )
            })
            .child(
                icon_button("app-menu", MENU_ICON, 30.0, &theme, cx).on_click(cx.listener(
                    |workspace, _: &ClickEvent, _, cx| {
                        workspace.app_menu = !workspace.app_menu;
                        cx.notify();
                    },
                )),
            )
            // A gap wide enough that the caption buttons read as window controls
            // and not as more of the app's own toolbar.
            .child(div().w(px(8.0)));

        bar.child(window_button(
            "win-min",
            WINDOW_MINIMIZE,
            WindowControlArea::Min,
            false,
            &theme,
            cx.listener(|_workspace, _: &ClickEvent, window, _cx| {
                window.minimize_window();
            }),
        ))
        .child(window_button(
            "win-max",
            if maximized {
                WINDOW_RESTORE
            } else {
                WINDOW_MAXIMIZE
            },
            WindowControlArea::Max,
            false,
            &theme,
            cx.listener(|_workspace, _: &ClickEvent, window, _cx| {
                window.zoom_window();
            }),
        ))
        .child(window_button(
            "win-close",
            WINDOW_CLOSE,
            WindowControlArea::Close,
            true,
            &theme,
            cx.listener(|workspace, _: &ClickEvent, window, cx| {
                workspace.close_window(window, cx);
            }),
        ))
    }

    /// The app menu under the bar's `⋯`.
    ///
    /// The controls that used to be icons on the bar — theme, settings, about,
    /// update — live here now. None of them is used often enough to spend a
    /// button of its own on in a bar that also has to hold the workspace's name,
    /// and a menu is what the `⋯` in that bar already promises.
    pub(super) fn render_app_menu(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = theme.muted_foreground;
        let wash = theme.muted;
        let hint = theme.muted_foreground.opacity(0.7);

        let row = move |id: &'static str, label: String, key: Option<&'static str>| {
            div()
                .id(ElementId::Name(id.into()))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .h(px(31.0))
                .px(px(9.0))
                .rounded(px(7.0))
                .cursor_pointer()
                .hover(move |this| this.bg(wash))
                .text_size(px(11.5))
                .text_color(muted)
                .child(label)
                .when_some(key, |this, key| {
                    this.child(
                        div()
                            .ml_auto()
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(hint)
                            .child(key),
                    )
                })
        };

        let update = self.update_badge();
        let menu = div()
            .w(px(210.0))
            .p(px(5.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            .rounded(px(11.0))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
            })
            .child(
                row("menu-settings", "设置".to_owned(), Some("Ctrl+,")).on_click(cx.listener(
                    |workspace, _: &ClickEvent, _, cx| {
                        workspace.app_menu = false;
                        workspace.settings = true;
                        cx.notify();
                    },
                )),
            )
            .child(
                row("menu-about", "关于 myterm".to_owned(), None).on_click(cx.listener(
                    |workspace, _: &ClickEvent, _, cx| {
                        workspace.app_menu = false;
                        workspace.about = true;
                        cx.notify();
                    },
                )),
            )
            .child(match &update {
                Some((label, true)) => row("menu-update", label.clone(), None)
                    .text_color(theme.ring)
                    .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                        workspace.app_menu = false;
                        workspace.install_update(cx);
                    })),
                _ => row(
                    "menu-check",
                    if self.updating {
                        "正在下载…".to_owned()
                    } else {
                        "检查更新".to_owned()
                    },
                    None,
                )
                .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                    workspace.app_menu = false;
                    workspace.check_for_update(cx);
                })),
            })
            .child(
                row(
                    "menu-theme",
                    if theme.mode == gpui_kit::component::ThemeMode::Light {
                        "切换到深色主题".to_owned()
                    } else {
                        "切换到浅色主题".to_owned()
                    },
                    Some("Ctrl+Shift+L"),
                )
                .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                    workspace.app_menu = false;
                    workspace.toggle_theme(cx);
                })),
            )
            .child(div().h(px(1.0)).mx(px(8.0)).my(px(5.0)).bg(theme.border))
            .child(
                row("menu-quit", "退出".to_owned(), None).on_click(cx.listener(
                    |workspace, _: &ClickEvent, window, cx| {
                        workspace.app_menu = false;
                        workspace.close_window(window, cx);
                    },
                )),
            );

        div()
            .absolute()
            .inset_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseDownEvent, _, cx| {
                    workspace.app_menu = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .top(px(TITLE_BAR_HEIGHT))
                    .right(px(140.0))
                    .child(menu),
            )
            .into_any_element()
    }

    /// What is left when every workspace is closed.
    ///
    /// Not a black window and not a consolation sentence: the one thing an empty
    /// list can still tell you is where the next one comes from, so it says that
    /// and gives the button.
    pub(super) fn render_empty(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .pr(px(PANE_GAP))
            .pb(px(PANE_GAP))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme.muted_foreground)
                    .child("没有打开的会话"),
            )
            .child(
                div()
                    .id("empty-new")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(7.0))
                    .h(px(30.0))
                    .px(px(12.0))
                    .rounded(px(8.0))
                    .bg(theme.muted)
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(theme.muted_foreground)
                    .hover(move |this| this.bg(theme.secondary_hover).text_color(theme.foreground))
                    .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| {
                        workspace.new_tab(window, cx);
                    }))
                    .child(icon_data(PLUS_SMALL, 13.0, theme.muted_foreground))
                    .child("新建工作区"),
            )
            .into_any_element()
    }

    /// What the status bar has to say, if anything.
    ///
    /// Everything that used to sit there permanently — the session count, the
    /// branch, a reminder about keys — already has a better home in the bar
    /// above or the list beside it. What is left is the state that exists only
    /// while it is true.
    pub(super) fn status_hint(&self) -> Option<String> {
        // One case, as the prototype has it. Background output and a pending
        // update are both already reported where they belong — a dot on the
        // workspace's row, a badge in the bar — and repeating them here made a
        // row that was on most of the time, which is the opposite of the point.
        self.zoomed
            .is_some()
            .then(|| "已缩放 · Esc 退出".to_owned())
    }

    /// The status bar: 26 px that exist only while `status_hint` has a line.
    pub(super) fn render_status_bar(
        &self,
        hint: &str,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .h(px(STATUS_BAR_HEIGHT))
            .pl(px(16.0))
            .pr(px(10.0))
            .text_size(px(10.5))
            .text_color(theme.muted_foreground)
            .child(hint.to_owned())
            .child(div().flex_1())
            .when(self.zoomed.is_some(), |this| {
                this.child(
                    div()
                        .id("status-unzoom")
                        .px(px(6.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .hover(move |this| this.bg(theme.muted))
                        .on_click(cx.listener(|workspace, _: &ClickEvent, _, cx| {
                            if let Some(id) = workspace.zoomed {
                                workspace.toggle_zoom(id, cx);
                            }
                        }))
                        .child("退出缩放"),
                )
            })
            .into_any_element()
    }
}
