# Glossary

vemry's interface, both READMEs and both copy catalogs (`locales/en.json`,
`locales/zh-CN.json`) **share this one table**.

Why it exists: when the same thing is called "Projection" in the interface, "View"
in the documentation and 「投影视图」 in Chinese, nobody can say which is right
three months later, and each translation drifts on its own. A new user-visible term
**enters this table before it enters a catalog**.

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
| 待确认 | pending review | unreviewed, draft |
| 块引用 | block citation | block reference, block link |
| 位置 id | positional id | temporary id, transient id |

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
| 块把手 | block handle | drag handle, gripper, grabber |
| 转换为 | turn into | convert, change type |
| 块选择 | block selection | multi-select, block mode |
| 折叠 | fold | collapse |

The interface uses the word that reads best (**Reader**); architecture prose uses the concept word (**reading projection**). That is a division of labour, not an inconsistency.

## The eight projections

| Chinese | English |
|---|---|
| 日程 | Agenda |
| 看板 | Board |
| 时间线 | Timeline |
| 块 | Blocks |
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
| 零锁定 | no lock-in |
| 离线优先 | offline-first |
| 纯 Markdown | plain Markdown |
| 网络出口 | network exit |
| 无遥测、无账号 | no telemetry, no account |
| 改完再应用 (the permission gate's third option) | edit, then apply |
