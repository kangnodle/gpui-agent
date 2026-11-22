# 아키텍처

gpui-agent는 Claude, ChatGPT/Codex, Antigravity의 **네이티브 런타임**(각 제공자의 CLI/프로세스)을
구독 계정으로 구동하는 로컬 우선(local-first) 에이전트 호스트입니다. 하나의 호스트가 HTTP API와
MCP 엔드포인트를 노출하고, 데스크톱(GPUI)·웹·CLI 클라이언트가 같은 애플리케이션 서비스를 공유합니다.

핵심 원칙은 **의존성이 항상 안쪽(순수한 쪽)을 향한다**는 것입니다. 이 규칙은
[`crates/application/tests/architecture.rs`](../crates/application/tests/architecture.rs)가 `cargo metadata`로
강제하며, 새 워크스페이스 멤버는 이 테스트에 규칙을 등록하지 않으면 실패합니다.

## 계층과 크레이트

```
                    ┌──────────────────────────────────────────────┐
  실행 파일          │ apps/desktop   apps/server   apps/cli        │  조합(composition) / UI
                    └───────┬──────────────┬───────────┬───────────┘
                            │              │           │
  전송 어댑터        ┌───────▼──────┐       │           │
                    │  crates/api  │       │           │            HTTP·MCP: 디코드→인가→서비스 호출→매핑
                    └───────┬──────┘       │           │
  호스트 I/O 어댑터   ┌───────▼───────────────▼──┐  ┌─────▼─────┐  ┌──────────┐  ┌───────────┐
                    │ infrastructure (조합/SQLite│  │ agent     │  │ control   │  │ history   │
                    │  /git/파일/설정)           │  │ (런타임)  │  │ (브라우저·│  │ (SQLite)  │
                    └───────┬───────────────────┘  └─────┬─────┘  │ 데스크톱) │  └─────┬─────┘
                            │                            │        └─────┬─────┘        │
  유스케이스         ┌───────▼──────┐                      │              │              │
                    │ application │                      │              │              │
                    └───────┬──────┘                      │              │              │
  포트(효과 인터페이스)┌───────▼──────────────────────────────▼──────────────▼──────────────▼───┐
                    │ ports                                                                 │
                    └───────┬───────────────────────────────────────────────────────────────┘
  계약/순수 정책     ┌───────▼───────┐   ┌───────────────┐
                    │ contracts     │──▶│ domain        │   presentation ──▶ contracts
                    └───────────────┘   └───────────────┘
                    dci ──▶ contracts, domain (application이 사용)
                              process: 소유된 프로세스 트리 (agent, control이 공유)
```

| 크레이트 | 패키지 | 책임 | 허용되는 의존성 |
| --- | --- | --- | --- |
| `crates/domain` | `gpui-agent-domain` | DDD Bounded Context별 순수 도메인 데이터·값 객체·ADT 상태 머신과 순수 전이 함수(`Transition<S, E>`, `JobMachine`, `VerificationMachine`, `PullRequestWatchMachine`, `ProposalLifecycle`, `ComputerLeaseState`, `McpOAuthMachine` 등). I/O 없음 | `serde`, `serde_json`, `serde_yaml_ng`, `base64`, `url` |
| `crates/contracts` | `gpui-agent-contracts` | 클라이언트·서버 간 전송 계약(요청/응답/이벤트). 순수 모델은 domain에서 재노출 | domain |
| `crates/dci` | `gpui-agent-dci` | DCI(Data, Context, Interaction) 순수 역할(Role) 트레이트와 협력 컨텍스트(`job_execution`, `pull_request_watch`, `proposal_governance`, `computer_control`, `oauth_session`). 함수형 순수 전이와 효과 서술만 수행하며 I/O 없음 | domain, contracts, `serde_json` |
| `crates/presentation` | `gpui-agent-presentation` | 클라이언트 상태와 명령의 결정론적 로직(대화, 스크롤, 초안, 도구 활동). 렌더링/I/O 없음 | contracts |
| `crates/ports` | `gpui-agent-ports` | 애플리케이션이 소유하는 효과 인터페이스(`Clock`, `DiagnosticSink`, `ConversationRepository`, `HostControl`, `FileContextReader`, `GuidanceSource`, `PluginSource`, `ProjectVerifier`, MCP 포트, 런타임 포트 등). 선언만 하고 실행하지 않음 | domain, contracts, `anyhow` |
| `crates/application` | `gpui-agent-application` | 유스케이스와 작업(operation) 수명주기: `dci` 순수 컨텍스트와 `domain` ADT 상태 머신을 구동하고 부수효과는 `ports`로 해석. 구체 I/O에 의존하지 않음 | domain, contracts, dci, ports, `anyhow`, `serde_json`, `tokio`(`macros/rt/sync/time`만) |
| `crates/agent` | `gpui-agent` | 네이티브 런타임 어댑터: 구독 인증, Claude/Codex/Antigravity 스트리밍 대화, MCP OAuth, 복구, 사용량 | domain, ports, process |
| `crates/control` | `gpui-agent-control` | 소유한 Chromium 자동화, 수동 브라우저, Windows 데스크톱 제어 어댑터 | ports, contracts, process |
| `crates/history` | `gpui-agent-history` | SQLite 대화·컨텍스트 전송 영속화 | ports, domain, `anyhow`, `rusqlite`, `serde_json` |
| `crates/process` | `gpui-agent-process` | 프로세스 트리 소유(Windows는 Job Object). 워크스페이스 의존성 없음 | — |
| `crates/infrastructure` | `gpui-agent-infrastructure` | 구체 조합: `JobStore`, `Projects`, `Settings`, `McpSessions`, git, 파일, 체크포인트, 프로젝트 검증기 등을 application 포트에 연결 | agent, application, contracts, domain, history |
| `crates/api` | `gpui-agent-api` | 로컬 HTTP(axum) 및 `/mcp` 어댑터. 런타임·저장소는 호스트가 주입 | application, contracts |
| `apps/server` | `gpui-agent-server` | 헤드리스 호스트(기본 포트 7431, 루프백 바인딩) | agent, api, infrastructure, control |
| `apps/desktop` | `gpui-agent-desktop` | GPUI 데스크톱 클라이언트 | agent, api, contracts, infrastructure, control, presentation |
| `apps/cli` | `gpui-agent-cli` | `login/status/logout/models/ask/skills/plugins` 명령줄 클라이언트 | agent, infrastructure |
| `apps/web` | — | 정적 웹 클라이언트(`app.js`, PWA 매니페스트·서비스 워커). 서버가 서빙 | HTTP API |
| `crates/harness` | `gpui-agent-harness` | 개발 하네스(`cargo harness ...`). 제품 크레이트에 의존하지 않음(테스트 전용으로만 process 사용) | — |

테스트·설정으로 강제되는 계층 불변 조건(`ARCH-INV-*`)과 검증 게이트는
[`requirements/architecture.sdoc`](requirements/architecture.sdoc)에서 관리합니다.
게이트 사용법은 [`build-system.md`](build-system.md)를 참고하세요.

## 정책은 어디에 두는가

정책 위치 규칙(`ARCH-POLICY-*`)은 [`requirements/architecture.sdoc`](requirements/architecture.sdoc)에서 관리합니다.

## 주요 흐름

### 일반 채팅의 필수 검증 루프

`domain/verification.rs`는 프로젝트 명령 설정의 유효성, 명령별 결과와 코드 버전,
모든 명령의 종료 코드가 0이고 검사 전후 버전이 같아야 통과한다는 순수 정책을 소유합니다.
`application::Jobs`는 프로젝트 설정을 작업 시작 시 고정하고 제공자의 완료 이벤트를 보류합니다.
실패하면 같은 대화의 내부 수정 턴을 실행하고 전체 명령을 다시 검사하며, 시도 한도에 도달하면
작업을 실패로 종료합니다. 승인·질문·취소는 기존 작업 수명주기를 그대로 사용합니다.

`ProjectVerifier` 포트(`crates/ports/src/verifier.rs`)를 `infrastructure/project_verifier.rs`가 구현해
소유된 프로세스 실행, 시간 제한과 Git/파일 지문 계산을 담당합니다(`Jobs::with_project_verifier`로 주입).
설정은 `Projects`에, 결과는 작업 저널과 해당 대화 턴의 타임라인에 저장합니다. HTTP는 운영자 설정만
받아 서비스에 전달하고, 데스크톱과 웹은 같은 검증 이벤트를 작업 패널에 표시합니다.

### 잡 제출 (HTTP 또는 MCP)

1. 클라이언트가 인증된 요청(`POST /v1/providers/{provider}/jobs` 또는 `/mcp` 도구 호출)을 보냅니다.
2. `api`가 토큰/세션 범위를 확인하고 `application::Jobs`를 호출합니다.
3. 명령 영수증(receipt)과 잡이 **함께 커밋**되어 같은 명령의 재전송은 하나의 잡만 만듭니다.
4. 워커가 `agent`의 제공자 어댑터로 네이티브 프로세스를 구동하고, 이벤트가 저널에 기록됩니다.
5. 클라이언트는 `GET /v1/jobs/{id}/events`로 이벤트를 읽고, 승인 요청에는
   `POST /v1/jobs/{id}/interactions/{request_id}`로 응답합니다.
6. 호스트를 재시작해도 결과가 보존되며, 중단된 외부 작업은 재실행하지 않고 복구 경로로 처리됩니다.

제거된 워크플로·목표·위임 기능의 이전 데이터는 시작 시 `infrastructure/jobs.rs`의
`retire_workflow_tables`가 정리합니다. 해당 기능에 속한 대기 중 잡은 취소하고 관련 테이블은 삭제합니다.

### 제공자 런타임

`agent`는 제공자별 모듈(`claude`, `codex`, `antigravity`)과 공통 `adapter`, `config`, `process`, `rpc`, `mcp`,
`mcp_oauth`, `browser`, `tools`, `recovery`, `usage`로 구성됩니다. 프로필(로그인 상태)은 `--home`(또는 설정)의 제공자별 디렉터리에 두며
저장소에 올리지 않습니다. 호스트가 주입한 MCP 세션(`McpSessions`)이 에이전트에게 범위가 제한된
프로젝트 도구(대화·잡 관리)를 노출합니다. 쓰기 도구는 `--mcp-write`로만 열립니다.

### 제어(브라우저·데스크톱)

`control`이 Chromium 자동화, 수동 브라우저, Windows 데스크톱 제어를 소유하고, application의
`automate_browser`/`control_device`/`grant_device`가 `HostControl` 포트를 통해 호출합니다.
데스크톱 제어는 명시적 승인(grant) 이후에만 활성화됩니다.

Windows computer use는 `Jobs::grant_computer`/`control_computer`와 같은 포트를 사용합니다.
`ChatRequest.computer_use`는 승인 전 대기를 요청하는 저장된 의도이며 권한 자체가 아닙니다.
application은 durable 작업에만 `computer/request_control` interaction을 열고 응답 전까지
제공자 실행을 시작하지 않습니다. 데스크톱은 기존 제어 패널로 작업 권한을 승인한 뒤
실행을 시작하고, 네이티브 MCP 요청은 별도로 한 번씩 승인합니다. 캡처 JSON의 이미지
해석은 순수 presentation에, 렌더링은 데스크톱에 둡니다.
승인 화면이 가져간 Windows 포커스는 명시적 네이티브 승인 시 control 어댑터의
`AllowSetForegroundWindow`로 위임합니다. 상태의 PID와 생성 시각이 실제 로컬
프로세스와 일치할 때만 위임하고, 작업 권한과 입력 직전 인가는 별도로 유지합니다.
`ComputerLease`의 소유권·관찰 revision 정책은 contracts에, 작업/계정/대화 인가는 application에,
단일 데스크톱 임대·단조 시계 만료·캡처와 입력은 control에 둡니다. 운영자가 작업 하나를 승인하면
첫 캡처 실행이 제어권을 얻습니다. 네이티브 MCP 자격증명은 작업 ID와 실행 ID에 고정되며,
작업 종료·취소·자격증명 만료·승인 철회를 실제 입력 직전에도 확인합니다. 수동 제어와는 상호 배제됩니다.
HTTP 승인과 MCP 캡처/행동/종료 경로, 픽스처 및 Windows 검증은
[`requirements/harness.sdoc`](requirements/harness.sdoc)의 `HARNESS-COMPUTER-*`, `HARNESS-LIVE-EDITOR-*`를 참고하세요.

## HTTP / MCP 표면 (`crates/api`)

| 영역 | 라우트(`/v1` 접두) |
| --- | --- |
| 제공자 | `providers`, `providers/{p}/status|models|login|logout|chat|context|files|accounts` |
| 대화 | `providers/{p}/conversations[/{id}/state|usage|versions|revisions]` |
| 잡·작업 | `providers/{p}/jobs`, `jobs[/{id}[/events|interactions/{rid}]]`, `operations/{id}[/interactions/{rid}]` |
| 프로젝트·워크스페이스 | `projects[/{id}[/verification|worktrees]]`, `workspaces/{id}[/git|changes|diff]` |
| MCP 설정 | `mcp`, `mcp/{p}[/{name}/oauth]`, `mcp-proposals[/{id}]` |
| 지침·스킬·플러그인 | `guidance[/files[/read]]`, `skills[/import|read|enabled]`, `skill-proposals[/{id}]`, `plugins[/import|claude|enabled|remove]` |
| 제어 | `control`, `control/browser`, `control/desktop[/grant]`, `control/devices[/grant]`, `control/computer[/grant]` |
| 환경 | `environment[/preferences]`, `runtimes/{p}` |
| MCP | `POST /mcp` — 초기화, 도구 탐색, 인가된 application 서비스 호출 |

MCP 응답과 진단은 호스트가 생성한 요청 ID를 공유하며, 성공한 잡 호출에는 durable 잡 ID와 작업 ID가 함께 실립니다.

## 상태와 보안

상태 저장 위치, 바인딩, 진단 로그 규칙(`ARCH-SEC-*`)은 [`requirements/architecture.sdoc`](requirements/architecture.sdoc)에서 관리합니다.

## 변경 시 점검할 표면

계약 변경 영향 확인 순서, 실패하는 공개 경계 테스트로 시작하기, 요구사항 선기록 규칙(`ARCH-CHANGE-*`)은
[`requirements/architecture.sdoc`](requirements/architecture.sdoc)에서 관리합니다. 테스트 명령은 [`harness.md`](harness.md)를 참고하세요.

| 계층 | 대표 테스트 |
| --- | --- |
| 의존성 방향·순수성 | `application --test architecture` |
| 유스케이스 | `application --test use_cases`, `--test contracts` |
| 순수 정책 | `domain` (`job_policy`, `codex_policy`, `project_verification`) |
| 제공자 프로토콜 | `gpui-agent --test protocols|oauth|recordings` (네이티브 픽스처·녹화 재생) |
| 영속성 | `infrastructure` (`queue`, `mcp`, `pull_requests`, `runtimes` …), `history` |
| 전송 | `api` (`http`, `jobs`, `devices`, `control`, `project_tools`, `pull_requests` …) |
| 클라이언트 | `presentation`, `apps/desktop` 단위 테스트, `apps/web/test.cjs`, `client-fixture` |

## 알려진 제약

툴체인 고정, 벤더 패치, 데스크톱 검증 호스트 제약(`ARCH-CONSTRAINT-*`)은
[`requirements/architecture.sdoc`](requirements/architecture.sdoc)에서 관리합니다.

## 제공자 독립 지침과 스킬

### 공통 플러그인

`domain/plugins.rs`는 manifest·파일 경로·크기·구성요소 검증과 공개 모델을 소유합니다.
`ports::PluginSource`에 설치·조회·활성 상태·제거 효과를 선언하고,
`application::Plugins`를 운영자 HTTP, 데스크톱·웹, CLI가 함께 사용합니다.
`infrastructure::PluginFiles`는 공통 홈과 선택된 작업공간에 복사본을 저장하며,
기존 스킬·활성 설정·계정별 MCP JSON을 변경하지 않습니다.

활성 패키지의 스킬은 기존 `Guidance` snapshot/read 경로에 포함됩니다.
패키지 MCP는 `Settings::agent`와 CLI에서 `Config::plugin_mcp`로 전달되어 세 제공자의
공통 `Config::mcp`에서 기존 연결과 합쳐집니다. OAuth는 기존 계정별 경로를 사용합니다.
관리 API는 운영자 전용이며 scoped MCP에는 스킬 조회·읽기만 기존처럼 노출합니다.
형식과 사용법은 [공통 플러그인](plugins.md)을 참고하세요.

`domain/guidance.rs`는 frontmatter 파싱, 지침 예산, 동명 스킬 선택, 문맥 렌더링을 순수하게 처리합니다.
`ports::GuidanceSource`는 snapshot/리소스 읽기/활성 설정 저장 효과를 선언하고,
`infrastructure::GuidanceFiles`가 bounded 파일 탐색, canonical 경로 제한, 원자적 설정 교체를 구현합니다.
`application::Guidance`와 `Application::chat_turn`이 같은 workspace/account 문맥에서 준비합니다.
세 제공자 adapter는 `Prompt::runtime_text`를 전송하며 공통 해석을 native Codex RPC에 위임하지 않습니다.
CLI도 같은 application use case와 파일 adapter를 사용합니다. `Settings::guidance_home`은 추가 계정에도
공통 roots를 유지하고 provider 인증 home과 분리됩니다.

HTTP 조회/설정과 scoped MCP `host_skills_list`/`host_skills_read`는 같은 use case를 호출합니다.
선택은 `ChatRequest`를 통해 durable queue에, 적용 출처/revision은 history의 `message_guidance`에 저장합니다.
다른 provider로 보내는 context snapshot은 과거 선택을 새 지침으로 자동 실행하지 않습니다.
아키텍처 검사는 외부 workspace path dependency와 Codex crate 의존성을 거절합니다.
형식·제한·native 중복 로딩의 범위는 [공통 지침 사용 방법](guidance.md)에 있습니다.
