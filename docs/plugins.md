# 공통 플러그인

플러그인은 스킬과 MCP 연결을 함께 설치하고 관리하는 gpui-agent의 기능 묶음입니다.
설정의 **플러그인 → 설치됨**에서 로컬 폴더를 가져오고, 내용을 확인한 뒤 활성화하세요.
**스킬**, **MCP 연결** 탭에서는 기존 개별 설정을 계속 관리할 수 있습니다.

가져온 폴더는 복사되며 처음에는 비활성 상태입니다(아래 데스크톱 자동 가져오기는 예외). 가져오기는 스크립트를 실행하거나
MCP 서버를 시작하지 않습니다. 활성화·비활성화·삭제는 다음 대화 턴부터 반영됩니다.
실행 중인 작업을 취소하거나 과거 대화의 선택을 변경하지 않습니다.

## 폴더 형식

```text
review-tools/
  plugin.json
  skills/
    review/
      SKILL.md
      references/guide.md
```

`plugin.json`:

```json
{
  "name": "review-tools",
  "display_name": "리뷰 도구",
  "description": "변경 사항을 검토하는 스킬과 도구 모음",
  "version": "1.0.0",
  "mcp_servers": []
}
```

`skills/review/SKILL.md`:

```markdown
---
name: review
description: 변경된 코드의 오류와 회귀 가능성을 검토할 때 사용합니다.
---
변경 내용을 읽고 관찰 가능한 문제와 검증 방법을 설명하세요.
```

- `name`: 1~64자의 소문자 영문·숫자·하이픈. 처음은 영문 또는 숫자이며 끝에 하이픈을 쓰지 않습니다.
- `description`: 필수 설명(최대 4096바이트). `display_name`(최대 256바이트), `version`(최대 256바이트),
  `mcp_servers`는 생략할 수 있습니다. 그 밖의 필드는 거절합니다.
- `skills/<이름>/SKILL.md` 또는 MCP 서버가 하나 이상 있어야 합니다. 스킬은 이 깊이에만 둘 수 있습니다.
- MCP 설정은 기존 gpui-agent `McpServer` 형식을 사용합니다. 다른 제품의 manifest,
  Hooks·Agents·Rules, 마켓플레이스 형식은 변환하지 않습니다. Claude Code 플러그인 폴더만 예외입니다(아래 참고).
- 파일은 최대 128개, 각 64 KiB, 합계 1 MiB이며 경로 깊이는 최대 6입니다.
  심볼릭 링크·Windows reparse point·경로 이탈·`.`으로 시작하는 경로·Windows 예약 이름·
  대소문자 충돌·중복 설치는 거절합니다.
- 바이너리 리소스는 원래 바이트로 보존합니다. 실행 파일은 기존 MCP와 같이 API 호스트에서
  사용할 수 있는 경로로 설정하세요. 패키지 경로 변수 치환이나 의존성 설치는 제공하지 않습니다.

## 범위와 기존 설정

**앱 공통**은 gpui-agent 공통 홈의 `plugins/`에, **현재 프로젝트**는 선택된 작업공간의
`.agents/plugins/`에 저장합니다. ID는 각각 `user:review-tools`, `repo:review-tools`입니다.
활성 상태는 별도 파일에 저장합니다. 앱 공통은 공통 홈의 `plugin-settings.json`,
스킬만 포함한 현재 프로젝트 플러그인은 작업공간의 `.agents/plugin-settings.json`을 사용합니다.
MCP 서버가 포함된 프로젝트 플러그인의 승인은 작업공간 밖 공통 홈의 `repo-plugin-approvals/`에 저장하며
작업공간의 실제 경로, 플러그인 이름, 승인한 MCP 서버 설정에 연결됩니다.
이전 버전에서 활성화한 프로젝트 MCP 플러그인은 서버 설정을 확인한 뒤 **활성화**를 다시 누르세요.
다른 작업공간으로 복사하거나 서버 설정을 변경하면 해당 작업공간에서 다시 승인해야 합니다.

기존 스킬 원본·`skill-settings.json`·제공자/계정별 `mcp.json`은 그대로 유지합니다.
활성 플러그인의 MCP 서버는 실행 시 기존 연결 목록과 합쳐집니다. 활성화할 때 공통 MCP 설정이나
다른 활성 플러그인과 서버 이름이 겹치면 설정을 덮어쓰지 않고 활성화를 거절합니다.
활성 플러그인에 진단 오류가 있으면 실행 시 MCP 연결 목록을 만들지 못해 오류가 나므로, 플러그인을 고치거나
진단 옆의 **비활성화**(데스크톱)로 끄세요. 플러그인 삭제는 설치된 복사본만 제거합니다.
가져온 원본 폴더와 기존 개별 설정, MCP 인증 프로필은 삭제하지 않습니다.

## Claude 설치 플러그인 가져오기

**설치됨**의 **Claude 설치 플러그인 가져오기**는 앱이 관리하는 Claude 제공자 프로필(기본과 추가 계정)에
Claude CLI로 설치된 플러그인을 선택한 범위로 복사합니다. 루트에 `plugin.json`이 없고
`.claude-plugin/plugin.json`이 있는 폴더는 Claude Code 플러그인으로 보고 `name`, `description`,
`version`과 `skills/`만 가져옵니다. Hooks·스크립트 등 다른 내용은 가져오지 않으며 `skills/`가 없으면
건너뜁니다. 이미 설치된 플러그인과 형식이 맞지 않는 플러그인은 사유와 함께 건너뛰며, 가져온 항목은
비활성 상태입니다.

데스크톱은 에이전트 턴이 끝날 때마다 같은 가져오기를 **앱 공통** 범위로 자동 실행합니다.
이때 스킬만 포함한 플러그인은 바로 활성화하고 현재 대화의 선택에 추가합니다.
MCP 서버를 포함한 플러그인은 가져온 뒤 운영자가 서버 설정을 검토하고 직접 활성화해야 합니다.

## 상태와 대화별 선택

**설치됨**, **활성화됨**, 인증 연결은 서로 다른 상태입니다. OAuth가 필요한
서버는 플러그인을 활성화한 뒤 **MCP 연결**에서 현재 제공자·계정으로 로그인하세요.
네이티브 제공자와 실제 서버가 도구를 지원하는지는 실행 시 확인됩니다.

데스크톱 대화 헤더의 **+ 플러그인 연결**에서 플러그인, 개별·패키지 스킬, MCP를 검색하고
체크합니다. 체크한 항목만 유형별 아이콘과 이름으로 헤더에 표시됩니다. 헤더나 선택창에서
체크를 해제하면 현재 대화의 다음 턴부터 제외되며 설치·공통 활성 상태, MCP 설정과 OAuth
인증은 유지됩니다. MCP의 **등록됨**은 설정이 존재한다는 뜻이며 실제 연결·인증 성공을
뜻하지 않습니다. **설치·연결 관리**에서 공통 설정을 변경할 수 있습니다.
좁은 창에서는 헤더 아래줄로 배치하고, 이름이 많으면 목록 영역을 스크롤할 수 있습니다.

선택은 같은 대화의 이후 턴에도 유지되며 새 대화에는 이어지지 않습니다. 전송한 선택은
기존 지침 영수증과 함께 기록되고 대화를 다시 열면 복원됩니다. 미전송 변경은 기존 초안에
보관됩니다. 실행 중에는 선택을 변경할 수 없으며, 이미 제공자 문맥에 전달한 내용을 지우지는
않습니다. 설치된 스킬의 내용을 바꾸려면
원본 폴더를 수정하고 플러그인을 삭제한 뒤 다시 가져오세요. 개별 스킬 편집기는
직접 추가한 스킬을 위한 기능입니다.

## CLI와 HTTP

채팅/수정 요청의 선택적 `extensions: {plugins: ["repo:review"], mcp: ["local-tools"]}`는
해당 턴의 패키지와 MCP를 지정합니다. 개별 스킬은 기존 `skills` 필드로 지정합니다.
명시적인 빈 선택은 선택적 패키지·MCP와 미선택 스킬 메타데이터를 제외합니다.
필드를 생략하는 기존 CLI·웹·HTTP/MCP 요청의 동작은 유지됩니다. 호스트 도구와 기존
승인 절차는 유지되며, 존재하지 않거나 비활성화된 선택은 갱신 후 재시도를 요청합니다.

```text
gpui-agent --guidance-home <공통홈> --cwd <작업공간> plugins
gpui-agent --guidance-home <공통홈> --cwd <작업공간> plugins --import <폴더> --scope repo
gpui-agent --guidance-home <공통홈> --cwd <작업공간> plugins --enable repo:review-tools
gpui-agent --guidance-home <공통홈> --cwd <작업공간> plugins --disable repo:review-tools
gpui-agent --guidance-home <공통홈> --cwd <작업공간> plugins --remove repo:review-tools
gpui-agent --guidance-home <공통홈> --cwd <작업공간> plugins --sync-claude
```

관리 명령은 제공자 실행 파일이나 모델 로그인을 요구하지 않습니다. 관리 동작은 한 번에
하나만 지정합니다. 가져오기 기본 범위는 `repo`이며 `--import`와 함께 `--scope user`도 지원합니다.
`--sync-claude`는 Claude 설치 플러그인을 `repo` 범위에 비활성 상태로 가져오고 결과를 JSON으로 출력합니다.

운영자 인증 HTTP API:

| 메서드 | 경로 | 요청 |
| --- | --- | --- |
| GET | `/v1/plugins` | 없음 |
| POST | `/v1/plugins/import` | `{scope, files}` — 상대 경로별 원본 바이트 배열 |
| POST | `/v1/plugins/claude` | `{scope, enable?}` — Claude 설치 플러그인 가져오기, `enable` 기본값 false |
| PUT | `/v1/plugins/enabled` | `{id, enabled, mcp_servers?}` — 프로젝트 MCP 플러그인을 활성화할 때는 검토한 `manifest.mcp_servers` 전체를 전달 |
| POST | `/v1/plugins/remove` | `{id}` |

`/v1/plugins/claude`는 `{catalog, imported, skipped}`를, 나머지는 `{plugins, diagnostics}`를
반환합니다. 기존 작업공간 선택 헤더가 적용됩니다.
웹·원격 클라이언트는 선택한 폴더의 바이트를 업로드하며, API 호스트에 클라이언트의
로컬 절대 경로를 읽게 하지 않습니다. 에이전트에 주입되는 scoped MCP는 활성 스킬의
조회·읽기를 지원하며 플러그인 관리 권한을 추가하지 않습니다.
