# Glossary

vemry's interface, both READMEs and both copy catalogs (`locales/en.json`,
`locales/zh-CN.json`) **share this one table**.

Why it exists: when the same thing is called "Projection" in the interface, "View"
in the documentation and 「投影视图」 in Chinese, nobody can say which is right
three months later, and each translation drifts on its own. A new user-visible term
**enters this table before it enters a catalog**.

The interface languages beyond English and Chinese (Japanese, Korean, French,
German, Spanish) are overlays in `locales/src/`; each gets a column here as its translation
starts, and `locales/src/README.md` fixes the register per language (です・ます,
하세요체, du, vous, tú). A concept word is decided here, never by the translator.

## Core concepts

| Chinese | English | Do not use |
|---|---|---|
| 知识库 | vault | repository, workspace, library |
| 块 / 块索引 | block / block index | database, record |
| 块 ID | block id | anchor, uid, block reference id |
| 引用（指向块或小节） | reference | citation (taken by evidence), pointer |
| 嵌入 | embed | transclusion, include, import |
| 小节 | section | chapter, part, heading (that is the line itself) |
| 字段 | field | property, attribute, metadata |
| 状态字段 | state field | status field (status is a key's name, not the concept), enum, lifecycle |
| 档（状态字段的一个取值） | step | stage, phase, level |
| 标签片（字段、标签、日期在行上的小胶囊） | chip | badge, pill, tag (taken by #tag), capsule |
| 投影视图 | projection | dashboard, module, widget |
| 断言 | claim | assertion, statement |
| 证据 | evidence | citation, source |
| 权限门 | permission gate | approval flow, confirmation |
| 隐私区 | privacy zone | private folder, locked folder, vault (already taken) |
| 私密块 | private block | hidden block, redacted block |
| 记忆文件（VEMRY.md） | vault memory | config, rules, instructions |
| 技能 | skill | plugin, extension (`extension` is reserved for the future extension mechanism) |
| 用户脚本 | user script | macro, plugin |
| 审计日志 | audit log | history, activity |
| 出处 | provenance | origin (the frontmatter key only), source (taken by clipping), authorship |
| 未审阅 | unreviewed | pending, awaiting review, draft (the code keeps `pending` as the ledger state's name) |
| 撤回 | revert | reject, roll back, undo (undo is the editor's ⌘Z) |
| 块引用 | block citation | block reference, block link |
| 位置 id | positional id | temporary id, transient id |

## The hosted account

| Chinese | English | Do not use |
|---|---|---|
| vemry 托管 | vemry hosted | cloud, managed, official API |
| 自带 key | bring your own key | custom endpoint (that is the setting's internal name), own API |
| 账户 | account | membership, profile |
| 算力包 | credit pack | top-up, balance (the pack has a balance; it is not one), tokens |
| 配额 | quota | allowance, limit (that is a cap) |
| 设备令牌 | device token | API key (it doubles as one, but the user never sees it as such), session |

## Interface elements

| Chinese | English | Do not use |
|---|---|---|
| 命令面板 | command palette | quick switcher, launcher |
| 活动栏 | activity bar | sidebar (that means the side panels), toolbar |
| 侧栏 | sidebar | panel |
| 大纲 | outline | table of contents |
| 状态栏 | status bar | statusline (an internal codename; never outward-facing) |
| 标签页 | tab | document tab |
| 速记 | quick capture | memo, scratch, inbox |
| 研究面板 | research panel | chat, assistant |
| 阅读投影 | Reader (interface) / reading projection (architecture prose) | reader mode, article view |
| 对照翻译 | side-by-side translation (the view is "Translation") | bilingual mode, immersive translation |
| 译文 / 原文 | translation / original | target text, source text |
| 块把手 | block handle | drag handle, gripper, grabber |
| 转换为 | turn into | convert, change type |
| 块选择 | block selection | multi-select, block mode |
| 折叠 | fold | collapse |

The interface uses the word that reads best (**Reader**); architecture prose uses the concept word (**reading projection**). That is a division of labour, not an inconsistency.

## Tasks

| Chinese | English | Do not use |
|---|---|---|
| 截止日期 | due date | deadline (in the interface), end date |
| 排期日期 | scheduled date | start date (that is ⏩), planned date |
| 开始日期 | start date | begin date, available from |
| 创建日期 | created | added on |
| 完成日期 / 取消日期 | completed on / cancelled on | done date, closed date |
| 已取消 | cancelled | abandoned, dropped, discarded |
| 优先级 | priority | importance, urgency (that is one step of it) |
| 紧急 / 重要 / 以后 / 有空再说 | urgent / important / later / someday | highest / high / low / lowest (the `priority::` values, not the copy) |
| 重复 | repeat | recurrence, recurring |
| 完成即弃 | removed once done | delete on completion, auto-delete |
| 编号（任务的） | id | task id, handle (taken by the block handle) |
| 受阻 | blocked | waiting, locked, dependent |
| 估时 | estimate | effort, duration, time box |

## The seven projections

| Chinese | English |
|---|---|
| 日程 | Agenda |
| 看板 | Board |
| 时间线 | Timeline |
| 图谱 | Graph |
| 断言 | Claims |
| 证据 | Evidence |
| 会话回放 | Session Replay |

## Clipping and browsing

| Chinese | English | Do not use |
|---|---|---|
| 剪藏 (verb and noun) | clip | save, bookmark, capture (taken by quick capture) |
| 剪藏工具条 | clip bar | toolbar |
| 落点笔记 | clip target | destination |
| 收藏 | bookmark | favorite, star |

## Fixed phrasings in outward-facing copy

These recur throughout both READMEs and are phrased identically every time:

| Chinese | English |
|---|---|
| 本地优先 | local-first |
| 研读工作台（产品定位，一句话里说 vemry 是什么） | study workbench |
| 研读（把一篇论文、长文、书读通：导入、分章、复述、卡片） | study; the study loop |
| 零锁定 | no lock-in |
| 离线优先 | offline-first |
| 纯 Markdown | plain Markdown |
| 网络出口 | network exit |
| 无遥测、无账号 | no telemetry, no account |
| 改完再应用 (the permission gate's third option) | edit, then apply |
| 操心更少，懂得更透，记得更久（三拍收益句：README「What vemry is」、官网信任段；对外只有这一种写法） | Fuss less. Understand more. Remember longer. |
| 帮你提升，不替你思考（原则第 4 条的内部行；不进对外文案） | helps you get better, never thinks for you |
