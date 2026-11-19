use super::*;
use gpui_kit::component::FocusableExt;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum ShellChoice {
    Auto,
    Pwsh,
    WindowsPowerShell,
    CommandPrompt,
    WslDefault,
    WslInstance { name: String, is_default: bool },
}

impl ShellChoice {
    pub(super) fn options() -> Vec<Self> {
        let mut choices = vec![
            Self::Auto,
            Self::Pwsh,
            Self::WindowsPowerShell,
            Self::CommandPrompt,
        ];
        let instances = pty_command::wsl_instances();
        if instances.is_empty() {
            choices.push(Self::WslDefault);
        } else {
            choices.extend(instances.iter().map(|instance| Self::WslInstance {
                name: instance.name().to_owned(),
                is_default: instance.is_default(),
            }));
        }
        choices
    }

    pub(super) fn selected_index(config: &config::Config, choices: &[Self]) -> usize {
        let exact = match config.shell {
            config::Shell::Auto => Some(Self::Auto),
            config::Shell::Pwsh => Some(Self::Pwsh),
            config::Shell::Windows => Some(Self::WindowsPowerShell),
            config::Shell::Cmd => Some(Self::CommandPrompt),
            config::Shell::Wsl => config
                .wsl_distribution
                .as_ref()
                .and_then(|saved| {
                    choices.iter().find(
                        |choice| matches!(choice, Self::WslInstance { name, .. } if name == saved),
                    )
                })
                .cloned()
                .or_else(|| {
                    choices
                        .iter()
                        .find(|choice| {
                            matches!(
                                choice,
                                Self::WslInstance {
                                    is_default: true,
                                    ..
                                }
                            )
                        })
                        .cloned()
                })
                .or_else(|| {
                    choices
                        .iter()
                        .find(|choice| matches!(choice, Self::WslInstance { .. }))
                        .cloned()
                })
                .or(Some(Self::WslDefault)),
        };
        exact
            .and_then(|selected| choices.iter().position(|choice| *choice == selected))
            .unwrap_or(0)
    }

    fn apply(&self, config: &mut config::Config) {
        match self {
            Self::Auto => config.shell = config::Shell::Auto,
            Self::Pwsh => config.shell = config::Shell::Pwsh,
            Self::WindowsPowerShell => config.shell = config::Shell::Windows,
            Self::CommandPrompt => config.shell = config::Shell::Cmd,
            Self::WslDefault => {
                config.shell = config::Shell::Wsl;
                config.wsl_distribution = None;
            }
            Self::WslInstance { name, .. } => {
                config.shell = config::Shell::Wsl;
                config.wsl_distribution = Some(name.clone());
            }
        }
    }
}

impl SelectItem for ShellChoice {
    type Value = Self;

    fn title(&self) -> SharedString {
        let title = match self {
            Self::Auto => "自动".to_owned(),
            Self::Pwsh => "PowerShell 7".to_owned(),
            Self::WindowsPowerShell => "Windows PowerShell".to_owned(),
            Self::CommandPrompt => "命令提示符".to_owned(),
            Self::WslDefault => "WSL · 系统默认实例".to_owned(),
            Self::WslInstance { name, is_default } => {
                if *is_default {
                    format!("WSL · {name}（默认）")
                } else {
                    format!("WSL · {name}")
                }
            }
        };
        title.into()
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

impl Workspace {
    fn render_shell_picker(&self, theme: &gpui_kit::component::Theme) -> impl IntoElement {
        let actual = pty_command::display_name(&self.command);
        let available = pty_command::is_available(self.config.shell)
            && (self.config.shell != config::Shell::Wsl
                || self.config.wsl_distribution.as_ref().is_none_or(|name| {
                    pty_command::wsl_instances()
                        .iter()
                        .any(|instance| instance.name() == name)
                }));
        let status = if available {
            format!("新窗格启动 {actual}")
        } else {
            format!("所选 Shell 不可用，回退到 {actual}")
        };

        div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .py(px(11.0))
            .border_b_1()
            .border_color(theme.border.opacity(0.45))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.0))
                    .child(div().text_size(px(12.5)).child("默认 Shell"))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(if available {
                                theme.muted_foreground
                            } else {
                                theme.warning
                            })
                            .child(status),
                    ),
            )
            .child(
                div().flex_shrink_0().w(px(128.0)).h(px(24.0)).child(
                    Select::new(&self.shell_selector)
                        .id("sh-default-shell")
                        .small()
                        .w_full()
                        .h_full()
                        .px(px(8.0))
                        .rounded(px(6.0))
                        .bg(theme.muted.opacity(0.6))
                        .border_color(theme.border.opacity(0.45))
                        .focus_ring(false)
                        .menu_width(px(208.0))
                        .menu_max_h(px(256.0))
                        .icon(gpui_kit::component::Icon::default().data(CHEVRON_DOWN))
                        .accessibility_label("新窗格的默认 Shell"),
                ),
            )
    }

    pub(super) fn apply_shell_choice(&mut self, choice: ShellChoice, cx: &mut Context<Self>) {
        choice.apply(&mut self.config);
        self.command = build_command(
            self.config.shell,
            self.config.shell_integration,
            self.config.wsl_distribution.as_deref(),
        );
        self.config.save();
        cx.notify();
    }

    pub(super) fn render_settings(
        &self,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = theme.muted_foreground;
        let size = theme.mono_font_size.as_f32();
        let light = self.config.light;
        let wash = theme.muted;
        let accent = theme.ring;
        let id = |name: &str| ElementId::Name(name.to_owned().into());

        let tab = |id: &'static str, label: &'static str, active: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex()
                .items_center()
                .h(px(28.0))
                .px(px(12.0))
                .rounded(px(7.0))
                .cursor_pointer()
                .text_size(px(11.5))
                .text_color(if active { accent } else { muted })
                .when(active, |this| this.bg(theme.ring.opacity(0.13)))
                .when(!active, |this| {
                    this.hover(move |this| this.bg(wash).text_color(theme.muted_foreground))
                })
                .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                    workspace.settings_tab = match id {
                        "sh-tab-term" => SettingsTab::Terminal,
                        "sh-tab-keys" => SettingsTab::Keys,
                        _ => SettingsTab::Look,
                    };
                    cx.notify();
                }))
                .child(label)
        };

        let seg = |id: ElementId, label: &'static str, active: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .h(px(24.0))
                .px(px(10.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(11.0))
                .text_color(if active { theme.foreground } else { muted })
                .when(active, |this| {
                    this.bg(theme.popover)
                        .font_weight(FontWeight::MEDIUM)
                        .shadow_sm()
                })
                .when(!active, |this| {
                    this.hover(move |this| this.text_color(theme.muted_foreground))
                })
                .child(label)
        };

        let track = || {
            div()
                .flex()
                .flex_row()
                .gap(px(2.0))
                .p(px(2.0))
                .rounded(px(8.0))
                // The track is the neutral pill, not the window background: in
                // light mode the background is white and so is the chip that
                // sits on it, which made every selector invisible.
                .bg(theme.muted)
        };

        // Each tab body is built here and the sheet picks one, so the sheet's
        // own structure stays a header, a body and a footer whatever the tab.
        let body = match self.settings_tab {
            SettingsTab::Look => {
                let theme_row = track()
                    .child(
                        seg(id("sh-theme-system"), "跟随", light.is_none()).on_click(cx.listener(
                            |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.config.light = None;
                                workspace.config.save();
                                // Back to the desktop's own answer, resolved
                                // rather than remembered: the setting may have
                                // changed since start-up.
                                let mode = if system_prefers_light() {
                                    gpui_kit::component::ThemeMode::Light
                                } else {
                                    gpui_kit::component::ThemeMode::Dark
                                };
                                workspace.apply_theme(mode, cx);
                            },
                        )),
                    )
                    .child(
                        seg(id("sh-theme-light"), "浅色", light == Some(true)).on_click(
                            cx.listener(|workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.apply_theme(gpui_kit::component::ThemeMode::Light, cx);
                            }),
                        ),
                    )
                    .child(
                        seg(id("sh-theme-dark"), "深色", light == Some(false)).on_click(
                            cx.listener(|workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.apply_theme(gpui_kit::component::ThemeMode::Dark, cx);
                            }),
                        ),
                    );

                let mut swatches = div().flex().flex_row().items_center().gap(px(8.0));
                for option in config::Accent::ALL {
                    let active = self.config.accent == option;
                    let (ink, _) = palette::accent_ink(option, theme.mode);
                    swatches = swatches.child(
                        div()
                            .id(ElementId::Name(
                                format!("sh-accent-{}", option.as_str()).into(),
                            ))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(26.0))
                            .rounded(px(999.0))
                            // A swatch is a colour chip, so it keeps a ring:
                            // it is the only control whose job is to show a
                            // colour, and the ring is what says which one is
                            // chosen.
                            .when(active, |this| this.border_1().border_color(ink))
                            .cursor_pointer()
                            .on_click(cx.listener(move |workspace, _: &ClickEvent, _, cx| {
                                workspace.set_accent(option, cx);
                            }))
                            .child(div().size(px(16.0)).rounded(px(999.0)).bg(ink)),
                    );
                }

                div()
                    .flex()
                    .flex_col()
                    .child(sgroup("外观", theme))
                    .child(setting_row(
                        "主题",
                        "跟随系统时使用 Windows 的深浅设置",
                        theme,
                        theme_row.into_any_element(),
                    ))
                    .child(setting_row(
                        "强调色",
                        "只用于「当前」和「有动静」，不用于装饰",
                        theme,
                        swatches.into_any_element(),
                    ))
            }
            SettingsTab::Terminal => {
                let scrollback_row = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("sh-scrollback-less")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded(px(6.0))
                            .bg(theme.background)
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(muted)
                            .hover(move |this| this.bg(wash).text_color(theme.foreground))
                            .on_click(cx.listener(
                                |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.set_scrollback(
                                        workspace.config.scrollback.saturating_sub(SCROLLBACK_STEP),
                                        cx,
                                    );
                                },
                            ))
                            .child(icon_data(MINUS_ICON, 12.0, muted)),
                    )
                    .child(
                        div()
                            .w(px(64.0))
                            .text_center()
                            .text_size(px(11.5))
                            .text_color(theme.foreground)
                            .child(format!("{} 行", self.config.scrollback)),
                    )
                    .child(
                        div()
                            .id("sh-scrollback-more")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded(px(6.0))
                            .bg(theme.background)
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(muted)
                            .hover(move |this| this.bg(wash).text_color(theme.foreground))
                            .on_click(cx.listener(
                                |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.set_scrollback(
                                        workspace.config.scrollback.saturating_add(SCROLLBACK_STEP),
                                        cx,
                                    );
                                },
                            ))
                            .child(icon_data(PLUS_SMALL, 12.0, muted)),
                    );

                let integration_row = track()
                    .child(
                        seg(
                            id("sh-integration-off"),
                            "关闭",
                            !self.config.shell_integration,
                        )
                        .on_click(cx.listener(
                            |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.set_shell_integration(false, cx);
                            },
                        )),
                    )
                    .child(
                        seg(
                            id("sh-integration-on"),
                            "启用",
                            self.config.shell_integration,
                        )
                        .on_click(cx.listener(
                            |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.set_shell_integration(true, cx);
                            },
                        )),
                    );

                let mut cursor_row = track();
                for option in [
                    None,
                    Some(config::Cursor::Bar),
                    Some(config::Cursor::Block),
                    Some(config::Cursor::Underline),
                ] {
                    let label = match option {
                        None => "跟随程序",
                        Some(config::Cursor::Bar) => "竖线",
                        Some(config::Cursor::Block) => "块",
                        Some(config::Cursor::Underline) => "下划线",
                    };
                    let id = match option {
                        None => "sh-cursor-auto".to_owned(),
                        Some(cursor) => format!("sh-cursor-{}", cursor.as_str()),
                    };
                    cursor_row = cursor_row.child(
                        seg(
                            ElementId::Name(id.into()),
                            label,
                            self.config.cursor == option,
                        )
                        .on_click(cx.listener(
                            move |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                workspace.config.cursor = option;
                                workspace.config.save();
                                cx.notify();
                            },
                        )),
                    );
                }

                let font = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("sh-font-smaller")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded(px(6.0))
                            .bg(theme.background)
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(muted)
                            .hover(move |this| this.bg(wash).text_color(theme.foreground))
                            .on_click(cx.listener(
                                move |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.set_font_size(size - FONT_STEP, cx);
                                },
                            ))
                            .child(icon_data(MINUS_ICON, 12.0, muted)),
                    )
                    .child(
                        div()
                            .w(px(56.0))
                            .text_center()
                            .text_size(px(11.5))
                            .text_color(theme.foreground)
                            .child(format!("{size} px")),
                    )
                    .child(
                        div()
                            .id("sh-font-larger")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded(px(6.0))
                            .bg(theme.background)
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(muted)
                            .hover(move |this| this.bg(wash).text_color(theme.foreground))
                            .on_click(cx.listener(
                                move |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.set_font_size(size + FONT_STEP, cx);
                                },
                            ))
                            .child(icon_data(PLUS_SMALL, 12.0, muted)),
                    )
                    .child(
                        div()
                            .id("sh-font-reset")
                            .h(px(24.0))
                            .px(px(8.0))
                            .flex()
                            .items_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .text_size(px(11.0))
                            .text_color(muted)
                            .hover(move |this| this.bg(wash).text_color(theme.foreground))
                            .on_click(cx.listener(
                                |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.set_font_size(config::DEFAULT_FONT_SIZE, cx);
                                },
                            ))
                            .child("重置"),
                    );

                div()
                    .flex()
                    .flex_col()
                    .child(sgroup("终端", theme))
                    .child(self.render_shell_picker(theme))
                    .child(setting_row(
                        "字号",
                        "只影响网格，不影响界面",
                        theme,
                        font.into_any_element(),
                    ))
                    .child(setting_row(
                        "光标",
                        "块状时会反色重绘它下面的字符",
                        theme,
                        cursor_row.into_any_element(),
                    ))
                    .child(setting_row(
                        "回滚行数",
                        "超出后丢弃最旧的输出，搜索也只看这么多",
                        theme,
                        scrollback_row.into_any_element(),
                    ))
                    .child(setting_row(
                        "shell 集成",
                        "让 shell 报告自己的目录，路径与分支才有内容；只对 PowerShell 生效，会改写它的提示符函数，新窗格开始使用",
                        theme,
                        integration_row.into_any_element(),
                    ))
            }
            SettingsTab::Keys => {
                let mut list = div().flex().flex_col();
                for (keys, label) in KEYMAP {
                    list = list.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .h(px(34.0))
                            .text_size(px(11.5))
                            .text_color(muted)
                            .child(*label)
                            .child(div().ml_auto().flex().flex_row().gap(px(3.0)).children(
                                keys.split('+').map(|part| {
                                    div()
                                        .flex()
                                        .items_center()
                                        .h(px(16.0))
                                        .px(px(4.0))
                                        .rounded(px(4.0))
                                        .bg(theme.muted)
                                        .text_size(px(9.5))
                                        .text_color(muted)
                                        .child(part.to_owned())
                                }),
                            )),
                    );
                }
                div()
                    .flex()
                    .flex_col()
                    .child(sgroup("快捷键", theme))
                    .child(list)
            }
        };

        div()
            .absolute()
            // Under the bar, not over it: the sheet is a workspace tool, and
            // covering the window's own controls with it would make closing the
            // window depend on closing settings first.
            .relative()
            .flex_shrink_0()
            .w(px(436.0))
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme.title_bar)
            // The column starts at the bar's own line, so its title sits on the
            // same row as the workspace's rather than a row above it.
            .pt(px(0.0))
            // The sheet is a column of the window, not a card over one: laid
            // over the work area it covered the panes' rounded corners and sat
            // on a scrim that swallowed clicks. As a sibling it takes its own
            // width and the panes narrow beside it, which is what a live preview
            // was always meant to look like.
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    // A press inside the sheet is the sheet's. Without this the
                    // click falls through to the pane underneath and puts the
                    // caret there, which is a strange thing to have happen while
                    // you are changing a setting.
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .flex_shrink_0()
                            .h(px(52.0))
                            .pl(px(18.0))
                            .pr(px(10.0))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.foreground)
                                    .child("设置"),
                            )
                            .child(pane_action(
                                ("settings-close", 0u64),
                                WINDOW_CLOSE,
                                theme,
                                false,
                                cx.listener(|workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                    workspace.settings = false;
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(2.0))
                            .px(px(14.0))
                            .pb(px(10.0))
                            .child(tab(
                                "sh-tab-look",
                                "外观",
                                self.settings_tab == SettingsTab::Look,
                                cx,
                            ))
                            .child(tab(
                                "sh-tab-term",
                                "终端",
                                self.settings_tab == SettingsTab::Terminal,
                                cx,
                            ))
                            .child(tab(
                                "sh-tab-keys",
                                "快捷键",
                                self.settings_tab == SettingsTab::Keys,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .id("settings-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(px(14.0))
                            .pb(px(14.0))
                            .child(body),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .flex_shrink_0()
                            .px(px(16.0))
                            .py(px(12.0))
                            .child(
                                div()
                                    .id("settings-about")
                                    .cursor_pointer()
                                    .text_size(px(11.0))
                                    .text_color(muted)
                                    .hover(move |this| this.text_color(accent))
                                    .on_click(cx.listener(
                                        |workspace: &mut Workspace, _: &ClickEvent, _, cx| {
                                            workspace.settings = false;
                                            workspace.about = true;
                                            cx.notify();
                                        },
                                    ))
                                    .child("关于 myterm"),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(muted)
                                    .child("更改立即生效"),
                            ),
                    ),
            )
            .into_any_element()
    }
}
