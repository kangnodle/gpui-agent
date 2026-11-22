use crate::{app::ChatApp,
            ui::*};
use gpui::{self,
           assets::IconName,
           base::{InteractiveElementExt as _,
                  TestSupportExt as _},
           *};
use gpui_agent_presentation::RightPanels;
use std::path::Path;

pub(super) type TimelineStamp = (usize, Option<i64>, usize, usize, u64);

#[derive(Clone)]
pub(super) struct TimelineFacts {
    pub has_work: bool,
    pub usage: gpui_agent_contracts::UsageReport,
}

pub(super) fn load(language_path: Option<&Path>) -> RightPanels {
    let mut layout = RightPanels::default();
    let Some(path) = language_path else {
        return layout;
    };
    let saved = std::fs::read_to_string(path.with_file_name("ui-right-panels.json"))
        .ok()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok());
    if let Some(saved) = saved {
        layout.work_open = saved["work_open"].as_bool().unwrap_or(layout.work_open);
        layout.reference_open = saved["reference_open"].as_bool().unwrap_or(layout.reference_open);
        layout.width = saved["width"].as_f64().map(|value| value as f32).unwrap_or(layout.width);
        layout.tool_width = saved["tool_width"].as_f64().map(|value| value as f32);
        layout.work_ratio = saved["work_ratio"].as_f64().map(|value| value as f32).unwrap_or(layout.work_ratio);
    } else if let Some(width) = std::fs::read_to_string(path.with_file_name("ui-reference-width"))
        .ok()
        .and_then(|value| value.trim().parse().ok())
    {
        layout.width = width;
    }
    layout.normalized()
}

#[derive(Clone)]
struct ResizePanelSplit;
impl Render for ResizePanelSplit {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { Empty }
}

impl ChatApp {
    pub(super) fn save_right_panels(&mut self) {
        let Some(path) = &self.language_path else {
            return;
        };
        let path = path.with_file_name("ui-right-panels.json");
        let layout = self.right_panels.normalized();
        let saved = serde_json::json!({"work_open":layout.work_open,"reference_open":layout.reference_open,"width":layout.width,"work_ratio":layout.work_ratio,"tool_width":layout.tool_width});
        let result = std::fs::create_dir_all(path.parent().expect("preference directory")).and_then(|_| std::fs::write(path, saved.to_string()));
        if let Err(error) = result {
            self.conversations[self.selected].error = Some(format!("Cannot save panel layout: {error}"));
        }
    }

    pub(super) fn toggle_reference(&mut self) {
        self.right_panels.reference_open = !self.right_panels.reference_open || self.chat_tool.is_some();
        self.close_chat_tool();
        self.save_right_panels();
    }

    pub(super) fn finish_panel_resize(&mut self) {
        let width = std::mem::take(&mut self.reference_resizing);
        let split = std::mem::take(&mut self.panel_split_resizing);
        let tool = std::mem::take(&mut self.tool_resizing);
        if width || split || tool {
            self.save_right_panels();
        }
    }

    /// Cheap identity of the selected conversation's timeline. Caches keyed by
    /// it are rebuilt when the timeline is replaced, cleared and refilled, or
    /// appended to.
    pub(super) fn timeline_stamp(&self) -> TimelineStamp {
        use std::hash::{Hash as _,
                        Hasher as _};
        let conversation = &self.conversations[self.selected];
        let timeline = &conversation.timeline;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for entry in [timeline.first(), timeline.last()].into_iter().flatten() {
            (entry.message_id, std::mem::discriminant(&entry.event)).hash(&mut hasher);
        }
        (self.selected, conversation.id, timeline.as_ptr() as usize, timeline.len(), hasher.finish())
    }

    pub(super) fn has_work_activity(&self) -> bool { self.cached_timeline_facts().has_work }

    /// Whole-timeline scans (work events, usage totals) run only when the
    /// timeline changes, not on every frame.
    pub(super) fn cached_timeline_facts(&self) -> TimelineFacts {
        let timeline = &self.conversations[self.selected].timeline;
        let stamp = self.timeline_stamp();
        let mut cache = self.timeline_facts.borrow_mut();
        if let Some((cached, facts)) = cache.as_ref()
            && *cached == stamp
        {
            return facts.clone();
        }
        let facts = TimelineFacts {
            has_work: timeline.iter().any(|entry| gpui_agent_presentation::is_work_event(&entry.event)),
            usage: gpui_agent_contracts::UsageReport::recorded(timeline, 0),
        };
        *cache = Some((stamp, facts.clone()));
        facts
    }

    fn reference_panel(&self, cx: &Context<Self>) -> AnyElement {
        column()
            .id("reference-panel")
            .test_support()
            .gap_0()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                row()
                    .h(px(44.))
                    .flex_shrink_0()
                    .px_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().flex_1().min_w_0().child(self.text("참고 대화")))
                    .child(self.icon_button("close-reference", "참고 패널 닫기", IconName::X, true, cx, |s, _, _| {
                        s.right_panels.reference_open = false;
                        s.save_right_panels();
                    })),
            )
            .children(self.reference.as_ref().map(|r| note(r.title.clone(), cx).px_3()))
            .child(self.messages_view(crate::messages::MessageSource::Reference, cx))
            .into_any_element()
    }

    pub(super) fn right_panel_stack(&self, resize_width: bool, cx: &Context<Self>) -> AnyElement {
        let work = self.right_panels.work_open && self.has_work_activity();
        let reference = self.right_panels.reference_open;
        let mut stack = column()
            .id("right-panels")
            .test_support()
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_0()
            .on_drag_move(cx.listener(|s, event: &DragMoveEvent<ResizePanelSplit>, _, cx| {
                let height = f32::from(event.bounds.size.height);
                if height > 0. {
                    let previous = s.right_panels.work_ratio;
                    s.right_panels.work_ratio = f32::from(event.event.position.y - event.bounds.top()) / height;
                    s.right_panels = s.right_panels.normalized();
                    if previous != s.right_panels.work_ratio {
                        s.panel_split_resizing = true;
                        cx.notify();
                    }
                }
            }));
        if work {
            let pane = div().flex().min_h_0().min_w_0().child(self.work_panel(cx));
            stack = stack.child(if reference {
                pane.relative().h(relative(self.right_panels.work_ratio)).min_h(px(80.)).flex_shrink_0().child(
                    gpui::component::resizable::resize_handle("right-panel-split", Axis::Vertical)
                        .inside(gpui::component::resizable::HandleEdge::Trailing)
                        .on_drag(ResizePanelSplit, |drag, _, _, cx| cx.new(|_| drag.clone()))
                        .on_mouse_up(MouseButton::Left, cx.listener(|s, _, _, _| s.finish_panel_resize()))
                        .on_double_click(cx.listener(|s, _, _, cx| {
                            s.right_panels.work_ratio = 0.5;
                            s.save_right_panels();
                            cx.notify();
                        })),
                )
            } else {
                pane.flex_1()
            });
        }
        if reference {
            stack = stack.child(self.reference_panel(cx));
        }
        if resize_width {
            stack = stack.child(
                gpui::component::resizable::resize_handle("reference-resize", Axis::Horizontal)
                    .inside(gpui::component::resizable::HandleEdge::Leading)
                    .on_drag(crate::views::ResizeReference, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .on_mouse_up(MouseButton::Left, cx.listener(|s, _, _, _| s.finish_panel_resize()))
                    .on_double_click(cx.listener(|s, _, _, cx| {
                        s.right_panels.width = 320.;
                        s.save_right_panels();
                        cx.notify();
                    })),
            );
        }
        stack.into_any_element()
    }
}
