# 프로젝트 목표

Windows 10/11에서 실행되는 초경량 LLM 사용량·할당량 모니터링 오버레이 애플리케이션을 구현하라.

이 프로그램은 항상 화면 최상단에 작은 floating overlay 형태로 존재하며, 다음 Agent들의 현재 quota와 실제 작업 여부를 한눈에 확인하기 위한 도구다.

대상 Agent:

- ChatGPT Work
- Claude Code
- Google Antigravity

단순한 UI mockup이 아니라 실제 로컬 인증정보와 Agent 상태를 이용하여 동작하는 완성된 Windows 애플리케이션을 구현해야 한다.

---

# 핵심 요구사항

프로그램은 다음 정보를 제공한다.

각 Agent의:

- 5시간 할당량
- 1주 할당량
- 각 할당량 초기화까지 남은 시간
- 현재 Agent가 실제 작업 중인지 여부

단, activity 상태는 별도의 `ACTIVE`, `RUNNING` 같은 텍스트로 표시하지 않는다.

현재 작업 중인 Agent는 자신의 accent 요소가 아주 옅게 pulse animation을 하도록 표현한다.

불필요한 정보는 전부 제외한다.

다음 정보는 표시하지 않는다.

- token count
- token cost
- request count
- 오늘 사용량
- 월 사용량
- model별 token 사용량
- API 가격
- subscription plan
- account email
- 최근 갱신 시각
- 로그
- Agent 상태 텍스트
- 기타 설명 문구

---

# 기존 구현 조사

구현을 시작하기 전에 기존 오픈소스 프로젝트를 조사한다.

우선순위:

1. `CodeZeno/Claude-Code-Usage-Monitor`
2. `lee890211/limitdock`
3. `robinebers/openusage`

특히 `CodeZeno/Claude-Code-Usage-Monitor`는 Windows 환경에서 다음 provider들을 이미 처리하므로 우선적으로 분석한다.

- Claude Code
- Codex / ChatGPT quota
- Antigravity

provider 인증, quota 조회, credential 탐색, fallback 처리 등은 가능하면 기존 구현을 참고하거나 라이선스 조건에 맞춰 재사용한다.

이미 해결된 provider-specific 문제를 불필요하게 다시 reverse engineering하지 않는다.

단, UI는 본 프로젝트 요구사항에 맞춰 새롭게 구현한다.

---

# 기술 스택

권장:

- Tauri 2
- Rust
- React
- TypeScript
- Vite

대상:

- Windows 10 x64
- Windows 11 x64

Electron은 특별한 이유가 없는 이상 사용하지 않는다.

이 애플리케이션은 항상 실행되는 작은 overlay이므로 idle CPU/RAM 사용량이 중요하다.

---

# 전체 Architecture

provider-specific logic과 UI를 분리한다.

권장 구조:

```text
src-tauri/
  providers/
    chatgpt.rs
    claude.rs
    antigravity.rs
    mod.rs

  auth/
    codex.rs
    claude.rs
    antigravity.rs

  activity/
    mod.rs
    ipc.rs
    state.rs
    watchdog.rs

  quota/
    model.rs
    formatter.rs
    cache.rs

  settings/
    mod.rs

src/
  components/
    Overlay.tsx
    Provider.tsx
    QuotaRow.tsx
    ActivityIndicator.tsx

  hooks/
    useQuota.ts
    useCountdown.ts
    useActivity.ts
```

provider 하나가 실패해도 나머지 provider의 조회 및 UI 동작에 영향을 주지 않아야 한다.

---

# Backend / Frontend 보안 경계

Provider 인증정보 접근, OAuth 처리, HTTP quota 요청은 Rust backend에서만 수행한다.

다음 정보는 frontend에 절대 전달하지 않는다.

- access token
- refresh token
- auth.json
- credential contents
- OAuth secret
- API key

React frontend에는 정규화된 데이터만 전달한다.

예:

```ts
interface QuotaWindow {
  remainingPercent: number | null;
  resetAt: string | null;
}

interface QuotaPool {
  name?: string;
  fiveHour?: QuotaWindow;
  weekly?: QuotaWindow;
}

interface ProviderQuota {
  provider: string;
  pools: QuotaPool[];
}
```

Activity 데이터:

```ts
type AgentActivity =
  | "active"
  | "idle"
  | "unknown";

interface ProviderActivity {
  provider: string;
  state: AgentActivity;
}
```

---

# Quota Provider Abstraction

각 provider 응답을 직접 UI에서 처리하지 않는다.

다음과 같은 adapter 구조를 만든다.

```text
QuotaProvider
 ├─ ChatGPTProvider
 ├─ ClaudeProvider
 └─ AntigravityProvider
```

각 Provider는 최종적으로 동일한 내부 데이터 구조를 반환한다.

---

# 1. ChatGPT Work

UI에서는 이름을 반드시:

```text
ChatGPT Work
```

로 표시한다.

UI에 `Codex`라는 이름을 quota provider 이름으로 노출하지 않는다.

ChatGPT Work와 Codex가 공유하는 포함 quota를 이용한다.

ChatGPT 웹 UI를 scraping하지 않는다.

DOM scraping, browser automation, OCR 방식은 사용하지 않는다.

가능하면 로컬 Codex 인증정보를 이용하여 실제 quota backend를 호출한다.

우선 확인할 credential:

```text
%USERPROFILE%\.codex\auth.json
```

또는:

```text
CODEX_HOME\auth.json
```

현재 설치된 Codex CLI 및 기존 monitor 구현을 분석하여 사용 중인 quota API와 응답 구조를 확인한다.

예를 들어 response가 다음 개념을 제공할 수 있다.

```text
primary_window
secondary_window
```

이를 각각:

```text
5시간
주간
```

quota로 정규화한다.

API가 사용량 비율을 반환하는 경우:

```text
remainingPercent = 100 - usedPercent
```

로 변환한다.

---

# 2. Claude Code

Claude Code가 이미 로그인되어 있는 환경을 기본 전제로 한다.

다음 credential source들을 확인한다.

```text
%USERPROFILE%\.claude\.credentials.json
```

환경변수:

```text
CLAUDE_CONFIG_DIR
CLAUDE_CODE_OAUTH_TOKEN
```

필요하면 WSL 내부 Claude Code credential도 지원한다.

Claude Code subscription quota를 조회한다.

가능하면 provider가 직접 반환하는 실제 quota utilization과 reset timestamp를 사용한다.

필요 정보:

```text
5-hour utilization
5-hour reset timestamp

weekly utilization
weekly reset timestamp
```

단순히 로컬 token 소비량을 누적하여 quota를 추정하지 않는다.

provider가 반환한 utilization을 우선한다.

현재 OAuth usage endpoint가 동작한다면 이를 사용하되, 해당 endpoint가 public/stable API가 아닐 가능성을 고려하여 provider adapter 내부에 완전히 격리한다.

endpoint 변경에 대비해 기존 monitor들의 fallback 구현도 조사한다.

---

# 3. Google Antigravity

Antigravity quota를 하나의 pool로 합치지 않는다.

현재 quota pool 구조를 조사하여 최소 다음 두 종류를 독립적으로 지원한다.

```text
Gemini
Claude/GPT
```

각 pool별로 다음을 표시한다.

```text
5시간 quota
주간 quota
```

우선순위:

1. 실행 중인 Antigravity local language server quota summary
2. Antigravity local OAuth credential을 통한 quota summary
3. 기존 monitor 구현의 fallback

가능하면 Antigravity 자체가 사용하는 quota summary 데이터를 그대로 이용한다.

Antigravity CLI `/usage` 화면을 OCR하거나 terminal text scraping 방식으로 읽지 않는다.

UI는 다음 구조를 사용한다.

```text
Antigravity

Gemini
5h
Week

Claude/GPT
5h
Week
```

Antigravity API 구조가 바뀌었을 경우 현재 설치된 Antigravity가 실제 사용하는 provider call을 조사해 compatibility layer를 작성한다.

---

# Quota Percentage Normalization

UI에서 표시하는 percentage는 항상:

```text
남은 할당량
```

이다.

즉 provider가 사용률을 제공하면:

```text
remaining = 100 - utilization
```

percentage는 항상:

```text
0 <= remainingPercent <= 100
```

범위로 clamp한다.

---

# Reset Time

각 quota window는 가능한 경우 정확한 reset timestamp를 저장한다.

Frontend에서는:

```text
resetAt - now
```

를 계산하여 countdown을 표시한다.

여러 단위를 동시에 표시하지 않는다.

가장 높은 단위 하나만 표시하며 나머지는 버린다.

예:

```text
5일 2시간 2분 1초
→ 5일
```

```text
3시간 25분 42초
→ 3시간
```

```text
47분 51초
→ 47분
```

```text
38초
→ 38초
```

구현 규칙:

```text
>= 86400초
→ floor(totalSeconds / 86400) + "일"

>= 3600초
→ floor(totalSeconds / 3600) + "시간"

>= 60초
→ floor(totalSeconds / 60) + "분"

else
→ floor(totalSeconds) + "초"
```

음수 시간은 절대 표시하지 않는다.

reset timestamp가 현재 시각보다 과거라면 즉시 해당 provider를 refresh한다.

refresh가 완료되기 전에는 최소:

```text
0초
```

까지만 표시한다.

---

# Network Refresh

기본 quota polling:

```text
60초
```

reset countdown은 매초 network request를 보내지 않는다.

로컬 timestamp를 이용하여 계산한다.

Provider request 실패 시 이전 성공 데이터를 즉시 제거하지 않는다.

last known good value를 유지한다.

실패가 반복되면 exponential backoff를 적용한다.

HTTP 상태 코드별 처리:

```text
401 / 403
→ credential 또는 login 문제
→ 무한 retry 금지

429
→ Retry-After가 있다면 이를 따른다

5xx / network failure
→ exponential backoff
```

---

# Activity Detection

각 Agent가 단순히 실행 중인지가 아니라:

```text
현재 실제 사용자 요청을 처리 중인지
```

를 감지한다.

프로세스가 존재한다는 사실만으로 `active`로 판단하지 않는다.

가능하면 Agent lifecycle hook을 사용한다.

상태:

```ts
type AgentActivity =
  | "active"
  | "idle"
  | "unknown";
```

의미:

```text
active
= 사용자 요청을 처리 중
= model generation / tool loop / agent execution이 진행 중

idle
= Agent 실행은 가능하지만 현재 사용자 요청을 처리하지 않음

unknown
= 정확한 activity 정보를 얻지 못함
```

---

# Activity Architecture

가능하면 각 Agent의 user-level lifecycle hook이 로컬 overlay backend로 작은 event를 전달하는 구조를 사용한다.

권장 IPC:

```text
Windows Named Pipe
```

예:

```text
\\.\pipe\llm-quota-overlay
```

hook payload 예:

```json
{
  "provider": "claude",
  "event": "active",
  "timestamp": 1234567890
}
```

또는:

```json
{
  "provider": "claude",
  "event": "idle",
  "timestamp": 1234567890
}
```

다음 정보는 IPC에 포함하지 않는다.

- prompt
- response
- tool arguments
- source code
- credential
- API key
- OAuth token
- conversation content

activity 여부만 전송한다.

---

# ChatGPT Work Activity

ChatGPT Work quota는 Codex/shared quota를 통해 가져오더라도 activity detection은 실제 사용 환경을 고려한다.

Codex Agent lifecycle hook을 사용할 수 있다면 우선 사용한다.

Agent turn이 시작되면:

```text
active = true
```

Agent execution이 종료되면:

```text
active = false
```

현재 Codex 버전이 제공하는 hook 종류를 확인하고 start / stop pair를 선택한다.

예:

```text
turn start
→ active

turn completed
→ idle
```

Codex 프로세스가 존재하는 것만으로 active 처리하지 않는다.

만약 ChatGPT Work 자체가 Codex와 독립적인 frontend/activity 구조를 사용하고 lifecycle hook으로 감지할 수 없다면:

```text
quota는 정상 제공
activity = unknown
```

으로 graceful degradation한다.

activity를 억지로 잘못 추정하지 않는다.

---

# Claude Code Activity

Claude Code lifecycle hooks를 우선 사용한다.

가능한 event 예:

```text
UserPromptSubmit
PreToolUse
PostToolUse
Stop
StopFailure
SessionEnd
```

권장 상태 변경:

```text
UserPromptSubmit
→ active = true
```

tool execution 중:

```text
active 상태 유지
```

```text
Stop
→ active = false
```

```text
StopFailure
→ active = false
```

```text
SessionEnd
→ active = false
```

Claude Code terminal이 켜져 있어도 prompt 입력을 기다리는 상태라면:

```text
idle
```

이다.

---

# Antigravity Activity

Antigravity lifecycle hook을 사용한다.

가능한 경우 다음 event를 이용한다.

```text
PreInvocation
PostInvocation
PreToolUse
PostToolUse
Stop
```

권장:

```text
PreInvocation
→ active = true
```

`PostInvocation`만으로 바로 idle 처리하지 않는다.

한 user turn 안에서:

```text
model invocation
→ tool call
→ model invocation
```

처럼 여러 invocation이 반복될 수 있기 때문이다.

최종 execution loop 종료 event:

```text
Stop
```

을 idle 판정 기준으로 사용한다.

즉:

```text
PreInvocation
→ active
```

```text
Stop
→ idle
```

tool execution 동안에도 active 상태를 유지한다.

---

# Activity Watchdog

hook event가 항상 정상적으로 종료된다는 보장은 없다.

Agent crash, terminal 강제 종료, hook 실패 등에 대비한다.

따라서 watchdog을 구현한다.

예:

```text
active event 수신
→ state = active
```

이후 일정 시간 activity event가 없을 경우:

```text
state = idle
```

권장 timeout:

```text
10분
```

단, 장시간 실제 실행 가능한 Agent를 고려하여 timeout은 코드 내부 상수 또는 설정값으로 쉽게 변경 가능하게 한다.

provider process 종료가 감지된다면 즉시 idle 전환할 수 있다.

단 process detection은 lifecycle hooks의 fallback으로만 사용한다.

---

# Activity Pulse UI

Agent가:

```text
active
```

상태일 때 해당 Agent의 accent 요소가 매우 은은하게 pulse한다.

강한 깜빡임은 금지한다.

사용자가 하루 종일 화면에 띄워놔도 거슬리지 않아야 한다.

권장 animation:

```css
@keyframes agent-pulse {
  0%, 100% {
    opacity: 0.45;
  }

  50% {
    opacity: 0.95;
  }
}
```

duration:

```text
2.0 ~ 3.0초
```

권장:

```text
2.5초
```

timing:

```text
ease-in-out
infinite
```

다음 요소 중 하나 또는 조합을 pulse시킨다.

- Agent 이름 옆 작은 status dot
- Agent accent line
- 매우 약한 border glow

추천:

```text
Agent 이름 옆 작은 원형 indicator
+
아주 약한 accent glow
```

전체 카드 background가 번쩍이는 방식은 사용하지 않는다.

Quota progress bar 자체가 계속 밝아졌다 어두워지는 것도 피한다.

Activity indication은 작고 주변적인 시각 요소만 이용한다.

---

# Activity UI Semantics

activity 상태를 텍스트로 출력하지 않는다.

금지:

```text
ACTIVE
WORKING
RUNNING
GENERATING
PROCESSING
IDLE
```

idle:

```text
indicator 고정
```

active:

```text
indicator가 천천히 pulse
```

unknown:

```text
idle과 동일하거나 약간 더 낮은 opacity
```

unknown 상태 때문에 warning icon 또는 오류문을 상시 표시하지 않는다.

여러 Agent가 동시에 작업 중이면 각각 독립적으로 pulse한다.

예:

```text
ChatGPT Work active
Claude Code active
Antigravity idle
```

이라면 앞의 두 개만 동시에 pulse한다.

Activity animation은 quota refresh 주기와 완전히 독립적으로 동작한다.

---

# Overlay Window

애플리케이션은 일반 dashboard가 아니라 작은 desktop HUD 형태다.

조건:

- frameless
- transparent
- always-on-top
- draggable
- taskbar에 일반 application window로 표시하지 않음
- resize는 기본적으로 불필요
- focus를 불필요하게 뺏지 않음

가능하면 overlay를 클릭하지 않을 때 일반 작업 흐름을 방해하지 않도록 한다.

---

# Drag

빈 공간 또는 provider title 영역을 드래그하여 overlay 전체를 움직일 수 있어야 한다.

텍스트 selection 때문에 drag가 끊기지 않도록 한다.

마지막 window position을 저장한다.

재실행 시 복원한다.

---

# Multi-monitor

다중 모니터 환경을 지원한다.

저장할 정보:

```text
window x
window y
```

필요하다면 monitor ID도 저장한다.

모니터가 제거되었거나 저장된 좌표가 현재 visible desktop 범위 밖이라면:

```text
primary monitor visible area
```

안으로 자동 이동시킨다.

화면 바깥에 영구적으로 떠버리는 상태가 발생해서는 안 된다.

---

# Opacity

overlay 전체 opacity를 조절할 수 있어야 한다.

상시 slider는 표시하지 않는다.

권장 interaction:

```text
Ctrl + Mouse Wheel
```

예:

```text
Ctrl + Wheel Up
→ opacity +5%

Ctrl + Wheel Down
→ opacity -5%
```

범위:

```text
30% ~ 100%
```

값은 저장한다.

프로그램 재시작 후 유지한다.

대안:

```text
Right Click Context Menu
→ Opacity
```

를 추가할 수 있다.

둘 다 제공해도 된다.

---

# UI Layout

Agent별 정보를 명확하게 구분한다.

예:

```text
● ChatGPT Work

5h     ███████░░░  72%   3시간
Week   █████████░  89%   5일


● Claude Code

5h     ████░░░░░░  43%   1시간
Week   ██████░░░░  61%   3일


● Antigravity

Gemini
5h     ████████░░  81%   4시간
Week   █████░░░░░  54%   2일

Claude/GPT
5h     █████████░  94%   4시간
Week   ███████░░░  76%   4일
```

`●`가 activity indicator다.

실제 progress bar는 매우 얇게 만든다.

overlay 전체 크기는 필요한 최소한으로 유지한다.

과도한 padding을 사용하지 않는다.

---

# UI Density

목표는:

```text
Compact
Readable
Minimal
Glanceable
```

이다.

dashboard처럼 거대한 card layout을 만들지 않는다.

각 Agent마다 지나치게 큰 box를 만들지 않는다.

window width는 내용이 충분히 읽히는 최소 크기로 유지한다.

권장:

```text
280 ~ 360px
```

범위를 시작점으로 사용한다.

실제 text alignment를 보고 조정한다.

---

# Color

Agent별 accent color를 고정한다.

색상은:

- Agent name
- activity indicator
- progress bar
- 아주 약한 accent glow

정도에만 사용한다.

전체 카드나 전체 background를 provider color로 칠하지 않는다.

예:

```text
ChatGPT Work
→ green 계열

Claude Code
→ warm orange / beige 계열

Antigravity
→ blue / violet 계열
```

정확한 hue는 전체 dark overlay와 조화를 고려해 결정한다.

Antigravity 내부:

```text
Gemini
Claude/GPT
```

는 동일한 Antigravity 디자인 계열을 유지한다.

색을 완전히 다른 provider처럼 갈라놓지 않는다.

필요하면 brightness 또는 saturation만 미세하게 차이낸다.

---

# Typography

Font:

```text
Pretendard
```

오타 없이 사용한다.

Weight:

```text
SemiBold
Medium
```

을 주로 사용한다.

권장:

```text
Agent name
→ SemiBold

percentage
→ SemiBold

5h / Week
→ Medium

reset countdown
→ Medium

pool name
→ Medium 또는 SemiBold
```

과도한 Bold는 사용하지 않는다.

Pretendard가 시스템에 설치되지 않은 환경에서도 정상 표시되도록 한다.

필요하다면 font asset을 application에 포함한다.

fallback:

```text
"Segoe UI", sans-serif
```

---

# Background / Visual Style

overlay background는:

```text
dark translucent
```

스타일을 사용한다.

예:

```text
rgba(15, 15, 18, 0.80)
```

정도의 방향성을 사용하되 최종 수치는 디자인 과정에서 조정한다.

가능하다면:

- subtle blur
- 아주 약한 border
- 작은 border radius

를 적용한다.

지나친 glassmorphism은 피한다.

정보 가독성이 최우선이다.

---

# Error Display

provider quota 조회 실패 시 긴 오류 메시지를 UI에 표시하지 않는다.

실패한 값만:

```text
—
```

로 표시한다.

예:

```text
5h   —   —
```

상세 오류는 debug log에만 기록한다.

credential이나 개인정보는 log에 포함하지 않는다.

---

# Cache

마지막 성공 quota 값을 로컬 cache에 저장한다.

network 일시 장애가 발생해도 이전 값을 유지한다.

단 cache가 오래되었다면 내부적으로 stale 상태를 구분한다.

stale 여부를 반드시 UI에 텍스트로 표시할 필요는 없다.

---

# Local Storage

저장 가능한 값:

- window position
- opacity
- refresh interval
- activity timeout
- last known quota
- autostart preference

저장하지 않아야 할 값:

- OAuth access token 복제본
- refresh token 복제본
- API key 복제본
- prompt
- response
- conversation content

기존 Agent credential을 가능한 그대로 읽는다.

새 credential 저장이 불가피하다면:

```text
Windows Credential Manager
```

또는:

```text
Windows DPAPI
```

를 사용한다.

---

# Security

이 애플리케이션은 local-first다.

금지:

- 자체 backend server
- telemetry
- analytics
- usage tracking
- credential 업로드
- crash report 자동 전송
- auth file 전체 log
- token log
- frontend localStorage에 credential 저장
- React state에 credential 저장

모든 provider request는 사용자 컴퓨터에서 직접 발생한다.

---

# Autostart

Windows 로그인 시 자동 시작 기능을 제공한다.

첫 실행 시 강제로 활성화하지 않는다.

사용자가 선택할 수 있게 한다.

권장 위치:

```text
Right Click Context Menu
→ Start with Windows
```

가능하면 관리자 권한 없이 동작하게 한다.

---

# Context Menu

overlay에 우클릭 context menu를 제공할 수 있다.

최소 기능:

```text
Refresh Now
Opacity
Start with Windows
Quit
```

설정 UI를 별도의 거대한 창으로 만들 필요는 없다.

---

# Manual Refresh

context menu의:

```text
Refresh Now
```

를 누르면 모든 provider를 즉시 refresh한다.

각 provider는 병렬로 조회하되 서로 실패를 전파하지 않는다.

---

# Performance

always-on application이라는 점을 고려한다.

목표:

```text
Idle CPU
→ 사실상 0%에 가깝게

Quota network request
→ 기본 60초 간격

Countdown
→ 가벼운 local timer

Animation
→ CSS 기반

Unnecessary rerender
→ 최소화
```

React component 전체를 매초 rerender하지 않는다.

reset countdown 값이 변해야 하는 부분만 업데이트한다.

---

# Hook Installation

activity detection에 필요한 hook 설정을 자동 또는 반자동으로 설치할 수 있게 한다.

가능하면 첫 실행 시 현재 설치된 Agent들을 감지한다.

예:

```text
Claude Code detected
Codex detected
Antigravity detected
```

해당 Agent가 hook을 지원하면 현재 user-level hook 설정을 읽는다.

기존 사용자의 hook 설정을 삭제하거나 덮어쓰지 않는다.

반드시 merge한다.

hook 제거 시에도 본 애플리케이션이 추가한 부분만 제거한다.

다른 tool의 hook configuration을 파괴하지 않는다.

---

# Hook Helper

hook command는 가능한 한 매우 작은 helper binary를 사용한다.

예:

```text
llm-overlay-hook.exe
```

역할:

```text
provider + event
→ named pipe로 전달
→ 즉시 종료
```

실행 시간은 매우 짧아야 한다.

Agent response latency에 눈에 띄는 영향을 주면 안 된다.

hook helper가 실패하더라도 Agent 실행 자체가 실패하면 안 된다.

즉 hook은:

```text
best effort
```

방식으로 동작한다.

---

# Activity Event Debounce

짧은 연속 tool call 때문에 activity state가 깜빡이지 않도록 한다.

예:

```text
PostToolUse
→ 바로 idle 처리 금지
```

activity 시작:

```text
즉시 active
```

activity 종료:

```text
실제 turn 종료 event에서만 idle
```

필요하면 idle event에:

```text
100 ~ 300ms
```

정도의 작은 debounce를 줄 수 있다.

단 사용자 눈에 느껴질 정도의 지연은 만들지 않는다.

---

# Accessibility

강한 flashing을 사용하지 않는다.

pulse는 느리고 부드러워야 한다.

frequency는 seizure risk나 시각적 피로를 유발할 정도로 빠르면 안 된다.

가능하다면 OS의:

```text
prefers-reduced-motion
```

또는 Windows animation preference를 존중한다.

reduced motion 환경에서는:

```text
pulse animation
→ 고정된 조금 더 밝은 indicator
```

로 대체한다.

---

# 테스트

최소 다음 테스트를 작성한다.

## Quota

1. used percentage → remaining percentage 변환
2. percentage clamp
3. malformed provider response
4. credential 누락
5. HTTP 401
6. HTTP 403
7. HTTP 429
8. provider 5xx
9. provider 하나 실패 시 나머지 정상 동작
10. last known quota cache
11. Antigravity Gemini / Claude-GPT pool 분리

## Reset timer

12. days formatting
13. hours formatting
14. minutes formatting
15. seconds formatting
16. expired timestamp
17. negative countdown 방지

## Activity

18. active event
19. idle event
20. Stop event 누락
21. watchdog timeout
22. process crash
23. hook helper failure
24. 동시에 두 provider active
25. 동시에 세 provider active
26. provider activity unknown

## UI

27. drag
28. opacity 저장
29. window position 저장
30. multi-monitor position restore
31. monitor 제거 후 visible region 복귀
32. always-on-top
33. reduced motion
34. context menu
35. autostart toggle

---

# Graceful Degradation

일부 Agent의 quota 또는 activity API를 사용할 수 없더라도 애플리케이션 전체가 실패하면 안 된다.

예:

```text
ChatGPT quota 성공
ChatGPT activity unknown

Claude quota 성공
Claude activity active

Antigravity quota 실패
Antigravity activity idle
```

이 상태에서도 정상 실행되어야 한다.

quota 실패:

```text
—
```

activity 감지 실패:

```text
pulse 없음
```

으로 처리한다.

---

# Logging

debug logging은 제공하되 기본적으로 조용하게 동작한다.

로그에 남길 수 있는 정보:

- provider request success / failure
- HTTP status
- parser error
- hook event type
- cache hit / miss

로그에 남기면 안 되는 정보:

- access token
- refresh token
- API key
- account email
- prompt
- model response
- tool arguments
- conversation content
- raw credential file contents

민감값은 반드시 redact한다.

---

# Build

최종적으로 Windows release build를 생성한다.

목표:

```text
installer 또는 portable executable
```

가능하면 둘 다 제공한다.

release mode에서 debug console window가 불필요하게 뜨지 않도록 한다.

---

# 완료 조건

다음 조건을 모두 만족해야 완료로 간주한다.

1. Windows에서 실제 실행 가능
2. always-on-top overlay 정상 동작
3. drag 정상 동작
4. opacity 조절 가능
5. position 유지
6. multi-monitor 대응
7. ChatGPT Work 5h quota 표시
8. ChatGPT Work weekly quota 표시
9. Claude Code 5h quota 표시
10. Claude Code weekly quota 표시
11. Antigravity Gemini 5h quota 표시
12. Antigravity Gemini weekly quota 표시
13. Antigravity Claude/GPT 5h quota 표시
14. Antigravity Claude/GPT weekly quota 표시
15. 각 quota reset countdown 표시
16. countdown 단위 버림 정상 동작
17. Claude Code activity detection
18. Antigravity activity detection
19. 가능한 범위에서 ChatGPT/Codex activity detection
20. active Agent에 은은한 pulse 표시
21. 여러 Agent 동시 pulse 가능
22. provider 하나 실패해도 나머지 정상 동작
23. credential frontend 미노출
24. credential log 미노출
25. autostart 기능
26. release build 성공

---

# 구현 순서

다음 순서로 진행한다.

## Phase 1

기존 repository를 조사한다.

특히:

```text
CodeZeno/Claude-Code-Usage-Monitor
```

의 provider / auth / quota 구현을 먼저 확인한다.

현재 설치된:

- Codex
- Claude Code
- Antigravity

버전도 확인한다.

---

## Phase 2

각 provider별 quota retrieval proof-of-concept를 만든다.

UI보다 먼저:

```text
ChatGPT Work
Claude Code
Antigravity
```

quota를 실제로 읽는 것부터 검증한다.

---

## Phase 3

Agent activity lifecycle hook을 검증한다.

각 Agent에서:

```text
start event
stop event
```

를 실제로 받아볼 수 있는지 확인한다.

hook 이름이나 설정 구조가 현재 버전에서 바뀌었다면 현재 문서를 기준으로 수정한다.

---

## Phase 4

공통 backend abstraction과 cache를 구현한다.

---

## Phase 5

Tauri overlay UI를 구현한다.

---

## Phase 6

activity pulse를 연결한다.

---

## Phase 7

autostart, context menu, persistence, multi-monitor를 구현한다.

---

## Phase 8

실제 Agent를 실행하여 end-to-end 테스트한다.

---

# 중요한 구현 원칙

UI부터 만들고 mock data만 연결한 상태로 끝내지 않는다.

가장 먼저 실제 quota source와 activity source를 검증한다.

Provider의 undocumented/internal endpoint를 사용해야 할 경우 해당 로직을 provider adapter 안에 격리한다.

endpoint가 변경될 가능성을 고려하여 parser와 request layer를 분리한다.

기존 monitor 구현이 이미 안정적으로 해결한 부분은 적극 참고한다.

하지만 오래된 코드를 맹목적으로 복사하지 말고 현재 설치된 Agent 버전의 실제 동작을 우선한다.

사용자의 기존 Agent 설정을 파괴하거나 credential을 재발급하지 않는다.

기본 목표는 다음 한 문장으로 요약할 수 있다.

> 화면 구석에 항상 떠 있으면서 ChatGPT Work, Claude Code, Antigravity의 남은 5시간/주간 quota와 reset까지 남은 시간을 최소한의 UI로 보여주고, 현재 실제 작업 중인 Agent만 아주 은은하게 숨 쉬듯 pulse하는 Windows overlay를 구현하라.