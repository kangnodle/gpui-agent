use crate::{app::ChatApp,
            content,
            ui::*};
use gpui::{assets::IconName,
           base::TestSupportExt as _,
           component::scroll::ScrollableElement,
           *};
use gpui_agent_contracts::*;
use gpui_agent_presentation::Action as Command;
use std::{collections::HashMap,
          hash::{DefaultHasher,
                 Hash as _,
                 Hasher as _},
          rc::Rc};

#[derive(Clone, Copy)]
pub(super) enum MessageSource {
    Chat,
    Reference,
}

pub(super) struct MessageList {
    scope: String,
    pub(super) state: ListState,
    /// Per-row layout fingerprints from the previous frame; only rows whose
    /// fingerprint changed are remeasured.
    signatures: Vec<u64>,
    /// Non-work timeline entries, rebuilt once per frame instead of per row.
    timeline: Rc<[TimelineEntry]>,
}

impl Default for MessageList {
    fn default() -> Self {
        Self {
            scope: String::new(),
            state: ListState::new(0, ListAlignment::Top, px(200.)),
            signatures: Vec::new(),
            timeline: Rc::from([]),
        }
    }
}

impl MessageList {
    pub(super) fn prepare(&mut self, scope: String, signatures: Vec<u64>, timeline: Rc<[TimelineEntry]>) -> ListState {
        let count = signatures.len();
        self.timeline = timeline;
        if self.scope != scope {
            self.scope = scope;
            self.state.reset(count);
            self.state.set_follow_mode(FollowMode::Normal);
        } else {
            let old = self.state.item_count();
            if old != count {
                self.state.splice(old.min(count) .. old, count.saturating_sub(old));
            }
            // Streaming text, expanded tools and parsed Markdown change row
            // heights; width changes are handled by the list itself.
            let mut run: Option<usize> = None;
            for n in 0 ..= count {
                let changed = signatures
                    .get(n)
                    .is_some_and(|sig| self.signatures.get(n).is_some_and(|previous| previous != sig));
                match (changed, run) {
                    | (true, None) => run = Some(n),
                    | (false, Some(start)) => {
                        self.state.remeasure_items(start .. n);
                        run = None;
                    },
                    | _ => {},
                }
            }
        }
        self.signatures = signatures;
        self.state.clone()
    }
}

struct MessageData<'a> {
    id: String,
    messages: &'a [SavedMessage],
    timeline: &'a [TimelineEntry],
    query: &'a str,
    provider: Provider,
    editable: bool,
    live: bool,
}

impl ChatApp {
    fn message_data(&self, source: MessageSource) -> MessageData<'_> {
        match source {
            | MessageSource::Chat => {
                let c = &self.conversations[self.selected];
                let id = format!("chat-{:?}-{}", self.client.workspace_id(), self.selected);
                MessageData {
                    id,
                    messages: &c.messages,
                    timeline: &c.timeline,
                    query: &c.view_query,
                    provider: PROVIDERS[self.selected],
                    editable: true,
                    live: self.busy,
                }
            },
            | MessageSource::Reference => {
                let r = self.reference.as_ref();
                let id = r
                    .map(|r| format!("reference-{:?}-{}-{}", self.client.workspace_id(), r.provider, r.conversation.id))
                    .unwrap_or_default();
                MessageData {
                    id,
                    messages: r.map_or(&[], |r| &r.conversation.messages),
                    timeline: r.map_or(&[], |r| &r.conversation.timeline),
                    query: "",
                    provider: r.map_or(PROVIDERS[0], |r| PROVIDERS[r.provider]),
                    editable: false,
                    live: false,
                }
            },
        }
    }

    /// Identity of the conversation shown by `source`; a change resets the
    /// list.
    fn message_scope(&self, source: MessageSource, id: &str) -> String {
        match source {
            | MessageSource::Chat => {
                let c = &self.conversations[self.selected];
                format!("{id}-{:?}-{:?}", c.account_id, c.id)
            },
            | MessageSource::Reference => id.to_owned(),
        }
    }

    /// Fingerprint of everything that can change a row's height, so unchanged
    /// rows keep their measured size.
    fn row_signatures(&self, data: &MessageData<'_>) -> Vec<u64> {
        let mut entries: HashMap<i64, usize> = HashMap::new();
        for entry in data.timeline {
            *entries.entry(entry.message_id).or_default() += 1;
        }
        let (versions, editable_question) = if data.editable {
            let c = &self.conversations[self.selected];
            (c.versions.len(), c.last_completed_question())
        } else {
            (0, None)
        };
        let last = data.messages.len().wrapping_sub(1);
        data.messages
            .iter()
            .enumerate()
            .map(|(n, m)| {
                let mut hasher = DefaultHasher::new();
                (
                    m.text.len(),
                    m.user,
                    m.incomplete,
                    m.attachments.len(),
                    m.skills.len(),
                    m.guidance.is_some(),
                    m.id.and_then(|id| entries.get(&id).copied()),
                    data.query.is_empty(),
                    data.live && n == last,
                    versions,
                    editable_question == Some(n),
                    // Only toggles that belong to this row (or verification
                    // checks, which are shared) can change its height.
                    self.expanded_tools
                        .iter()
                        .filter(|key| key.starts_with(&format!("{}-{n}-", data.id)) || key.starts_with("verification-"))
                        .count(),
                )
                    .hash(&mut hasher);
                hasher.finish()
            })
            .collect()
    }

    pub(super) fn messages_view(&self, source: MessageSource, cx: &Context<Self>) -> impl IntoElement + Styled + use<> {
        let data = self.message_data(source);
        let scope = self.message_scope(source, &data.id);
        let signatures = self.row_signatures(&data);
        let timeline: Rc<[TimelineEntry]> = if data.editable {
            data.timeline
                .iter()
                .filter(|entry| !gpui_agent_presentation::is_work_event(&entry.event))
                .map(|entry| TimelineEntry {
                    message_id: entry.message_id,
                    event: entry.event.clone(),
                })
                .collect()
        } else {
            Rc::from([])
        };
        let state = self.message_lists.borrow_mut()[source as usize].prepare(scope, signatures, timeline);
        let id = match source {
            | MessageSource::Chat => "messages-scroll",
            | MessageSource::Reference => "reference-scroll",
        };
        div()
            .id(id)
            .test_support()
            .role(Role::Log)
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .child(
                list(state.clone(), cx.processor(move |s, index, _, cx| s.message_row(source, index, cx)))
                    .size_full()
                    .py_4(),
            )
            .vertical_scrollbar(&state)
    }

    fn message_row(&self, source: MessageSource, n: usize, cx: &Context<Self>) -> AnyElement {
        let MessageData {
            id,
            messages,
            timeline,
            query,
            provider,
            editable,
            live,
            ..
        } = self.message_data(source);
        let Some(m) = messages.get(n) else {
            return Empty.into_any_element();
        };
        let filtered = self.message_lists.borrow()[source as usize].timeline.clone();
        let conversation_timeline = if editable { &*filtered } else { timeline };
        let key = format!("{id}-{n}");
        let text = m.text.clone();
        let mut item = column()
            .id(format!("{key}-bubble"))
            .test_support()
            .w_full()
            .max_w(relative(if m.user { 0.8 } else { 1. }))
            .gap_2()
            .p_3()
            .rounded(px(16.))
            .bg(if m.user { cx.theme().muted } else { cx.theme().background })
            .child(self.attachments_view(&key, &m.attachments, cx));
        if m.guidance.is_some() && !m.skills.is_empty() {
            item = item.child(note(
                format!(
                    "{}: {}",
                    self.text("이 턴에 적용된 스킬"),
                    m.skills.iter().map(|skill| skill.id.as_str()).collect::<Vec<_>>().join(", ")
                ),
                cx,
            ));
        }
        if query.is_empty() && !m.user {
            item = item.child(content::markdown(key.clone(), m.text.clone()));
        } else if !query.is_empty() {
            let ranges = match_ranges(&m.text, query).into_iter().map(|r| {
                (
                    r,
                    HighlightStyle {
                        background_color: Some(cx.theme().selection),
                        ..Default::default()
                    },
                )
            });
            item = item.child(StyledText::new(m.text.clone()).with_highlights(ranges));
        } else {
            item = item.child(div().whitespace_normal().line_height(relative(1.6)).child(m.text.clone()));
        }
        if !m.user {
            item = item.child(self.timeline_view(&key, conversation_timeline, m.id.unwrap_or(-1), cx));
            if editable {
                for tool in gpui_agent_presentation::tools(timeline, m.id.unwrap_or(-1))
                    .iter()
                    .filter(|tool| tool.status.as_deref() == Some("failed"))
                {
                    item = item.child(error(
                        format!("{} · {}", self.text("도구 실행 실패"), tool.title.as_deref().unwrap_or(&tool.id)),
                        cx,
                    ));
                }
            }
        }
        if m.incomplete {
            item = item.child(note(self.text(crate::views::incomplete_note(live && n + 1 == messages.len())), cx));
        }
        if m.user {
            item = item.ml_auto();
        } else {
            item = item.px_1().py(px(2.));
        }
        let copy = self.clipboard(format!("{key}-copy"), "복사", text).accessibility_label(format!(
            "{} · {}",
            if m.user { self.text("나") } else { display_name(provider).into() },
            self.text("복사")
        ));
        let mut actions = row().gap_1().child(copy);
        if editable && self.conversations[self.selected].last_completed_question() == Some(n) {
            actions = actions.child(
                self.icon_button(format!("{key}-edit"), "질문 수정", IconName::SquarePen, !self.busy, cx, |s, _, _| {
                    let conversation = &mut s.conversations[s.selected];
                    if let Some(index) = conversation.last_completed_question() {
                        conversation.editing = Some(conversation.messages[index].text.clone());
                        conversation.skills_before_edit = Some(std::mem::replace(
                            &mut conversation.guidance.selected,
                            conversation.messages[index].skills.clone(),
                        ));
                        s.workspaces.focus_composer = true;
                    }
                })
                .opacity(0.)
                .group_hover("message", |style| style.opacity(1.))
                .focus(|style| style.opacity(1.)),
            );
        }
        if editable {
            let conversation = &self.conversations[self.selected];
            if conversation.versions.len() > 1
                && conversation.versions.first().is_some_and(|version| version.question_index == n)
                && let Some(current) = conversation
                    .versions
                    .iter()
                    .position(|version| Some(version.conversation_id) == conversation.id)
            {
                for (offset, label) in [(-1isize, "이전 버전"), (1, "다음 버전")] {
                    let target = current
                        .checked_add_signed(offset)
                        .and_then(|index| conversation.versions.get(index))
                        .map(|version| version.conversation_id);
                    actions = actions.child(
                        self.button(format!("{key}-version-{offset}"), label, !self.busy && target.is_some(), cx, move |s, _, _| {
                            if let Some(id) = target {
                                s.command(Command::Load {
                                    id,
                                    message_id: None,
                                });
                            }
                        })
                        .ghost(),
                    );
                    if offset == -1 {
                        actions = actions.child(note(format!("{} {}/{}", self.text("버전"), current + 1, conversation.versions.len()), cx));
                    }
                }
            }
        }
        let actions = if m.user { actions.justify_end() } else { actions };
        div()
            .w_full()
            .px_4()
            .py_3()
            .child(column().group("message").w_full().max_w(px(736.)).mx_auto().gap_1().child(item).child(actions))
            .into_any_element()
    }
}
