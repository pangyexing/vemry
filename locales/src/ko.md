# Korean (`ko`) translation conventions

The rules `ko.json` is written to. Concept words come from
[docs/GLOSSARY.md](../../docs/GLOSSARY.md); these settle everything else two
translators would otherwise do differently. A new string follows them, and a
changed rule changes the catalog with it.

## Style

- Register: every sentence (hints, toasts, errors, confirmations, empty states) is 해요체/하세요체: -어요, -예요, -세요, -ㄹ까요. Never 합니다/하십시오체, never 반말 (-다, -해). Buttons, menu items, tabs, column headers, chips and command titles carry no sentence ending: a noun ("삭제", "적용", "검토 시작") or the -기 nominal ("이름 바꾸기", "노트로 저장"); never "-하세요" on a button.
- Fragments stay fragments: an English fragment (a cmd.*.desc like "Every task in the vault, by due date") becomes a noun phrase with no ending ("보관소의 모든 작업, 기한순"); an English sentence becomes a 해요체 sentence ("Closes the current document" → "현재 문서를 닫아요"). End with a period only where the English has one.
- "You" is omitted: no 당신, 귀하, 사용자님. "Your" is dropped ("Open your vault first" → "먼저 보관소를 여세요"); 내 only where ownership is the point (default name "내 보관소", "내 키 사용"). The provenance origin "You" = "나".
- The AI is a tool, not a person: "AI" stays Latin ("AI가", "AI의"); dative is "AI에" (inanimate), never "AI에게/AI한테"; no honorific -시- for the AI, the model or the agent; no human verbs (생각해요, 고민해요, 알아요, 원해요, 도와드려요). The model's thinking feature is 추론 ("추론 깊이", "추론 중", "추론 접기"); agent = 에이전트, model = 모델, session = 세션, turn = 턴, context = 컨텍스트, token = 토큰.
- Particles after placeholders: restructure first so the particle lands on a fixed noun ("Delete {name}?" → "‘{name}’ 노트를 삭제할까요?", or put the placeholder last after a colon). If a particle must follow a placeholder ({name}, {title}, {field}, {vault}, {folder}), use the paired form, never a guessed one: "{name}을(를)", "{name}이(가)", "{name}은(는)", "{name}과(와)", "{name}(으)로". 의 and 에/에서 need no pairing.
- Counters: noun + number + counter, the counter attached to the number: "노트 {n}개", "블록 {n}개", "작업 {n}개", "파일 {n}개", "변경 {n}개", "링크 {n}개", "카드 {n}장", "{n}단계", "{n}일", "{n}줄", "{n}단어", "{n}자", "{n}크레딧". Not "{n}개의 노트". Plural keys carry `_other` only, and counted nouns take no -들.
- Spacing (띄어쓰기): standard spelling, bound nouns spaced ("할 수 있어요", "여는 중", "진행 중"); glossary compounds are written exactly as in this file, spaces included ("보관소 메모리", "프라이버시 구역", "블록 ID", "실행 취소", "새로고침", "세션 다시 보기"); "AI" is a separate word ("AI 변경", "AI 블록").
- Em dash "—": do not carry it over. A cause→consequence dash becomes a connective ending or two sentences ("The agent is running — cannot fork" → "에이전트가 실행 중이라 분기할 수 없어요"); a label-plus-explanation dash in a hint becomes ": " ("Privacy zone — invisible to the AI" → "프라이버시 구역: AI에 보이지 않아요"); an aside becomes parentheses. Window titles "X — vemry" keep the dash.
- Ellipsis: keep the single character "…" (U+2026) exactly where the English has it — menu items that open a further dialog or picker, and progress states ("보관소 전환…", "불러오는 중…"); no space before it; never three periods.
- Punctuation: half-width . , : ? ! ( ) only, no full-width marks or 「」. No space before "(" ("사이드바 접기({key})"). Names quoted with “ ” in English take Korean single quotes ‘ ’ ("No claims match “{query}”." → "‘{query}’과(와) 일치하는 주장이 없어요."). The menu-path separator › and the middle dot · stay, with a space on each side ("설정 › 계정").
- Confirmations ask with -ㄹ까요? and state the consequence ("이 노트를 삭제할까요? 되돌릴 수 없어요."); the confirm button repeats the verb ("삭제"), not 예/아니요; click-again confirmations read "한 번 더 누르면 …".
- Errors and toasts: say what failed, then what to do: "-하지 못했어요" / "-할 수 없어요" + "-하세요" ("연결하지 못했어요. 잠시 후 다시 시도하세요."). Never 실패했습니다, 오류가 발생했습니다, 죄송합니다. A completed action's toast is past tense -했어요 ("변경 3개를 되돌렸어요"); state chips, columns and badges use nouns or -됨/중 ("취소됨", "미검토", "진행 중"). Task states: to-do 할 일, in progress 진행 중, done 완료, cancelled 취소됨.
- Key names (key.*) stay exactly as in English, in Latin letters: Ctrl, Shift, Alt, Backspace, Win; likewise Enter, Esc, Tab, Space and ⌘ ⌥ ⇧; combinations join with "+" and no spaces ("Ctrl+Shift+P"). Never 컨트롤, 시프트, 엔터, 윈도우 키.
- Untranslated, in Latin letters: vemry (always lower case, sentence-initial too), AI, Pro, PDF, URL, YAML, frontmatter, diff, ^id, [[…]], file and folder names (clips/, inbox.md, VEMRY.md, .vemry/), field keys and query words (status::, due_before, today), every slash.* string, langName.*. "Markdown" in prose is 마크다운; on-disk callout type names (`> [!note]`) stay English while their shown labels translate (Note 참고, Tip 팁, Warning 경고, To do 할 일).
- Word pairs that must not merge: Undo = 실행 취소 vs revert = 되돌리기; Deny (permission gate) = 거부 vs Reject (a plan) = 거절; reviewing AI changes = 검토 (unreviewed 미검토, "Mark as reviewed" = "검토 완료로 표시") vs flashcard Review / Study ahead = 복습 / 미리 복습; chat Plan = 계획 vs subscription Plan = 요금제; reference = 참조 vs citation = 인용 vs blockquote = 인용구.
- More pairs: provenance = 출처, so the source a clip, embed or citation came from = 원본 ("원본 페이지", "원본 열기") and the AI's cited web sources = 자료 ("자료 목록"); quick capture and a capture = 빠른 기록 vs clip = 스크랩; note history = 노트 이력, browser history = 방문 기록, audit log = 감사 로그; Agenda = 일정 vs scheduled date = 예정일; due date = 기한 (never 마감, which is a deadline; overdue = 기한 지남); fold/Collapse = 접기, unfold/Expand = 펼치기; ad blocking = 차단 vs a blocked task = 막힘.
- Concision: labels as short as the English allows; drop articles and demonstratives English needs ("the", "this") unless they disambiguate; prefer a Sino-Korean noun to a native clause on a button ("검토 시작", not "검토를 시작하기"). Shown dates and numbers come from Intl — never hand-format a date inside a string; "{n} d" → "{n}일", "Ln {line}, Col {col}" → "줄 {line}, 열 {col}".

## Words decided outside the glossary

Interface words the glossary does not cover, fixed while the catalog was written
so that every string uses the same one.

| English | Korean |
|---|---|
| follow (the note the AI works in), working note | 작업 노트 |
| Conventions (the VEMRY.md section) | 규칙 |
| method (a skill, as the picker calls it) | 방법 |
| action items | 실행 항목 |
| write (noun, an AI write) | 쓰기 |
| discard | 폐기 / 폐기됨 |
| undo (a turn's writes) / revert | 실행 취소 / 되돌리기 |
| attachment bucket | 폴더 |
| chat | 채팅 |
| schedule (a script's) | 예약 |
