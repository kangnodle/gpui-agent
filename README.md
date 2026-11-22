# gpui-agent

Claude, ChatGPT/Codex, Antigravity를 **각 제공자의 네이티브 런타임**과 구독 계정으로 사용하는
로컬 우선 에이전트 호스트입니다. 하나의 호스트가 HTTP API와 MCP 엔드포인트를 제공하고,
데스크톱(GPUI)·웹·CLI 클라이언트가 같은 서비스를 공유합니다.

## 주요 기능

- **다중 제공자**: Claude, Codex(ChatGPT), Antigravity(Gemini) 로그인, 모델 조회, 스트리밍 대화
- **공통 지침·스킬**: 독립적인 AGENTS.md 해석, Skills 선택과 필요 시 본문 읽기. 세 제공자가 같은 호스트 정책을 사용합니다. [사용 방법](docs/guidance.md)
- **내구성 있는 잡**: 멱등한 명령 접수, 이벤트 저널, 재시작 후 복구, 우선순위·큐·취소
- **승인 흐름**: 도구 사용·권한 요청에 대한 사용자 승인/응답
- **대화 관리**: 보관, 포크, 수정 버전, 사용량, 컨텍스트 전송, 체크포인트
- **워크스페이스**: 프로젝트 등록, git worktree, 변경/diff 조회, PR 도구
- **제어**: Chromium 브라우저 자동화와 Windows 데스크톱 제어(명시적 승인 필요)
- **MCP**: `/mcp`로 에이전트에게 범위가 제한된 프로젝트 도구 제공, MCP OAuth 지원

## 구성

| 경로 | 설명 |
| --- | --- |
| `apps/desktop` | GPUI 데스크톱 클라이언트 |
| `apps/server` | 헤드리스 로컬 API 서버 (기본 `127.0.0.1:7431`) |
| `apps/cli` | 명령줄 클라이언트 (`login`, `status`, `logout`, `models`, `ask`) |
| `apps/web` | 서버가 서빙하는 정적 웹 클라이언트(PWA) |
| `crates/*` | 도메인, 계약, 유스케이스, 포트, 어댑터, 하네스 |

계층 구조와 의존성 규칙은 [docs/architecture.md](docs/architecture.md)를 참고하세요.

## 요구 사항

- Rust `nightly-2026-09-27` (`rust-toolchain.toml`로 자동 선택됨), 에디션 2024
- mise (`mise.toml`의 Node.js 버전과 개발 작업 관리)
- 사용할 제공자의 계정(구독 로그인). 자격 증명은 애플리케이션 프로필에만 저장되며 저장소에 올라가지 않습니다.
- 데스크톱 앱은 지원되는 네이티브 빌드 호스트(Windows 포함)에서 빌드합니다.

## 시작하기

프로젝트 설정을 검토한 뒤 신뢰하고 도구와 의존성을 준비합니다. Rust는 기존 rustup이
`rust-toolchain.toml`을 기준으로 관리합니다.

```bash
mise trust
mise install
mise run doctor
mise run setup
```

개발 서버를 실행합니다. 상태는 체크아웃 밖의 사용자 데이터 디렉터리에 체크아웃별로 저장되고, 포트는 OS가 할당하며 주소를 출력합니다. 개발 홈 경로는 `mise run doctor`에서 확인할 수 있습니다.

```bash
mise run dev
```

데스크톱 앱을 개발 모드로 실행하려면:

```bash
mise run dev --surface desktop
```

서버를 직접 실행할 수도 있습니다.

```bash
cargo run --locked --package gpui-agent-server -- --port 7431 --cwd .
```

| 옵션 | 설명 |
| --- | --- |
| `--port` | 바인딩 포트(기본 7431, 루프백 전용) |
| `--cwd` | 작업 디렉터리 |
| `--home` | `claude`/`codex`/`antigravity` 프로필 루트 |
| `--token-file` | 베어러 토큰 파일 위치 |
| `--mcp-write` | 주입된 세션이 범위 내 대화·잡을 관리하도록 허용 |

서버와 원격 데스크톱 클라이언트의 토큰 파일은 현재 사용자 소유의 일반 파일이어야 합니다. Unix에서는 소유자만 접근할 수 있어야 하며, Windows에서는 상속을 끄고 소유자에게만 모든 권한을 주는 ACL이 필요합니다. 심볼릭 링크와 하드 링크는 사용할 수 없습니다. 기존 토큰의 권한이 너무 넓었다면 서버를 중지하고 해당 토큰 파일을 삭제한 뒤 다시 시작하세요. 연결된 클라이언트에도 새 토큰을 설정하세요.

CLI 사용 예:

```bash
cargo run --locked --package gpui-agent-cli -- login claude
cargo run --locked --package gpui-agent-cli -- ask claude "이 저장소를 요약해줘"
```

데스크톱 앱 실행 스크립트는 `scripts/run.sh`(`scripts/run.ps1`), 릴리스 패키징은
`scripts/release.sh`(`scripts/release.ps1`)입니다.

## 개발

```bash
mise run test --package gpui-agent-application --test architecture   # 의존성 방향 검사
mise run test --package <package> [--test <target>] [--filter <name>]
mise run lint --package <package>
mise run fmt                                                         # 검사만 수행, 파일을 수정하지 않음
mise run check                                                       # 헤드리스 전체 게이트
mise run check --include-desktop                                     # GPUI 변경 시
```

- 변경은 Given/When/Then 시나리오와 공개 경계의 실패 테스트로 시작합니다.
- 결정론적 정책은 domain/contracts/presentation, 유스케이스는 application, 호스트 I/O는 어댑터에 둡니다.
- 외부 시스템은 픽스처로 대체하되 검증 대상 정책은 대체하지 않습니다.
- UI 흐름 검증은 `mise run client-fixture`(실제 웹 자산 + 합성 네이티브 제공자)를 사용합니다.

빌드는 `mise run build`(헤드리스), `mise run build:desktop`(GPUI),
패키징은 `mise run release`로 실행합니다. 기존 `cargo harness` 명령도 유지됩니다.
도구 설치, 보안 감사, 전환 안내는 [빌드 시스템 문서](docs/build-system.md)를 참고하세요.

요구사항·수락 기준·설계 규칙은 StrictDoc 문서 [docs/requirements/](docs/requirements/)에서 관리하며
`mise run docs`로 검증합니다. 개발 명령은 [docs/harness.md](docs/harness.md)에, 기여 규칙은
[AGENTS.md](AGENTS.md)에 있습니다. CI는 Linux와 Windows에서 헤드리스 게이트를 실행합니다
(`.github/workflows/ci.yml`).

## 보안

- 서버는 루프백에만 바인딩합니다.
- API는 베어러 토큰으로 인증합니다.
- 토큰, 프롬프트, 첨부, 응답은 진단 로그에 기록되지 않습니다.
- 스케줄은 서버가 실행 중일 때만 동작합니다.

## 라이선스

Proprietary — All rights reserved. 자세한 내용은 [LICENSE](LICENSE)를 참고하세요.
