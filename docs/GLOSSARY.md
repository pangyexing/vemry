# Glossary

vemry's interface, both READMEs and every copy catalog (`locales/*.json`) **share
this one table**.

Why it exists: when the same thing is called "Projection" in the interface, "View"
in the documentation and 「投影视图」 in Chinese, nobody can say which is right
three months later, and each translation drifts on its own. A new user-visible term
**enters this table before it enters a catalog**.

The interface languages beyond English and Chinese (Japanese, Korean, French,
German, Spanish) are overlays in `locales/src/`; each has a column here, and
`locales/src/README.md` fixes the register per language (です・ます, 하세요체, du,
vous, tú). A concept word is decided here, never by the translator.

## Core concepts

| Chinese | English | Japanese | Korean | French | German | Spanish | Do not use |
|---|---|---|---|---|---|---|---|
| 知识库 | vault | 保管庫 | 보관소 | coffre | Wissensbasis | bóveda | repository, workspace, library |
| 块 / 块索引 | block / block index | ブロック / ブロックインデックス | 블록 / 블록 인덱스 | bloc / index des blocs | Block / Blockindex | bloque / índice de bloques | database, record |
| 块 ID | block id | ブロック ID | 블록 ID | identifiant de bloc | Block-ID | ID de bloque | anchor, uid, block reference id |
| 引用（指向块或小节） | reference | 参照 | 참조 | référence | Verweis | referencia | citation (taken by evidence), pointer |
| 嵌入 | embed | 埋め込み | 임베드 | intégration (verbe : intégrer) | Einbettung | incrustación (verbo: incrustar) | transclusion, include, import |
| 小节 | section | セクション | 섹션 | section | Abschnitt | sección | chapter, part, heading (that is the line itself) |
| 字段 | field | フィールド | 필드 | champ | Feld | campo | property, attribute, metadata |
| 状态字段 | state field | 状態フィールド | 상태 필드 | champ d'état | Zustandsfeld | campo de estado | status field (status is a key's name, not the concept), enum, lifecycle |
| 档（状态字段的一个取值） | step | ステップ | 단계 | étape | Stufe | paso | stage, phase, level |
| 标签片（字段、标签、日期在行上的小胶囊） | chip | チップ | 칩 | pastille | Chip | chip | badge, pill, tag (taken by #tag), capsule |
| 投影视图 | projection | 投影ビュー | 투영 뷰 | projection | Projektion | proyección | dashboard, module, widget |
| 断言 | claim | 主張 | 주장 | affirmation | These | afirmación | assertion, statement |
| 证据 | evidence | 根拠 | 근거 | preuve | Beleg | evidencia | citation, source |
| 权限门 | permission gate | 許可ゲート | 권한 게이트 | barrière d'autorisation | Berechtigungsschranke | control de permisos | approval flow, confirmation |
| 隐私区 | privacy zone | プライバシーゾーン | 프라이버시 구역 | zone privée | Privatzone | zona privada | private folder, locked folder, vault (already taken) |
| 私密块 | private block | プライベートブロック | 비공개 블록 | bloc privé | privater Block | bloque privado | hidden block, redacted block |
| 记忆文件（VEMRY.md） | vault memory | 保管庫メモリ | 보관소 메모리 | mémoire du coffre | Gedächtnisdatei | memoria de la bóveda | config, rules, instructions |
| 技能 | skill | スキル | 스킬 | compétence | Skill | habilidad | plugin, extension (`extension` is reserved for the future extension mechanism) |
| 用户脚本 | user script | ユーザースクリプト | 사용자 스크립트 | script utilisateur | Benutzerskript | script de usuario | macro, plugin |
| 审计日志 | audit log | 監査ログ | 감사 로그 | journal d'audit | Auditprotokoll | registro de auditoría | history, activity |
| 出处 | provenance | 来歴 | 출처 | provenance | Herkunft | procedencia | origin (the frontmatter key only), source (taken by clipping), authorship |
| 未审阅 | unreviewed | 未確認 | 미검토 | non relu | ungeprüft | sin revisar | pending, awaiting review, draft (the code keeps `pending` as the ledger state's name) |
| 撤回 | revert | 破棄 | 되돌리기 | retirer (la modification de l'IA) | zurücknehmen | revertir | reject, roll back, undo (undo is the editor's ⌘Z) |
| 块引用 | block citation | ブロック引用 | 블록 인용 | citation de bloc | blockgenaue Quellenangabe | cita de bloque | block reference, block link |
| 位置 id | positional id | 位置 ID | 위치 ID | identifiant positionnel | Positions-ID | ID posicional | temporary id, transient id |

## The hosted account

| Chinese | English | Japanese | Korean | French | German | Spanish | Do not use |
|---|---|---|---|---|---|---|---|
| vemry 托管 | vemry hosted | vemry ホスティング | vemry 호스팅 | vemry hébergé | vemry-Hosting | vemry alojado | cloud, managed, official API |
| 自带 key | bring your own key | 自分のキー | 내 키 사용 | votre propre clé | eigener Schlüssel | tu propia clave | custom endpoint (that is the setting's internal name), own API |
| 账户 | account | アカウント | 계정 | compte | Konto | cuenta | membership, profile |
| 算力包 | credit pack | クレジットパック | 크레딧 팩 | pack de crédits | Credit-Paket | paquete de créditos | top-up, balance (the pack has a balance; it is not one), tokens |
| 积分 | credits | クレジット | 크레딧 | crédits | Credits | créditos | points, yuan or ¥, dollars (a credit is $0.001 of hosted use; the interface counts credits, never money) |
| 额度 | quota | 利用枠 | 제공량 | quota | Kontingent | cupo | 配额 (reads as rationing), allowance, limit (that is a cap) |
| 设备令牌 | device token | デバイストークン | 기기 토큰 | jeton d'appareil | Gerätetoken | token del dispositivo | API key (it doubles as one, but the user never sees it as such), session |

## Interface elements

| Chinese | English | Japanese | Korean | French | German | Spanish | Do not use |
|---|---|---|---|---|---|---|---|
| 命令面板 | command palette | コマンドパレット | 명령 팔레트 | palette de commandes | Befehlspalette | paleta de comandos | quick switcher, launcher |
| 活动栏 | activity bar | アクティビティバー | 활동 막대 | barre d'activité | Aktivitätsleiste | barra de actividad | sidebar (that means the side panels), toolbar |
| 侧栏 | sidebar | サイドバー | 사이드바 | barre latérale | Seitenleiste | barra lateral | panel |
| 大纲 | outline | アウトライン | 개요 | structure | Gliederung | esquema | table of contents |
| 状态栏 | status bar | ステータスバー | 상태 표시줄 | barre d'état | Statusleiste | barra de estado | statusline (an internal codename; never outward-facing) |
| 标签页 | tab | タブ | 탭 | onglet | Tab | pestaña | document tab |
| 速记 | quick capture | クイック記録 | 빠른 기록 | capture rapide | Schnellerfassung | captura rápida | memo, scratch, inbox |
| 研究面板 | research panel | リサーチパネル | 리서치 패널 | panneau Recherche | Recherchebereich | panel de investigación | chat, assistant |
| 阅读投影 | Reader (interface) / reading projection (architecture prose) | リーダー / 閲覧投影ビュー | 리더 (인터페이스) / 읽기 투영 뷰 (아키텍처 설명) | Lecteur / projection de lecture | Leseansicht (Oberfläche) / Leseprojektion (Architekturtexte) | Lector (interfaz) / proyección de lectura (prosa de arquitectura) | reader mode, article view |
| 对照翻译 | side-by-side translation (the view is "Translation") | 対訳表示（ビュー名は「対訳」） | 대조 번역 (뷰 이름도 "대조 번역") | traduction côte à côte (la vue s'appelle « Traduction ») | Parallelübersetzung (die Projektion heißt „Übersetzung“) | traducción en paralelo (la vista se llama “Traducción”) | bilingual mode, immersive translation |
| 译文 / 原文 | translation / original | 訳文 / 原文 | 번역 / 원문 | traduction / original | Übersetzung / Original | traducción / original | target text, source text |
| 块把手 | block handle | ブロックハンドル | 블록 핸들 | poignée de bloc | Blockgriff | asa del bloque | drag handle, gripper, grabber |
| 转换为 | turn into | 変換 | 전환 | transformer en | umwandeln in | convertir en | convert, change type |
| 块选择 | block selection | ブロック選択 | 블록 선택 | sélection de blocs | Blockauswahl | selección de bloques | multi-select, block mode |
| 折叠 | fold | 折りたたみ | 접기 | replier (unfold : déplier) | falten | plegar (unfold: desplegar) | collapse |

The interface uses the word that reads best (**Reader**); architecture prose uses the concept word (**reading projection**). That is a division of labour, not an inconsistency.

## Tasks

| Chinese | English | Japanese | Korean | French | German | Spanish | Do not use |
|---|---|---|---|---|---|---|---|
| 截止日期 | due date | 期日 | 기한 | échéance | Fälligkeitsdatum | fecha de vencimiento | deadline (in the interface), end date |
| 排期日期 | scheduled date | 予定日 | 예정일 | date planifiée | Plandatum | fecha programada | start date (that is ⏩), planned date |
| 开始日期 | start date | 開始日 | 시작일 | date de début | Startdatum | fecha de inicio | begin date, available from |
| 创建日期 | created | 作成日 | 생성일 | date de création | erstellt am | creada el | added on |
| 完成日期 / 取消日期 | completed on / cancelled on | 完了日 / 中止日 | 완료일 / 취소일 | date d'achèvement / date d'annulation | erledigt am / abgebrochen am | completada el / cancelada el | done date, closed date |
| 已取消 | cancelled | 中止 | 취소됨 | annulée | abgebrochen | cancelada | abandoned, dropped, discarded |
| 优先级 | priority | 優先度 | 우선순위 | priorité | Priorität | prioridad | importance, urgency (that is one step of it) |
| 紧急 / 重要 / 以后 / 有空再说 | urgent / important / later / someday | 緊急 / 重要 / 後で / いつか | 긴급 / 중요 / 나중에 / 언젠가 | urgent / important / plus tard / à l'occasion | dringend / wichtig / später / irgendwann | urgente / importante / más adelante / algún día | highest / high / low / lowest (the `priority::` values, not the copy) |
| 重复 | repeat | 繰り返し | 반복 | répétition | Wiederholung | repetición | recurrence, recurring |
| 完成即弃 | removed once done | 完了で消去 | 완료하면 제거 | supprimée une fois terminée | nach Erledigung entfernt | se quita al completarse | delete on completion, auto-delete |
| 编号（任务的） | id | ID | ID | identifiant (pastille : id) | ID | ID | task id, handle (taken by the block handle) |
| 受阻 | blocked | 着手不可 | 막힘 | bloquée | blockiert | bloqueada | waiting, locked, dependent |
| 估时 | estimate | 見積もり | 예상 시간 | estimation | Schätzung | estimación | effort, duration, time box |

## The eight projections

| Chinese | English | Japanese | Korean | French | German | Spanish |
|---|---|---|---|---|---|---|
| 日程 | Agenda | 日程 | 일정 | Agenda | Agenda | Agenda |
| 看板 | Board | ボード | 보드 | Tableau | Board | Tablero |
| 时间线 | Timeline | タイムライン | 타임라인 | Chronologie | Zeitleiste | Cronología |
| 图谱 | Graph | グラフ | 그래프 | Graphe | Graph | Grafo |
| 断言 | Claims | 主張 | 주장 | Affirmations | Thesen | Afirmaciones |
| 证据 | Evidence | 根拠 | 근거 | Preuves | Belege | Evidencia |
| 复习 | Review | 復習 | 복습 | Révision | Wiederholen | Repaso |
| 会话回放 | Session Replay | セッション再生 | 세션 다시 보기 | Déroulé de session | Sitzungswiedergabe | Reproducción de sesión |

## Clipping and browsing

| Chinese | English | Japanese | Korean | French | German | Spanish | Do not use |
|---|---|---|---|---|---|---|---|
| 剪藏 (verb and noun) | clip | クリップ | 스크랩 | clip (nom) / clipper (verbe) | Clip / clippen | recortar (verbo) / recorte (sustantivo) | save, bookmark, capture (taken by quick capture) |
| 剪藏工具条 | clip bar | クリップバー | 스크랩 바 | barre de clip | Clip-Leiste | barra de recorte | toolbar |
| 落点笔记 | clip target | クリップ先 | 스크랩 대상 | cible du clip | Clip-Ziel | nota de destino | destination |
| 收藏 | bookmark | ブックマーク | 북마크 | signet | Lesezeichen | marcador | favorite, star |

## Fixed phrasings in outward-facing copy

These recur throughout both READMEs and are phrased identically every time:

| Chinese | English | Japanese | Korean | French | German | Spanish |
|---|---|---|---|---|---|---|
| 本地优先 | local-first | ローカルファースト | 로컬 우선 | local avant tout | local-first | local ante todo |
| 研读工作台（产品定位，一句话里说 vemry 是什么） | study workbench | 精読ワークベンチ | 공부 작업대 | atelier d'étude | Werkbank zum Studieren | mesa de estudio |
| 研读（把一篇论文、长文、书读通：导入、分章、复述、卡片） | study; the study loop | 精読; 精読ループ | 공부; 공부 루프 | étude ; la boucle d'étude | studieren; der Studierkreislauf | estudio; el ciclo de estudio |
| 零锁定 | no lock-in | ロックインなし | 종속 없음 | aucun enfermement | keine Anbieterbindung | sin ataduras |
| 离线优先 | offline-first | オフラインファースト | 오프라인 우선 | hors ligne avant tout | offline-first | sin conexión ante todo |
| 纯 Markdown | plain Markdown | プレーンな Markdown | 순수 마크다운 | Markdown brut | reines Markdown | Markdown plano |
| 网络出口 | network exit | ネットワーク出口 | 네트워크 출구 | sortie réseau | Netzwerkausgang | salida de red |
| 无遥测、无账号 | no telemetry, no account | テレメトリなし、アカウント不要 | 원격 측정 없음, 계정 불필요 | sans télémétrie, sans compte | keine Telemetrie, kein Konto | sin telemetría, sin cuenta |
| 改完再应用 (the permission gate's third option) | edit, then apply | 編集してから適用 | 편집 후 적용 | modifier, puis appliquer | bearbeiten, dann übernehmen | editar y aplicar |
| 操心更少，懂得更透，记得更久（三拍收益句：README「What vemry is」、官网信任段；对外只有这一种写法） | Fuss less. Understand more. Remember longer. | 手間は少なく。理解は深く。記憶は長く。 | 신경은 덜 쓰고. 이해는 더 깊게. 기억은 더 오래. | Moins de tracas. Mieux comprendre. Retenir plus longtemps. | Weniger verwalten. Mehr verstehen. Länger behalten. | Preocúpate menos. Entiende más. Recuerda por más tiempo. |
| 帮你提升，不替你思考（原则第 4 条的内部行；不进对外文案） | helps you get better, never thinks for you | 上達を助け、代わりに考えることはしない | 더 잘하도록 돕되, 대신 생각하지 않아요 | vous aide à progresser, ne pense jamais à votre place | hilft dir, besser zu werden – denkt nie für dich | te ayuda a mejorar, nunca piensa por ti |
