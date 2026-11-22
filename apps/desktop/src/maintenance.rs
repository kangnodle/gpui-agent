use gpui::component::{IndexPath,
                      Sizable,
                      combobox::{Combobox,
                                 ComboboxState},
                      searchable_list::SearchableVec};
use gpui_agent_contracts::*;
#[derive(Default)]
pub struct MaintenanceUi {
    pub(super) infos: [Option<RuntimeInfo>; gpui_agent_contracts::PROVIDERS.len()],
    pub(super) confirm: Option<(Provider, RuntimeRelease)>,
    pub(super) refresh_accounts: Option<Provider>,
    pub(super) pending: Option<Provider>,
    pub(super) error: Option<(Provider, String)>,
    antigravity_versions: Vec<String>,
    antigravity_picker: Option<Entity<ComboboxState<SearchableVec<String>>>>,
}
impl MaintenanceUi {
    pub fn apply(&mut self, info: RuntimeInfo) {
        let index = provider_index(info.provider);
        self.refresh_accounts = (info.active_path.is_some() && info.issue.is_none()).then_some(info.provider);
        self.infos[index] = Some(info);
    }
}

use crate::{app::ChatApp,
            ui::*};
use gpui::{self,
           base::TestSupportExt as _,
           prelude::FluentBuilder as _,
           *};
impl ChatApp {
    pub(super) fn runtime_panel(&mut self, provider: Provider, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut body = column();
        let mut card = column();
        let info = &self.maintenance.infos[provider_index(provider)];
        let needs_install = info.as_ref().map_or_else(
            || self.providers.iter().any(|p| p.provider == provider && !p.available),
            |info| info.active_path.is_none() && !info.override_active,
        );
        if needs_install {
            card = card.child(note(self.text("설치 가능한 버전을 확인하고 설치한 뒤 로그인하세요."), cx));
        }
        if let Some((target, message)) = &self.maintenance.error
            && *target == provider
        {
            card = card.child(div().id(format!("runtime-error-{provider:?}")).test_support().child(error(message.clone(), cx)));
        }
        if let Some(info) = info {
            card = card.child(note(info.active_path.clone().unwrap_or_else(|| self.text("사용 불가")), cx));
            if let Some(issue) = &info.issue {
                card = card.child(error(issue.clone(), cx));
            }
            if provider == Provider::Antigravity && !info.available_releases().is_empty() {
                let versions: Vec<_> = info.available_releases().iter().map(|release| release.version.clone()).collect();
                if self.maintenance.antigravity_picker.is_none() || self.maintenance.antigravity_versions != versions {
                    let selected = self.maintenance.antigravity_picker.as_ref().and_then(|picker| picker.read(cx).selected_value());
                    let index = versions.iter().position(|version| Some(version) == selected.as_ref()).unwrap_or(0);
                    self.maintenance.antigravity_picker =
                        Some(cx.new(|cx| ComboboxState::new(SearchableVec::new(versions.clone()), vec![IndexPath::new(index)], window, cx)));
                    self.maintenance.antigravity_versions = versions;
                }
                if let Some(picker) = self.maintenance.antigravity_picker.clone() {
                    let enabled = !self.busy && !info.override_active;
                    card = card.child(note(self.text("설치 가능 버전"), cx)).child(
                        row()
                            .child(
                                div().id("runtime-versions-Antigravity").test_support().w(px(240.)).h_8().child(
                                    Combobox::new(&picker)
                                        .small()
                                        .w_full()
                                        .placeholder(self.text("설치 가능 버전"))
                                        .disabled(!enabled),
                                ),
                            )
                            .child(self.button(
                                "runtime-review-Antigravity",
                                "이 버전 설치 / 업데이트",
                                enabled,
                                cx,
                                move |s, _, cx| {
                                    s.maintenance.confirm = picker
                                        .read(cx)
                                        .selected_value()
                                        .and_then(|version| {
                                            s.maintenance.infos[provider_index(provider)]
                                                .as_ref()?
                                                .available_releases()
                                                .iter()
                                                .find(|release| release.version == version)
                                                .cloned()
                                        })
                                        .map(|release| (provider, release));
                                },
                            )),
                    );
                }
            } else {
                for release in info.available_releases() {
                    let release = release.clone();
                    let id = if info.releases.is_empty() {
                        format!("runtime-review-{provider:?}")
                    } else {
                        format!("runtime-review-{provider:?}-{}", release.version)
                    };
                    card = card
                        .child(note(format!("{}: {}", self.text("설치 가능 버전"), release.version), cx))
                        .child(self.button(
                            id,
                            "이 버전 설치 / 업데이트",
                            !self.busy && !info.override_active,
                            cx,
                            move |s, _, _| s.maintenance.confirm = Some((provider, release.clone())),
                        ));
                }
            }
        }
        for (name, command) in [("설치 상태", RuntimeCommand::Inspect), ("업데이트 확인", RuntimeCommand::Check)] {
            let install_check = needs_install && matches!(command, RuntimeCommand::Check);
            let label = if install_check { "설치 가능한 버전 확인" } else { name };
            card = card.child(
                self.button(format!("runtime-{provider:?}-{name}"), label, !self.busy, cx, move |s, _, _| {
                    s.command(gpui_agent_presentation::Action::Runtime {
                        provider,
                        command: command.clone(),
                    })
                })
                .when(install_check, |button| button.primary()),
            );
        }
        body = body.child(
            self.settings_section(self.text("런타임 관리"), card, cx)
                .id(format!("runtime-{provider:?}"))
                .test_support(),
        );
        if let Some((target, release)) = self.maintenance.confirm.clone()
            && target == provider
        {
            body = body
                .child(heading(format!("{} {}", display_name(provider), release.version)))
                .child(note(release.url.clone(), cx))
                .child(note(format!("SHA-256: {}", release.sha256), cx))
                .child(note(
                    self.text("다운로드·무결성·실행 검증에 성공한 뒤 활성 버전을 바꿉니다. 이전 버전은 보존합니다."),
                    cx,
                ))
                .child(
                    row()
                        .child(self.button("runtime-cancel", "취소", true, cx, |s, _, _| s.maintenance.confirm = None))
                        .child(self.button("runtime-install", "설치 실행", !self.busy, cx, move |s, _, _| {
                            s.maintenance.confirm = None;
                            s.command(gpui_agent_presentation::Action::Runtime {
                                provider,
                                command: RuntimeCommand::Install {
                                    version: release.version.clone(),
                                    sha256: release.sha256.clone(),
                                },
                            });
                        })),
                );
        }
        body.into_any_element()
    }
}
