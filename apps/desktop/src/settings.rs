use crate::{app::ChatApp,
            language::Language,
            ui::*};
use gpui::{assets::IconName,
           base::TestSupportExt as _,
           component::{Icon,
                       Theme,
                       ThemeMode,
                       scroll::ScrollableElement},
           *};

// Search metadata for the same sections rendered by each settings page.
const SEARCH_ITEMS: &[(Panel, &str, &[&str])] = &[
    (Panel::Guidance, "지침", &["AGENTS.md", "생성", "불러오기", "편집", "저장"]),
    (
        Panel::Plugins,
        "플러그인",
        &["Plugins", "스킬", "Skills", "SKILL.md", "MCP", "OAuth", "가져오기", "활성"],
    ),
    (Panel::General, "언어", &["앱에서 사용할 언어", "한국어", "English"]),
    (
        Panel::General,
        "키보드 단축키",
        &["보내기", "줄바꿈", "Enter", "Shift+Enter", "Ctrl", "Esc", "대화 목록", "참고 대화"],
    ),
    (Panel::Appearance, "테마", &["작업 화면의 색상", "밝게", "어둡게"]),
    (Panel::Appearance, "윈도우 타이틀바", &["타이틀바 스타일", "네이티브", "GPUI"]),
    (Panel::Appearance, "글꼴", &["현재 글꼴"]),
    (
        Panel::Accounts,
        "런타임 관리",
        &["로그인", "로그아웃", "계정 추가", "Claude", "Codex", "Antigravity", "설치", "업데이트"],
    ),
    (Panel::Workspace, "프로젝트 열기", &["폴더 선택", "프로젝트 경로"]),
    (Panel::Workspace, "프로젝트", &["worktree", "브랜치", "추가", "제거"]),
    (Panel::Mcp, "연결 대상", &["제공자", "계정"]),
    (Panel::Mcp, "MCP stdio · HTTP 서버 설정", &["MCP JSON", "MCP 불러오기", "MCP 저장", "OAuth"]),
];

impl ChatApp {
    fn set_settings_search(&mut self, query: String) {
        self.settings_search = query;
        let matches = self.matching_settings();
        if let Some(first) = matches.first().copied()
            && self.panel.is_some_and(|panel| !matches.contains(&panel))
        {
            let query = std::mem::take(&mut self.settings_search);
            let navigation = self.settings_nav_open;
            self.open_panel(first);
            self.settings_search = query;
            self.settings_nav_open = navigation;
        }
        self.settings_result = self.panel.and_then(|panel| matches.iter().position(|p| *p == panel)).unwrap_or(0);
        self.settings_scroll.set_offset(Point::default());
    }

    pub(super) fn settings_section_matches(&self, title: &str) -> bool {
        if self.settings_search.trim().is_empty() || self.panel.is_none() || self.chat_tool.is_some() {
            return true;
        }
        let panel = self.panel.expect("settings page");
        self.matches_setting_text(panel.title())
            || self.matches_setting_text(panel.description())
            || self.matches_setting_text(title)
            || SEARCH_ITEMS.iter().any(|(owner, section, items)| {
                *owner == panel
                    && (self.text(section) == title || *section == title || (panel == Panel::Workspace && *section == "프로젝트"))
                    && items.iter().any(|value| self.matches_setting_text(value))
            })
    }

    fn matches_setting_text(&self, value: &str) -> bool {
        let query = self.settings_search.trim().to_lowercase();
        value.to_lowercase().contains(&query) || self.text(value).to_lowercase().contains(&query)
    }

    pub(super) fn matching_settings(&self) -> Vec<Panel> {
        Panel::SETTINGS_GROUPS
            .iter()
            .flat_map(|(_, panels)| panels.iter().copied())
            .filter(|panel| {
                self.matches_setting_text(&format!(
                    "{} {} {} {}",
                    panel.title(),
                    self.text(panel.nav_title()),
                    self.text(panel.title()),
                    self.text(panel.description())
                )) || SEARCH_ITEMS
                    .iter()
                    .any(|(owner, section, items)| owner == panel && std::iter::once(section).chain(items.iter()).any(|value| self.matches_setting_text(value)))
            })
            .collect()
    }

    fn move_settings_result(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_search.trim().is_empty() {
            return;
        }
        if let Some(Field::Line {
            state, ..
        }) = self.inputs.get("settings-search")
            && state.update(cx, |state, cx| state.marked_text_range(window, cx).is_some())
        {
            return;
        }
        let count = self.matching_settings().len();
        if count > 0 {
            self.settings_result = (self.settings_result + if backwards { count - 1 } else { 1 }) % count;
            self.settings_scroll.scroll_to_item(self.settings_result);
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn settings_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let filter = self.field(
            "settings-search",
            "설정 검색",
            self.settings_search.clone(),
            FieldKind::Line,
            true,
            window,
            cx,
            |s, value| {
                s.set_settings_search(value);
            },
        );
        let matches = self.matching_settings();
        let mut links = column().gap_1();
        let searching = !self.settings_search.trim().is_empty();
        for (index, (label, panels)) in Panel::SETTINGS_GROUPS.iter().enumerate() {
            let mut group = column().gap_1();
            for panel in panels.iter().filter(|panel| matches.contains(panel)) {
                let panel = *panel;
                let active = if searching {
                    matches.get(self.settings_result) == Some(&panel)
                } else {
                    self.panel == Some(panel) || panel == Panel::Plugins && matches!(self.panel, Some(Panel::Skills | Panel::Mcp))
                };
                let button = self
                    .sidebar_button(
                        format!("settings-nav-{panel:?}"),
                        panel.nav_title(),
                        panel.icon(),
                        true,
                        cx,
                        move |s, window, cx| {
                            s.open_panel(panel);
                            s.focus.focus(window, cx);
                        },
                    )
                    .selected(active)
                    .bg(if active { cx.theme().muted } else { transparent_black() });
                if searching {
                    links = links.child(button);
                } else {
                    group = group.child(button);
                }
            }
            if !searching && panels.iter().any(|panel| matches.contains(panel)) {
                links = links.child(
                    column()
                        .gap_2()
                        .pb_3()
                        .child(
                            div()
                                .id(format!("settings-group-{index}"))
                                .test_support()
                                .role(Role::Heading)
                                .aria_label(self.text(label))
                                .child(note(self.text(label), cx).px_2()),
                        )
                        .child(group),
                );
            }
        }
        if matches.is_empty() {
            links = links.child(note(self.text("검색 결과가 없습니다"), cx).px_2().py_4());
        }
        let mut header = row()
            .h(px(52.))
            .px_3()
            .flex_shrink_0()
            .child(self.icon_button("settings-back", "대화로 돌아가기", IconName::ArrowLeft, true, cx, |s, _, _| {
                s.close_panel()
            }))
            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(self.text("설정")));
        if window.viewport_size().width < px(720.) {
            header = header.child(self.icon_button("sidebar-toggle", "사이드바 접기", IconName::PanelLeft, true, cx, |s, _, _| {
                s.settings_nav_open = false
            }));
        }
        column()
            .id("settings-sidebar")
            .test_support()
            .w_full()
            .h_full()
            .gap_0()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(header)
            .child(
                row()
                    .flex_nowrap()
                    .gap_1()
                    .px_3()
                    .pb_4()
                    .capture_action(cx.listener(|s, action: &gpui::component::input::Enter, window, cx| {
                        let composing = if let Some(Field::Line {
                            state, ..
                        }) = s.inputs.get("settings-search")
                        {
                            state.update(cx, |state, cx| state.marked_text_range(window, cx).is_some())
                        } else {
                            false
                        };
                        if !action.shift
                            && !composing
                            && let Some(panel) = s.matching_settings().get(s.settings_result).copied()
                        {
                            s.open_panel(panel);
                            s.focus.focus(window, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }
                    }))
                    .capture_action(cx.listener(|s, _: &gpui::component::input::MoveDown, window, cx| s.move_settings_result(false, window, cx)))
                    .capture_action(cx.listener(|s, _: &gpui::component::input::MoveUp, window, cx| s.move_settings_result(true, window, cx)))
                    .child(Icon::new(IconName::Search).small())
                    .child(div().flex_1().min_w_0().child(filter))
                    .children((!self.settings_search.is_empty()).then(|| {
                        self.icon_button(
                            "settings-search-clear",
                            "검색 지우기",
                            IconName::X,
                            !self.settings_search.is_empty(),
                            cx,
                            |s, window, cx| {
                                s.settings_search.clear();
                                s.settings_result = 0;
                                if let Some(Field::Line {
                                    state, ..
                                }) = s.inputs.get("settings-search")
                                {
                                    state.update(cx, |state, cx| state.focus(window, cx));
                                }
                            },
                        )
                    })),
            )
            .child(
                links
                    .id("settings-nav-scroll")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.settings_scroll)
                    .px_2()
                    .pb_4(),
            )
            .child(
                row()
                    .p_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(note(format!("Gpui Agent {}", env!("CARGO_PKG_VERSION")), cx)),
            )
            .into_any_element()
    }

    pub(super) fn settings_header(&self, panel: Panel, cx: &Context<Self>) -> AnyElement {
        let panel = if matches!(panel, Panel::Skills | Panel::Mcp) { Panel::Plugins } else { panel };
        let group = Panel::SETTINGS_GROUPS
            .iter()
            .find_map(|(label, panels)| panels.contains(&panel).then_some(*label))
            .unwrap_or("설정");
        row()
            .id("settings-header")
            .test_support()
            .h(px(52.))
            .flex_nowrap()
            .flex_shrink_0()
            .gap_2()
            .px_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(self.icon_button("sidebar-toggle", "탐색 메뉴", IconName::PanelLeft, true, cx, |s, window, _| {
                if window.viewport_size().width < px(720.) {
                    s.settings_nav_open = !s.settings_nav_open;
                } else {
                    s.sidebar_open = !s.sidebar_open;
                }
            }))
            .child(note(self.text(group), cx))
            .child(note("/", cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(self.text(panel.title())),
            )
            .child(self.icon_button("panel-close", "대화로 돌아가기", IconName::ArrowLeft, true, cx, |s, _, _| {
                s.close_panel()
            }))
            .into_any_element()
    }

    pub(super) fn settings_page(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let display_panel = if matches!(panel, Panel::Skills | Panel::Mcp) { Panel::Plugins } else { panel };
        if !self.settings_search.trim().is_empty() {
            let matches = self.matching_settings();
            if matches.is_empty() {
                return column()
                    .id("settings-no-results")
                    .test_support()
                    .p_6()
                    .child(empty_state(self.text("검색 결과가 없습니다"), self.text("다른 검색어를 입력해 보세요.")))
                    .into_any_element();
            }
            if !matches.contains(&display_panel) {
                self.set_settings_search(self.settings_search.clone());
                return self.settings_page(matches[0], window, cx);
            }
        }
        let content = self.panel_view(panel, window, cx);
        let content = if matches!(panel, Panel::Plugins | Panel::Skills | Panel::Mcp) {
            let wide = self.settings_content_wide(window);
            let tabs = self.plugin_tabs(wide, cx);
            if wide {
                row()
                    .items_start()
                    .flex_nowrap()
                    .gap_4()
                    .child(tabs)
                    .child(div().flex_1().min_w_0().child(content))
                    .into_any_element()
            } else {
                column().gap_6().child(tabs).child(content).into_any_element()
            }
        } else {
            content
        };
        let body = column()
            .id("settings-content")
            .test_support()
            .w_full()
            .max_w(px(896.))
            .mx_auto()
            .px_5()
            .pt_6()
            .pb_12()
            .gap_6()
            .child(
                column()
                    .gap_2()
                    .child(heading(self.text(display_panel.title())))
                    .child(note(self.text(display_panel.description()), cx)),
            );
        div()
            .id("panel-scroll")
            .test_support()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.panel_scroll)
            .vertical_scrollbar(&self.panel_scroll)
            .child(body.child(content))
            .into_any_element()
    }

    pub(super) fn settings_content_wide(&self, window: &Window) -> bool {
        let width: f32 = window.viewport_size().width.into();
        let sidebar = if width >= 720. && self.sidebar_open {
            self.sidebar_width_for(width)
        } else {
            0.
        };
        width - sidebar >= 760.
    }

    pub(super) fn general_panel(&self, cx: &Context<Self>) -> AnyElement {
        let language = self.selection_group(
            "settings-language-group",
            [(Language::Korean, "한국어"), (Language::English, "English")]
                .into_iter()
                .map(|(value, label)| (format!("settings-language-{value:?}"), label.to_owned(), value, None)),
            self.language,
            true,
            cx,
            |s, value, _, _| s.set_language(value),
        );
        let mut shortcuts = column().gap_0();
        let modifier = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
        for (key, label) in [
            ("enter".into(), "보내기"),
            ("shift-enter".into(), "줄바꿈"),
            (format!("{modifier}-enter"), "보내기"),
            (format!("{modifier}-b"), "대화 목록"),
            (format!("{modifier}-shift-r"), "참고 대화"),
            (format!("{modifier}-shift-l"), "한국어 / English"),
            ("escape".into(), "대화로 돌아가기"),
        ] {
            shortcuts = shortcuts.child(
                row()
                    .justify_between()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(note(self.text(label), cx))
                    .child(gpui::component::kbd::Kbd::new(Keystroke::parse(&key).expect("static shortcut"))),
            );
        }
        let mut body = column()
            .gap_6()
            .child(self.settings_section(
                self.text("언어"),
                row().justify_between().child(note(self.text("앱에서 사용할 언어"), cx)).child(language),
                cx,
            ))
            .child(self.settings_section(self.text("키보드 단축키"), shortcuts, cx));
        if let Some(message) = &self.conversations[self.selected].error {
            body = body.child(error(message.clone(), cx));
        }
        body.into_any_element()
    }

    pub(super) fn appearance_panel(&self, cx: &Context<Self>) -> AnyElement {
        let titlebar = self.selection_group(
            "settings-titlebar-group",
            [
                (crate::titlebar::TitlebarMode::Native, "네이티브"),
                (crate::titlebar::TitlebarMode::Gpui, "GPUI"),
            ]
            .into_iter()
            .map(|(value, label)| (format!("titlebar-{value:?}"), label.to_owned(), value, None)),
            self.titlebar_mode,
            true,
            cx,
            |s, value, window, cx| s.set_titlebar_mode(value, window, cx),
        );
        let choices = self.selection_group(
            "settings-theme-group",
            [(ThemeMode::Light, "밝게", IconName::Sun), (ThemeMode::Dark, "어둡게", IconName::Moon)]
                .into_iter()
                .map(|(value, label, icon)| (format!("appearance-{value:?}"), label.to_owned(), value, Some(icon))),
            cx.theme().mode,
            true,
            cx,
            |_, value, window, cx| {
                Theme::change(value, Some(window), cx);
                apply_theme(cx);
            },
        );
        let mut body = column()
            .gap_6()
            .child(self.settings_section(
                self.text("테마"),
                row().justify_between().child(note(self.text("작업 화면의 색상"), cx)).child(choices),
                cx,
            ))
            .child(
                self.settings_section(
                    self.text("윈도우 타이틀바"),
                    column()
                        .child(row().justify_between().child(note(self.text("타이틀바 스타일"), cx)).child(titlebar))
                        .child(note(self.text("전환 시 창을 다시 열며 대화와 작업은 유지됩니다."), cx)),
                    cx,
                ),
            )
            .child(
                self.settings_section(
                    self.text("글꼴"),
                    row()
                        .justify_between()
                        .child(note(self.text("현재 글꼴"), cx))
                        .child(div().child(cx.theme().font_family.clone())),
                    cx,
                ),
            );
        if let Some(message) = &self.conversations[self.selected].error {
            body = body.child(error(message.clone(), cx));
        }
        body.into_any_element()
    }
}
