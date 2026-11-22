use crate::{app::ChatApp,
            ui::*};
use gpui::{self,
           base::TestSupportExt as _,
           *};
use gpui_agent_contracts::{Attachment,
                           FileContextRequest,
                           PROVIDERS,
                           WorkspaceFiles};
use gpui_agent_presentation::Action;

pub(super) struct CachedImage {
    source: Attachment,
    image: Result<std::sync::Arc<gpui::RenderImage>, String>,
}

pub(super) struct DiffPreview {
    source: String,
    state: Result<Entity<gpui::component::diff::DiffState>, String>,
}

impl ChatApp {
    pub(super) fn diff_view(&mut self, id: &'static str, patch: &str, cx: &mut Context<Self>) -> AnyElement {
        use gpui::component::diff::{Diff,
                                    DiffFile,
                                    DiffState};
        let cached = self.workspaces.diff_views.entry(id).or_insert_with(|| DiffPreview {
            source: String::new(),
            state: Ok(cx.new(|cx| DiffState::new([], cx))),
        });
        if cached.source != patch {
            cached.source = patch.to_owned();
            cached.state = DiffFile::parse(patch)
                .map(|files| cx.new(|cx| DiffState::new(files, cx)))
                .map_err(|e| e.to_string());
        }
        if patch.trim().is_empty() {
            return div()
                .id(format!("{id}-empty"))
                .test_support()
                .child(empty_state(
                    self.text("변경 사항이 없습니다."),
                    self.text("다른 파일이나 스테이징 영역을 선택하세요."),
                ))
                .into_any_element();
        }
        match &cached.state {
            | Ok(state) => div().id(id).test_support().child(Diff::new(state).h(px(360.)).w_full()).into_any_element(),
            | Err(message) => column()
                .child(error(message.clone(), cx))
                .child(markdown(id, gpui_agent_presentation::plain(patch)))
                .into_any_element(),
        }
    }
}

#[derive(Default)]
pub(super) struct AttachUi {
    pub open: bool,
    pub workspace: bool,
    pub path: String,
    pub start: String,
    pub end: String,
    pub error: Option<String>,
    pub picker: Option<ContextPicker>,
}

pub(super) struct ContextPicker {
    path: String,
    result: Option<Result<WorkspaceFiles, String>>,
    pending: Option<std::sync::mpsc::Receiver<Result<WorkspaceFiles, String>>>,
}

impl AttachUi {
    pub fn poll(&mut self) {
        if let Some(picker) = &mut self.picker
            && let Some(pending) = &picker.pending
            && let Ok(result) = pending.try_recv()
        {
            picker.result = Some(result);
            picker.pending = None;
        }
    }
}

pub fn terminal_plain(text: &str, path: Option<&str>) -> String {
    let fenced = gpui_agent_presentation::plain(text);
    let language = if serde_json::from_str::<serde_json::Value>(text).is_ok() {
        Some("json")
    } else {
        path.and_then(|path| path.rsplit('.').next())
            .filter(|extension| extension.len() <= 16 && extension.chars().all(|c| c.is_ascii_alphanumeric()))
    };
    language.map_or(fenced.clone(), |language| fenced.replacen('\n', &format!("{language}\n"), 1))
}

/// Rendering a multi-hundred-KB output as one markdown block costs tens of
/// milliseconds per frame, so only the head is shown until the operator asks
/// for everything. Returns the text to show and the number of hidden bytes.
pub const OUTPUT_PREVIEW_BYTES: usize = 32 * 1024;

pub fn truncate_output(text: &str, full: bool) -> (&str, usize) {
    if full || text.len() <= OUTPUT_PREVIEW_BYTES {
        return (text, 0);
    }
    let mut end = OUTPUT_PREVIEW_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    // Cut at a line boundary when one is close, so the last line is whole.
    if let Some(newline) = text[.. end].rfind('\n').filter(|n| *n > end / 2) {
        end = newline;
    }
    (&text[.. end], text.len() - end)
}

pub fn terminal(id: impl Into<SharedString>, text: &str, path: Option<&str>) -> impl IntoElement {
    type Highlighter = dyn Fn(&gpui::base::text::CodeBlock) -> Vec<(std::ops::Range<usize>, HighlightStyle)> + Send + Sync;
    static HIGHLIGHTER: std::sync::OnceLock<std::sync::Arc<Highlighter>> = std::sync::OnceLock::new();
    gpui::base::TextView::markdown(ElementId::Name(id.into()), terminal_plain(text, path))
        .selectable(true)
        .scrollable(false)
        .style(
            gpui::base::TextViewStyle::default()
                .with_foreground(rgb(0xd4d4d4).into())
                .with_code_background(rgb(0x1e1e1e).into())
                .with_border(rgb(0x333333).into())
                .with_dark(true),
        )
        .shared_code_block_highlighter(
            HIGHLIGHTER
                .get_or_init(|| std::sync::Arc::new(|block| terminal_highlights(&block.code(), block.lang().as_deref())))
                .clone(),
        )
}

fn terminal_highlights(code: &str, language: Option<&str>) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    use syntect::{easy::HighlightLines,
                  highlighting::ThemeSet,
                  parsing::SyntaxSet,
                  util::LinesWithEndings};
    static SYNTAXES: std::sync::OnceLock<SyntaxSet> = std::sync::OnceLock::new();
    static THEMES: std::sync::OnceLock<ThemeSet> = std::sync::OnceLock::new();
    // shortcut: large output is unhighlighted, upgrade to asynchronous
    // highlighting for large files.
    if code.len() > 256 * 1024 {
        return Vec::new();
    }
    let syntaxes = SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines);
    let themes = THEMES.get_or_init(ThemeSet::load_defaults);
    let Some(syntax) = language.and_then(|lang| syntaxes.find_syntax_by_token(lang)) else {
        return Vec::new();
    };
    let Some(theme) = themes.themes.get("base16-ocean.dark") else {
        return Vec::new();
    };
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut offset = 0;
    let mut highlights = Vec::new();
    for line in LinesWithEndings::from(code) {
        let Ok(spans) = highlighter.highlight_line(line, syntaxes) else {
            return Vec::new();
        };
        for (style, span) in spans {
            let end = offset + span.len();
            highlights.push((
                offset .. end,
                HighlightStyle {
                    color: Some(rgb(u32::from(style.foreground.r) << 16 | u32::from(style.foreground.g) << 8 | u32::from(style.foreground.b)).into()),
                    ..Default::default()
                },
            ));
            offset = end;
        }
    }
    highlights
}

pub fn markdown(id: impl Into<SharedString>, text: String) -> impl IntoElement {
    // Document images never fetch remote URLs or decode unbounded data URLs.
    gpui::base::TextView::markdown(ElementId::Name(id.into()), text)
        .selectable(true)
        .scrollable(false)
        .image_source(|_| {
            static EMPTY: std::sync::OnceLock<std::sync::Arc<gpui::RenderImage>> = std::sync::OnceLock::new();
            EMPTY
                .get_or_init(|| std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(image::RgbaImage::new(1, 1))])))
                .clone()
                .into()
        })
        .on_link_click(|url, _, _, cx| {
            if ["https://", "http://", "mailto:"].iter().any(|scheme| url.starts_with(scheme)) {
                cx.open_url(url);
            }
        })
}

pub(super) fn image_preview(attachment: &Attachment) -> Result<std::sync::Arc<gpui::RenderImage>, String> {
    let bytes = attachment.image_bytes().map_err(str::to_owned)?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "Image preview unavailable (invalid image or exceeds 4096×4096)".to_owned())?;
    let image = if image.width() > 800 || image.height() > 440 {
        image.thumbnail(800, 440)
    } else {
        image
    }
    .to_rgba8();
    let mut image = image;
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Ok(std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(image)])))
}

impl ChatApp {
    fn browse_context(&mut self, path: String, cx: &mut Context<Self>) {
        let (provider, account) = (
            PROVIDERS[self.selected],
            self.conversations[self.selected].account_id.clone().unwrap_or_else(|| "default".into()),
        );
        let client = self.client.for_account(provider, account, self.client.workspace_id());
        let directory = path.clone();
        let (send, request) = std::sync::mpsc::channel();
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let result = client.workspace_files(provider, &directory).map_err(|error| error.to_string());
            let _ = send.send(result);
            wake.notify();
        });
        self.attachment_ui.picker = Some(ContextPicker {
            path,
            result: None,
            pending: Some(request),
        });
        cx.notify();
    }

    fn context_picker(&self, cx: &Context<Self>) -> AnyElement {
        let picker = self.attachment_ui.picker.as_ref().expect("opened context picker");
        let loading = picker.result.is_none();
        let parent = picker.path.rsplit_once('/').map(|(parent, _)| parent).unwrap_or("").to_owned();
        let mut body = column()
            .child(note(
                self.text("연결된 작업공간의 파일을 선택합니다. 선택 후 줄 범위를 지정하고 첨부하세요."),
                cx,
            ))
            .child(
                row()
                    .child(
                        self.button("context-root", "작업공간 루트", !loading && !picker.path.is_empty(), cx, |s, _, cx| {
                            s.browse_context(String::new(), cx)
                        }),
                    )
                    .child(
                        self.button("context-parent", "상위 폴더", !loading && !picker.path.is_empty(), cx, move |s, _, cx| {
                            s.browse_context(parent.clone(), cx)
                        }),
                    ),
            )
            .child(note(if picker.path.is_empty() { "/".into() } else { picker.path.clone() }, cx));
        match &picker.result {
            | None =>
                body = body.child(
                    div()
                        .id("context-loading")
                        .test_support()
                        .child(note(self.text("파일 목록을 불러오는 중…"), cx)),
                ),
            | Some(Err(message)) => {
                let path = picker.path.clone();
                body = body
                    .child(error(message.clone(), cx))
                    .child(self.button("context-retry", "다시 시도", true, cx, move |s, _, cx| s.browse_context(path.clone(), cx)));
            },
            | Some(Ok(listing)) => {
                if listing.entries.is_empty() {
                    body = body.child(
                        div()
                            .id("context-empty")
                            .test_support()
                            .child(note(self.text("이 폴더에 파일이 없습니다."), cx)),
                    );
                }
                let mut entries = column().gap_1();
                for (index, entry) in listing.entries.iter().enumerate() {
                    let path = entry.path.clone();
                    let directory = entry.directory;
                    let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
                    let mut button = self
                        .button(format!("context-entry-{index}"), name, true, cx, move |s, _, cx| {
                            if directory {
                                s.browse_context(path.clone(), cx);
                            } else {
                                s.attachment_ui.path = path.clone();
                                s.attachment_ui.picker = None;
                            }
                        })
                        .ghost()
                        .max_w_full()
                        .overflow_hidden()
                        .tooltip(entry.path.clone());
                    if directory {
                        button = button.icon(gpui::assets::IconName::Folder);
                    }
                    entries = entries.child(row().child(button));
                }
                body = body.child(div().id("context-files").max_h(px(320.)).overflow_y_scroll().child(entries));
                if listing.truncated {
                    body = body.child(note(self.text("항목이 많아 일부만 표시합니다. 파일 경로를 직접 입력할 수 있습니다."), cx));
                }
            },
        }
        body = body.child(self.button("context-cancel", "취소", true, cx, |s, _, _| s.attachment_ui.picker = None));
        self.settings_section(self.text("작업공간 파일 선택"), body, cx).into_any_element()
    }

    fn attach_action(&mut self, action: Action) {
        self.close_chat_tool();
        let wake = self.wake.clone();
        self.start(action, &wake);
        self.panel = None;
    }

    fn cached_image(&self, key: String, attachment: &Attachment) -> Result<std::sync::Arc<gpui::RenderImage>, String> {
        self.used_images.borrow_mut().insert(key.clone());
        let mut cache = self.images.borrow_mut();
        if cache.get(&key).is_none_or(|cached| cached.source != *attachment) {
            cache.insert(
                key.clone(),
                CachedImage {
                    source: attachment.clone(),
                    image: image_preview(attachment),
                },
            );
        }
        cache[&key].image.clone()
    }

    pub(super) fn attachment_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.attachment_ui.poll();
        if self.attachment_ui.picker.is_some() {
            return self.context_picker(cx);
        }
        let a = &self.attachment_ui;
        let mut body = column().child(
            row()
                .child(self.choice("attach-local", "내 컴퓨터 파일", !a.workspace, !self.busy, cx, |s, _, _| {
                    s.attachment_ui.workspace = false
                }))
                .child(self.choice("attach-context", "작업공간 파일 문맥", a.workspace, !self.busy, cx, |s, _, _| {
                    s.attachment_ui.workspace = true
                })),
        );
        body = body.child(self.field(
            "attachment-path",
            "파일 경로 (또는 창으로 파일 드롭)",
            a.path.clone(),
            FieldKind::Line,
            !self.busy,
            window,
            cx,
            |s, v| s.attachment_ui.path = v,
        ));
        if self.attachment_ui.workspace {
            body = body
                .child(self.field(
                    "context-start",
                    "시작 줄",
                    self.attachment_ui.start.clone(),
                    FieldKind::Line,
                    !self.busy,
                    window,
                    cx,
                    |s, v| s.attachment_ui.start = v,
                ))
                .child(self.field(
                    "context-end",
                    "끝 줄",
                    self.attachment_ui.end.clone(),
                    FieldKind::Line,
                    !self.busy,
                    window,
                    cx,
                    |s, v| s.attachment_ui.end = v,
                ))
                .child(note(self.text("줄 범위를 비우면 전체 파일을 첨부합니다."), cx));
        }
        if let Some(e) = &self.attachment_ui.error {
            body = body.child(error(e.clone(), cx));
        }
        body = body.child(note("UTF-8 256 KiB · PNG/JPEG/WebP 2 MiB · 8 files / 4 MiB", cx));
        let mut actions = row();
        if self.attachment_ui.workspace {
            actions = actions.child(self.button("attach-browse", "파일 선택…", !self.busy, cx, |s, _, cx| {
                s.browse_context(String::new(), cx)
            }));
        } else {
            actions = actions.child(self.button("attach-browse", "파일 선택…", !self.busy, cx, |s, _, cx| {
                let picked = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: true,
                    prompt: Some(s.text("파일 선택…").into()),
                });
                cx.spawn(async move |view, cx| {
                    let picked = picked.await;
                    let _ = view.update(cx, |s, cx| {
                        match picked {
                            | Ok(Ok(Some(paths))) if !paths.is_empty() => s.attach_action(Action::AttachFiles(paths)),
                            | Ok(Err(error)) => s.attachment_ui.error = Some(error.to_string()),
                            | _ => {},
                        }
                        cx.notify();
                    });
                })
                .detach();
            }));
        }
        let body = body.child(actions.child(self.button(
            "attach-add",
            "첨부 추가",
            !self.busy && !self.attachment_ui.path.trim().is_empty(),
            cx,
            |s, _, _| {
                let action = if s.attachment_ui.workspace {
                    let line = |v: &str| {
                        if v.trim().is_empty() {
                            Ok(None)
                        } else {
                            v.trim().parse::<usize>().map(Some).map_err(|_| "줄 번호를 확인하세요.")
                        }
                    };
                    line(&s.attachment_ui.start)
                        .and_then(|start_line| {
                            line(&s.attachment_ui.end).map(|end_line| FileContextRequest {
                                path: s.attachment_ui.path.trim().into(),
                                start_line,
                                end_line,
                            })
                        })
                        .and_then(|r| r.validate().map(|_| Action::Context(r)))
                } else {
                    Ok(Action::AttachFiles(vec![s.attachment_ui.path.trim().into()]))
                };
                match action {
                    | Ok(a) => {
                        s.attach_action(a);
                    },
                    | Err(e) => s.attachment_ui.error = Some(e.into()),
                }
            },
        )));
        self.settings_section(self.text("파일 첨부 / 문맥"), body, cx).into_any_element()
    }

    pub(super) fn attachments_view(&self, id: &str, attachments: &[Attachment], cx: &Context<Self>) -> AnyElement {
        let mut body = column();
        for (i, a) in attachments.iter().enumerate() {
            let mut item = column()
                .p_2()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius)
                .child(note(format!("📎 {}", a.name()), cx));
            match a {
                | Attachment::Text {
                    text,
                    source,
                    ..
                } => {
                    if let Some(source) = source {
                        item = item.child(note(
                            format!(
                                "{}:{}–{}",
                                source.path,
                                source.start_line.unwrap_or(1),
                                source.end_line.map(|v| v.to_string()).unwrap_or_default()
                            ),
                            cx,
                        ));
                    }
                    let copy = text.clone();
                    item = item.child(self.clipboard(format!("{id}-copy-{i}"), "첨부 복사", copy)).child(
                        div()
                            .id(ElementId::Name(format!("{id}-text-{i}").into()))
                            .max_h(px(180.))
                            .overflow_y_scroll()
                            .child(markdown(format!("{id}-source-{i}"), gpui_agent_presentation::plain(text))),
                    );
                },
                | Attachment::Image {
                    ..
                } => match self.cached_image(format!("{id}-image-{i}"), a) {
                    | Ok(image) => item = item.child(img(image).max_w(px(400.)).max_h(px(220.)).object_fit(ObjectFit::Contain)),
                    | Err(e) => item = item.child(error(e, cx)),
                },
            }
            body = body.child(item);
        }
        body.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::image_preview;
    use gpui_agent_contracts::Attachment;
    use gpui_agent_presentation::plain;
    #[test]
    fn given_json_output_when_rendered_then_its_language_is_identified() {
        assert!(super::terminal_plain(r#"{"name": "blog"}"#, None).starts_with("```json\n"));
        assert!(super::terminal_plain("fn main() {}", Some("src/main.rs")).starts_with("```rs\n"));
        let spans = super::terminal_highlights(r#"{"name": true}"#, Some("json"));
        assert!(spans.len() > 1);
        assert!(spans.windows(2).any(|pair| pair[0].1.color != pair[1].1.color));
    }
    #[test]
    fn raw_text_keeps_embedded_markdown_and_fences_literal() {
        assert_eq!(plain("# hi\n```rust\na()\n```"), "````\n# hi\n```rust\na()\n```\n````");
    }
    #[test]
    fn attachment_decoder_bounds_images_and_uploads_bgra_to_gpui() {
        let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 20, 10, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let attachment = Attachment::from_bytes("test.png".into(), bytes.get_ref()).unwrap();
        let preview = image_preview(&attachment).unwrap();
        assert_eq!(&preview.as_bytes(0).unwrap()[.. 4], &[10, 20, 255, 255]);
        let oversized = image::RgbaImage::new(4097, 1);
        let mut bytes = std::io::Cursor::new(Vec::new());
        oversized.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let attachment = Attachment::from_bytes("large.png".into(), bytes.get_ref()).unwrap();
        assert!(image_preview(&attachment).is_err());
    }
}

#[cfg(test)]
mod output_preview_tests {
    use super::{OUTPUT_PREVIEW_BYTES,
                truncate_output};

    #[test]
    fn given_a_short_output_then_it_is_shown_whole() {
        assert_eq!(truncate_output("short", false), ("short", 0));
    }

    #[test]
    fn given_a_huge_output_then_only_a_whole_line_prefix_is_shown_until_full_is_requested() {
        let text = "한글 line\n".repeat(20_000);
        let (shown, hidden) = truncate_output(&text, false);
        assert!(shown.len() <= OUTPUT_PREVIEW_BYTES && shown.ends_with("line"));
        assert_eq!(shown.len() + hidden, text.len());
        assert_eq!(truncate_output(&text, true), (text.as_str(), 0));
    }
}
