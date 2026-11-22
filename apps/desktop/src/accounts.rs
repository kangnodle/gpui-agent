use crate::{app::ChatApp,
            ui::*};
use gpui::{base::TestSupportExt as _,
           *};
use gpui_agent_contracts::*;
use gpui_agent_presentation::{Action,
                              Conversation,
                              Update};
use reqwest::Method;
use std::sync::mpsc::{self,
                      Receiver,
                      SyncSender};

pub(super) struct Account {
    pub profile: AccountProfile,
    pub auth: Conversation,
    pub busy: bool,
    pub operation: Option<u64>,
}

pub(super) struct ProviderAccounts {
    pub accounts: Vec<Account>,
    pub loading: bool,
    pub error: Option<String>,
    pub managing: bool,
    pub label: String,
}

pub(super) enum Change {
    Catalog(Provider, Result<Vec<AccountProfile>, String>),
    Created(Provider, Result<AccountProfile, String>),
    Account(Provider, String, Box<Update>),
}

pub(super) struct AccountsUi {
    pub selected: usize,
    pub providers: [ProviderAccounts; 3],
    pub tx: SyncSender<Change>,
    pub rx: Receiver<Change>,
}

impl Account {
    fn new(profile: AccountProfile) -> Self {
        Self {
            profile,
            auth: Conversation::default(),
            busy: false,
            operation: None,
        }
    }
}

impl Default for AccountsUi {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel(512);
        Self {
            selected: 0,
            providers: PROVIDERS.map(|provider| ProviderAccounts {
                accounts: vec![Account::new(AccountProfile {
                    id: "default".into(),
                    provider,
                    label: "default".into(),
                })],
                loading: false,
                error: None,
                managing: false,
                label: String::new(),
            }),
            tx,
            rx,
        }
    }
}

impl AccountsUi {
    pub fn busy(&self, provider: Provider, id: &str) -> bool { self.providers[provider_index(provider)].accounts.iter().any(|a| a.profile.id == id && a.busy) }
}

impl ChatApp {
    pub(super) fn refresh_accounts(&mut self, provider: Provider) {
        let state = &mut self.accounts.providers[provider_index(provider)];
        if state.loading {
            return;
        }
        state.loading = true;
        state.error = None;
        let client = self.client.clone();
        let tx = self.accounts.tx.clone();
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                Ok(client
                    .response(Method::GET, &format!("/v1/providers/{}/accounts", provider_name(provider)), None)?
                    .json()?)
            })();
            let _ = tx.send(Change::Catalog(provider, result.map_err(|e| e.to_string())));
            wake.notify();
        });
    }

    pub(super) fn create_account(&mut self, provider: Provider) {
        let state = &mut self.accounts.providers[provider_index(provider)];
        if state.loading || state.label.trim().is_empty() {
            return;
        }
        state.loading = true;
        state.error = None;
        let label = state.label.trim().to_owned();
        let client = self.client.clone();
        let tx = self.accounts.tx.clone();
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                Ok(client
                    .response(
                        Method::POST,
                        &format!("/v1/providers/{}/accounts", provider_name(provider)),
                        Some(serde_json::json!({"label": label})),
                    )?
                    .json()?)
            })();
            let _ = tx.send(Change::Created(provider, result.map_err(|e| e.to_string())));
            wake.notify();
        });
    }

    fn account_locked(&self, provider: Provider, id: &str) -> bool {
        self.accounts.busy(provider, id)
            || self.job_monitor.profile_active(provider, id)
            || (self.busy && self.selected == provider_index(provider) && id == "default")
    }

    pub(super) fn account_action(&mut self, provider: Provider, id: String, action: Action) {
        if self.account_locked(provider, &id) {
            return;
        }
        let Some(account) = self.accounts.providers[provider_index(provider)]
            .accounts
            .iter_mut()
            .find(|a| a.profile.id == id)
        else {
            return;
        };
        account.busy = true;
        account.auth.error = None;
        account.operation = None;
        let client = self.client.for_account(provider, id.clone(), self.client.workspace_id());
        let tx = self.accounts.tx.clone();
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let send = |update| {
                let ok = tx.send(Change::Account(provider, id.clone(), Box::new(update))).is_ok();
                wake.notify();
                ok
            };
            let result = if matches!(action, Action::Refresh) {
                (|| -> anyhow::Result<()> {
                    let status: AuthStatus = client
                        .response(Method::GET, &format!("/v1/providers/{}/status", provider_name(provider)), None)?
                        .json()?;
                    send(Update::Status(status));
                    Ok(())
                })()
            } else {
                client.run(provider, action, "", &send)
            };
            if let Err(error) = result {
                send(Update::Error(error.to_string()));
            }
            send(Update::Finished);
        });
    }

    pub(super) fn account_updates(&mut self) -> bool {
        let mut changed = false;
        while let Ok(change) = self.accounts.rx.try_recv() {
            changed = true;
            match change {
                | Change::Catalog(provider, result) => {
                    let state = &mut self.accounts.providers[provider_index(provider)];
                    state.loading = false;
                    match result {
                        | Ok(profiles) => {
                            let mut previous = std::mem::take(&mut state.accounts);
                            state.accounts = profiles
                                .into_iter()
                                .map(|profile| {
                                    if let Some(index) = previous.iter().position(|a| a.profile.id == profile.id) {
                                        let mut account = previous.remove(index);
                                        account.profile = profile;
                                        account
                                    } else {
                                        Account::new(profile)
                                    }
                                })
                                .collect();
                            let ids: Vec<_> = state.accounts.iter().map(|a| a.profile.id.clone()).collect();
                            for id in ids {
                                self.account_action(provider, id, Action::Refresh);
                            }
                        },
                        | Err(error) => state.error = Some(error),
                    }
                },
                | Change::Created(provider, result) => {
                    let state = &mut self.accounts.providers[provider_index(provider)];
                    state.loading = false;
                    match result {
                        | Ok(profile) => {
                            let id = profile.id.clone();
                            state.accounts.push(Account::new(profile));
                            state.label.clear();
                            self.account_action(provider, id, Action::Refresh);
                        },
                        | Err(error) => state.error = Some(error),
                    }
                },
                | Change::Account(provider, id, update) => {
                    let Some(account) = self.accounts.providers[provider_index(provider)]
                        .accounts
                        .iter_mut()
                        .find(|a| a.profile.id == id)
                    else {
                        continue;
                    };
                    match *update {
                        | Update::Status(status)
                        | Update::Event(StreamEvent::Authenticated {
                            status,
                        }) => {
                            account.auth.authorization = None;
                            account.auth.status = Some(status.clone());
                            if id == "default" {
                                let conversation = &mut self.conversations[provider_index(provider)];
                                if !status.authenticated {
                                    conversation.models.clear();
                                }
                                conversation.status = Some(status.clone());
                            }
                        },
                        | Update::Models(models) =>
                            if id == "default" {
                                self.conversations[provider_index(provider)].models = models;
                            },
                        | Update::Event(event) => {
                            if let StreamEvent::Started {
                                operation_id,
                            } = &event
                            {
                                account.operation = Some(*operation_id);
                            }
                            if provider != Provider::Claude
                                && let StreamEvent::Agent(Event::Authorization {
                                    url, ..
                                }) = &event
                            {
                                self.wake.open_url(url);
                            }
                            account.auth.event(event);
                        },
                        | Update::Error(error) => {
                            account.auth.error = Some(error);
                            account.auth.authorization = None;
                        },
                        | Update::Finished => {
                            account.busy = false;
                            account.operation = None;
                        },
                        | _ => {},
                    }
                },
            }
        }
        changed
    }

    pub(super) fn accounts_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let wide = self.settings_content_wide(window);
        let mut providers = if wide { column().w(px(176.)).flex_shrink_0() } else { row().w_full() };
        for (index, provider) in PROVIDERS.into_iter().enumerate() {
            let select = move |s: &mut Self, _: &mut Window, _: &mut Context<Self>| {
                s.accounts.selected = index;
                s.panel_scroll.set_offset(Point::default());
            };
            let id = format!("accounts-provider-{}", provider_name(provider));
            let button = if wide {
                self.sidebar_button(id, display_name(provider), gpui::assets::IconName::Bot, true, cx, select)
            } else {
                self.button(id, display_name(provider), true, cx, select)
                    .ghost()
                    .icon(gpui::assets::IconName::Bot)
            };
            providers = providers.child(button.selected(self.accounts.selected == index));
        }
        let mut cards = column().flex_1();
        for provider in PROVIDERS {
            let index = provider_index(provider);
            if index != self.accounts.selected {
                continue;
            }
            let key = provider_name(provider);
            let state = &self.accounts.providers[index];
            let installed = self.providers.iter().find(|p| p.provider == provider);
            let mut card = column()
                .id(format!("accounts-{key}"))
                .test_support()
                .flex_1()
                .min_w_0()
                .w_full()
                .p_4()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius_lg)
                .bg(cx.theme().background)
                .child(
                    row().w_full().justify_between().child(heading(display_name(provider))).child(
                        self.button(
                            format!("accounts-refresh-{key}"),
                            "계정 상태 새로고침",
                            !state.loading,
                            cx,
                            move |s, _, _| s.refresh_accounts(provider),
                        )
                        .ghost(),
                    ),
                )
                .child(
                    row().child(
                        self.button(format!("accounts-manage-{key}"), "계정 관리", true, cx, move |s, _, _| {
                            s.accounts.providers[index].managing = !s.accounts.providers[index].managing
                        })
                        .ghost(),
                    ),
                );
            if state.loading {
                card = card.child(note(self.text("계정 확인 중…"), cx));
            }
            if let Some(error_text) = &state.error {
                card = card.child(error(error_text.clone(), cx));
            }
            if let Some(issue) = installed.and_then(|p| p.issue.as_ref()) {
                card = card.child(error(issue.clone(), cx));
            }
            for account in &state.accounts {
                let id = &account.profile.id;
                let status = account.auth.status.as_ref();
                let logged = status.is_some_and(|s| s.authenticated);
                let enabled = !self.account_locked(provider, id);
                let label = if id == "default" {
                    self.text("기본 계정")
                } else {
                    account.profile.label.clone()
                };
                let status_label = if account.busy {
                    "처리 중…"
                } else if logged {
                    "로그인됨"
                } else if installed.is_some_and(|p| !p.available) {
                    "사용 불가"
                } else if status.is_some() {
                    "로그인 필요"
                } else {
                    "상태 확인 필요"
                };
                let login_id = id.clone();
                let logout_id = id.clone();
                let mut profile = column()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(label))
                    .child(
                        div()
                            .text_sm()
                            .text_color(if logged { cx.theme().success } else { cx.theme().muted_foreground })
                            .child(self.text(status_label)),
                    );
                if let Some(identity) = status.and_then(|s| s.account.as_ref()) {
                    profile = profile.child(note(identity.clone(), cx));
                } else if logged {
                    profile = profile.child(note(format!("{} · {id}", self.text("계정 정보 미제공 · 프로필 ID로 구분")), cx));
                }
                profile = profile.child(
                    row()
                        .child(
                            self.button(
                                format!("account-login-{key}-{id}"),
                                if logged { "다시 로그인" } else { "로그인" },
                                enabled && installed.is_some_and(|p| p.available),
                                cx,
                                move |s, _, _| s.account_action(provider, login_id.clone(), Action::Login),
                            )
                            .primary(),
                        )
                        .child(
                            self.button(format!("account-logout-{key}-{id}"), "로그아웃", enabled && logged, cx, move |s, _, _| {
                                s.account_action(provider, logout_id.clone(), Action::Logout)
                            }),
                        ),
                );
                if self.job_monitor.profile_active(provider, id) {
                    profile = profile.child(note(self.text("실행 중인 작업이 끝난 후 계정을 변경할 수 있습니다."), cx));
                }
                if let Some((url, code)) = &account.auth.authorization {
                    let url = url.clone();
                    profile = profile.child(
                        self.button(format!("account-browser-{key}-{id}"), "브라우저에서 계속", true, cx, move |s, _, _| {
                            s.wake.open_url(&url)
                        }),
                    );
                    if let Some(code) = code {
                        profile = profile.child(note(code.clone(), cx));
                    }
                }
                if let Some(operation) = account.operation {
                    let id = id.clone();
                    profile = profile.child(self.button(format!("account-cancel-{key}-{id}"), "중지", true, cx, move |s, _, _| {
                        s.client.for_account(provider, id.clone(), s.client.workspace_id()).cancel(operation);
                    }));
                }
                if let Some(message) = &account.auth.error
                    && installed.and_then(|p| p.issue.as_ref()) != Some(message)
                {
                    profile = profile.child(error(message.clone(), cx));
                }
                card = card.child(profile);
            }
            if state.managing {
                let label = state.label.clone();
                let loading = state.loading;
                card = card
                    .child(self.field(
                        format!("account-label-{key}"),
                        "새 계정 이름",
                        label.clone(),
                        FieldKind::Line,
                        !loading,
                        window,
                        cx,
                        move |s, v| s.accounts.providers[index].label = v,
                    ))
                    .child(self.button(
                        format!("account-add-{key}"),
                        "계정 추가",
                        !loading && !label.trim().is_empty(),
                        cx,
                        move |s, _, _| s.create_account(provider),
                    ));
            }
            cards = cards.child(card);
        }
        let mut body = if wide { row().flex_nowrap().items_start().gap_4() } else { column().gap_4() };
        cards = cards.child(self.runtime_panel(PROVIDERS[self.accounts.selected], window, cx));
        body = body.child(providers).child(cards);
        if window.viewport_size().height < px(520.) && PROVIDERS[self.selected] == Provider::Claude {
            body = body.child(self.permission_modes(None, self.conversations[self.selected].permission_mode, !self.busy, cx));
        }
        body.into_any_element()
    }
}
