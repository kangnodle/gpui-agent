# 공통 AGENTS.md와 Skills

gpui-agent가 지침과 스킬을 직접 읽고 Claude, Codex, Gemini/Antigravity에 같은 사용자 수준의 문맥을 전달한다. 모델 제공자가 설치되어 있지 않아도 목록 조회와 파일 해석은 동작한다. Codex 저장소의 코드·크레이트·빌드 산출물에 의존하지 않는다.

## AGENTS.md

전역 지침은 앱의 `--home` 아래 `AGENTS.override.md`, `AGENTS.md` 순서로 첫 비어 있지 않은 파일을 읽는다. 추가 계정도 같은 공통 지침 home을 사용한다. provider별 인증 디렉터리는 공통 지침 home과 별개다.

프로젝트는 가장 가까운 `.git`이 있는 디렉터리부터 작업 cwd까지 각 디렉터리의 `AGENTS.override.md`, `AGENTS.md`를 확인한다. Git 저장소가 아니면 선택한 cwd만 확인한다. 프로젝트에서는 빈 override도 같은 디렉터리의 AGENTS.md를 가린다. cwd 아래 다른 프로젝트를 재귀 탐색하지 않는다.

전역 → 저장소 root → 하위 디렉터리 순서로 적용하고 출처와 revision을 함께 전달한다. 전역과 프로젝트 본문에 각각 32 KiB 예산을 적용한다. UTF-8 문자 경계를 보존하며 초과·읽기 오류는 조회 결과의 진단에 표시한다. 파일 변경과 삭제는 다음 턴에 반영한다.

## Skills

공통 검색 위치:

- `<app-home>/skills/<skill>/SKILL.md`
- 저장소 root부터 cwd까지 각 디렉터리의 `.agents/skills/<skill>/SKILL.md`

예:

```markdown
---
name: verify
description: 변경된 코드를 공개 인터페이스의 테스트로 검증할 때 사용한다.
---
관련 패키지 테스트를 실행하고 실행한 명령과 결과를 보고한다.
필요하면 references/checks.md를 읽는다.
```

YAML frontmatter의 description은 필수다. name이 없으면 디렉터리 이름을 사용한다. 종료 delimiter가 EOF에 있으면 마지막 줄바꿈 없이 빈 본문도 허용한다. 선택적 `agents/openai.yaml`의 `interface.display_name`과 `policy.allow_implicit_invocation`을 읽는다. 암묵 호출 기본값은 true다. YAML은 [serde_yaml_ng 0.10.0](https://docs.rs/serde_yaml_ng/0.10.0/serde_yaml_ng/)으로 파싱한다.

동명 스킬도 경로가 다르면 별도로 표시한다. 명시 선택은 `repo:.agents/skills/verify/SKILL.md` 또는 `user:skills/verify/SKILL.md` 같은 scoped ID를 사용한다. `$verify`는 활성 스킬 중 이름이 유일할 때 선택한다. 모호한 이름은 ID 선택을 요청하는 오류가 된다.

한 턴에 최대 8개, 선택 본문 합계 64 KiB까지 사용한다. 본문을 읽지 않은 스킬은 메타데이터만 제공하며 초기 catalog 예산은 JSON 기준 약 8 KiB다. 모델용 진단에도 별도 약 8 KiB 예산과 누락 개수를 적용하며 운영자 조회에는 전체 진단을 보존한다. 탐색 깊이 6, 디렉터리 2,000개, 엔트리 20,000개, 스킬 512개로 탐색을 제한한다. 개별 skill/resource는 UTF-8 텍스트 64 KiB 이하다.

호스트 MCP 연결이 있으면 모델이 `host_skills_list`, `host_skills_read`로 본문과 보조 자료를 필요할 때 읽는다. 읽기 도구는 skill ID와 skill 내부의 상대 resource 경로만 받는다. 비활성 스킬, `..`, 절대 경로, 허용 root 밖으로 나가는 symlink/junction은 거절한다. manifest와 보조 metadata의 링크도 발견한 스킬 디렉터리 밖으로 나갈 수 없다. 스크립트를 읽어도 자동 실행하지 않으며 기존 도구 승인과 sandbox를 유지한다.

## 클라이언트

데스크톱은 대화 헤더의 **+ 플러그인 연결**에서 플러그인·스킬·MCP를 함께 선택한다.
체크한 항목만 유형별 아이콘과 이름으로 표시하고, 체크 해제는 해당 대화에만 적용한다.
선택한 스킬은 같은 대화의 다음 턴에도 유지되며 설치·공통 활성 상태는 변경하지 않는다.
웹은 입력부의 **플러그인** 목록에서 선택한 스킬을 현재 턴에 적용한다. 지침이 없거나 조회하지 못한 상태는
구분해서 표시하며 작업공간과 계정 사이에 선택을 섞지 않는다.

데스크톱 **설정 → 지침**, 웹 **설정 · 지침**에서 AGENTS.md와 override 파일, 적용 출처와 진단을 확인하고 편집한다. 데스크톱은 **앱 공통 / 현재 프로젝트** 카드의 **AGENTS.md 만들기**를, 웹은 생성 위치를 고른 뒤 **AGENTS.md 생성 / 열기**를 누른다.
데스크톱 **설정 → 플러그인 → 스킬**, 웹 **설정 · 플러그인 → 스킬 → 스킬 관리**에서 SKILL.md를 관리하고 공통 활성 상태를 변경한다. 데스크톱은 **앱 공통 / 현재 프로젝트** 탭을 고른 뒤 스킬 추가에서 이름과 설명을 입력하고 **만들기**를, 웹은 **새 스킬**에서 스킬 폴더 이름과 설명을 입력하고 **Skill 생성 / 열기**를 누른다. 각 설정은 해당 종류의 파일만 표시하며, 저장하지 않은 초안이 있으면 저장하거나 **편집 취소** 후 다른 종류로 전환한다.
현재 프로젝트의 새 파일은 현재 workspace cwd에 저장한다. 기존 파일 목록에는 적용 범위의 상위 지침, override, 비활성 스킬도 포함된다.
Markdown 본문을 직접 작성하거나 로컬 UTF-8 Markdown 파일을 불러온 뒤 **저장**을 누른다.
업로드는 AGENTS.md 또는 SKILL.md 한 파일을 대상으로 하며 zip이나 스킬 보조 파일을 설치하지 않는다.

저장 후 입력부의 목록을 갱신한다. AGENTS.md는 다음 턴에 자동 적용되고 스킬은 기존 선택 또는 `$이름`으로 사용한다.
동일 폴더의 AGENTS.override.md 우선순위와 비활성 스킬 설정은 유지한다.
편집 중 파일 전환은 저장 또는 **편집 취소** 후 가능하며, 파일 불러오기는 현재 초안을 교체한다. 파일 선택 취소나 저장 실패는 초안을 유지한다.
읽은 내용의 revision과 달라졌거나 새 파일의 경로가 이미 존재하면 저장을 거절한다. 초안을 복사하거나 편집 취소 후 최신 파일을 다시 열어 병합한다.
32 KiB를 넘는 AGENTS, 64 KiB를 넘는 스킬, 잘못된 스킬 frontmatter, 탐색 범위 밖의 경로와 symlink/junction을 통한 편집은 거절한다.

CLI:

```text
gpui-agent --guidance-home <app-home> --cwd <workspace> skills
gpui-agent --guidance-home <app-home> --cwd <workspace> skills --disable repo:.agents/skills/verify/SKILL.md
gpui-agent --guidance-home <app-home> --cwd <workspace> ask claude "변경을 검증해줘" --skill repo:.agents/skills/verify/SKILL.md
gpui-agent --guidance-home <app-home> --cwd <workspace> ask codex "변경을 검증해줘" --skill repo:.agents/skills/verify/SKILL.md
gpui-agent --guidance-home <app-home> --cwd <workspace> ask antigravity "변경을 검증해줘" --skill repo:.agents/skills/verify/SKILL.md
```

CLI의 기존 `--home`은 provider 인증 디렉터리다. `--guidance-home`은 공통 지침 디렉터리이며 기본값은 `~/.gpui-agent`다. 독립 CLI의 명시 선택은 동작하지만 host MCP를 연결하지 않은 실행에서는 암묵 본문 읽기 기능을 제공하지 않는다.

HTTP는 기존 operator 인증과 `X-KND-Workspace`/계정 scope를 사용한다.

| 경로 | 동작 |
| --- | --- |
| `GET /v1/guidance` | 호스트가 준비하는 AGENTS 원문·출처, 스킬 metadata, revision, 진단 |
| `GET /v1/skills` | 스킬 metadata·진단·revision |
| `POST /v1/skills/read` | `{ "id": "…", "resource": "SKILL.md" }` 읽기 |
| `PUT /v1/skills/enabled` | `{ "id": "…", "enabled": false }` 공통 설정 저장 |
| `GET /v1/guidance/files` | 편집 가능한 파일 ID 목록과 현재 프로젝트 상대 접두사 |
| `POST /v1/guidance/files/read` | `{ "id": "repo:AGENTS.md" }` → 원문과 revision, 없는 파일은 빈 원문과 null revision |
| `PUT /v1/guidance/files` | `{ "id": "…", "text": "…", "expected_revision": null }` 생성 또는 읽은 revision으로 편집 저장 |

편집 API는 operator 전용이며 기존 workspace/account scope를 따른다. 에이전트 MCP는 파일을 직접 쓰지 못하고, 아래 "에이전트 스킬 제안"의 제안만 만들 수 있다.

채팅/잡 요청의 `skills`는 `[{"id":"repo:.agents/skills/verify/SKILL.md"}]` 형식이다. MCP `conversation_send`도 같은 선택을 지원한다. MCP 에이전트는 호출 세션의 workspace 안에서 조회·읽기와 스킬 제안만 할 수 있으며 공통 활성 설정을 바꿀 수 없다.

선택은 큐 요청과 대화 기록에 보존한다. 원문 전체 대신 적용 출처/revision 영수증을 저장한다. 질문 수정·fork에서도 선택을 복원하며 다른 제공자로 문맥을 전송할 때 과거 선택을 자동 실행하지 않는다. 큐 대기 중 파일이 변경되면 실행 시점 내용을 사용하고, 명시 선택이 삭제/비활성되면 오류로 종료한다. 내용 버전을 고정하는 replay 기능은 제공하지 않는다.

공통 설정은 `<app-home>/skill-settings.json`에 잠금과 원자적 교체를 사용해 저장한다. user 스킬의 활성 상태는 전역으로, repo 스킬은 해당 workspace root를 기준으로 저장한다. 제공자별 설정 파일을 편집하지 않는다.

## 네이티브 런타임과의 관계

공통 문맥은 provider의 시스템/developer 지침보다 우선하지 않는다. 현재 턴의 호스트 블록이 이전 호스트 블록을 대체함을 전달하지만 native conversation history 자체를 삭제하지는 않는다.

Codex에는 턴 실행 시 `project_doc_max_bytes=0`, `skills.include_instructions=false`를 전달해 자동 프로젝트 지침과 catalog 중복을 줄인다. 전역 native AGENTS 및 모든 내부 skill 선택까지 끈다는 보장은 없다. Claude에는 공통 문맥이 있을 때 `--disable-slash-commands`를 사용해 native skill 중복을 줄인다. OAuth를 읽지 않는 `--bare`는 사용하지 않는다. Claude의 CLAUDE.md/메모리와 Antigravity의 native 지침은 추가로 적용될 수 있다.

Antigravity의 MCP 지원은 기존 capability 검사에 따른다. 설치 runtime이 필요한 전송을 지원하지 않으면 명시 본문 전달과 암묵 읽기 지원을 구분해야 한다. 이 호스트의 source 미리보기는 native 내부의 전체 문맥을 보여주는 기능이 아니다.

플러그인 설치와 Claude 설치 플러그인 가져오기는 [공통 플러그인](plugins.md)에서 다룬다. 그 밖의 외부 provider skill cache 가져오기, cloud skill authority, custom fallback/root markers, watcher, 본문 snapshot replay는 범위에서 제외한다.

## 스킬 폴더 가져오기와 확인

데스크톱의 **앱 공통에 폴더 가져오기…** / **이 프로젝트에 폴더 가져오기…** 또는 웹의 **스킬 폴더 가져오기**로 기존 스킬을 복사한다.
앱 공통/현재 프로젝트 선택이 복사 위치를 결정한다. 원본 폴더는 수정하거나 동기화하지 않는다.
같은 이름의 대상 폴더가 있으면 거절하며, 다른 이름의 원본 폴더를 선택해야 한다.
`SKILL.md`, `agents/openai.yaml`, references/scripts/assets를 포함한 일반 파일을 보존한다.
링크와 Windows reparse 경로는 가져오지 않는다. 스크립트는 가져오기만으로 실행하지 않는다.
한 폴더 최대 128파일, 파일당 64 KiB, 전체 1 MiB, 경로 깊이 6까지 허용한다.
웹에서는 브라우저가 제공하는 폴더 파일 목록을 같은 호스트 검증에 전달한다.

스킬 검색은 이름, 설명, scoped ID를 대소문자 구분 없이 검색한다. 자세히·편집에서 manifest를 읽고
수정할 수 있다. 자동 사용 허용 여부는 metadata 정책을 보여주며 실행 보장은 아니다.
저장된 대화의 적용 기록이 있는 명시 스킬은 메시지에 표시된다. 자동 호출로 본문을 읽은 경우는
기존 도구 실행 내역의 `host_skills_read`에서 확인한다. 목록의 활성 상태만으로 사용됐다고 표시하지 않는다.

`POST /v1/skills/import`는 operator 전용이고 `{id, files}`를 받는다.
files는 스킬 내부 상대 경로에서 바이트 배열로의 매핑이다. application 검증과 GuidanceSource 파일 adapter가
데스크톱·웹 요청을 동일하게 처리하며 에이전트 MCP에는 설치 권한을 제공하지 않는다.
CLI의 기존 선택/조회 계약과 세 provider의 전송 계약은 그대로 사용한다.

### 목록과 생성 흐름

스킬 설정은 검색·출처·설명·활성 상태가 있는 목록으로 시작한다. 스킬 추가(웹은 **새 스킬**)를 누르면 이름과 사용할 상황을 입력하고,
이 값으로 frontmatter가 생성된 Markdown 초안을 연다. 기존 스킬은 자세히·편집에서 Markdown을 수정한다.
데스크톱은 frontmatter·크기·경로 오류를 저장 전에 표시하고 저장 버튼을 잠근다. 호스트 검증은 모든 클라이언트에서 유지된다.
목록이 비었거나 검색 결과가 없는 상태를 표시하며, 조회·파일 오류는 진단과 오류 영역에 표시한다.

대화 초안 끝에 `$`와 이름 일부를 입력하면 활성 스킬 후보가 표시된다. 후보를 선택하면 해당 토큰을 제거하고 scoped ID
선택 칩을 유지한다. 이미 선택한 스킬을 다시 지정해도 중복 선택하거나 기존 선택을 제거하지 않는다.
제공자 상태는 직접 선택, 호스트 연결 시 자동 사용 가능, 런타임 capability 확인 필요를 구분한다.
지원 표시는 모델의 스킬 사용을 보장하지 않는다. 폴더 패키지 가져오기는 기존 경로를 사용하며 ZIP 지원은 추가하지 않았다.

## 에이전트 스킬 제안

에이전트가 목적 달성에 필요한 스킬을 스스로 만들 수 있도록 MCP `host_skill_propose`를 제공한다. 제안은 설치가 아니다.
운영자가 승인하기 전에는 파일이 생기지 않고 `host_skills_list`에도 나타나지 않는다.

| 도구 | 동작 |
| --- | --- |
| `host_skill_propose` | `{name, content, resources?}`로 `repo:.agents/skills/<name>/SKILL.md` 제안을 만들고 대기 중 제안 ID를 즉시 반환. 쓰기 가능한 세션만 호출 |
| `host_skill_proposal_status` | 같은 세션 scope의 제안 상태(`pending/applying/applied/rejected/failed/expired`)와 실패 사유 조회 |

운영자 HTTP:

| 경로 | 동작 |
| --- | --- |
| `GET /v1/skill-proposals` | 대기 중인 제안과 전체 본문 |
| `POST /v1/skill-proposals/{id}` | `{ "allow": true \| false }` 승인 또는 거절. 이미 결정되었으면 409 |

제안은 워크스페이스 repo scope에만 만들 수 있고 user(앱 공통) scope는 불가하다. 이름은 ASCII 영숫자·`-`·`_` 1–64자이며 `-`로 시작할 수 없다. `SKILL.md`는 `content`로만 전달하고 `resources`에 넣을 수 없다.
승인은 기존 스킬 가져오기와 같은 검증(`SkillImportRequest`)을 거치므로 frontmatter·크기·경로 규칙이 같고 기존 스킬을 덮어쓰지 않는다.
스크립트는 저장만 하며 실행 권한을 부여하지 않는다. 제안은 메모리에만 있어 호스트 재시작 시 사라지고, 30분 후 만료되며,
세션 scope당 대기 제안은 8개로 제한한다. 에이전트 세션 토큰은 승인 경로를 호출할 수 없다(401).
승인은 운영자 화면에서 한다. 웹은 **에이전트 제안** 영역이 5초마다 목록을 갱신하며, 데스크톱은 **설정 → 플러그인 → 스킬**의 **현재 프로젝트** 범위에서
제안을 보여준다(열 때와 **제안 새로 고침**을 누를 때 갱신하며 자동 갱신은 없다). 두 화면 모두 설치될 모든 파일의 경로와 내용을
보여 주고 **승인하고 설치** 또는 **거절**을 받는다. 승인 후 스킬 목록이 갱신된다.
독립 실행 CLI는 호스트 서버 없이 동작하므로 제안 레지스트리가 없고 승인 명령도 제공하지 않는다.
MCP 서버 제안, 승인 후 이어가기, 턴당 제안 횟수 제한과 감사 기록은 아래 절에서 설명한다.

## 에이전트 MCP 서버 제안

스킬과 같은 방식으로 에이전트가 필요한 MCP 서버를 제안할 수 있다. 승인 전에는 아무것도 설치하거나 시작하지 않는다.

| 도구 | 동작 |
| --- | --- |
| `host_mcp_propose` | `{server, rationale}` 제안을 만들고 대기 중 제안 ID를 즉시 반환. 쓰기 가능한 세션만 호출 |
| `host_mcp_proposal_status` | 같은 세션 scope의 제안 상태와 실패 사유 조회 |

`server`는 기존 `McpServer` 형식이다. stdio는 호스트에 실제로 있는 실행 파일의 절대 경로가 필요하고, HTTP는 HTTPS 또는
loopback HTTP만 허용한다. 인증은 환경 변수 **이름**(`bearer_token_env_var`, `env_http_headers`)만 받으며 비밀 값은 받지 않는다.
OAuth 서버는 대화형 로그인이 필요해 제안할 수 없고, 에이전트에게는 운영자가 MCP 설정에서 등록하고 로그인하라고 안내한다. `rationale`은 필수이며 1–1000자다.

**환경 변수 준비 확인.** 환경 변수가 없으면 해당 서버가 활성화된 동안 모든 턴이 실패하므로, 제안과 승인 시점 모두에 호스트
프로세스에 변수가 있고 값이 헤더로 쓸 수 있는지(`valid_mcp_header_value`, 실제 턴과 같은 규칙) 확인한다. 준비되지 않았으면
변수 **이름**만 알려 거절하며, 운영자가 호스트를 시작하기 전에 설정해야 한다. 값은 저장·로그·응답·설치된 `plugin.json` 어디에도 남지 않고,
운영자 목록(`GET /v1/mcp-proposals`)에는 변수 이름과 `ready` 여부만 표시한다. 승인 직전에 변수가 사라지면 아무것도 설치하지 않고 실패로 기록한다.

운영자 HTTP는 `GET /v1/mcp-proposals`(대기 목록)와 `POST /v1/mcp-proposals/{id}`(`{ "allow": true | false }`)이다.
승인하면 서버 하나짜리 워크스페이스 플러그인 `repo:mcp-<이름>`(이름은 소문자, `_`는 `-`)으로 설치하고 바로 활성화한다.
기존 플러그인 설치·활성화 검증을 그대로 거치므로 공통 MCP 설정이나 다른 활성 플러그인과 이름이 겹치면 활성화가 거절되고,
이 경우 방금 설치한 복사본도 제거해 아무것도 남기지 않는다. 같은 플러그인이 이미 있으면 덮어쓰지 않고 실패한다.
적용은 다음 대화 턴부터 반영되며 실행 중인 작업에는 영향을 주지 않는다.
제안은 스킬 제안과 같은 규칙(메모리 전용, 30분 만료, scope당 대기 8개)을 따른다.
웹은 **에이전트 제안**, 데스크톱은 **설정 → 플러그인 → 스킬**(**현재 프로젝트** 범위)에 스킬 제안과 함께 표시한다.

## 승인 후 대화 이어가기

스킬과 MCP 서버는 제공자가 **턴을 시작할 때** 읽으므로, 실행 중인 턴에는 승인 결과가 반영되지 않는다.
제안을 만든 세션이 대화에 연결되어 있으면(`conversation_id`), 운영자가 **승인**한 직후 호스트가 같은 대화에
이어서 실행할 턴을 큐에 넣는다. 이 턴에는 제안 ID, 설치된 스킬 ID 또는 플러그인 ID와 "원래 작업을 이어서 진행하라"는 안내만 담는다.
제안자가 정한 이유나 본문은 넣지 않는다.

- 큐가 같은 프로필의 진행 중인 턴 뒤로 순서를 맞추므로, 에이전트는 폴링하지 않고 턴을 끝내도 된다.
- 제안 ID로 멱등 키(`proposal-<id>`)를 만들어 같은 제안이 두 번 이어지지 않는다.
- 거절·만료·실패는 이어가지 않는다. 에이전트는 `host_*_proposal_status`로 확인한다.
- 대화에 연결되지 않은 세션의 제안은 이어갈 대화가 없으므로 설치만 한다.
- 이어가기 큐잉에 실패해도 승인은 이미 적용되었으므로 실패로 돌리지 않고, 응답의 `message`에 사유를 담는다.
- 운영자 화면은 별도 조작 없이 같은 대화에 새 턴이 나타나는 것으로 확인한다. 이 턴은 기존 큐 승인 규칙을 따른다.

## 에이전트 제안의 안전장치

제안은 어디까지나 요청이며 자동으로 승인되는 경로가 없다. 승인·거절 API는 운영자 인증만 받고, 읽기 전용 세션은 제안 자체를 만들 수 없다.

- **프롬프트 주입 대비.** 호스트는 제안이 사용자 지시에서 나왔는지 웹 페이지·도구 결과에서 나왔는지 알 수 없다. 그래서 출처를 추정해 표시하지 않고,
  제안 시점에 호스트가 저장된 대화에서 읽은 **사용자의 마지막 요청**(최대 500자, 제어 문자 제거)을 승인 화면에 보여 준다. 에이전트가 쓴 값이 아니며,
  운영자는 제안이 실제 요청과 맞는지 대조한다. 대화에 연결되지 않았거나 읽을 수 없으면 "확인할 수 없음"으로 표시하고 제안은 막지 않는다.
- **횟수 제한.** 한 에이전트 실행(turn)은 거절된 제안을 포함해 4개까지만 제안할 수 있고, scope당 대기 제안은 8개로 제한한다.
  한도에 닿으면 에이전트는 사용자에게 묻거나 가진 도구로 마무리하라는 안내를 받는다.
- **감사 기록.** 제안의 생성·승인 처리·적용·거절·실패·만료마다 한 줄의 JSON을 stderr에 남긴다
  (`event=agent_proposal`, `kind`, `proposal`, `outcome`, `provider`, `account`, `conversation_id`). 제안 본문, 사용자 요청, 오류 문장, 토큰은
  기록하지 않는다. 단위 테스트가 본문이 기록에 없음을 확인한다.
