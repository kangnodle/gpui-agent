# 빌드 시스템과 모노레포 게이트

mise가 로컬 개발과 CI의 공통 실행 진입점입니다. 실제 Rust 빌드와 의존성 관리는 Cargo가,
프로젝트 검증은 기존 `cargo harness`가 담당합니다. Rust 버전은 `rust-toolchain.toml`에서만
고정하고 rustup으로 설치합니다. Node.js는 `mise.toml`에서 고정합니다.

| 도구 | 하네스 명령 | 설정 | 지키는 것 |
| --- | --- | --- | --- |
| cargo-deny | `cargo harness deny` | [`deny.toml`](../deny.toml) | 보안 권고, 라이선스 허용 목록, 금지 크레이트(openssl 등), 허용된 레지스트리만 사용 |
| cargo-machete | `cargo harness unused` | — (`[package.metadata.cargo-machete]`로 오탐 예외) | `crates/`, `apps/`의 미사용 의존성 |
| cargo-geiger | `cargo harness unsafe` | 루트 `Cargo.toml`의 `[workspace.metadata.harness] unsafe-free` | `unsafe-free` 크레이트의 모든 타깃 루트에 `#![forbid(unsafe_code)]`(컴파일러가 `unsafe`를 거부, geiger `--forbid-only`가 속성 존재를 검증) |
| Cargo 계층 검사 | `mise run layers` | `tools/layers.toml` | 승인된 워크스페이스 의존 방향과 새 멤버 검사 |

`mise run audit`는 `cargo harness audit`를 통해 deny → unused → unsafe를 차례로 실행합니다.
CI의 `verify` 잡은 Linux와 Windows에서 `mise run check`를 실행하고, Windows에서는
데스크톱 lint와 테스트도 실행합니다. `audit` 잡은 기존 세 보안·품질 검사를 유지합니다.

## 도구 설치

```bash
mise trust                   # 저장소의 mise.toml을 검토한 뒤 신뢰
mise install                 # 고정된 Node.js 설치
rustup show                  # rust-toolchain.toml의 Rust 설치/확인
mise run setup               # cargo fetch --locked
mise run audit:install       # cargo-deny, cargo-machete, cargo-geiger 설치
```

## 일상 작업

```bash
mise run build                       # 헤드리스 전체 타깃
mise run build -- --release           # 최적화 빌드
mise run build:desktop                # 네이티브 GPUI 앱
mise run check                       # 포맷 검사, Clippy, Rust/웹 테스트
mise run check -- --include-desktop   # GPUI 및 Windows 접근성 패치 검사 포함
mise run fmt                         # 포맷 검사만
mise run lint -- --package gpui-agent-desktop
mise run layers                      # 워크스페이스 의존 방향 검사
mise run test -- --package gpui-agent-domain
mise run test:web
mise run audit
mise run dev                         # 격리된 개발 서버
mise run dev -- --surface desktop
mise run client-fixture              # 실제 웹 자산 + 네이티브 프로토콜 픽스처
mise run doctor                      # 툴체인 확인
mise run release                     # 기존 네이티브 검사·패키징 스크립트
```

`fmt`는 파일을 수정하지 않는 검사입니다. `test`/`lint`/`check` 인수는 기존 하네스로 전달되고,
첫 실패 시 종료 코드가 호출자에게 전달됩니다. `release`는 Windows에서 PowerShell,
Linux/macOS에서 Bash 스크립트를 사용하며 패키징 형식과 감사 절차를 유지합니다.
기존 스크립트의 `SKIP_GATES=1`은 감사 우회 동작을 유지합니다. CI에서는 설정하지 않습니다.

`scripts/`의 PowerShell·Bash 스크립트도 mise가 필요합니다. 감사는 `mise run audit`,
빌드·앱 실행·패키징 및 도구 확인은 `mise exec -- cargo ...`로 실행하므로 셸에 mise를
활성화하지 않아도 체크아웃의 도구·환경 설정을 사용합니다. 기존 `run`/`release` 스크립트의
`cargo fmt --all` 자동 수정, 실행 인수와 `.gpui-agent-dev` 기본 프로필은 유지됩니다.
`release` 스크립트는 `mise run release`를 다시 호출하지 않습니다.

## 계층 규칙 갱신 절차

`tools/layers.toml`의 `normal`/`dev`가 승인된 의존 방향 표입니다. `normal`은 일반·빌드
의존성을, `dev`는 일반 의존성에 없는 테스트 전용 의존성을 기록합니다.
`mise run layers`는 `cargo metadata --no-deps --offline --locked`로 전체 워크스페이스를
검사하므로 이름을 바꾼 의존성과 현재 호스트 이외의 플랫폼별 의존성도 빠뜨리지 않습니다.
새 멤버나 의존성을 추가·제거하면 표도 검토해 갱신해야 합니다.
[`architecture.md`](architecture.md)의 규칙과 application의 기존 아키텍처 테스트(`crates/application/tests/architecture.rs`)도 유지됩니다.

## Bazel에서 전환

Bazelisk/Bazel, `BUILD.bazel`, 모듈·툴체인 설정과 별도 CI 잡을 제거했습니다.
`cargo harness bazel build` 대신 `mise run build`, `cargo harness bazel test` 대신
`mise run check`를 사용하세요. Bazel 타깃 문자열과 플래그는 더 이상 지원하지 않습니다.
패키지를 선택하려면 `mise run test -- --package <Cargo 패키지명>`을 사용합니다.
그 외 기존 `cargo harness` 명령과 스크립트는 계속 사용할 수 있습니다.

Cargo.lock, 날짜가 고정된 Rust, 고정된 Node, `--locked`와 CI 검증은 유지됩니다.
다만 Bazel의 샌드박스·격리 빌드는 제공하지 않습니다. 네이티브 컴파일러, SDK와 시스템
라이브러리는 빌드 호스트에서 준비해야 합니다. 계층 위반은 Cargo 빌드 자체가 아니라
테스트/CI 게이트에서 차단합니다. 실제 빌드 속도 향상은 별도 측정이 필요합니다.

## 알려진 예외

- `deny.toml`의 `advisories.ignore`는 비어 있습니다. 예외를 추가할 때는 이유와 해제 조건을 함께 적습니다.
- `bans.wildcards`는 경고로 둡니다. 경로 의존성은 루트 `[workspace.dependencies]`에서 버전을 명시하므로
  보고되는 와일드카드는 실제 문제입니다.
- `bans.multiple-versions`는 경고이며, GPUI가 고정한 의존성 그래프(`gpui-kit`)는 `skip-tree`로 제외합니다.
- `unsafe-free` 목록에 없는 `process`, `control`, `desktop`은 플랫폼 FFI를 소유합니다.
