# my-llm-tracker

Claude · OpenAI(Codex) · OpenCode Go **구독의 남은 사용량**을 작업표시줄 트레이에서 바로 보여주는 Windows 네이티브 앱입니다.

- **가볍습니다.** Rust + Win32 API만 사용합니다. 웹뷰도, GUI 프레임워크도, 별도 TLS 라이브러리도 없습니다. 실행 파일은 약 370KB이고, 작성자 PC(Windows 11)에서 측정한 상주 메모리는 유휴 시 작업 집합 약 0.3MB, 팝업을 띄우고 조회 중일 때도 프라이빗 약 2~4MB였습니다.
- **조용합니다.** 평소에는 메시지 루프에서 잠들어 있고, 5분(기본)마다 한 번 조회합니다.
- **계정 기준입니다.** 서버가 집계한 값을 읽기 때문에, 다른 PC·맥에서 쓴 양도 함께 반영됩니다.

> ⚠️ **에이전틱 코딩으로 만든 프로젝트입니다.** 이 프로젝트의 코드는 AI 코딩 에이전트([Claude Code](https://claude.com/claude-code))가 사람과 대화하며 작성·디버깅했습니다. 자세한 내용은 [에이전틱 코딩으로 만들었습니다](#에이전틱-코딩으로-만들었습니다)를 참고하세요.

## 이렇게 보입니다

트레이 아이콘을 **좌클릭**하면 서비스별 남은 % 와 리셋까지 남은 시간이 팝업으로 뜹니다. 바깥을 클릭하면 닫힙니다.

```
Claude                                   max
5h   ███████████████████░   97%  3h 17m
7d   ███████████████████░   97%  5d 23h

OpenAI                                prolite
7d   ████████████████████   99%  6d 17h

OpenCode Go
5h   ████████████████████  100%  -
7d   ████████████████████  100%  4d 13h
30d  ████████████░░░░░░░░   62%  2d 12h
```

- 막대는 **남은 양**입니다. 40% 이상 초록, 15~40% 노랑, 15% 미만 빨강입니다.
- 아이콘에 마우스를 올리면 서비스별 요약이 툴팁으로 나옵니다.
- **우클릭**하면 메뉴가 나옵니다: 지금 새로고침 / OpenCode 쿠키 붙여넣기 / Windows 시작 시 실행 / 설정 파일 열기 / 종료.
- 한 서비스가 실패해도 나머지는 계속 표시되고, 실패한 서비스에는 사유가 표시됩니다.

## 동작 방식

각 서비스가 **공식 문서화된 사용량 API를 제공하지 않기 때문에**, 각 서비스의 CLI·웹 콘솔이 내부적으로 쓰는 엔드포인트를 읽습니다.

로그인 정보는 이미 PC에 있는 것을 읽습니다.

- **Claude** (Pro/Max)
  - 표시: 5시간 · 7일
  - 조회: `api.anthropic.com/api/oauth/usage`
  - 로그인 정보: `%USERPROFILE%\.claude\.credentials.json` (또는 `CLAUDE_CONFIG_DIR` 폴더)
- **OpenAI** (ChatGPT/Codex)
  - 표시: 플랜이 주는 창 (5시간 · 7일)
  - 조회: `chatgpt.com/backend-api/wham/usage`
  - 로그인 정보: `%CODEX_HOME%\auth.json` (환경변수가 없으면 `~\.codex\auth.json`)
- **OpenCode Go**
  - 표시: 5시간 · 7일 · 30일
  - 조회: `opencode.ai/console/api/go/status`
  - 로그인 정보: 브라우저의 콘솔 세션 쿠키 (아래 [서비스별 준비](#서비스별-준비) 참고)

## 설치

### 빌드

[Rust](https://rustup.rs/)(stable, MSVC 툴체인)가 필요합니다.

```bash
git clone https://github.com/demolabskr/my-llm-tracker.git
cd my-llm-tracker
cargo build --release
```

결과물은 `target\release\llm-tracker.exe` 입니다. 원하는 위치에 두고 실행하세요. 트레이 우클릭 → **Windows 시작 시 실행**을 켜면 *그 시점에 실행 중인 exe의 경로*가 자동 실행으로 등록되므로, 파일을 옮기지 않을 위치에서 켜세요.

### 서비스별 준비

**Claude / OpenAI** — 이 PC의 Claude Code, Codex CLI에 로그인되어 있으면 추가 설정이 없습니다. 토큰이 만료되면 해당 CLI를 한 번 실행해 갱신하거나, 아래 `auto_refresh`를 켜세요.

**OpenCode Go** — 사용량을 주는 공개 API가 없어 콘솔의 세션 쿠키가 필요합니다. 한 번만 등록하면 됩니다.

1. [opencode.ai](https://opencode.ai)에 로그인한 브라우저에서 `F12` → **Application** → **Cookies** → `https://opencode.ai`
2. 이름이 **`__Host-console_session`** 인 쿠키의 **Value**(보통 `st_`로 시작)를 복사
3. 앱 트레이 우클릭 → **OpenCode 쿠키 붙여넣기 (클립보드)**

쿠키는 Windows DPAPI로 암호화되어 현재 Windows 사용자만 읽을 수 있게 저장됩니다. 쿠키가 만료되면 앱이 "세션 만료"라고 알려주니 같은 방법으로 다시 붙여넣으면 됩니다.

## 설정

`%APPDATA%\llm-tracker\config.json` (트레이 우클릭 → **설정 파일 열기**). 앱은 조회할 때마다 이 파일을 다시 읽으므로 재시작이 필요 없습니다(`interval_min`은 재시작 후 적용).

- `interval_min` (기본 `5`): 조회 주기(분). 1~120
- `auto_refresh` (기본 `false`): `true`면 만료된 Claude/OpenAI OAuth 토큰을 앱이 직접 갱신합니다 (아래 주의 참고)
- `codex_home` (기본 없음): Codex 로그인 폴더를 직접 지정합니다. 없으면 `CODEX_HOME` 환경변수, 그다음 `~\.codex`를 씁니다
- `autorun`: Windows 시작 시 실행 여부입니다. 트레이 메뉴가 관리하며, 시작할 때 자동 실행 등록이 사라져 있으면 이 값을 보고 복구합니다
- `opencode_cookie_dpapi`: 앱이 관리하는 암호화된 쿠키입니다. 직접 수정하지 마세요

조회에 실패하면 전체 에러 문구가 `%APPDATA%\llm-tracker\last-error.txt`에 남습니다(모두 성공하면 삭제됩니다).

### `auto_refresh` 주의

켜면 앱이 만료된 토큰을 갱신하고 **로그인 파일(`.credentials.json`, `auth.json`)을 새 토큰으로 다시 씁니다.** OAuth refresh token은 갱신 때마다 교체되므로, CLI가 같은 순간에 갱신하면 충돌해 재로그인이 필요할 수 있습니다. 기본값이 `false`인 이유입니다.

## 보안·프라이버시

- 로그인 정보(토큰/쿠키)는 **각 서비스의 공식 도메인으로만** 전송됩니다: `api.anthropic.com`, `platform.claude.com`, `chatgpt.com`, `auth.openai.com`, `opencode.ai`.
- 이 앱은 텔레메트리·분석·자체 서버 통신이 없습니다.
- 쿠키는 DPAPI로 암호화해 저장합니다. 토큰은 메모리에서만 쓰고 화면·로그에 출력하지 않습니다(에러 로그에는 서버가 돌려준 오류 문구만 남습니다).
- 직접 빌드해서 쓰기를 권합니다. 인증 정보를 다루는 프로그램이므로 [소스](src/)를 먼저 읽어보세요.

## 한계

- **비공식 엔드포인트**를 사용합니다. 서비스 측이 형식을 바꾸면 조회가 깨질 수 있습니다(이미 OpenCode 콘솔은 개발 중 구조가 바뀌어 파서를 다시 만들었습니다). 서비스별 모듈로 분리되어 있어 한 곳이 깨져도 나머지는 동작합니다.
- Windows 전용입니다(Win32 API, WinHTTP, DPAPI 사용). macOS의 Claude Code는 로그인 정보를 키체인에 저장하므로 이 방식이 통하지 않습니다.
- OpenAI는 플랜에 따라 5시간 창 없이 7일 창만 내려올 수 있습니다. OpenCode의 5시간 창은 사용 중인 창이 없으면 리셋 시각이 `-`로 표시됩니다.
- 서비스의 요금제·한도 정책은 서비스가 정합니다. 이 앱의 수치는 참고용입니다.

## 개발

```bash
cargo test            # 파서/유틸 단위 테스트
cargo build           # 디버그 빌드
```

- 디버그 빌드에서는 `--demo`(가짜 데이터)와 `--show`(시작 시 팝업을 열고 유지) 옵션을 쓸 수 있어 UI를 확인하기 좋습니다. `--show`는 릴리스 빌드에서도 동작합니다.
- 구조: `src/main.rs`(트레이·팝업·메뉴) · `src/http.rs`(WinHTTP 클라이언트) · `src/store.rs`(설정·DPAPI) · `src/util.rs`(시간·base64·JSON) · `src/providers/{claude,openai,opencode}.rs`

## 에이전틱 코딩으로 만들었습니다

이 프로젝트는 **에이전틱 코딩(agentic coding)** 으로 만들어졌습니다. 사람이 요구사항을 말하고 결과를 확인·피드백하는 동안, AI 코딩 에이전트(Anthropic [Claude Code](https://claude.com/claude-code))가 코드 작성, 디버깅, 문서 작성, 실제 환경에서의 검증을 수행했습니다.

- 설계·코드·이 README의 대부분을 AI가 작성했습니다. 비공식 API의 형식은 AI가 설치된 CLI 바이너리와 웹 콘솔 번들을 직접 분석해 확인했고, 작성자 PC에서 실제로 조회가 되는 것까지 검증했습니다.
- 그럼에도 **다른 환경에서는 검증되지 않았을 수 있습니다.** 인증 정보를 다루는 코드이니 사용 전에 직접 검토해 주세요.

## 면책

이 프로젝트는 Anthropic, OpenAI, OpenCode와 **제휴·후원·승인 관계가 없는 비공식 도구**입니다. 각 서비스의 이름과 상표는 해당 소유자의 것입니다. 비공식 엔드포인트의 사용이 각 서비스의 약관에 부합하는지는 사용자가 직접 확인해야 하며, 사용에 따른 책임은 사용자에게 있습니다.

## 라이선스

[MIT License](LICENSE) © 2026 demolabskr
