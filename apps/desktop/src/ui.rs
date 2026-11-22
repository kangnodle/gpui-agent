use crate::{app::ChatApp,
            language::Language};
pub use gpui_kit::component::{ActiveTheme,
                              Selectable,
                              Sizable,
                              button::ButtonVariants};
use gpui_kit::{component::{Disableable,
                           button::{Button,
                                    Toggle,
                                    ToggleGroup,
                                    ToggleVariants as _},
                           input::{Editor,
                                   EditorState,
                                   Input,
                                   InputEvent,
                                   InputState,
                                   Textarea,
                                   TextareaState}},
               prelude::FluentBuilder as _,
               *};

pub enum UiEvent {
    Changed,
    OpenUrl(String),
}

/// Wakes the GPUI entity from the existing blocking API workers.
#[derive(Clone)]
pub struct Wake(pub async_channel::Sender<UiEvent>);
impl Wake {
    pub fn notify(&self) { let _ = self.0.try_send(UiEvent::Changed); }

    pub fn open_url(&self, url: &str) { let _ = self.0.try_send(UiEvent::OpenUrl(url.into())); }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    General,
    Appearance,
    Accounts,
    Guidance,
    Skills,
    Plugins,
    Workspace,
    Git,
    Pr,
    Mcp,
    Attach,
}
impl Panel {
    pub const SETTINGS_GROUPS: &[(&str, &[Self])] = &[
        ("작업 환경", &[Self::Accounts, Self::Workspace]),
        ("에이전트 구성", &[Self::Guidance, Self::Plugins]),
        ("앱 환경", &[Self::General, Self::Appearance]),
    ];

    pub fn is_chat_tool(self) -> bool { matches!(self, Self::Attach | Self::Pr | Self::Git) }

    pub fn icon(self) -> gpui_kit::assets::IconName {
        use gpui_kit::assets::IconName;
        match self {
            | Self::General => IconName::Settings,
            | Self::Guidance | Self::Skills => IconName::Settings,
            | Self::Appearance => IconName::Palette,
            | Self::Accounts => IconName::Settings,
            | Self::Workspace | Self::Attach => IconName::Folder,
            | Self::Git => IconName::GitBranch,
            | Self::Pr => IconName::GitPullRequest,
            | Self::Mcp | Self::Plugins => IconName::Plug,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            | Self::General => "일반",
            | Self::Guidance => "지침",
            | Self::Skills => "스킬",
            | Self::Plugins => "플러그인",
            | Self::Appearance => "모양",
            | Self::Accounts => "계정",
            | Self::Workspace => "프로젝트",
            | Self::Git => "Git 상태",
            | Self::Pr => "GitHub PR",
            | Self::Mcp => "MCP 서버",
            | Self::Attach => "파일 첨부 / 문맥",
        }
    }

    pub fn nav_title(self) -> &'static str {
        match self {
            | Self::Workspace => "프로젝트",
            | Self::Attach => "첨부 파일",
            | _ => self.title(),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            | Self::General => "앱 언어와 키보드 단축키를 확인합니다.",
            | Self::Guidance => "앱 공통·프로젝트 지침(AGENTS.md)을 관리합니다. 다음 턴에 자동 적용됩니다.",
            | Self::Skills => "스킬(SKILL.md)을 생성·편집하고 공통 활성 상태를 관리합니다. 대화에서 사용할 스킬을 선택하세요.",
            | Self::Plugins => "플러그인을 가져오고 스킬과 MCP 연결을 관리합니다.",
            | Self::Appearance => "작업 화면의 테마, 타이틀바와 글꼴을 설정합니다.",
            | Self::Accounts => "제공자별 로그인 상태와 계정, 런타임 설치와 업데이트를 관리합니다. 기본 계정은 대화에서 사용합니다.",
            | Self::Workspace => "프로젝트를 열고 프로젝트별 메뉴에서 worktree를 관리합니다.",
            | Self::Git => "변경 파일을 검토하고 커밋과 작업 상태를 관리합니다.",
            | Self::Pr => "브랜치의 변경 사항을 검토하고 GitHub PR을 만듭니다.",
            | Self::Mcp => "제공자와 계정별 MCP stdio·HTTP 서버를 설정합니다.",
            | Self::Attach => "내 컴퓨터 파일이나 작업공간의 파일 문맥을 첨부합니다.",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Line,
    Text,
    Code,
}

pub enum Field {
    Code {
        state: Entity<EditorState>,
        last_value: String,
        _subscription: Subscription,
    },
    Line {
        state: Entity<InputState>,
        last_value: String,
        _subscription: Subscription,
    },
    Text {
        state: Entity<TextareaState>,
        last_value: String,
        _subscription: Subscription,
    },
}

pub fn column() -> Div { div().flex().flex_col().gap_3().min_w_0() }
pub fn row() -> Div { div().flex().flex_row().flex_wrap().items_center().gap_2() }
pub fn note(text: impl Into<SharedString>, cx: &App) -> Div { div().text_sm().text_color(cx.theme().muted_foreground).child(text.into()) }
pub fn error(text: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    row()
        .id("error-alert")
        .role(Role::Alert)
        .items_start()
        .p_3()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().danger.opacity(0.3))
        .bg(cx.theme().danger.opacity(0.08))
        .text_sm()
        .text_color(cx.theme().danger)
        .child(gpui::component::Icon::new(gpui::assets::IconName::CircleAlert).small())
        .child(div().flex_1().min_w_0().child(text.into()))
}
pub fn heading(text: impl Into<SharedString>) -> Div { div().text_lg().font_weight(FontWeight::SEMIBOLD).child(text.into()) }

pub fn empty_state(title: impl Into<SharedString>, description: impl Into<SharedString>) -> gpui::component::empty::Empty {
    use gpui::component::empty::{Empty,
                                 EmptyDescription,
                                 EmptyHeader,
                                 EmptyTitle};
    Empty::new().header(
        EmptyHeader::new()
            .title(EmptyTitle::new().child(title.into()))
            .description(EmptyDescription::new().child(description.into())),
    )
}

pub fn settings_group() -> Div { column().gap_4().p_4().border_1().rounded(px(12.)) }

pub fn apply_theme(cx: &mut App) {
    gpui_kit::component::Theme::update(cx, |theme| {
        let dark = theme.is_dark();
        theme.background = rgb(if dark { 0x0a0a0a } else { 0xfcfcfc }).into();
        theme.foreground = rgb(if dark { 0xf5f5f5 } else { 0x27272a }).into();
        theme.sidebar = rgb(if dark { 0x111111 } else { 0xfafafa }).into();
        theme.sidebar_foreground = theme.foreground;
        theme.muted = rgb(if dark { 0x181818 } else { 0xf4f4f5 }).into();
        theme.muted_foreground = rgb(if dark { 0x8e8e8e } else { 0x71717a }).into();
        theme.border = rgb(if dark { 0x202020 } else { 0xe4e4e7 }).into();
        theme.sidebar_border = theme.border;
        theme.input = theme.border;
        theme.popover = rgb(if dark { 0x141414 } else { 0xffffff }).into();
        theme.popover_foreground = theme.foreground;
        theme.accent = theme.muted;
        theme.accent_foreground = theme.foreground;
        theme.secondary = theme.muted;
        theme.secondary_foreground = theme.foreground;
    });
}

impl ChatApp {
    pub(super) fn clipboard(&self, id: impl Into<SharedString>, label: &str, value: String) -> gpui::component::clipboard::Clipboard {
        use gpui::component::{WindowExt as _,
                              notification::{Notification,
                                             NotificationType}};
        let copied = self.text("복사했습니다");
        gpui::component::clipboard::Clipboard::new(ElementId::Name(id.into()))
            .accessibility_label(self.text(label))
            .tooltip(self.text(label))
            .value(value)
            .on_copied(move |_, window, cx| {
                let message = copied.clone();
                window.push_notification(
                    Notification::new()
                        .with_type(NotificationType::Success)
                        .id::<gpui::component::clipboard::Clipboard>()
                        .content(move |_, _, _| div().id("clipboard-feedback").test_support().child(message.clone()).into_any_element()),
                    cx,
                );
            })
    }

    pub(super) fn settings_section(&self, title: impl Into<SharedString>, body: impl IntoElement, cx: &App) -> Div {
        let title = title.into();
        if !self.settings_section_matches(&title) {
            return div();
        }
        column()
            .gap_2()
            .child(note(title, cx).px_3())
            .child(settings_group().border_color(cx.theme().border).child(body))
    }

    pub(super) fn text(&self, text: &str) -> String { self.language.text(text).to_owned() }

    pub(super) fn icon_button(
        &self,
        id: impl Into<SharedString>,
        label: &str,
        icon: gpui_kit::assets::IconName,
        enabled: bool,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        self.button(id, "", enabled, cx, action)
            .icon(icon)
            .ghost()
            .size_7()
            .accessibility_label(self.text(label))
            .tooltip(self.text(label))
    }

    pub(super) fn button(
        &self,
        id: impl Into<SharedString>,
        label: impl AsRef<str>,
        enabled: bool,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        Button::new(ElementId::Name(id.into()))
            .small()
            .flex_shrink_0()
            .self_start()
            .when(!label.as_ref().is_empty(), |button| button.label(self.text(label.as_ref())))
            .disabled(!enabled)
            .on_click(cx.listener(move |this, _, window, cx| {
                let previous_modal = (this.panel, this.chat_tool, this.dialog.is_some());
                action(this, window, cx);
                if previous_modal != (this.panel, this.chat_tool, this.dialog.is_some()) {
                    // Modal transitions can unmount the focused input.
                    this.focus.focus(window, cx);
                }
                cx.notify();
            }))
    }

    pub(super) fn sidebar_button(
        &self,
        id: impl Into<SharedString>,
        label: &str,
        icon: gpui_kit::assets::IconName,
        enabled: bool,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        self.button(id, "", enabled, cx, action)
            .ghost()
            .w_full()
            .h_8()
            .accessibility_label(self.text(label))
            .tooltip(self.text(label))
            .child(
                row().w_full().flex_nowrap().gap_2().child(gpui_kit::component::Icon::new(icon).small()).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(self.text(label)),
                ),
            )
    }

    pub(super) fn choice(
        &self,
        id: impl Into<SharedString>,
        label: impl AsRef<str>,
        selected: bool,
        enabled: bool,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        self.button(id, label, enabled, cx, action).selected(selected).toggled(selected)
    }

    pub(super) fn selection_group<T: Copy + PartialEq + 'static>(
        &self,
        id: impl Into<String>,
        choices: impl IntoIterator<Item = (String, String, T, Option<gpui_kit::assets::IconName>)>,
        selected: T,
        enabled: bool,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let id = id.into();
        let choices: Vec<_> = choices.into_iter().collect();
        let toggles = choices
            .iter()
            .map(|(id, label, value, icon)| {
                let label = self.text(label);
                let toggle = Toggle::new(id.clone()).gap_1().tooltip(label.clone()).checked(*value == selected);
                let toggle = if let Some(icon) = icon { toggle.icon(*icon) } else { toggle };
                toggle.label(label)
            })
            .collect::<Vec<_>>();
        let group = ToggleGroup::new(format!("{id}-control"))
            .segmented()
            .outline()
            .small()
            .disabled(!enabled)
            .children(toggles)
            .on_click(cx.listener(move |s, states: &Vec<bool>, window, cx| {
                if let Some(((_, _, value, _), _)) = choices.iter().zip(states).find(|((_, _, value, _), checked)| **checked != (*value == selected)) {
                    action(s, *value, window, cx);
                    cx.notify();
                }
            }));
        div().id(id).test_support().child(group).into_any_element()
    }

    /// A deliberately flat checkbox on the headless base widget. The styled
    /// component's nested flex tree (spring-animated mark, `flex_1` label
    /// column) cost ~5x a plain text row in layout, which added up on form
    /// panels with several checkboxes.
    pub(super) fn check(
        &self,
        id: impl Into<SharedString>,
        label: impl AsRef<str>,
        checked: bool,
        enabled: bool,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, bool) + 'static,
    ) -> gpui::base::Checkbox {
        let theme = cx.theme();
        let (border, fill, mark) = if enabled {
            (
                if checked { theme.primary } else { theme.input },
                if checked { theme.primary } else { theme.input_background() },
                theme.primary_foreground,
            )
        } else {
            (
                theme.input.opacity(0.5),
                if checked { theme.primary.opacity(0.5) } else { theme.input_background() },
                theme.primary_foreground.opacity(0.5),
            )
        };
        let text_color = if enabled { theme.foreground } else { theme.muted_foreground };
        let indicator = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(16.))
            .flex_shrink_0()
            .border_1()
            .border_color(border)
            .bg(fill)
            .rounded(px(4.))
            .text_xs()
            .text_color(mark)
            .children(checked.then_some("✓"));
        let view = cx.entity().downgrade();
        gpui::base::Checkbox::new(ElementId::Name(id.into()))
            .checked(checked)
            .disabled(!enabled)
            .accessibility_label(self.text(label.as_ref()))
            .on_change(move |next, _, _, cx| {
                let value = matches!(next, gpui::base::CheckboxState::Checked);
                let _ = view.update(cx, |this, cx| {
                    action(this, value);
                    cx.notify();
                });
            })
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .text_color(text_color)
            .child(indicator)
            .child(div().min_w_0().child(self.text(label.as_ref())))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn field(
        &mut self,
        id: impl Into<String>,
        label: &str,
        value: String,
        kind: FieldKind,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        update: impl Fn(&mut Self, String) + 'static,
    ) -> AnyElement {
        let id = id.into();
        self.used_fields.insert(id.clone());
        if !self.inputs.contains_key(&id) {
            let field = if kind == FieldKind::Code {
                let state = cx.new(|cx| {
                    EditorState::new(window, cx)
                        .language("json")
                        .line_number(true)
                        .folding(true)
                        .default_value(value.clone())
                });
                let input_id = id.clone();
                let subscription = cx.subscribe_in(&state, window, move |this, state, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = state.read(cx).value().to_string();
                        if let Some(Field::Code {
                            last_value, ..
                        }) = this.inputs.get_mut(&input_id)
                        {
                            *last_value = value.clone();
                        }
                        update(this, value);
                        cx.notify();
                    }
                });
                Field::Code {
                    state,
                    last_value: value.clone(),
                    _subscription: subscription,
                }
            } else if kind == FieldKind::Text {
                let composer = id.starts_with("composer-");
                let state = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .auto_grow(if id == "guidance-text" { 12 } else { 3 }, if id == "guidance-text" { 18 } else { 8 })
                        .submit_on_enter(composer)
                        .default_value(value.clone())
                });
                let input_id = id.clone();
                let subscription = cx.subscribe_in(&state, window, move |this, state, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = state.read(cx).value().to_string();
                        // A local echo must not overwrite the next IME preedit
                        // on redraw.
                        let mut previous = None;
                        if let Some(Field::Text {
                            last_value, ..
                        }) = this.inputs.get_mut(&input_id)
                        {
                            previous = Some(std::mem::replace(last_value, value.clone()));
                        }
                        let editing = this.conversations[this.selected].editing.is_some();
                        let redraw = composer_change_needs_redraw(&input_id, previous.as_deref(), &value, editing);
                        update(this, value);
                        if redraw {
                            cx.notify();
                        }
                    }
                });
                Field::Text {
                    state,
                    last_value: value.clone(),
                    _subscription: subscription,
                }
            } else {
                let state = cx.new(|cx| InputState::new(window, cx).default_value(value.clone()));
                let input_id = id.clone();
                let subscription = cx.subscribe_in(&state, window, move |this, state, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = state.read(cx).value().to_string();
                        if let Some(Field::Line {
                            last_value, ..
                        }) = this.inputs.get_mut(&input_id)
                        {
                            *last_value = value.clone();
                        }
                        update(this, value);
                        cx.notify();
                    }
                });
                Field::Line {
                    state,
                    last_value: value.clone(),
                    _subscription: subscription,
                }
            };
            self.inputs.insert(id.clone(), field);
        }
        let label = self.text(label);
        // List rows (e.g. one verification command per row) carry their own
        // heading, so they render as bare inputs labelled by placeholder.
        let list_row = id.starts_with("verification-command-");
        let placeholder = if id.starts_with("composer-") || id.starts_with("search-") || id == "settings-search" || list_row {
            label.clone()
        } else {
            String::new()
        };
        // Change events can still be pending during a redraw; only external
        // model changes replace an edit.
        let input = match self.inputs.get_mut(&id).expect("field was inserted above") {
            | Field::Code {
                state,
                last_value,
                ..
            } => {
                if *last_value != value {
                    *last_value = value.clone();
                    if state.read(cx).value().as_ref() != value {
                        state.update(cx, |s, cx| s.set_value(value, window, cx));
                    }
                }
                Editor::new(state).aria_label(label.clone()).disabled(!enabled).h(px(240.)).into_any_element()
            },
            | Field::Line {
                state,
                last_value,
                ..
            } => {
                if *last_value != value {
                    *last_value = value.clone();
                    if state.read(cx).value().as_ref() != value {
                        state.update(cx, |s, cx| s.set_value(value, window, cx));
                    }
                }
                if state.read(cx).presentation().placeholder().as_ref() != placeholder {
                    state.update(cx, |s, cx| s.set_placeholder(placeholder.clone(), window, cx));
                }
                Input::new(state)
                    .aria_label(label.clone())
                    .disabled(!enabled)
                    .appearance(!id.starts_with("search-") && id != "settings-search")
                    .bordered(!id.starts_with("search-") && id != "settings-search")
                    .into_any_element()
            },
            | Field::Text {
                state,
                last_value,
                ..
            } => {
                if *last_value != value {
                    *last_value = value.clone();
                    if state.read(cx).value().as_ref() != value {
                        state.update(cx, |s, cx| s.set_value(value, window, cx));
                    }
                }
                {
                    if state.read(cx).presentation().placeholder().as_ref() != placeholder {
                        state.update(cx, |s, cx| s.set_placeholder(placeholder.clone(), window, cx));
                    }
                    let input = Textarea::new(state)
                        .aria_label(label.clone())
                        .disabled(!enabled)
                        .appearance(!id.starts_with("composer-"))
                        .bordered(!id.starts_with("composer-"));
                    if id.starts_with("composer-") && window.viewport_size().height < px(520.) {
                        input.h(px(48.)).into_any_element()
                    } else {
                        input.into_any_element()
                    }
                }
            },
        };
        if id == "settings-search" || id.starts_with("composer-") || id.starts_with("search-") || list_row {
            input
        } else if self.panel.is_some()
            && self.dialog.is_none()
            && !matches!(kind, FieldKind::Text | FieldKind::Code)
            && window.viewport_size().width >= px(1040.)
        {
            row()
                .flex_nowrap()
                .items_start()
                .gap_6()
                .py_2()
                .child(div().w(px(200.)).flex_shrink_0().child(note(label, cx)))
                .child(div().flex_1().min_w_0().child(input))
                .into_any_element()
        } else {
            column().gap_1().child(note(label, cx)).child(input).into_any_element()
        }
    }

    pub(super) fn open_panel(&mut self, mut panel: Panel) {
        if matches!(panel, Panel::Guidance | Panel::Skills) {
            if self.busy && matches!(self.panel, Some(Panel::Guidance | Panel::Skills)) && self.panel != Some(panel) {
                return;
            }
            let skill = panel == Panel::Skills;
            if self.guidance_editor.file.as_ref().is_some_and(|file| file.id.ends_with("/SKILL.md") != skill) {
                if self.guidance_editor.dirty() {
                    self.guidance_editor.message = self.text("저장하지 않은 변경이 있습니다. 저장하거나 편집을 취소하세요.");
                    panel = if skill { Panel::Guidance } else { Panel::Skills };
                } else {
                    self.guidance_editor.file = None;
                    self.guidance_editor.text.clear();
                    self.guidance_editor.message.clear();
                }
            }
        }
        if panel.is_chat_tool() {
            self.settings_search.clear();
            self.close_panel();
            self.chat_tool_scroll.set_offset(Point::default());
            self.chat_tool = Some(panel);
            self.wake.notify();
            return;
        }
        self.close_chat_tool();
        if self.panel.is_none() {
            self.accounts.selected = self.selected;
        }
        if self.panel != Some(panel) {
            self.panel_scroll.set_offset(Point::default());
        }
        self.panel = Some(panel);
        self.settings_nav_open = false;
        self.settings_search.clear();
        self.settings_result = 0;
        match panel {
            | Panel::Plugins => self.open_plugins(),
            | Panel::Guidance | Panel::Skills => self.open_guidance_editor(),
            | Panel::Accounts =>
                for provider in gpui_agent_contracts::PROVIDERS {
                    let index = gpui_agent_contracts::provider_index(provider);
                    if let Some(account) = self.accounts.providers[index].accounts.iter_mut().find(|a| a.profile.id == "default")
                        && !account.busy
                        && self.conversations[index].status.is_some()
                    {
                        account.auth.status = self.conversations[index].status.clone();
                    }
                    self.refresh_accounts(provider);
                },
            | Panel::Mcp => {
                self.mcp.open = true;
                self.open_plugins();
            },
            | _ => {},
        }
        self.wake.notify();
    }

    pub(super) fn close_panel(&mut self) {
        self.close_chat_tool();
        self.panel = None;
        self.settings_nav_open = false;
        self.mcp.open = false;
    }

    pub(super) fn close_chat_tool(&mut self) {
        self.computer.confirm = None;
        self.attachment_ui.picker = None;
        self.chat_tool = None;
    }

    pub(super) fn set_language(&mut self, language: Language) {
        self.language = language;
        if let Some(path) = &self.language_path
            && let Err(error) = crate::language::save(language, path)
        {
            self.conversations[self.selected].error = Some(format!("Cannot save language preference: {error}"));
        }
    }

    pub(super) fn tick(&mut self, cx: &mut gpui::Context<Self>) {
        if self.poll() {
            cx.notify();
        }
    }

    pub(super) fn handle_wakes(&mut self, events: impl IntoIterator<Item = UiEvent>, cx: &mut gpui::Context<Self>) {
        let mut force = false;
        for event in events {
            match event {
                | UiEvent::Changed => force = true,
                | UiEvent::OpenUrl(url) => {
                    cx.open_url(&url);
                    force = true;
                },
            }
        }
        let changed = self.poll();
        if changed || force {
            cx.notify();
        }
    }

    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        let wake = self.wake.clone();
        changed |= self.updates(&wake);
        changed |= self.poll_git();
        changed |= self.poll_plugins();
        changed |= self.account_updates();
        changed |= self
            .job_monitor
            .poll(&wake, &self.client, self.providers.iter().any(|p| p.capabilities.background_jobs));
        changed |= self.mcp.poll(&wake, &self.client);
        if std::mem::take(&mut self.mcp.catalog_changed) {
            self.plugins.conversation_mcp_loaded = false;
            self.plugins.conversation_mcp_pending = None;
        }
        changed |= self.computer.poll(&wake, &self.client, false);
        if let Some(job) = self.computer_gate().map(|(_, job)| job).or_else(|| self.computer_pending_job()) {
            self.computer.target = Some(job);
            changed |= self.computer.poll(&wake, &self.client, true);
        }
        if !self.busy && self.search_changed_at.is_some_and(|at| at.elapsed() >= std::time::Duration::from_millis(250)) {
            changed = true;
            self.command(gpui_agent_presentation::Action::Search(self.conversations[self.selected].search.clone()));
        }
        if !self.busy && self.job_monitor.activity.take_changed(self.client.workspace_id()) {
            changed = true;
            self.command(gpui_agent_presentation::Action::Search(self.conversations[self.selected].query.clone()));
        }
        let remaining = self.computer.remaining();
        if (self.computer_pending_job().is_some() || self.computer_gate().is_some()) && remaining != self.last_computer_remaining {
            changed = true;
        }
        self.last_computer_remaining = remaining;
        changed
    }
}

/// Typing in the chat composer only needs a root redraw when something outside
/// the textarea depends on the draft: the send button (emptiness), `$skill`
/// suggestions, or edit-mode revision state. Other keystrokes are drawn by the
/// textarea itself.
pub(super) fn composer_change_needs_redraw(id: &str, previous: Option<&str>, value: &str, editing: bool) -> bool {
    if !id.starts_with("composer-") || editing {
        return true;
    }
    let Some(previous) = previous else {
        return true;
    };
    previous.trim().is_empty() != value.trim().is_empty() || previous.contains('$') || value.contains('$')
}

#[cfg(test)]
mod composer_redraw_tests {
    use super::composer_change_needs_redraw as redraw;

    #[test]
    fn given_plain_typing_in_the_composer_when_text_stays_non_empty_then_root_is_not_redrawn() {
        assert!(!redraw("composer-None-0", Some("hel"), "hell", false));
    }

    #[test]
    fn given_the_draft_becomes_empty_or_non_empty_or_uses_skills_then_root_is_redrawn() {
        assert!(redraw("composer-None-0", Some(""), "h", false));
        assert!(redraw("composer-None-0", Some("h"), "", false));
        assert!(redraw("composer-None-0", Some("a"), "a $", false));
        assert!(redraw("composer-None-0", Some("a $s"), "a ", false));
    }

    #[test]
    fn given_other_fields_or_edit_mode_then_root_is_redrawn() {
        assert!(redraw("guidance-text", Some("a"), "ab", false));
        assert!(redraw("composer-None-0", Some("a"), "ab", true));
    }
}
