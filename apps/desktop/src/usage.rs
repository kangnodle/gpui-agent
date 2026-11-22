use crate::{language::Language,
            ui::{column,
                 note}};
use gpui::{IntoElement,
           ParentElement,
           Styled};
use gpui_agent_contracts::*;
/// Renders an already computed report, so callers that cache it avoid
/// rescanning the whole timeline on every frame.
pub fn report_view(report: UsageReport, provider: Provider, language: Language, cx: &gpui::App) -> impl IntoElement {
    let mut body = column().gap_1();
    if let Some(usage) = report.usage {
        let number = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or("—".into());
        body = body.child(note(
            format!(
                "{} · in {} · out {} · cache {} · write {} · total {} · context {}",
                language.text("보고된 세션 사용량"),
                number(usage.input_tokens),
                number(usage.output_tokens),
                number(usage.cached_input_tokens),
                number(usage.cache_write_tokens),
                number(usage.total_tokens),
                number(usage.context_window)
            ),
            cx,
        ));
        if let Some(cost) = usage.cost_usd {
            body = body.child(note(format!("{}: ${cost:.6}", language.text("런타임 비용 지표 · 실제 청구액 아님")), cx));
        }
        if report.missing_turns > 0 {
            body = body.child(note(format!("{}: {}", language.text("사용량 미보고 턴"), report.missing_turns), cx));
        }
    } else {
        body = body.child(note(
            language.text(if provider == Provider::Antigravity {
                "Antigravity ACP는 사용량·비용을 보고하지 않습니다."
            } else {
                "사용량이 아직 보고되지 않았습니다."
            }),
            cx,
        ));
    }
    body
}
