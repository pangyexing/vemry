#!/usr/bin/env python3
"""Build locales/en.json and locales/zh-CN.json from one table so keys cannot drift.

Run: python3 scripts/gen-locales.py

Each entry is `key -> (english, chinese)`, so a new message physically cannot land
in one catalog and miss the other. Wording follows docs/GLOSSARY.md — the two
READMEs, the two catalogs and the UI must call the same thing the same name.

The generated JSON is committed: `src/lib/i18n/i18n.svelte.ts` and
`src-tauri/src/i18n.rs` both read it at build time, and `pnpm check` /
`cargo test -p vemry-app i18n` fail on a missing key.
"""
import json
import os
import re

M = {}


def add(prefix, table):
    for k, v in table.items():
        M[f"{prefix}{k}"] = v


# ---------------------------------------------------------------- common
add("common.", {
    "close": ("Close", "关闭"),
    "cancel": ("Cancel", "取消"),
    "back": ("Back", "后退"),
    "forward": ("Forward", "前进"),
    "clearFilter": ("Clear filter", "清除过滤"),
    "ok": ("OK", "确定"),
})

# ---------------------------------------------------------------- command groups
add("group.", {
    "nav": ("Navigate", "导航"),
    "note": ("Notes", "笔记"),
    "clip": ("Clip", "剪藏"),
    "view": ("Views", "视图"),
    "research": ("Research", "研究"),
    "vault": ("Vaults", "知识库"),
    "script": ("Scripts", "脚本"),
    "system": ("System", "系统"),
})

# ---------------------------------------------------------------- projection views
add("view.", {
    "agenda": ("Agenda", "Agenda(任务日程)"),
    "kanban": ("Board", "看板"),
    "timeline": ("Timeline", "时间线"),
    "graph": ("Graph", "图谱"),
    "claims": ("Claims", "断言"),
    "evidence": ("Evidence", "证据"),
    "sessions": ("Session Replay", "会话回放"),
    "review": ("Review", "复习"),
    "reading": ("Reader", "阅读投影"),
    "translate": ("Translation", "对照翻译"),
})

# ---------------------------------------------------------------- commands
CMD = {
    "open-vault": (
        ("Open vault…", "打开知识库…"),
        ("Pick a folder to use as a vault", "选择一个文件夹作为 vault"),
    ),
    "create-vault": (
        ("New vault…", "新建知识库…"),
        ("Pick a location for a new empty vault with a VEMRY.md",
         "选择位置,新建一个带 VEMRY.md 的空库"),
    ),
    "switch-vault": (
        ("Switch vault…", "切换知识库…"),
        ("Switch between recently opened vaults", "在最近打开的仓库间切换"),
    ),
    "new-note": (("New note", "新建笔记"), None),
    "open-note": (("Open note…", "打开笔记…"), None),
    "search": (("Search all notes", "全文搜索"), None),
    "command-palette": (("Command palette", "命令面板"), None),
    "quick-capture": (
        ("Quick capture", "速记"),
        ("One thought — Enter files it in inbox.md, {key} picks a target note",
         "一条想法,Enter 落进 inbox.md,{key} 选目标笔记"),
    ),
    "nav-back": (
        ("Back", "后退"),
        ("Return to the previous document; in Reader, to the previous page",
         "回到上一个文档;阅读投影里回上一页/来处"),
    ),
    "nav-forward": (("Forward", "前进"), None),
    "undo-file-op": (
        ("Undo file operation", "撤销文件操作"),
        ("Undo the last move, rename, create or import ({key} while the file tree has focus)",
         "撤销上一次移动/重命名/新建/导入(文件树聚焦时 {key})"),
    ),
    "toggle-left": (("Toggle file tree", "切换文件树"), None),
    "toggle-right": (("Toggle sidebar", "切换侧栏"), None),
    "toggle-sidebar-side": (
        ("Swap sidebar side", "侧栏换边"),
        ("Move the research and outline panels to the other side, swapping them with the "
         "file tree; drag a panel edge to resize, double-click to reset",
         "研究/链接面板移到另一侧,与文件树互换位置;拖动面板边缘可调宽,双击恢复默认"),
    ),
    "show-agent": (
        ("Research panel", "研究面板"),
        ("AI chat and browser controls in one sidebar", "AI 对话 + 浏览控制的统一侧栏"),
    ),
    "focus-agent": (
        ("Summon AI chat", "唤起 AI 对话"),
        ("Put the cursor in the AI input — focuses the panel when it is visible; press "
         "{key} again while focused to pop out a separate chat window",
         "光标落进 AI 输入框——面板可见时聚焦面板,已聚焦时再按 {key} 弹出独立的 AI 对话窗口"),
    ),
    "agent-follow-current": (
        ("AI follows the current note", "让 AI 跟随当前笔记"),
        ("The AI keeps this session's findings in the open note; appends and frontmatter changes on it apply without a dialog",
         "本会话的结论写进当前打开的笔记;对它的追加与 frontmatter 修改不再逐次确认"),
    ),
    "explain-selection": (
        ("Explain the selection", "解释选中内容"),
        ("Send the selected passage to the AI as a question — the passage travels as a selection reference",
         "把选中的段落发给 AI 解释;选区作为引用一起发出"),
    ),
    "open-journal-today": (
        ("Open today's journal", "打开今日日志"),
        ("journal/YYYY-MM-DD.md — created when missing; where the daily start and close land",
         "journal/YYYY-MM-DD.md,缺则新建;今日启动与收尾的落点"),
    ),
    "format-note": (
        ("Format note", "格式化笔记"),
        ("Run the Markdown formatter over the whole note: blank lines, list markers, tables, frontmatter order (undo with {key})",
         "整篇过一遍格式化器:空行、列表标记、表格、frontmatter 键序({key} 可撤销)"),
    ),
    "paste-plain-text": (
        ("Paste as plain text", "粘贴为纯文本"),
        ("Insert the clipboard's text as it is, without the rich-text conversion", "剪贴板文字原样插入,不转 Markdown"),
    ),
    "agent-attach-files": (
        ("Attach files to AI context", "附加文件到 AI 上下文"),
        ("Import images, PDFs or text files into the vault as context for your next "
         "message (images need a multimodal model)",
         "选图片/PDF/文本文件导入 vault,作为下一条消息的上下文(图片需多模态模型)"),
    ),
    "show-outline": (
        ("Outline", "大纲面板"),
        ("Headings and link relations of the current note", "当前笔记的标题大纲与链接关系"),
    ),
    "open-url": (
        ("Open URL…", "打开网址…"),
        ("Open in the built-in browser", "在内置浏览器中打开"),
    ),
    "open-browser": (
        ("Open browser", "打开浏览器"),
        ("Bring the browser out, or reopen the last page", "唤出浏览器,或重开上次浏览的页面"),
    ),
    "browser-new-tab": (
        ("New browser tab", "新标签页"),
        ("Open a new tab in the built-in browser", "在内置浏览器里新开一个标签页"),
    ),
    "browser-close-tab": (
        ("Close browser tab", "关闭浏览标签页"),
        ("Close the browser's current tab", "关闭内置浏览器的当前标签页"),
    ),
    "browser-reload": (
        ("Reload page", "刷新页面"),
        ("Reload the browser's current tab", "刷新内置浏览器的当前标签页"),
    ),
    "browser-toggle-dock": (
        ("Detach / dock the browser", "分离 / 贴回浏览器"),
        ("Move the browser between the main window's panel and its own window", "浏览器在主窗口面板与独立窗口之间切换"),
    ),
    "clear-browser-history": (
        ("Clear browsing history", "清除浏览历史"),
        ("Empty the built-in browser's history (bookmarks are untouched)",
         "清空内置浏览器的历史记录(收藏不受影响)"),
    ),
    "bookmark-toggle": (
        ("Bookmark current page", "收藏/取消收藏当前页面"),
        ("Save the browser's current page to bookmarks (reachable from the {key} list)",
         "内置浏览器当前页存入收藏({key} 列表可达)"),
    ),
    "clip-selection": (
        ("Clip selection → current or target note", "剪藏:选中文字 → 当前/目标笔记"),
        ("Turn the browser selection into a quote block: straight into a pinned target "
         "note, otherwise at the cursor, or into clips/ when no note editor is open",
         "浏览器选区变引用块;钉了目标笔记直达,否则插入光标处、无笔记编辑器时落 clips/"),
    ),
    "clip-selection-to-clips": (
        ("Clip selection → clips/", "剪藏:选中文字 → clips/"),
        ("Save the browser selection as its own excerpt note", "浏览器选区存为独立摘录笔记"),
    ),
    "clip-selection-to-note": (
        ("Clip selection → pick a note…", "剪藏:选中文字 → 指定笔记…"),
        ("Turn the browser selection into a quote block and append it to a note you pick "
         "(type a new name to create one)",
         "浏览器选区变引用块,选一篇笔记追加(输入新名字可创建)"),
    ),
    "clip-page": (
        ("Clip page → clips/", "剪藏:整页 → clips/"),
        ("Extract the article, convert it to Markdown, save it in clips/ (the domain goes "
         "in the frontmatter)",
         "Readability 抽正文转 markdown,落 clips/(域名记在 frontmatter)"),
    ),
    "clip-link": (
        ("Clip link → current note", "剪藏:链接 → 当前笔记"),
        ("Insert the current page as a [title](url) reference; goes to inbox when no note "
         "editor is visible",
         "当前页作为 [标题](url) 参考条目插入;无可见笔记编辑器时进 inbox"),
    ),
    "reload-scripts": (
        ("Reload user scripts", "重新加载用户脚本"),
        ("Rescan .vemry/scripts/ and reload all of them", "重新扫描 .vemry/scripts/ 并整体重载"),
    ),
    "init-memory": (
        ("Initialize VEMRY.md", "初始化 VEMRY.md"),
        ("The agent surveys your vault and drafts the memory file (reviews an existing "
         "one); the write comes back as a diff to approve",
         "agent 调研 vault 后起草记忆文件(已存在则审阅改进);写入出 diff 待确认"),
    ),
    "open-memory": (
        ("Open VEMRY.md", "打开 VEMRY.md"),
        ("Edit the agent's vault-level instructions", "编辑 agent 的 vault 级指令"),
    ),
    "block-type": (
        ("Block type…", "块类型…"),
        ("Open one of the vault's block types (.vemry/types/*.md), or type a new name to create it — book gets the full example",
         "打开知识库里的块类型（.vemry/types/*.md），或输入新名字创建——book 会得到完整示例"),
    ),
    "reveal-in-tree": (
        ("Reveal current note in file tree", "在文件树定位当前笔记"),
        ("Expand the left panel and scroll to the current note", "展开左栏并滚到当前笔记"),
    ),
    "copy-block-link": (
        ("Copy block link", "复制块链接"),
        ("Copy [[Note#^id]] for the block at the cursor, minting the id if the block has none", "复制光标所在块的 [[笔记#^id]]，块没有 id 时自动生成"),
    ),
    "copy-block-embed": (
        ("Copy block embed", "复制块嵌入"),
        ("Copy ![[Note#^id]] for the block at the cursor", "复制光标所在块的 ![[笔记#^id]]"),
    ),
    "block-move-up": (
        ("Move block up", "上移块"),
        ("Move the selected blocks, or the block at the cursor, above the previous one", "把选中的块（或光标所在块）移到上一个块之前"),
    ),
    "block-move-down": (
        ("Move block down", "下移块"),
        ("Move the selected blocks, or the block at the cursor, below the next one", "把选中的块（或光标所在块）移到下一个块之后"),
    ),
    "block-duplicate": (
        ("Duplicate block", "复制块"),
        ("Insert a copy of the selected blocks right after them", "在选中的块之后插入一份副本"),
    ),
    "block-delete": (
        ("Delete block", "删除块"),
        ("Delete the selected blocks, or the block at the cursor (undo with ⌘Z)", "删除选中的块（或光标所在块），可用 ⌘Z 撤销"),
    ),
    "toggle-provenance": (
        ("Show block origins", "显示出处"),
        ("Tint every block by who wrote it: you, the AI, a clip or a capture", "按写入者给每个块着色：你、AI、剪藏、速记"),
    ),
    "provenance-accept": (
        ("Mark AI change as reviewed", "AI 修改标为已审阅"),
        ("Mark the AI change at the cursor as reviewed and go on to the next", "把光标所在的 AI 修改标为已审阅，并前进到下一处"),
    ),
    "provenance-accept-all": (
        ("Mark all AI blocks in this note as reviewed", "本篇 AI 块全部标为已审阅"),
        ("Mark every unreviewed AI block of the current note as reviewed", "把当前笔记所有未审阅的 AI 块标为已审阅"),
    ),
    "clip-localize-images": (
        ("Save this clip's remote images into the vault", "把这篇剪藏的远程图片存进库"),
        ("Download the images a clipped note still loads from the web into clips/assets and point the note at them — for clips made before localisation, or whose images did not load", "把剪藏笔记里仍从网上加载的图片下载到 clips/assets 并改写引用——适合本地化之前的旧剪藏，或图片加载失败的情况"),
    ),
    "provenance-accept-vault": (
        ("Mark all AI blocks in the vault as reviewed", "全库 AI 块全部标为已审阅"),
        ("Mark every unreviewed AI block in every note as reviewed — after an agent run that wrote many notes", "把每篇笔记里所有未审阅的 AI 块标为已审阅——适合 agent 一次写了很多篇之后"),
    ),
    "provenance-reject": (
        ("Revert AI change", "撤回 AI 修改"),
        ("Put the AI change at the cursor back to what the write replaced and go on to the next", "把光标所在的 AI 修改恢复成写入前的内容，并前进到下一处"),
    ),
    "provenance-reject-all": (
        ("Revert all unreviewed AI changes in this note", "撤回本篇全部未审阅的 AI 修改"),
        ("Put every unreviewed AI block of the current note back to what the writes replaced", "把当前笔记所有未审阅的 AI 块恢复成写入前的内容"),
    ),
    "provenance-next-pending": (
        ("Next unreviewed AI change", "下一处未审阅的 AI 修改"),
        ("Jump to the next AI change you have not looked at — past the end of this note, to the next note that has one", "跳到下一处还没看过的 AI 修改——本篇看完则跳到下一篇有的笔记"),
    ),
    "provenance-prev-pending": (
        ("Previous unreviewed AI change", "上一处未审阅的 AI 修改"),
        ("Jump back to the previous AI change you have not looked at, into the previous note past this one's start", "跳回上一处还没看过的 AI 修改——本篇开头之前则跳到上一篇"),
    ),
    "provenance-review-start": (
        ("Review the AI changes", "开始审阅 AI 修改"),
        ("Open the first note with unreviewed AI blocks and land on its first change", "打开第一篇有未审阅 AI 块的笔记并落到第一处修改"),
    ),
    "note-history": (
        ("Note history", "笔记历史"),
        ("The text of this note before each write that replaced it — restore any of them",
         "这篇笔记每次被改写前的文本——可恢复到任一版本"),
    ),
    "delete-current-note": (
        ("Delete current note", "删除当前笔记"),
        ("Move to the system trash (recoverable)", "移入系统废纸篓(可恢复)"),
    ),
    "mark-private": (
        ("Mark current file private", "标记当前文件为隐私"),
        ("The AI never reads it, writes it, or sees it in a prompt — it does not exist to "
         "the model",
         "AI 永远不读、不写、不进 prompt — 对模型而言不存在"),
    ),
    "unmark-private": (
        ("Unmark current file as private", "取消当前文件的隐私标记"),
        ("Visible to the AI again", "重新对 AI 可见"),
    ),
    "view-agenda": (
        ("View: Agenda", "视图:Agenda(任务日程)"),
        ("Every task in the vault, projected by due date", "全库任务按到期时间投影"),
    ),
    "view-kanban": (
        ("View: Board", "视图:看板"),
        ("Every task in overdue / today / scheduled / unscheduled / done columns; drag a "
         "card to reschedule",
         "全库任务按过期/今天/已排期/未排期/已完成分列,拖卡片改期"),
    ),
    "view-timeline": (
        ("View: Timeline", "视图:时间线"),
        ("Notes as a river, by created or modified time", "笔记按创建/修改时间排成河流"),
    ),
    "view-claims": (
        ("View: Claims", "视图:断言"),
        ("Every claim in the vault and the evidence supporting it", "全库 claim 与支撑它的证据"),
    ),
    "view-evidence": (
        ("View: Evidence", "视图:证据"),
        ("Every piece of evidence and what it supports", "全库 evidence 及其支撑关系"),
    ),
    "view-graph": (
        ("View: Graph", "视图:图谱"),
        ("A force-directed graph of the links between notes", "笔记链接关系的力导向图"),
    ),
    "view-sessions": (
        ("View: Session replay", "视图:会话回放"),
        ("Replay an AI session step by step — reads, writes, permission decisions and "
         "privacy filtering, all on the record",
         "逐条回放 AI 会话的读写、权限决定与隐私过滤——都有账可查"),
    ),
    "view-review": (
        ("View: Review", "视图:复习"),
        ("The flashcards due today, one at a time; a grade is written back onto the card's line",
         "今天到期的卡片逐张复习;评分写回卡片所在行"),
    ),
    "view-reading": (
        ("View: Reader", "视图:阅读投影"),
        ("Return to the most recently opened web page", "回到最近打开的网页阅读投影"),
    ),
    "translate-browser-page": (
        ("Translate this web page side by side", "分屏翻译这个网页"),
        ("A translation pane opens beside the browser's current page, each paragraph aligned with its source; run again to close it", "在浏览器当前页旁开出译文栏,逐段与原文对齐;再执行一次关闭"),
    ),
    "translate-toggle": (
        ("Translate side by side", "对照翻译"),
        ("Read the open note beside its translation, one row per block; press again to leave ({key})",
         "把当前笔记与译文按段并排对照阅读;再按一次离开 ({key})"),
    ),
    "close-tab": (
        ("Close tab", "关闭标签页"),
        ("Close the current tab — a document or a projection ({key})",
         "关闭当前标签页——文档或投影视图({key})"),
    ),
    "next-tab": (
        ("Next tab", "下一个标签页"),
        ("Cycle forward ({key})", "顺序循环切换({key})"),
    ),
    "prev-tab": (
        ("Previous tab", "上一个标签页"),
        ("Cycle back ({key})", "顺序循环切换({key})"),
    ),
    "migrate-attachments": (
        ("Migrate attachment folders (bucket by full note path)", "迁移附件目录(按笔记完整路径分桶)"),
        ("attachments/<note name> → attachments/<note path>, rewriting links across the "
         "vault; mixed or orphaned buckets stay where they are",
         "attachments/<笔记名> → attachments/<笔记路径>,自动改写全库链接;混桶/孤儿原地保留"),
    ),
    "settings": (
        ("Settings", "设置"),
        ("Model / API key / endpoint", "模型 / API key / 端点"),
    ),
    "settings-ai": (
        ("AI settings", "AI 设置"),
        ("Model / API key / endpoint / thinking depth", "模型 / API key / 端点 / 思考深度"),
    ),
    "check-updates": (
        ("Check for updates", "检查更新"),
        ("Ask GitHub Releases for a newer version and offer to install it",
         "向 GitHub Releases 查询新版本,有则提示安装"),
    ),
    "clear-agent": (
        ("New AI session", "新建 AI 会话"),
        ("Archive the current conversation and start with a clean context",
         "当前对话归档到会话列表,开一个干净的上下文"),
    ),
}
for cid, (title, desc) in CMD.items():
    M[f"cmd.{cid}.title"] = title
    if desc:
        M[f"cmd.{cid}.desc"] = desc

# ---------------------------------------------------------------- activity bar
add("activityBar.", {
    "find": ("Search notes · open a web address", "搜索笔记 · 打开网址"),
    "label": ("Activity bar", "活动栏"),
    "fileTree": ("File tree", "文件树"),
    "openFileTree": ("Show file tree", "打开文件树"),
    "collapseFileTree": ("Hide file tree", "收起文件树"),
    "clickAgainToClose": ("Click again to close", "再点关闭"),
})

# ---------------------------------------------------------------- status bar
add("statusBar.", {
    "privateBlock": ("Private block", "私密块"),
    "privateBlockHint": ("Private block — invisible to the AI", "私密块——对 AI 不可见"),
    "privacyZone": ("Privacy zone", "隐私区"),
    "privacyZoneHint": ("Privacy zone — invisible to the AI", "隐私区 — 对 AI 不可见"),
    "cursor": ("Ln {line}, Col {col}", "行 {line}, 列 {col}"),
    "selectedChars": ("{n} selected", "已选 {n} 字符"),
    "selectedLines_one": ("{n} line", "{n} 行"),
    "selectedLines_other": ("{n} lines", "{n} 行"),
    "words_one": ("{n} word", "{n} 字"),
    "words_other": ("{n} words", "{n} 字"),
    "translate": ("Translate", "译"),
    "translateHint": ("This note is not in your language — read it side by side with a translation ({key})",
                      "这篇不是你的语言——按段对照译文阅读 ({key})"),
})

# ---------------------------------------------------------------- tab bar
add("tabBar.", {
    "close": ("Close", "关闭"),
    "closeOthers": ("Close others", "关闭其他"),
    "closeToRight": ("Close to the right", "关闭右侧"),
    "revealInTree": ("Reveal in file tree", "在文件树定位"),
    "copyRelativePath": ("Copy relative path", "复制相对路径"),
    "closeNamed": ("Close {name}", "关闭 {name}"),
    "unsaved": ("Unsaved ({key} to save)", "未保存({key} 保存)"),
    "viewTab": ("View: {name}", "视图:{name}"),
    "backWithKey": ("Back ({key})", "后退 ({key})"),
    "forwardWithKey": ("Forward ({key})", "前进 ({key})"),
    "listTabs": ("All open tabs ({n})", "全部打开的标签页（{n}）"),
    "closeAll": ("Close all", "关闭全部"),
})

# ---------------------------------------------------------------- command palette
add("palette.", {
    "findSpaces": ("Where to look", "找什么"),
    "modeNotes": ("Notes", "笔记"),
    "modeWeb": ("Web address", "网址"),
    "commands": ("Run a command…", "执行命令…"),
    "notes": ("Open a note…", "打开笔记…"),
    "pickNote": ("Pick a target note… (type a new name to create it; Tab or # for a section)",
                 "选择目标笔记…（输入新名字可创建;Tab 或 # 选小节）"),
    "search": ("Search all notes…", "全文搜索…"),
    "capture": ("Capture a thought — Enter files it in inbox.md, {key} picks a target note",
                "速记一条，Enter 存入 inbox.md，{key} 选目标笔记"),
    "vaults": ("Switch vault…", "切换知识库…"),
    "url": ("Type a URL, or search bookmarks and history… ({key} for a new window)",
            "输入网址，或搜索收藏与历史…（{key} 新窗口）"),
    "pickHeading": ("Pick a section of “{title}”… (Enter to confirm, Esc to go back)",
                    "选择「{title}」的小节…（Enter 确认,Esc 返回）"),
    "wholeNote": ("End of note", "整篇末尾"),
    "types": ("Open a block type… (type a new name to create it)", "打开块类型…（输入新名字可创建）"),
    "createType": ("Create block type “{name}”", "创建块类型「{name}」"),
    "createTypeHint": ("A skeleton with status and due fields, opened for editing", "按骨架（status/due 字段）新建并打开"),
    "typeExample": ("The example: a bullet with author, status and rating fields", "示例：带 author/status/rating 字段的列表项"),
    "typeDisabled": ("Disabled", "已停用"),
    "typeDisable": ("Disable", "停用"),
    "typeEnable": ("Enable", "启用"),
})

# ---------------------------------------------------------------- shared view chrome
add("viewChrome.", {
    "askAi": ("Ask AI", "问 AI"),
    "askAiHint": ("Hand what this view shows to the AI as context", "把这个视图此刻显示的内容交给 AI 作上下文"),
    "closeView": ("Close view (Esc)", "关闭视图 (Esc)"),
    "closeViewLabel": ("Close view", "关闭视图"),
    "countFiltered": ("{shown} of {total}", "{shown}/{total}"),
})

# ---------------------------------------------------------------- native menu (macOS)
add("menu.", {
    "file": ("File", "文件"),
    "edit": ("Edit", "编辑"),
    "view": ("View", "视图"),
    "research": ("Research", "研究"),
    "window": ("Window", "窗口"),
    "help": ("Help", "帮助"),
    "settings": ("Settings…", "设置…"),
    "checkUpdates": ("Check for Updates…", "检查更新…"),
    "quickCapture": ("Quick Capture…", "速记…"),
    "closeWindow": ("Close Tab / Window", "关闭标签页/窗口"),
    "helpGithub": ("vemry Help (GitHub)", "vemry 帮助(GitHub)"),
})

# ---------------------------------------------------------------- settings: language
add("settings.language.", {
    "label": ("Language", "语言"),
    "auto": ("Match system", "跟随系统"),
    "zh": ("简体中文", "简体中文"),
    "en": ("English", "English"),
    "hint": ("Applies immediately, including the menu bar.", "立即生效,菜单栏同步。"),
})

add("common.", {
    "delete": ("Delete", "删除"),
})

add("script.", {
    "fromScript": ("Script {name}", "脚本 {name}"),
})

add("palette.", {
    "bookmarks": ("Bookmarks", "收藏"),
    "recent": ("Recent", "最近"),
    "appendToSection": ("Append to the end of this section", "插到小节末尾"),
    "createNamed": ("Create “{title}”", "新建「{title}」"),
    "createAndPick": ("Create and select", "创建并选定"),
    "createAndOpen": ("Create and open", "创建并打开"),
    "currentVault": ("{path} · current", "{path} · 当前"),
    "openOtherVault": ("Open another vault…", "打开其他仓库…"),
    "pickFolder": ("Pick a folder", "选择文件夹"),
    "pickLocation": ("Pick a location", "选择位置"),
    "unstar": ("Remove bookmark", "取消收藏"),
    "backToBrowser": ("Back to the browser window", "回到浏览窗口"),
    "reopenLast": ("Reopen the last page", "重开上次页面"),
    "openUrl": ("Open {url}", "打开 {url}"),
    "builtInBrowser": ("Built-in browser", "内置浏览器"),
    "pickSection": ("Pick a section ›", "选小节 ›"),
    "pickHeadingLabel": ("Pick a section of “{title}”", "选「{title}」的小节"),
    "noMatch": ("No matches", "无匹配结果"),
    "noBookmarksOrHistory": ("No bookmarks or history yet — type a URL to start browsing",
                             "暂无收藏与历史，输入网址开始浏览"),
    "typeToFilter": ("Type to filter", "输入以过滤"),
    "clearHistoryHint": ("Clears the recent list; bookmarks are untouched", "最近列表清空,收藏不受影响"),
})

# ---------------------------------------------------------------- permission gate
add("permission.", {
    "title": ("Confirm write", "写入确认"),
    "newFile": ("(new file)", "（新文件）"),
    "summary.create": ("Create a new note", "创建新笔记"),
    "summary.overwrite": ("Overwrite", "覆盖写入"),
    "summary.edit": ("Edit", "编辑"),
    "summary.move": ("Move → {toPath}", "移动 → {toPath}"),
    "summary.append": ("Append", "追加"),
    "rewrites": ("Rewrites {links} links in {notes} notes", "改写 {notes} 篇里的 {links} 个链接"),
    "blockDiff": ("Blocks · moved {moved} · changed {changed} · added {added} · removed {removed}", "块 · 移动 {moved} · 修改 {changed} · 新增 {added} · 删除 {removed}"),
    "rawDiff": ("Raw diff", "原始 diff"),
    "blockKind.removed": ("Removed ({n})", "删除({n})"),
    "blockKind.changed": ("Changed ({n})", "修改({n})"),
    "blockKind.added": ("Added ({n})", "新增({n})"),
    "blockKind.moved": ("Moved ({n})", "移动({n})"),
    "deniedByUser": ("Denied by the user", "用户拒绝"),
    "denyReasonPlaceholder": ("Tell the agent why (optional)", "告诉 agent 为什么拒绝（可选）"),
    "confirmDeny": ("Confirm deny", "确认拒绝"),
    "viewDiff": ("View diff", "查看 diff"),
    "editThenApply": ("Edit, then apply", "编辑后应用"),
    "deny": ("Deny", "拒绝"),
    "denyHint": ("Esc denies too", "Esc 亦可拒绝"),
    "alwaysAllowVault": ("Always allow in this vault", "此库总是允许"),
    "alwaysAllowVaultHint": (
        "Write without review in this vault (stored in .vemry/config.json; revocable in Settings)",
        "本库永久免审写入(写进 .vemry/config.json;设置页可随时撤销)"),
    "allowSession": ("Allow for this session", "本会话均允许"),
    "allowSessionHint": ("Stop asking for the rest of this session (a new session reviews each write again)",
                         "本次会话内不再询问(新会话恢复逐条审批)"),
    "scopeHint": ("Allow for this session, only within a folder or this note", "本会话内只放行某个文件夹或这一篇"),
    "scopeTitle": ("For this session, stop asking within…", "本会话内不再询问的范围…"),
    "scopeFolder": ("Only {folder}", "仅 {folder}"),
    "scopeNote": ("Only {name}", "仅这篇:{name}"),
    "allow": ("Allow", "允许"),
    "allowHint": ("Enter works too", "Enter 亦可"),
})

# ---------------------------------------------------------------- script grant
add("scriptGrant.", {
    "title": ("Script write permission", "脚本写入授权"),
    "heading": ("A script is asking to write", "脚本请求写入权限"),
    "bodyBefore": ("Script ", "脚本 "),
    "bodyBetween": (" wants to write to ", " 想写入 "),
    "bodyAfter": (".", "。"),
    "noteBefore": ("Allowing it lets this script write in this vault without asking again. Every write is recorded in ",
                   "允许后该脚本在此知识库可直接写入（不再询问），所有写入记录在 "),
    "noteAfter": (". You can revoke this in Settings at any time.", "。可随时在设置里撤销。"),
    "allowWrite": ("Allow writes", "允许写入"),
    "promptTitle": ("Script input", "脚本输入"),
})

# ---------------------------------------------------------------- outline panel
add("outline.", {
    "openNote": ("Open note", "打开笔记"),
    "createAndOpenNote": ("Create and open this note", "创建并打开该笔记"),
    "emptyHint": ("Open a note to see its outline and links here", "打开一篇笔记后,这里展示大纲与链接关系"),
    "untitled": ("(untitled)", "(无标题)"),
    "noHeadings": ("This note has no headings", "本篇没有标题"),
    "backlinks": ("Backlinks", "反向链接"),
    "noBacklinks": ("No note links here yet", "还没有笔记链接到这里"),
    "outgoing": ("Outgoing links", "出链"),
    "notCreated": ("not created", "未创建"),
    "noLinks": ("This note has no [[links]] yet", "本篇还没有 [[链接]]"),
})

# ---------------------------------------------------------------- tasks (Agenda + Board)
add("task.", {
    "field.due": ("due date", "截止日期"),
    "field.scheduled": ("scheduled date", "排期日期"),
    "field.start": ("start date", "开始日期"),
    "field.created": ("created", "创建日期"),
    "field.done": ("completed on", "完成日期"),
    "field.cancelled": ("cancelled on", "取消日期"),
    # The four steps of a priority around the unwritten default
    "priority.highest": ("urgent", "紧急"),
    "priority.high": ("important", "重要"),
    "priority.low": ("later", "以后"),
    "priority.lowest": ("someday", "有空再说"),
    # The chips of the other attributes (the editor's tooltips, Agenda's chips)
    "chip.repeat": ("repeats {rule}", "重复 {rule}"),
    "chip.deleteOnDone": ("removed once done", "完成即弃"),
    "chip.id": ("id {id}", "编号 {id}"),
    "chip.blockedBy": ("waiting on {ids}", "等待 {ids}"),
    "chip.blocked": ("blocked", "受阻"),
    "chip.estimate": ("estimate {v}", "估时 {v}"),
    "menu.copyId": ("Copy task id", "复制任务编号"),
    "copiedId": ("Task id copied: {id}", "已复制任务编号 {id}"),
    "act.id": ("giving it an id", "分配编号"),
    "overdue_one": ("{n} day overdue", "逾期 {n} 天"),
    "overdue_other": ("{n} days overdue", "逾期 {n} 天"),
    "toggleDone": ("Toggle done", "切换完成状态"),
    "editDate": ("Change {field}", "修改{field}"),
    "editDateWith": ("Change {field} ({date})", "修改{field} {date}"),
    "setDate": ("Set {field}", "设定{field}"),
    "count_one": ("{n} task", "{n} 个任务"),
    "count_other": ("{n} tasks", "{n} 个任务"),
    "filterPlaceholder": ("Filter: task / note", "过滤:任务 / 笔记"),
    "onlyWith": ("Only tasks with a {field}, ordered by it", "只看有{field}的任务，并按它排序"),
    "onlyWithOff": ("Showing only tasks with a {field}, ordered by it — click to show all", "正在只看有{field}的任务并按它排序，再点一次显示全部"),
    "noneWithDate": ("No tasks have a {field}.", "没有带{field}的任务。"),
    "empty1": ("No tasks yet. Write ", "没有任务。用 "),
    "empty2": (" in a note to create one; ", " 语法在笔记里创建；"),
    "empty3": (" means in progress, and ", " 表示进行中，"),
    "empty4": (" is the day it is planned for.", " 是排期日期。"),
    "noMatch": ("No tasks match “{query}”.", "没有匹配「{query}」的任务。"),
    "dirtyGuard": ("This note has unsaved changes — press {key} to save before {action}",
                   "当前笔记有未保存的修改，请先 {key} 保存再{action}"),
    "act.toggle": ("checking it off", "勾选任务"),
    "act.editDate": ("changing the date", "修改日期"),
    "act.move": ("moving the task", "移动任务"),
    "act.state": ("changing its state", "修改状态"),
    "act.remove": ("removing the task", "移除任务"),
    # The context menu of a task: an Agenda row and a kanban card raise the same entries
    "menu.jumpToSource": ("Jump to the source line", "跳到原文"),
    "menu.markDoing": ("Mark in progress", "标记进行中"),
    "menu.markBackTodo": ("Mark back to to-do", "标回待办"),
    "menu.markDone": ("Mark done", "标记完成"),
    "menu.markCancelled": ("Mark cancelled", "标记取消"),
    "menu.reopen": ("Reopen", "重新打开"),
    "menu.remove": ("Remove task", "移除任务"),
    "menu.removeConfirm": ("Remove it? The line is deleted from the note.", "确认移除？将从笔记原文删除"),
})

add("agenda.", {
    "overdue": ("Overdue", "已过期"),
    "today": ("Today", "今天"),
    "week": ("Next 7 days", "未来 7 天"),
    "later": ("Later", "更晚"),
    "future": ("Not started yet", "未到开始日"),
    "estimate": ("{v} planned", "估时 {v}"),
    "doing": ("In progress", "进行中"),
    "nodate": ("No date", "无日期"),
    "done": ("Done", "已完成"),
})

add("kanban.", {
    "col.overdue": ("Overdue", "过期"),
    "col.today": ("Today", "今天"),
    "col.scheduled": ("Scheduled", "已排期"),
    "col.unscheduled": ("Unscheduled", "未排期"),
    "col.done": ("Done", "已完成"),
    "col.none": ("None", "无"),
    "groupBy": ("Group by", "分组"),
    "groupByState": ("State", "状态"),
    "sortBy": ("Sort by", "排序"),
    "sortAuto": ("Default", "默认"),
    "sortAutoHint": ("Default order: by due date; the unscheduled and done columns by when the "
                     "task was last edited, newest first",
                     "默认顺序：按截止日期；未排期与已完成两列按任务修改时间，最新在前"),
    "sortDate": ("Due date", "截止日期"),
    "sortModified": ("Task last edited", "任务修改时间"),
    "fieldsGroup": ("Fields", "字段"),
    "sortNote": ("Note", "笔记"),
    "sortText": ("Text", "文本"),
    "sortAsc": ("Ascending — click for descending", "升序，点击改为降序"),
    "sortDesc": ("Descending — click for ascending", "降序，点击改为升序"),
    "targetGone": ("Target note {path} no longer exists — pick another",
                   "目标笔记 {path} 已不存在,请重新选择"),
    "expand": ("Expand", "展开"),
    "collapse": ("Collapse", "折叠"),
    "dropHere": ("Drop here", "拖到此处"),
    "newTaskPlaceholder": ("New task, Enter to add", "新任务，回车提交"),
    "scheduleDate": ("Schedule date", "排期日期"),
    "pickTargetNote": ("Pick which note new tasks go into", "选择新任务落进哪篇笔记"),
    "searchNotes": ("Search notes…", "搜索笔记…"),
    "noMatchingNote": ("No matching note", "无匹配笔记"),
    "newTask": ("+ New task", "+ 新建任务"),
    "dropWhen": ("Schedule for which day", "排到哪天"),
    "showMore": ("Show {n} more", "还有 {n} 项，展开"),
})

add("timeline.", {
    "created": ("Created", "创建"),
    "modified": ("Modified", "修改"),
    "createdTime": ("Created at", "创建时间"),
    "modifiedTime": ("Modified at", "修改时间"),
    "today": ("Today", "今天"),
    "yesterday": ("Yesterday", "昨天"),
    "noteCount_one": ("{n} note", "{n} 篇笔记"),
    "noteCount_other": ("{n} notes", "{n} 篇笔记"),
    "sameMinuteCreated": ("created in the same minute", "同一分钟创建"),
    "sameMinuteModified": ("modified in the same minute", "同一分钟修改"),
    "filterPlaceholder": ("Filter: title / path / tag", "过滤:标题 / 路径 / 标签"),
    "noMatch": ("No notes match “{query}”.", "没有匹配「{query}」的笔记。"),
    "empty": ("No notes yet — the timeline lays out the whole vault by created and modified time.",
              "还没有笔记——时间线按创建/修改时间陈列全库笔记。"),
})

add("claims.", {
    "decision": ("decision", "决策"),
    "count_one": ("{n} claim", "{n} 断言"),
    "count_other": ("{n} claims", "{n} 断言"),
})

add("review.", {
    "count_one": ("{n} card", "{n} 张卡片"),
    "count_other": ("{n} cards", "{n} 张卡片"),
})

add("reviewView.", {
    "due": ("{n} due", "{n} 张到期"),
    "filterPlaceholder": ("Filter: question / note", "过滤:问题 / 笔记"),
    "empty1": ("No cards yet. Write ", "还没有卡片。在笔记里写 "),
    "empty2": (" in a note — the first line is the question, the quoted lines below it the answer — "
               "or ask the AI to make cards from a note.",
               " ——首行是问题,下面的引用行是答案——或者让 AI 从一篇笔记生成卡片。"),
    "noMatch": ("No cards match “{query}”.", "没有匹配「{query}」的卡片。"),
    "allDone": ("All caught up", "今天的都复习完了"),
    "nextDue": ("next card due {date}", "下一张 {date} 到期"),
    "studyAhead": ("Study ahead", "提前复习"),
    "aheadHint": ("studying ahead — not due yet", "提前复习——尚未到期"),
    "newCard": ("new", "新卡"),
    "showAnswer": ("Show answer", "看答案"),
    "noAnswer": ("(no answer written yet)", "(还没写答案)"),
    "grade.again": ("Again", "重来"),
    "grade.hard": ("Hard", "困难"),
    "grade.good": ("Good", "记得"),
    "grade.easy": ("Easy", "轻松"),
    "today": ("today", "今天"),
    "days": ("{n} d", "{n} 天"),
    "stale": ("The note changed underneath — reloaded, grade it again", "笔记刚被改动,已重新载入,请再评一次"),
})

add("evidence.", {
    "count_one": ("{n} item", "{n} 条"),
    "count_other": ("{n} items", "{n} 条"),
})

add("sessions.", {
    "count_one": ("{n} session", "{n} 个会话"),
    "count_other": ("{n} sessions", "{n} 个会话"),
})

# ---------------------------------------------------------------- claims / evidence / graph
add("claimsView.", {
    "unsupported": ("{n} unsupported", "{n} 未被支撑"),
    "filterPlaceholder": ("Filter: claim / note", "过滤:断言 / 笔记"),
    "empty1": ("No claims yet. Write ", "没有断言。用 "),
    "empty2": (" in a note, and link evidence with ", " 语法在笔记里创建，证据用 "),
    "empty3": (".", " 关联。"),
    "noMatch": ("No claims match “{query}”.", "没有匹配「{query}」的断言。"),
    "noEvidence": ("No supporting evidence yet", "尚无证据支撑"),
    "noId": (" · no ^id", " · 无 ^id"),
    "noIdHint": ("Give it a ^id so evidence can reference it with supports:[[note#^id]]",
                 "加上 ^id 后，证据才能用 supports:[[笔记#^id]] 引用它"),
})

add("evidenceView.", {
    "broken": ("{n} with broken links", "{n} 条有断链"),
    "filterPlaceholder": ("Filter: evidence / note", "过滤:证据 / 笔记"),
    "brokenOnly": ("Broken links only", "只看断链"),
    "empty1": ("No evidence yet. Write ", "没有证据。用 "),
    "empty2": (" in a note.", " 语法在笔记里创建。"),
    "noMatch": ("No evidence matches “{query}”.", "没有匹配「{query}」的证据。"),
    "noneBroken": ("No broken links — the filter hid all {n}. Turn it off to see everything.",
                   "没有断链的证据——「只看断链」筛掉了全部 {n} 条,关掉开关看全部。"),
    "unattached": ("Not attached to a claim", "未挂断言"),
    "unattachedHint": ("Use supports:[[note#^id]] to attach it to a claim",
                       "用 supports:[[笔记#^id]] 把它挂到一条断言上"),
    "noMatchingClaim": ("[[{target}]] has no matching claim", "[[{target}]] 无匹配断言"),
    "noMatchingClaimHint": ("The note exists but has no ^{id} claim", "目标笔记存在，但没有 ^{id} 断言"),
    "unresolved": ("[[{target}]] unresolved", "[[{target}]] 未解析"),
    "unresolvedHint": ("The target note does not exist", "目标笔记不存在"),
})

add("graph.", {
    "total": ("{nodes} nodes / {edges} edges", "{nodes} 个节点 / {edges} 条边"),
    "hint": ("Scroll to zoom · drag to pan · double-click to reset",
             "滚轮缩放 · 拖拽平移 · 双击复位"),
    "empty": ("No notes yet. Add a few [[links]] and come back.", "没有笔记。建几条 [[链接]] 后再来看图。"),
})

# ---------------------------------------------------------------- session replay
add("sessionsView.", {
    "viewWrittenBlocks": ("View the written blocks", "查看写入的块"),
    "appliedDiff": ("Applied with edits:", "编辑后应用的改动："),
    "rename": ("Rename…", "重命名…"),
    "delete": ("Delete session (with its audit log)", "删除会话(含审计记录)"),
    "deleteConfirm": ("Confirm delete", "确认删除"),
    "hint": ("Every read, write, permission decision and privacy filter is on the record",
             "每一次读写、权限决定与隐私过滤,都有账可查"),
    "filterPlaceholder": ("Filter: title / time", "过滤:标题 / 时间"),
    "titlePlaceholder": ("Session title", "会话标题"),
    "forked": ("Forked session", "分叉出的会话"),
    "turns": ("{n} turns", "{n} 轮"),
    "noMatch": ("No sessions match “{query}”.", "没有匹配「{query}」的会话。"),
    "empty": ("No sessions yet. Talk to the AI in the sidebar and the replay shows up here.",
              "还没有会话。与 AI 侧栏对话一次后,回放就会出现在这里。"),
    "calledTools": ("(called {n} tools)", "（调用了 {n} 个工具）"),
    "collapse": ("Collapse", "收起"),
    "expandResult": ("Expand result", "展开结果"),
    "privacyFiltered": ("Privacy filter: {n} results hidden from the model (tool: {tool})",
                        "隐私过滤：{n} 条结果对模型不可见（工具：{tool}）"),
    "webSearch": ("Web search", "联网搜索"),
    "webSearchResults": ("{n} results", "返回 {n} 条结果"),
    "markRetry": ("⟳ Regenerated — the previous output for this turn was discarded",
                  "⟳ 重新生成:上方本轮的旧输出已被放弃"),
    "markEditResend": ("✎ Edited and resent — the previous user message and its turn were discarded",
                       "✎ 编辑重发:上一条用户消息整轮被放弃"),
    "markCompact": ("▣ Context compacted ({level})", "▣ 上下文压缩({level})"),
    "planUpdate": ("Plan updated · {done}/{total}", "计划更新 · {done}/{total}"),
    "follow": ("Following {path}", "跟随 {path}"),
    "followEnd": ("Stopped following", "取消跟随"),
    "viaFollow": (" · working note", " · 跟随笔记"),
    "viaScope": (" · scoped grant", " · 范围放行"),
    "allowScope": ("Allowed for the session under {scope}", "会话内放行 {scope}"),
    "sourcesSettled": ("Sources this turn · {n}", "本轮来源 · {n}"),
    "actions": ("Action items proposed · {n}", "提出行动项 · {n}"),
    "actionsDecision.approve": ("Action items kept · {n}", "行动项已确认 · {n}"),
    "actionsDecision.reject": ("Action items discarded", "行动项已放弃"),
    "read": ("Pages proposed · {n}", "提出读取 · {n} 页"),
    "readDecision.pages": ("Pages read · {n}", "已读取 · {n} 页"),
    "readDecision.deny": ("Reading declined", "读取被拒绝"),
    "readAuto": (" · without asking", " · 未询问"),
    "readAutoOn": ("Reading without asking from here on in this session", "本会话从此不再询问,自动读取"),
    "readFailed": ("Could not read · {n}", "读取失败 · {n}"),
    "skill": ("Skill in effect: {name}", "按 {name} 方法处理"),
    "skillAttached": (" · attached by the user", " · 用户点名"),
    "skillRead": (" · read by the model", " · 模型自行读取"),
    "undo": ("↶ Undid this turn's writes · {n} reverted, {skipped} skipped", "↶ 撤销本轮写入 · 还原 {n},跳过 {skipped}"),
    "compactSummary": ("older conversation summarized", "旧对话摘要化"),
    "markEffortAnswer": ("Light pre-plan call answered without a plan — retaken at the session's thinking level", "计划前的轻推理调用直接作答——已按会话档位重取"),
    "markEffortWork": ("Light pre-plan call went to work without a plan — retaken at the session's thinking level", "计划前的轻推理调用未提计划就动手——已按会话档位重取"),
    "compactHandles": ("older tool results turned into handles", "旧工具结果句柄化"),
    "markFork": ("⑂ Forked from session {from} (before turn {turn})", "⑂ 分叉自会话 {from}(第 {turn} 轮之前)"),
    "nothingToShow": ("Nothing to replay in this session.", "此会话没有可显示的记录。"),
    "pickSession": ("Pick a session on the left to replay it.", "选择左侧的会话开始回放。"),
})

# ---------------------------------------------------------------- agent tool / decision labels
add("tool.", {
    "list_notes": ("List notes", "列出笔记"),
    "read_note": ("Read", "读取"),
    "search_notes": ("Search", "搜索"),
    "get_links": ("Links", "查看链接"),
    "query_blocks": ("Query blocks", "查询块"),
    "read_skill": ("Read skill", "读取 skill"),
    "write_note": ("Write", "写入"),
    "append_note": ("Append", "追加"),
    "set_frontmatter": ("Set frontmatter", "设置 frontmatter"),
    "edit_note": ("Edit", "编辑"),
    "move_note": ("Move", "移动"),
    "propose_plan": ("Propose a plan", "提出计划"),
    "update_plan": ("Update the plan", "更新计划"),
    "read_block": ("Read block", "读取块"),
    "clips_lookup": ("Look up clips", "查剪藏"),
    "list_sources": ("List sources", "列出来源"),
    "recent_changes": ("Recent changes", "最近变化"),
    "aggregate_blocks": ("Count blocks", "统计块"),
    "read_file": ("Read file", "读取文件"),
    "open_for_reading": ("Read pages", "读取网页"),
    "import_page": ("Import page", "导入网页"),
    "update_task": ("Update task", "更新任务"),
    "set_block_field": ("Set field", "设置字段"),
    "propose_actions": ("Propose actions", "提出行动项"),
    "move_file": ("Move file", "移动文件"),
    "list_files": ("List files", "列出文件"),
    "audit_links": ("Audit links", "检查链接"),
    "restructure_note": ("Restructure note", "重整笔记"),
    "move_blocks": ("Move blocks", "搬块"),
    "mixed": ("Tool calls", "工具调用"),
})

# The default title of a callout head written without one (callout.ts::calloutDefaultTitle);
# the keys are the recognised types, so `callout.<type>` resolves and an unknown type shows its own name.
add("callout.", {
    "note": ("Note", "备注"), "info": ("Info", "信息"), "todo": ("To do", "待办"),
    "abstract": ("Abstract", "摘要"), "summary": ("Summary", "摘要"), "tldr": ("TL;DR", "摘要"),
    "tip": ("Tip", "提示"), "hint": ("Hint", "提示"), "important": ("Important", "重要"),
    "success": ("Success", "成功"), "check": ("Check", "检查"), "done": ("Done", "完成"),
    "question": ("Question", "问题"), "help": ("Help", "帮助"), "faq": ("FAQ", "常见问题"),
    "warning": ("Warning", "警告"), "caution": ("Caution", "注意"), "attention": ("Attention", "注意"),
    "danger": ("Danger", "危险"), "error": ("Error", "错误"), "failure": ("Failure", "失败"),
    "fail": ("Fail", "失败"), "missing": ("Missing", "缺失"), "bug": ("Bug", "缺陷"),
    "example": ("Example", "示例"), "quote": ("Quote", "引用"), "cite": ("Cite", "引用"),
    "claim": ("Claim", "主张"), "decision": ("Decision", "决策"), "evidence": ("Evidence", "证据"),
    "private": ("Private", "私密"),
})

add("planDecision.", {
    "approve": ("Plan approved", "计划已确认"),
    "approve_edited": ("Plan approved with edits", "计划修改后确认"),
    "reject": ("Plan rejected", "计划被否决"),
})

add("decision.", {
    "allow": ("Allowed", "允许"),
    "allow_session": ("Allowed for the session", "会话内允许"),
    "allow_scope": ("Allowed for the session, scoped", "会话内范围放行"),
    "allow_always": ("Always allowed", "总是允许"),
    "allow_edited": ("Allowed after editing", "编辑后允许"),
    "deny": ("Denied", "拒绝"),
})

# ---------------------------------------------------------------- date menu
add("dateMenu.", {
    "today": ("Today", "今天"),
    "tomorrow": ("Tomorrow", "明天"),
    "nextWeek": ("Next week", "下周"),
    "pickDate": ("Pick a date", "选一个日期"),
    "removeDate": ("Remove date", "移除日期"),
})

# ---------------------------------------------------------------- file tree
add("fileTree.", {
    "newNote": ("New note", "新建笔记"),
    "newFile": ("New file", "新建文件"),
    "newFolder": ("New folder", "新建文件夹"),
    "pasteMove": ("Paste (move {name})", "粘贴（移动 {name}）"),
    "paste": ("Paste (file / screenshot)", "粘贴（文件/截图）"),
    "rename": ("Rename", "重命名"),
    "cut": ("Cut", "剪切"),
    "copyPath": ("Copy path", "复制路径"),
    "copyFullPath": ("Copy full path", "复制完整路径"),
    "markPrivate": ("Mark private", "标记为隐私"),
    "unmarkPrivate": ("Unmark private", "取消隐私标记"),
    "privacyInherited": ("Privacy zone — inherited from {zone}", "隐私区 — 继承自 {zone}"),
    "privacyCovered": ("In privacy zone · inherited from {zone}", "已在隐私区 · 继承自 {zone}"),
    "openWithSystemApp": ("Open with the system app", "用系统应用打开"),
    "followAi": ("Let the AI follow this note", "让 AI 跟随此笔记"),
    "delete": ("Delete (move to trash)", "删除（移入废纸篓）"),
    "copyFailed": ("Copy failed", "复制失败"),
    "noFolderIntoItself": ("A folder cannot be moved inside itself", "不能把文件夹移动到它自己内部"),
    "undoRename": ("Undo rename of “{name}”", "撤销重命名「{name}」"),
    "undoMove": ("Undo move of “{name}”", "撤销移动「{name}」"),
    "undoImport": ("Undo import of {n} files", "撤销导入 {n} 个文件"),
    "undoCreate": ("Undo creation of “{name}”", "撤销新建「{name}」"),
    "undoDelete": ("Undo deletion of “{name}”", "撤销删除「{name}」"),
    "promptNote": ("Title (.md by default; an explicit extension is kept)",
                   "标题(默认 .md,写扩展名则按原样)"),
    "promptFile": ("File name with extension, e.g. test.py", "文件名(含扩展名,如 test.py)"),
    "promptFolder": ("Folder name", "文件夹名"),
    "noteCount_one": ("{n} note", "{n} 篇"),
    "noteCount_other": ("{n} notes", "{n} 篇"),
    "removeFromRecent": ("Remove from the recent list", "从最近列表移除"),
    "memoryHint": ("VEMRY.md — vault conventions injected into the agent's system prompt",
                   "VEMRY.md — 注入 agent system prompt 的仓库约定"),
    "memoryMenuTitle": ("VEMRY.md · vault memory", "VEMRY.md · agent 仓库约定"),
    "emptyVault": ("Empty vault — {key} or right-click to create a note",
                   "空知识库 — {key} 或右键新建笔记"),
})

# ---------------------------------------------------------------- @ context picker
add("atPicker.", {
    "fromDisk": ("Add a file from your computer…", "从电脑添加文件…"),
    "fromDiskSub": ("Image / PDF / text", "图片 / PDF / 文本"),
    "selectionSub": ("Current selection", "当前选区"),
    "wholeNote": ("Whole note", "整篇笔记"),
    "blocksOf": ("Blocks of {note}", "{note} 的块"),
    "pageBrowserSub": ("Current web page", "当前网页"),
    "pageReadingSub": ("Current reader page", "当前阅读页"),
    "viewSub": ("Current view · {n} rows", "当前视图 · {n} 行"),
    "heading": ("Add context", "添加上下文"),
    "headingWithQuery": ("Add context: {query}", "添加上下文:{query}"),
    "keys": ("↑↓ move · ↵ confirm · esc close", "↑↓ 选择 · ↵ 确认 · esc 关闭"),
    "keysDrill": ("↑↓ move · ↵ confirm · ⇥ blocks · esc close", "↑↓ 选择 · ↵ 确认 · ⇥ 选块 · esc 关闭"),
    "keysStage": ("↑↓ move · ↵ confirm · ⌫ back · esc close", "↑↓ 选择 · ↵ 确认 · ⌫ 返回 · esc 关闭"),
    "empty": ("No match — Enter sends the text as-is", "无匹配——回车按原文发送"),
    "extracting": ("Extracting…", "抽取中…"),
})

# ---------------------------------------------------------------- the built-in skills, as the UI names them
# A built-in skill's file is English (the model reads it); what the welcome, the / picker
# and Settings show for it comes from here. A vault's own skill shows its own description.
# `.ask` is the composer's placeholder once the skill is on the row: what that method takes
# — a topic, a note to @, or nothing but Enter — so the user is never left with a bare chip
# and a generic "what should it work on?".
add("skill.", {
    "research.title": ("Research a topic", "调研主题"),
    "research.desc": ("The vault first, verified web sources, one note", "先查库,网页来源要核实,落成一篇笔记"),
    "research.ask": ('What to look into? One sentence — @ notes as material', '要调研什么?一句话说清;@ 笔记作为参考材料'),
    "study.title": ("Study a paper", "研读论文"),
    "study.desc": ("Import the source, read it in chapters, check your retelling, cards from the gaps", "导入原文、分章精读、对照批改复述、从缺口出卡片"),
    "study.ask": ('A link or a file to study — and what you read it for (understand / reproduce / survey / interview / review)', '给一个链接或文件;说一句读它为了什么(懂思想 / 复现 / 综述 / 面试 / 审稿)'),
    "tutor.title": ("Explain and quiz", "讲解与卡片"),
    "tutor.desc": ("Explain from the notes, make flashcards, quiz from the due cards", "按笔记讲解、做闪卡、从到期卡片考你"),
    "tutor.ask": ('Explain, make cards or quiz you? Say which — the open or @ note is the material', '讲解、做卡片还是考你?说一句;当前或 @ 的笔记就是材料'),
    "daily-start.title": ("Start the day", "今日启动"),
    "daily-start.desc": ("Overdue, due today, in progress — three things for today", "过期、今天到期、进行中,定今天三件事"),
    "daily-start.ask": ('Enter starts the day — add a line if today has a focus', '直接回车开始今天;有重点可以先写一句'),
    "daily-close.title": ("Close the day", "今日收尾"),
    "daily-close.desc": ("Done today, notes touched, carry-over — into the journal", "今天做完了什么、动了哪些笔记、结转什么,记进日志"),
    "daily-close.ask": ('Enter closes the day — anything to record goes first', '直接回车收尾今天;想补记的先写在这里'),
    "weekly-report.title": ("Weekly report", "周报"),
    "weekly-report.desc": ("Shipped, slipped, next week — appended to the week's note", "本周完成、延误、下周,追加到本周笔记"),
    "weekly-report.ask": ("Enter writes this week's report", '直接回车写本周的周报'),
    "meeting-actions.title": ("Meeting notes into actions", "会议记录变行动项"),
    "meeting-actions.desc": ("Who, what, by when — reviewed on a card, then written", "谁、做什么、何时,卡片上审阅后写入"),
    "meeting-actions.ask": ('@ the meeting notes, or paste them here', '@ 会议记录,或把记录粘贴在这里'),
    "prd.title": ("Write a PRD", "写 PRD"),
    "prd.desc": ("Scattered input into requirement, decision and question blocks", "零散输入变需求、决策与开放问题块"),
    "prd.ask": ('Which product or feature? @ the feedback, meeting notes and clips', '写哪个产品或功能?@ 反馈、会议记录、剪藏等材料'),
    "feedback-triage.title": ("Triage feedback", "反馈归集"),
    "feedback-triage.desc": ("Feedback into evidence blocks, linked to what they bear on", "反馈变证据块,链到它支持的需求或决策"),
    "feedback-triage.ask": ('Paste the feedback, or @ the note or clip holding it', '粘贴反馈,或 @ 装着反馈的笔记或剪藏'),
    "decision-log.title": ("Decision log", "决策记录"),
    "decision-log.desc": ("Which decisions have no evidence, what contradicts, what superseded", "哪些决策没有证据、哪条反馈相悖、谁被替代"),
    "decision-log.ask": ("Which PRD's decisions? @ it — Enter reviews them all", '审视哪份 PRD 的决策?@ 它;直接回车看全部'),
    "iteration-plan.title": ("Plan the iteration", "迭代规划"),
    "iteration-plan.desc": ("Score the backlog, pick within capacity, tag and write", "给 backlog 打分、按容量挑选、打标签并写迭代笔记"),
    "iteration-plan.ask": ('The capacity in tasks, and @ the PRD holding the backlog', '本轮容量(几个任务)?@ 装着 backlog 的 PRD'),
    "retro.title": ("Retro", "复盘"),
    "retro.desc": ("Shipped, hypotheses, decisions, carry-over — and the release notes", "交付、假设、决策、结转,以及发布说明"),
    "retro.ask": ('Which iteration? Name it, or @ its note', '复盘哪一轮?写它的名字,或 @ 迭代笔记'),
    "brief.title": ("Implementation brief", "实现简报"),
    "brief.desc": ("A brief for a coding agent, and its summary recorded back", "导出给编码 agent 的简报,再把它的总结回填"),
    "brief.ask": ("Export: @ the requirements · Backfill: paste the coding agent's summary", '导出:@ 要交付的需求 · 回填:粘贴编码 agent 的总结'),
    "tidy.title": ("Tidy a folder", "整理文件夹"),
    "tidy.desc": ("Audit, review every move on a card, links rewritten", "先审计,每一步移动先在卡片上审阅,链接随之改写"),
    "tidy.ask": ('Which folder? @ it, or name it', '整理哪个文件夹?@ 它,或写文件夹名'),
    "tidy-note.title": ("Restructure a note", "重整一篇笔记"),
    "tidy-note.desc": ("Outline first, blocks moved not retyped, ids kept", "先出大纲,块只移动不重打,id 保留"),
    "tidy-note.ask": ('Which note? @ it — Enter takes the open one', '重整哪篇?@ 它;直接回车重整当前打开的这篇'),
})
add("skillGroup.", {
    "research": ("Research", "研究"),
    "daily": ("Daily", "日常"),
    "product": ("Product", "产品"),
    "tidy": ("Tidy", "整理"),
    "vault": ("This vault", "库内"),
})

# ---------------------------------------------------------------- the composer's / picker (skills)
add("skillPicker.", {
    "heading": ("Choose a method", "选择方法"),
    "headingWithQuery": ("Choose a method: {query}", "选择方法:{query}"),
    "empty": ("No such skill — Enter sends the text as-is", "没有这个 skill——回车按原文发送"),
})

# ---------------------------------------------------------------- reader (reading projection)
add("reader.", {
    "image": ("image", "图片"),
    "clipped": ("Clipped → {path}", "已剪藏 → {path}"),
    "selectFirst": ("Select some text in the article first", "请先在正文中选中文字"),
    "savedToInbox": ("Saved to inbox (no note editor is visible in Reader)",
                     "已存入 inbox(阅读投影下没有可见的笔记编辑器)"),
    "attachedToContext": ("Attached as reference for your next message", "已作为下一条消息的参考资料"),
    "backToOrigin": ("Back to where you came from ({key})", "回到来处 ({key})"),
    "alreadyClipped": ("Clipped", "已剪藏"),
    "alreadyClippedHint": ("Clipped — click to open the local note", "已剪藏,点击打开本地笔记"),
    "clip": ("Clip", "剪藏"),
    "clipHint": ("Save the extracted article to clips/ (byte-for-byte what you see)",
                 "投影全文存 clips/(与所见字节一致)"),
    "selectionToTarget": ("Selection → {name}", "选区→{name}"),
    "note": ("note", "笔记"),
    "selectionToTargetHint": ("Turn the selection into a quote block in {path}; {key}-click to change target",
                              "选中的文字变引用块,剪入 {path};{key} 点击换目标"),
    "selectionToInboxHint": ("Turn the selection into a quote block in inbox; {key} or ▾ picks a target note",
                             "选中的文字变引用块存入 inbox;{key} 或 ▾ 选择目标笔记"),
    "pickTargetHint": ("Pick a target note (pinned targets are one click away)", "选择目标笔记(钉住后单击直达)"),
    "clearTargetHint": ("Unpin the target and go back to the default", "解除目标,恢复默认落点"),
    "addToContext": ("Add to context", "加入上下文"),
    "addToContextHint": ("Hand the extracted article to the AI as reference for your next message",
                         "投影正文作为下一条消息的参考资料递给 AI"),
    "openInBrowser": ("Browser", "浏览窗"),
    "openInBrowserHint": ("Open the original page in the browser window (the escape hatch for logins, comments, images)",
                          "在浏览窗打开原页(交互逃生门:登录/评论/图片)"),
    "openInSystemBrowser": ("Open in the system browser", "在系统浏览器打开"),
    "leaveHint": ("Leave Reader and go back (Esc)", "离开投影,回到来处 (Esc)"),
    "translate": ("Translate", "译"),
    "translateHint": ("Read this page beside its translation, one row per block ({key})", "把这一页与译文按段并排对照阅读 ({key})"),
    "leave": ("Leave Reader", "离开投影"),
    "extracting": ("Extracting the article from {domain}…", "正在抽取 {domain} 的正文…"),
    "openInBrowserInstead": ("Open in the browser window instead", "改在浏览窗打开"),
    "retry": ("Retry", "重试"),
    "systemBrowser": ("System browser", "系统浏览器"),
    "degraded": ("Extraction fell back to the whole page, so the layout may be rough —",
                 "正文抽取走了整页降级,排版可能欠佳 —"),
    "viewOriginal": ("View the original page in the browser window", "在浏览窗查看原页"),
    "source": ("Source: ", "来源:"),
    "empty": ("Click a citation in an AI answer, or a source card, to read the extracted article here.",
              "点击 AI 回答里的引用,或来源卡片,在这里阅读抽取后的正文。"),
})

# ---------------------------------------------------------------- side-by-side translation
add("translate.", {
    "original": ("Original", "原文"),
    "translation": ("Translation", "译文"),
    "mode.side": ("Side by side", "并排"),
    "mode.stack": ("Interleaved", "交错"),
    "mode.target": ("Translation only", "仅译文"),
    "modeHint": ("How the two texts are laid out; a narrow pane interleaves on its own", "两栏怎么排;窄窗自动改为交错"),
    "targetHint": ("Target language", "目标语言"),
    "lang.zh-CN": ("Chinese (Simplified)", "简体中文"),
    "lang.zh-TW": ("Chinese (Traditional)", "繁体中文"),
    "lang.en": ("English", "英语"),
    "lang.ja": ("Japanese", "日语"),
    "lang.ko": ("Korean", "韩语"),
    "lang.other": ("Other…", "其他…"),
    "langPrompt": ("Target language (a name or a tag such as fr, de-DE):", "目标语言(名称或代码,如 fr、de-DE):"),
    "modelHint": ("The model translating; change it under Settings › AI", "翻译所用模型;在「设置 › AI」里更换"),
    "progress": ("{done} / {total}", "{done} / {total}"),
    "progressHint": ("Segments translated of those to translate", "已译段数 / 需译段数"),
    "complete": ("Done", "已完成"),
    "failedCount_one": ("{n} segment failed", "{n} 段失败"),
    "failedCount_other": ("{n} segments failed", "{n} 段失败"),
    "retryFailed": ("Retry", "重试"),
    "retranslate": ("Retranslate", "重译"),
    "copy": ("Copy", "复制"),
    "copied": ("Translation copied", "已复制译文"),
    "copyAll": ("Copy all", "复制全部"),
    "copiedAll": ("All translations copied", "已复制全部译文"),
    "save": ("Save as note", "保存为笔记"),
    "saveHint": ("Write the translation as a note beside the original, linked back through translation_of",
                 "把译文存成原文旁边的一篇笔记,frontmatter 里 translation_of 反链原文"),
    "saved": ("Saved → {path}", "已保存 → {path}"),
    "leave": ("Leave", "离开"),
    "leaveHint": ("Back to where you came from (Esc)", "回到来处 (Esc)"),
    "privateSegment": ("Private block — not translated", "私密块——不翻译"),
    "privateNote": ("This note is in a privacy zone and is not sent anywhere, so it cannot be translated",
                    "这篇笔记在隐私区,不会发送到任何地方,因此不能翻译"),
    "privateFile": ("This file is in a privacy zone and is not sent anywhere, so it cannot be translated",
                    "这个文件在隐私区,不会发送到任何地方,因此不能翻译"),
    "pageN": ("Page {n}", "第 {n} 页"),
    "pdfHint": ("The page on the left, its paragraphs translated on the right; the text is the PDF's extracted text",
                "左边是页面,右边是该页各段的译文;正文取自 PDF 抽取的文本"),
    "pdfEmpty": ("No text could be extracted from this PDF — it may be a scan", "这份 PDF 抽不出文本——可能是扫描件"),
    "nothingToTranslate": ("Nothing here to translate", "没有可翻译的内容"),
    "noSuchJob": ("This translation is no longer open", "这次翻译已经不在了"),
    "pageExpired": ("The page is no longer in the reader's cache — open it in the reader again, then translate",
                    "这一页已不在阅读投影的缓存里——重新在投影里打开后再翻译"),
    "asideHint": ("Not body text (a caption, a reference, a byline) — left as it is", "非正文(题注、参考文献、署名)——保持原样"),
    "pausedHere": ("Translated to here", "已译到这里"),
    "continue": ("Continue", "继续"),
    "translateAll": ("Translate everything", "翻译全文"),
    "notYet": ("Translated when it comes into view, or with “translate everything”", "滚到这里时翻译,或点「翻译全文」"),
    "savingRemaining": ("Translating the remaining {n}…", "正在翻译剩余 {n} 段…"),
    "usage": ("{n} tokens spent on this document ({input} in, {output} out)", "本文已用 {n} tokens(输入 {input},输出 {output})"),
    "noSource": ("Open a note first, then translate it", "先打开一篇笔记,再对照翻译"),
    "notANote": ("Only a note or a PDF can be translated", "只能翻译笔记或 PDF"),
    "waiting": ("Waiting…", "等待中…"),
    "sourceHint": ("Double-click a block to open it in the editor", "双击一段回到编辑器该块"),
    "openInEditor": ("Open in the editor", "在编辑器中打开"),
    "untranslated": ("Not translated", "未翻译"),
    "sendsText": ("The text is sent to the configured model", "正文会发送给所配置的模型"),
})

# ---------------------------------------------------------------- browser panel chrome (docked panel and the detached bar)
add("browserPanel.", {
    "closeBrowser": ("Close the browser (all tabs)", "关闭浏览器(所有标签页)"),
    "newTabEmpty": ("Type an address above — bookmarks and recents show up here", "在上面输入网址;收藏与最近会显示在这里"),
    "noMatch": ("Nothing matches", "没有匹配的"),
    "tabs": ("Browser tabs", "浏览标签页"),
    "newTab": ("New tab", "新标签页"),
    "closeTab": ("Close tab", "关闭标签页"),
    "detach": ("Detach into its own window", "分离成独立窗口"),
    "dock": ("Dock back into the main window", "贴回主窗"),
    "more": ("More", "更多"),
    "clip": ("Clip", "剪藏"),
    "clipMenuHint": ("Other clip targets", "其它剪藏落点"),
    "pickTarget": ("Pick a clip target…", "选落点…"),
    "aiDockedHint": ("Add this page to the conversation as context", "把这一页作为上下文加入对话"),
    "translate": ("Translate", "译"),
    "translateHint": ("Split this page with its translation: the page on the left, each paragraph's translation beside it on the right", "在页面里分屏显示译文:页面在左,每段译文在右侧对齐"),
    "translateActiveHint": ("Translating ({done}/{total}) · {tokens} tokens so far — click to close the translation pane", "翻译中({done}/{total})· 已用 {tokens} tokens——点击关闭译文栏"),
    "translateDoneHint": ("Translated · {tokens} tokens — click to close the translation pane", "已翻译 · 用了 {tokens} tokens——点击关闭译文栏"),
})

# ---------------------------------------------------------------- clip bar strings (reused by the browser panel chrome)
add("clipBar.", {
    "bookmarked": ("Bookmarked", "已收藏"),
    "unbookmarked": ("Bookmark removed", "已取消收藏"),
    "createAndClip": ("Create and clip into it", "创建并剪入"),
    "pickPlaceholder": ("Clip target: pick a note… (type a new name to create it; Tab for a section)",
                        "剪藏落点:选目标笔记…(输入新名字可创建;Tab 选小节)"),
    "pickSectionOf": ("Pick a section of “{title}”… (Enter to confirm, Esc to go back)",
                      "选择「{title}」的小节…(Enter 确认,Esc 返回)"),
    "pickSubHint": ("Pick a section of this note", "选这篇的小节"),
    "noMatchingSection": ("No matching section", "没有匹配的小节"),
    "noMatchingNote": ("No matching note", "没有匹配的笔记"),
    "urlPlaceholder": ("Type a URL, Enter to open", "输入网址,Enter 打开"),
    "addressHint": ("{url}\nClick to type a new URL", "{url}\n点击输入新网址"),
    "addressEmptyHint": ("Click to type a URL", "点击输入网址"),
    "addressEmpty": ("Type a URL…", "输入网址…"),
    "unstar": ("Remove bookmark", "取消收藏"),
    "star": ("Bookmark this page", "收藏此页"),
    "summonAgentHint": ("Open the AI chat window right here, with this page already in context",
                        "唤起 AI 对话窗就地提问,当前页面自动加入上下文"),
    "selectionToTargetHint": ("Clip the selection into {path}", "选中文字剪入 {path}"),
    "selectionDefaultHint": ("Turn the selection into a quote block in the main window's current note; ▾ pins a target",
                             "选中文字变引用块,插入主窗当前笔记;▾ 钉住目标笔记"),
    "unpinHint": ("Unpin and go back to the default target", "解除钉住,恢复默认落点"),
    "clipPage": ("Page→clips", "整页→clips"),
    "clipLink": ("Link→note", "链接→笔记"),
    "openClippedHint": ("Open the local clip note {path} (back in the main window)",
                        "打开本地剪藏笔记 {path}(回主窗)"),
})

# ---------------------------------------------------------------- research panel
add("agentPanel.", {
    "newSession": ("New session", "新会话"),
    "sources": ("Sources · {n}", "来源 · {n}"),
    "sourcesHint": ("Every web source this session cited, with whether a search confirmed it", "本会话引用过的全部网络来源,及是否经搜索核实"),
    "noSources": ("No web sources in this session yet", "本会话还没有网络来源"),
    "sourceVerified": ("verified", "已核实"),
    "copyLink": ("Copy link", "复制链接"),
    "linkCopied": ("Link copied", "已复制链接"),
    "closeSources": ("Close the source list", "关闭来源列表"),
    "justNow": ("just now", "刚刚"),
    "minutesAgo": ("{n} min ago", "{n} 分钟前"),
    "hoursAgo": ("{n} h ago", "{n} 小时前"),
    "legacyResume": ("Old record: the model context cannot be resumed, so a follow-up starts from scratch",
                     "旧记录:模型上下文不可续,追问将从零开始"),
    "inSeparateWindow": ("The conversation is open in a separate window", "对话已在独立窗口打开"),
    "goThere": ("Go there", "去那里"),
    "backToMainWindow": ("Back to the main window", "回到主窗"),
    "cannotSwitchWhileRunning": ("The agent is running — sessions cannot be switched", "agent 运行中,无法切换会话"),
    "switchSession": ("Switch session", "切换会话"),
    "newSessionHint": ("New session (the current conversation stays in the list)", "新建会话(当前对话保留在列表里)"),
    "newSessionButton": ("＋ New", "＋ 新建"),
    "closeSessionList": ("Close the session list", "关闭会话列表"),
    "noSessions": ("No sessions yet — the first message you send shows up here",
                   "还没有历史会话——发出第一条消息就会出现在这里"),
    "noResumable": ("No resumable sessions — the old records are in the drawer below",
                    "还没有可继续的会话——旧记录在下面的抽屉里"),
    "legacyHint": ("Single-turn logs from before the upgrade: readable, but the model context cannot be "
                   "resumed, so a follow-up starts from scratch",
                   "升级前的单轮日志:可回看,但模型上下文不可续,追问会从零开始"),
    "legacyDrawer": ("Old records · {n}", "旧记录 · {n} 条"),
    "confirmDelete": ("Confirm delete", "确认删除"),
    "adblockOn": ("Ad blocking active · {n} rules", "去广告已生效 · {n} 条规则"),
    "adblockFailed": ("Ad blocking failed: {error}", "去广告失败: {error}"),
    "reload": ("Reload", "刷新"),
})

# ---------------------------------------------------------------- agent chat
add("hover.", {
    "loading": ("Loading…", "加载中…"),
    "missing": ("The cited block is no longer in the note", "引用的块已不在笔记中"),
})

add("chat.", {
    "thisNote": ("this note", "本笔记"),
    "citationsMaterialized": ("Wrote {n} block ids into the cited notes", "已为引用的 {n} 个块写入 id"),
    "citationsDropped": ("{n} citations could not be pinned to a block and point to the note instead", "{n} 条引用无法定位到块，改为指向笔记"),
    "planRejectedByUser": ("Rejected by the user", "用户否决"),
    "planApproved": ("Approved ✓", "已确认 ✓"),
    "planApprovedEdited": ("Approved with edits ✓", "已修改并确认 ✓"),
    "planCancelled": ("Cancelled", "已取消"),
    "planRejected": ("Rejected", "已否决"),
    "usageContext": ("context {tokens}", "上下文 {tokens}"),
    "usageIdle": ("{t} · {context}", "用时 {t} · {context}"),
    "stepsLeft": ("{n} steps left", "还剩 {n} 步"),
    "stepsLeftHint": ("Model calls left in this turn's budget; the model has been asked to wrap up", "本轮调用预算所剩,模型已被要求收尾"),
    "elapsedHint": ("Time since this turn started", "本轮已运行的时间"),
    "answerSignature": ("AI research session", "AI 研究会话"),
    "copiedSelection": ("Selected excerpt copied", "已复制选中片段"),
    "copiedAnswer": ("Answer copied as Markdown", "已复制回答 Markdown"),
    "formatted": ("Formatted ({key} undoes)", "已格式化({key} 撤销)"),
    "formattedNoChange": ("Already formatted", "已经是规范格式"),
    "pastedAsMarkdown": ("Pasted as Markdown", "已转成 Markdown 粘贴"),
    "researchNoteName": ("Research {date}", "研究 {date}"),
    "savedAs": ("Saved as {path}", "已存为 {path}"),
    "forked": ("Forked into a new session (the original stays in the list)", "已分叉出新会话(原会话保留在列表里)"),
    "continueWord": ("continue", "继续"),
    "clipped": ("Clipped → {path}", "已剪藏 → {path}"),
    "placeholder1": ("Ask a question or give a task…", "提问或下达任务…"),
    "placeholder2": ("@ picks a note, folder, selection or web page as context",
                     "@ 挑笔记、文件夹、选区或网页作上下文"),
    "placeholder3": ("Start with # to record a convention in VEMRY.md", "# 开头,记一条约定进 VEMRY.md"),
    "placeholder4": ("Paste a URL and press Enter to open it", "粘贴网址,回车直接打开"),
    "urlHintMac": ("↵ open the page · ⌥↵ send as a question", "↵ 打开网页 · ⌥↵ 作为提问发送"),
    "urlHintPc": ("Enter opens the page · Alt+Enter sends as a question", "Enter 打开网页 · Alt+Enter 作为提问发送"),
    "memoHintMac": ("↵ record it in VEMRY.md · ⌥↵ send as a question", "↵ 记入 VEMRY.md 约定 · ⌥↵ 作为提问发送"),
    "memoHintPc": ("Enter records it in VEMRY.md · Alt+Enter sends as a question",
                   "Enter 记入 VEMRY.md 约定 · Alt+Enter 作为提问发送"),
    "openPageTitle": ("Open the page (⏎)", "打开网页(⏎)"),
    "openPageAria": ("Open the page", "打开网页"),
    "memoTitle": ("Record it in VEMRY.md (⏎)", "记入 VEMRY.md 约定(⏎)"),
    "memoAria": ("Record the convention", "记入约定"),
    "queueTitle": ("Queue it, send when this turn ends (⏎)", "排队,本轮结束后发送(⏎)"),
    "sendTitle": ("Send (⏎)", "发送(⏎)"),
    "queueAria": ("Queue", "排队"),
    "sendAria": ("Send", "发送"),
    "needsMultimodal": ("{path}\nSeeing images needs a multimodal model (switch to glm-4.6v / qwen-vl in Settings)",
                        "{path}\n需多模态模型才能看图(设置里换 glm-4.6v / qwen-vl 等)"),
    "currentNoteContext": ("The current note {note} goes in as context", "当前笔记 {note} 会作为上下文"),
    "tagCurrent": ("current", "当前"),
    "dropCurrentNote": ("Leave the current note out of this session", "本会话不带当前笔记"),
    "dropCurrentNoteHint": ("Leave the current note out (switching notes brings it back)",
                            "不带当前笔记(切到别的笔记会回来)"),
    "tagSelection": ("selection", "选区"),
    "dropSelection": ("Leave the selection out", "不带当前选区"),
    "dropSelectionHint": ("Leave this selection out (changing the selection brings it back; @ it to pin it)",
                          "不带这段选区(选区变了会回来;要钉住就 @ 一下它)"),
    "tagTruncated": ("truncated", "已截断"),
    "removeContext": ("Remove from context", "移除上下文"),
    "maxRefs": ("At most {n} context items", "最多附加 {n} 项上下文"),
    "unsupportedImage": ("{name} is in the vault, but this format cannot be sent to a model "
                         "(png/jpg/gif/webp only; use the system screenshot shortcut or save as PNG)",
                         "{name} 已存入 vault,但该格式不能发给模型(仅 png/jpg/gif/webp;截图请用系统截图快捷键或另存为 PNG)"),
    "deepseekNoImages": ("deepseek-v4-pro has no image support and will error on send; "
                         "switch to deepseek-flash or glm-4.6v in Settings",
                         "deepseek-v4-pro 不支持图片,发送会报错;可在设置里换 deepseek-flash 或 glm-4.6v 等多模态模型"),
    "unreadableFile": ("{name} is in the vault, but it is not text or PDF — the model cannot read this format",
                       "{name} 已存入 vault,但不是文本/PDF,模型读不了这种格式"),
    "fileImages": ("Images", "图片"),
    "fileText": ("Text", "文本"),
    "fileAll": ("All files", "所有文件"),
    "effortMenuTitle": ("Thinking depth (this session)", "思考深度(本会话)"),
    "effortLabel": ("Thinking · {level}", "思考·{level}"),
    "effortThisSession": (" (this session)", "(本会话)"),
    "effortFollowGlobal": ("Follow the global setting ({level})", "跟随全局({level})"),
    "modelChipHint": ("Model {model} · thinking depth · web search. Click to adjust (the model itself is changed in Settings)",
                      "模型 {model} · 思考深度 · 联网搜索,点击调整(模型本身在设置里更换)"),
    "changeModel": ("Change model… (Settings)", "更改模型…(设置)"),
    "copySelection": ("Copy the selected excerpt", "复制选中片段"),
    "copy": ("Copy", "复制"),
    "insertAsQuote": ("Insert as a quote block at the cursor in the current note", "作为引用块插入当前笔记光标处"),
    "insert": ("Insert", "插入"),
    "sendToTarget": ("Send the quote block to {name}", "引用块送到 {name}"),
    "sendToTargetPlain": ("Send to {name}", "送到 {name}"),
    "pinFirst": ("Pin a note first with “▾ pick target”", "先用「▾ 选目标」钉住一篇笔记"),
    "pinWord": ("pin", "钉住"),
    "planProgress": ("Plan · {done}/{total}", "计划 · {done}/{total}"),
    "planStatusDoing": ("in progress", "进行中"),
    "followTag": ("working", "跟随"),
    "followTitle": ("Working note {note}\nThe AI keeps its findings here; appends, frontmatter and edits on it apply without a dialog and show as unreviewed blocks in the editor",
                    "跟随中 {note}\nAI 把结论写进这篇;对它的追加、frontmatter 与编辑不再逐次确认,落盘后在编辑器里显示为未审阅块"),
    "followTitleAsk": ("Working note {note}\nThe AI keeps its findings here; every write still asks first",
                       "跟随中 {note}\nAI 把结论写进这篇;每次写入仍先确认"),
    "unfollow": ("Stop following", "取消跟随"),
    "followMenuTitle": ("Working note", "跟随笔记"),
    "followAutoOn": ("Local writes apply without asking", "局部写入自动放行"),
    "followAutoOff": ("Ask before every write", "每次写入先确认"),
    "followOpen": ("Open the note", "打开笔记"),
    "followed": ("Following {name}: findings land in this note", "已跟随 {name}:结论会写进这篇"),
    "unfollowed": ("Stopped following", "已取消跟随"),
    "resumeResearch": ("This note came from an AI session", "这篇笔记来自一次 AI 会话"),
    "resumeButton": ("Resume that session", "继续这项研究"),
    "explainPrompt": ("Explain the selected passage: what it says, one example, and where it stops applying.",
                      "解释选中的这段:说了什么、举一个例子、以及它在什么情况下不成立。"),
    "noSelectionToExplain": ("Select a passage in the editor first", "先在编辑器里选中一段文字"),
    "busyExplain": ("The AI is still answering — ask again when it finishes", "AI 还在回答,结束后再试"),
    "actionsTitle": ("Action items", "行动项"),
    "actionsWaiting": ("Review, then write", "审阅后写入"),
    "actionsWrite": ("Write {n} items", "写入 {n} 项"),
    "actionsWriteNone": ("Keep none", "都不要"),
    "actionsReject": ("✕ Discard", "✕ 放弃"),
    "actionsApproved": ("Written {n} ✓", "已写入 {n} 项 ✓"),
    "actionsRejected": ("Discarded", "已放弃"),
    "actionsCancelled": ("Cancelled", "已取消"),
    "actionsOwner": ("owner", "负责人"),
    "actionsDue": ("due", "到期"),
    "actionsTarget": ("target note", "目标笔记"),
    "actionsSource": ("from: {source}", "来自:{source}"),
    "actionsDuplicate": ("looks like an existing task → {path}", "疑似已有 → {path}"),
    "actionsRejectPlaceholder": ("Tell the agent why (optional)", "告诉 agent 为什么放弃(可选)"),
    "turnWrote": ("This turn wrote {m} notes · {n} blocks await review", "本轮写入 {m} 篇 · {n} 块待审阅"),
    "reviewStart": ("Review", "开始审阅"),
    "reviewStartHint": ("Open the first of those notes on its first AI change; {accept} marks a change reviewed and moves on, {next} steps without deciding", "打开第一篇，落到第一处 AI 修改；{accept} 标为已审阅并前进，{next} 只前进不决定"),
    "reviewAccept": ("✓ Mark all as reviewed", "✓ 全部标为已审阅"),
    "reviewAcceptHint": ("Mark every AI block this turn wrote as reviewed without opening the notes", "不打开笔记，把本轮写入的 AI 块全部标为已审阅"),
    "undoTurn": ("↶ Undo this turn's writes", "↶ 撤销本轮写入"),
    "undoTurnHint": ("Put back every write of this turn that has not been edited since (moves moved back, links restored)",
                     "把这一轮落盘的写入逐项还原(未被再编辑的才还原;移动移回、链接改回)"),
    "undone": ("Undone {n} writes{skipped}", "已撤销 {n} 项{skipped}"),
    "undoneSkipped": (", {n} skipped (edited since)", ",{n} 项已被编辑过跳过"),
    "viewTag": ("view", "视图"),
    "skillTag": ("method", "方法"),
    "skillTitle": ("This message follows the {name} skill", "本条消息按 {name} 方法处理"),
    "statusSkill": ("by {name}", "按 {name}"),
    "skillPlaceholder": ("What should “{name}” work on? Enter alone runs it as is", "要按「{name}」处理什么?直接回车也可以"),
    "welcomeSkills": ("Start from a method", "从一个方法开始"),
    "welcomeSkillsHint": ("or type / in the box at any time", "输入框里键入 / 随时可选"),
    "readTitle": ("Pages to read", "要读的网页"),
    "readWaiting": ("Your click is the fetch", "点一下才会抓取"),
    "readOpen": ("Read {n} pages", "读取 {n} 页"),
    "readOpenNone": ("Read none", "都不读"),
    "readDecline": ("Decline", "不读"),
    "readDeclined": ("Declined", "已拒绝"),
    "readDone": ("Read {n} ✓", "已读 {n} 页 ✓"),
    "readDoneAuto": ("Read {n} ✓ · without asking", "已自动读取 {n} 页 ✓"),
    "readAutoRunning": ("Reading without asking…", "自动读取中…"),
    "readFetching": ("Reading…", "读取中…"),
    "readNone": ("Nothing read", "没有读取"),
    "readAutoSources": ("Read without asking for the rest of this session", "本会话不再询问,自动读取"),
    "viewTitle": ("What the {view} shows right now goes in as context ({n} rows)", "{view} 此刻显示的内容作为上下文({n} 行)"),
    "planStatusSkipped": ("skipped", "已跳过"),
    "welcomeBody": ("Paste a paper, a page or a PDF to work through it: it arrives in your vault as "
                    "blocks, each chapter links back to the passage it came from, and what you did "
                    "not get becomes cards for later. Or ask a question — the answer draws on your "
                    "notes (and the web), with citations you can open.",
                    "贴一篇论文、网页或 PDF 来研读:原文以块落进你的知识库,每一章都链回出处,没读懂的地方变成卡片留给以后。也可以直接提问,回答基于你的笔记(与网络),引用点开即读。"),
    "welcomeMore": ("More methods ({n})", "更多方法({n})"),
    "welcomeLess": ("Fewer methods", "收起"),
    "welcomeHint": ("Type a URL to open the built-in browser ({key} works too); start with # to record a "
                    "convention in VEMRY.md; every write shows you a diff first.",
                    "输入网址可直接打开内置浏览器({key} 亦可);以 # 开头把一条约定记进 VEMRY.md;写入前都会展示 diff 请你确认。"),
    "toolFailures": (" · {n} failed", " · {n} 失败"),
    "privateRef": ("Private reference", "私密引用"),
    "privateRefHint": ("Marked private", "已标记为私密"),
    "editResend": ("✎ Edit and resend", "✎ 编辑重发"),
    "editResendHint": ("Take this turn back, edit it and resend (the original stays in the audit log)",
                       "撤回这一轮,编辑后重发(原轮保留在审计里)"),
    "fork": ("⑂ Fork", "⑂ 分叉"),
    "forkHint": ("Fork a new session from before this message (a research tree: the earlier context comes along)",
                 "从这条消息之前分叉出新会话(研究树:此前上下文带过去)"),
    "copyAnswerHint": ("Copy the answer as Markdown (only the selection when text is selected)",
                       "复制回答 Markdown(选中片段时只复制选区)"),
    "insertAnswerHint": ("Insert at the cursor in the current note (as a quote block when text is selected)",
                         "插入当前笔记光标处(选中片段时作为引用块)"),
    "saveAsNote": ("Save as note", "存为笔记"),
    "saveAsNoteHint": ("Save as a new note in research/, with the question and sources in the frontmatter",
                       "存为新笔记(research/,frontmatter 记问题与来源)"),
    "regenerate": ("⟳ Regenerate", "⟳ 重新生成"),
    "regenerateHint": ("Regenerate this turn's answer", "重新生成这一轮回答"),
    "planTitle": ("Plan", "计划"),
    "planWaiting": ("Waiting for you", "等待确认"),
    "planRejectPlaceholder": ("Tell the agent why (optional)", "告诉 agent 为什么否决（可选）"),
    "planConfirmReject": ("Confirm rejection", "确认否决"),
    "planReject": ("✕ Reject", "✕ 否决"),
    "planRestore": ("Restore the list", "还原清单"),
    "planEdit": ("✎ Edit", "✎ 修改"),
    "planApprove": ("✓ Run it", "✓ 确认执行"),
    "planSteps": ("{n} steps", "{n} 步"),
    "planExpandHint": ("Show the steps", "展开步骤"),
    "planCollapseHint": ("Collapse the steps", "收起步骤"),
    "sourcesCount": ("Sources · {n}", "来源 · {n}"),
    "searchHits": ("Search hits · {n}", "搜索命中 · {n}"),
    "moreHits": ("{n} more search hits the answer did not cite", "其余 {n} 条搜索命中(回答未引用)"),
    "searchMenuItem": ("Web search (the model decides when)", "联网搜索(何时搜由模型决定)"),
    "searchOnHint": ("Web search is on: from the next message the model may search through the provider "
                     "(searches are billed separately; the queries show on the timeline). Turn it off in this menu.",
                     "联网搜索已开:从下一条消息起模型可自行通过 provider 搜索(搜索单独计费,搜索词会显示在时间线)。在此菜单里可关闭"),
    "searchUnavailable": ("Web search needs the DeepSeek endpoint (Settings › AI)", "联网搜索需要 DeepSeek 端点(设置 › AI)"),
    "usageSearches": (" · {n} web searches", " · 联网搜索 {n} 次"),
    "sourcesClipped": (" · {n} clipped", " · 已剪藏 {n}"),
    "searched": ("Searched the web", "已联网搜索"),
    "searching": ("Searching the web…", "正在联网搜索…"),
    "expandSources": ("Expand the source list", "展开来源列表"),
    "collapseSources": ("Collapse the source list", "收起来源列表"),
    "queries": ("Queries: {list}", "搜索:{list}"),
    "unverified": ("unverified", "未核实"),
    "unverifiedHint": ("No web search ran this turn — this link appears only in the model's prose and may "
                       "be from memory. Open it, check it, and clip it if it holds up.",
                       "本轮没有联网搜索,该链接只出现在模型正文里——可能是凭记忆编的,点开核实后建议剪藏落地"),
    "clipSourceHint": ("Save to clips/ as a reference note; the citation then lands as a local link",
                       "存为 clips/ 参考笔记,引用随之「落地」为本地链接"),
    "retry": ("⟳ Retry", "⟳ 重试"),
    "truncated": ("Output was cut off at the length limit", "输出因长度上限被截断"),
    "continueGenerating": ("Keep generating", "继续生成"),
    "collapseThinking": ("▾ Collapse thinking", "▾ 收起思考"),
    "expandThinking": ("Click to expand the full reasoning (selectable)", "点击展开完整思考(可选中复制)"),
    "thinking": ("Thinking", "思考中"),
    "thinkingCount": ("Thinking · {n} chars", "思考中 · {n} 字"),
    "usageLastTurn": ("Last turn: {calls} model calls · context {prompt} tokens · cache hits {cache} · output {completion} tokens",
                      "上一轮:{calls} 次调用 · 上下文 {prompt} tokens · 缓存命中 {cache} · 输出 {completion} tokens"),
    "stop": ("Stop", "停止"),
    "backToBottom": ("↓ Back to the bottom", "↓ 回到底部"),
    "stopHint": ("Stop this turn", "停止本轮"),
    "statusWaitingPlan": ("Waiting for your decision on the plan", "等待你确认计划"),
    "statusWaitingRead": ("Waiting for you to pick the pages to read", "等待你选择要读的网页"),
    "statusWaitingActions": ("Waiting for your decision on the action items", "等待你确认行动项"),
    "statusWaitingWrite": ("Waiting for your approval of a write", "等待你批准写入"),
    "statusReading": ("Reading {x}", "正在读取 {x}"),
    "statusReadingPages": ("Reading the pages", "正在读取网页"),
    "statusSearching": ("Searching for {x}", "正在搜索 {x}"),
    "statusLooking": ("Looking up {x}", "正在查找 {x}"),
    "statusWriting": ("Writing {x}", "正在写入 {x}"),
    "statusTool": ("{tool} {x}", "正在{tool} {x}"),
    "statusAnswering": ("Writing the answer…", "正在撰写回答…"),
    "queued": ("Queued: {text}", "已排队:{text}"),
    "cancelQueued": ("Cancel the queued message", "取消排队消息"),
    "runningQueueHint": ("Running · type the next one, Enter queues it", "运行中 · 输入下一条,回车排队"),
    "composerAria": ("Ask a question or give a task", "提问或下达任务"),
    "addContextHint": ("Add context: a note, folder, selection or web page, or a file from your computer "
                       "(typing @ works too; images and files can be pasted straight in)",
                       "添加上下文:笔记 / 文件夹 / 选区 / 网页,或从电脑添加文件(输入 @ 亦可;图片和文件也可直接粘贴进输入框)"),
    "effortHint": ("Thinking depth: click to set it for this session (the global default is in Settings)",
                   "思考深度:点击调整本会话档位(全局默认在设置)"),
    "popOutHint": ("Press {key} again to pop out a separate window", "再按 {key} 弹出独立窗口"),
})

# ---------------------------------------------------------------- update dialog
add("updater.", {
    "checkingTitle": ("Checking for updates…", "正在检查更新…"),
    "availableTitle": ("A new version of vemry is available", "vemry 有新版本"),
    "availableBody": ("vemry {version} is available for download. You have {current}. Download it now?",
                      "vemry {version} 可供下载,你当前的版本是 {current}。要现在下载吗?"),
    "notes": ("Release notes", "更新说明"),
    "autoCheck": ("Check for updates automatically at startup", "以后启动时自动检查更新"),
    "skip": ("Skip this version", "跳过这个版本"),
    "later": ("Remind me later", "稍后提醒我"),
    "install": ("Install update", "安装更新"),
    "downloadingTitle": ("Updating vemry to {version}", "正在更新 vemry 到 {version}"),
    "downloadingBody": ("Downloading the update… the package is verified before it is installed.",
                        "正在下载更新…安装包会在安装前经签名校验。"),
    "background": ("Continue in background", "后台继续"),
    "readyTitle": ("Update installed", "更新已安装"),
    "readyBody": ("Restart vemry to finish updating to {version}.", "重启 vemry 以完成到 {version} 的更新。"),
    "restartLater": ("Later", "稍后"),
    "restartNow": ("Restart now", "立即重启"),
    "upToDateTitle": ("You're up to date", "已是最新版本"),
    "upToDateBody": ("vemry {current} is the newest version.", "vemry {current} 已是最新版本。"),
    "checkFailedTitle": ("Could not check for updates", "无法检查更新"),
    "installFailedTitle": ("The update could not be installed", "更新未能安装"),
})

# ---------------------------------------------------------------- settings dialog
add("settings.", {
    "title": ("Settings", "设置"),
    "categories": ("Settings categories", "设置分类"),
    "needsVault": ("Available once a vault is open", "打开知识库后可用"),
    "tab.general": ("General", "通用"),
    "tab.ai": ("AI & search", "AI 与搜索"),
    "tab.browser": ("Browser", "浏览器"),
    "tab.vault": ("Vault", "知识库"),
    "tab.updates": ("Updates", "更新"),
    "save": ("Save", "保存"),
    "saved": ("Saved ✓", "已保存 ✓"),
    "endpoint": ("API endpoint (OpenAI-compatible)", "API 端点（OpenAI 兼容）"),
    "endpointZhipu": ("Zhipu BigModel", "智谱 BigModel"),
    "endpointDeepseek": ("DeepSeek (web search)", "DeepSeek(可联网搜索)"),
    "endpointOllama": ("Ollama, local", "Ollama 本机"),
    "endpointHint": ("Zhipu BigModel by default; any OpenAI-compatible endpoint works (Ollama, DeepSeek…)",
                     "默认智谱 BigModel；可换任意 OpenAI 兼容端点（Ollama、DeepSeek…）"),
    "model": ("Model", "模型"),
    "modelZhipuFlash": ("Zhipu · free, supports tool calls", "智谱 · 免费,支持工具调用"),
    "modelZhipu": ("Zhipu · flagship", "智谱 · 旗舰"),
    "modelDeepseekFlash": ("DeepSeek · flash, reads images", "DeepSeek · flash,支持图片"),
    "modelDeepseekPro": ("DeepSeek · pro, no images", "DeepSeek · pro,不支持图片"),
    "modelHint": ("glm-4.7-flash is free and supports tool calls; a change takes effect from the next AI "
                  "session, and the panel header shows the current model",
                  "glm-4.7-flash 免费且支持工具调用;改动自下一个 AI 会话生效,面板顶栏可见当前模型"),
    "revokeWritePolicy": ("AI writes unreviewed in this vault ✕", "本库 AI 写入免审 ✕"),
    "revokeWritePolicyHint": ("Click to revoke: AI writes go back to per-write diff approval",
                              "点击撤销:AI 写入恢复逐条 diff 审批"),
    "revokeWritePolicyNote": ("You picked “always allow in this vault” in a permission dialog (.vemry/config.json)",
                              "此前在权限对话框里点过「此库总是允许」(.vemry/config.json)"),
    "writePolicyRevoked": ("Unreviewed writes revoked — AI writes go back to per-write approval",
                           "已撤销免审写入,AI 写入恢复逐条审批"),
    "apiKey": ("API key", "API Key"),
    "apiKeyPlaceholder": ("Get one at open.bigmodel.cn", "从 open.bigmodel.cn 获取"),
    "apiKeyEnv": ("ZHIPU_API_KEY found in the environment — it takes precedence",
                  "检测到 ZHIPU_API_KEY 环境变量，将优先使用"),
    "apiKeyNote": ("Stored only in this machine's app config directory, never in your vault",
                   "仅保存在本机应用配置目录，不会进入知识库"),
    "fontSize": ("Body text size", "正文字号"),
    "fontSizeCompact": ("15 px (compact)", "15 px（紧凑）"),
    "fontSizeDefault": ("16 px (default)", "16 px（默认）"),
    "fontSizeRoomy": ("18 px (roomy)", "18 px（舒展）"),
    "fontSizeHint": ("Applies to body text in the editor and Reader; UI size does not follow. "
                     "Takes effect on save.",
                     "作用于编辑器与阅读投影的正文；界面字号不随动，保存后即时生效"),
    "readableLineWidth": ("Readable line width", "可读行宽"),
    "readableLineWidthHint": ("Body text widens with the pane but stops at a limit — about 40 CJK "
                              "characters per line in a narrow pane, at most about 50 on a wide screen. "
                              "Turn it off and the editor and Reader fill the pane.",
                              "正文列宽随窗格增长但设上限:窄窗约 40 汉字/行,宽屏最多约 50;关闭后编辑器与阅读投影随窗口撑满"),
    "webSearch": ("AI web search", "AI 联网搜索"),
    "webSearchHint": ("The provider's server-side search (DeepSeek, any of its models; nothing runs locally), "
                      "billed separately from chat and off by default. The chat composer's model menu carries "
                      "this same switch, and a change takes effect from the next message. Search hits and sources "
                      "show on the AI timeline and open in the built-in browser.",
                      "走 provider 的服务端搜索(DeepSeek 端点,任意模型;本地不联网),按次单独计费,默认关闭。"
                      "对话输入栏的模型菜单里有同一个开关,改动从下一条消息起生效。搜索命中与来源显示在 AI 时间线,可点开内置浏览器。"),
    "provenanceMarks": ("Mark unreviewed AI blocks", "标记未审阅的 AI 块"),
    "provenanceMarksHint": ("An amber bar and a status-bar count for blocks the AI wrote under a standing grant, until you mark them reviewed; a marker, never a prompt. The origin overlay is a separate command", "AI 在常设授权下写入的块显示琥珀色条并在状态栏计数，直到你标为已审阅；只是标记，不会打断。出处着色是另一条命令"),
    "planConfirm": ("AI plan confirmation", "AI 计划确认"),
    "planConfirmHint": ("A multi-step task shows you a plan card first and waits for “run it / edit / "
                        "reject”; approving continues the same turn. Turned off, the AI recites the plan "
                        "and keeps going. Frozen per session along with the tool list, so a change takes "
                        "effect from the next session.",
                        "多步任务先出计划卡等你「确认执行 / 修改 / 否决」,确认后同轮继续;关闭后 AI 只复诵计划不停下。"
                        "与工具表一同按会话冻结,变更从新会话生效。"),
    "reasoningEffort": ("Thinking depth", "思考深度"),
    "reasoningDefaultSuffix": ("(default)", "(默认)"),
    # Short name plus an optional parenthetical. An empty hint means the level
    # needs no explanation — the UI renders the parenthesis only when non-empty.
    "effort.none": ("Off", "无"),
    "effort.noneHint": ("thinking disabled", "关闭思考"),
    "effort.low": ("Low", "低"),
    "effort.lowHint": ("", ""),
    "effort.high": ("High", "高"),
    "effort.highHint": ("", ""),
    "effort.max": ("Max", "最高"),
    "effort.maxHint": ("", ""),
    "reasoningHint": ("The global default; the chat composer can change it for one session. Higher levels "
                      "answer more deeply and more slowly. Only the DeepSeek endpoint uses it.",
                      "全局默认档,对话输入栏可临时调整本会话;档位越高回答越深入也越慢。仅 DeepSeek 端点生效。"),
    "reasoningInactive": (" This endpoint is not api.deepseek.com, so the level is not sent.",
                          "当前端点不是 api.deepseek.com,该档位不会发送。"),
    "appUpdates": ("App updates", "应用更新"),
    "currentVersion": ("Current version {version}", "当前版本 {version}"),
    "downloading": ("Downloading…", "下载中…"),
    "downloadInstall": ("Install {version}…", "安装 {version}…"),
    "restartToFinish": ("Restart to finish the update", "重启以完成更新"),
    "checking": ("Checking…", "检查中…"),
    "upToDate": ("Up to date", "已是最新版本"),
    "updateAvailable": ("Version {version} is available", "新版本 {version} 可用"),
    "installedRestart": ("Installed — takes effect after a restart", "已安装,重启后生效"),
    "autoCheck": ("Check for updates at startup", "启动时自动检查更新"),
    "autoCheckHint": ("Off by default (offline-first: without it, the app only goes online when you click "
                      "“check for updates”). On, it asks GitHub Releases once at startup and only shows "
                      "the update dialog when there is a new version (a version you skipped stays quiet); "
                      "downloading and installing always need your confirmation, and the package "
                      "signature is verified.",
                      "默认关闭(离线优先:不开启则只有点「检查更新」才联网)。开启后启动时向 GitHub Releases "
                      "静默查询一次,发现新版才弹出更新提示(跳过的版本不再提示);下载与安装始终需要你确认,"
                      "安装包经签名校验。"),
    "adblock": ("Ad blocking (subscription rules, native WebKit blocking, macOS only)",
                "去广告(订阅规则,WebKit 原生拦截,仅 macOS)"),
    "adblockActive": ("Active · {n} rules", "已生效 · {n} 条规则"),
    "adblockCompiling": ("Compiling rules…", "规则编译中…"),
    "adblockDownloading": ("Downloading rules…", "规则下载中…"),
    "adblockNoRules": ("No rules yet — they download when you open the first page, or fetch them now.",
                       "还没有规则——打开第一个页面时会自动下载,也可以现在就取。"),
    "openLinksIn": ("How links and citations open", "链接与引用的打开方式"),
    "openLinksReading": ("Reader (main window, centre pane — default)", "阅读投影(主窗中栏,默认)"),
    "openLinksInternal": ("Built-in browser", "内置浏览器"),
    "openLinksSystem": ("System default browser", "系统默认浏览器"),
    "openLinksHint": ("Reader extracts the article and renders it in the main window; {key} always gives "
                      "you a real browser window.",
                      "阅读投影抽正文在主窗渲染;{key} 永远给你一个真浏览器窗口。"),
    "clickOpensLinks": ("Single click opens links in the editor", "编辑器里单击打开链接"),
    "clickOpensLinksHint": ("A rendered link opens on a single click, {modClick} opens it another way; the "
                            "line with the cursor shows the source, and clicking there still edits. "
                            "{modEnter} opens the link at the cursor. Turned off, only {modClick} opens.",
                            "渲染态的链接单击即打开,{modClick} 换一种方式打开;光标所在行显示原文,点原文仍是编辑,"
                            "{modEnter} 打开光标处链接。关闭后回到仅 {modClick} 打开。"),
    "readingImages": ("Load images in Reader", "阅读投影加载图片"),
    "readingImagesHint": ("Images come through a local proxy: no cookies, the target address is checked in "
                          "Rust, and there is a disk cache. Reader only; a clipped note on disk never makes "
                          "a network request. Turned off, images show as placeholder chips you can click to "
                          "view in the browser window.",
                          "图片经本机代理拉取:不带 cookie、Rust 侧校验目标地址、磁盘缓存。只影响投影;"
                          "落盘的剪藏笔记永远不发网络请求。关闭后图片显示为占位标签,点击在浏览窗查看。"),
    "formatSection": ("Formatting on save", "落盘格式"),
    "formatAgentWrites": ("Format what the AI writes", "AI 写入时格式化"),
    "formatAgentWritesHint": ("New notes and appended sections the AI writes go through the Markdown formatter before the diff is shown; your own lines are never touched",
                              "AI 新建的笔记与追加的段落落盘前先过格式化器,diff 里看到的就是落盘的;你自己的行不动"),
    "formatClips": ("Format clips and rich-text pastes", "剪藏与粘贴时格式化"),
    "formatClipsHint": ("A clipped page and a rich-text paste turned into Markdown are formatted the same way",
                        "剪藏落盘的笔记与富文本粘贴转出的 Markdown 同样过一遍"),
    "formatCjkSpacing": ("Space between CJK and Latin text", "中英文之间加空格"),
    "formatPunctuation": ("Full-width punctuation after CJK text", "中文后标点转全角"),
    "formatOptInHint": ("These change the words' bytes, so the AI's block marks and change times move with them",
                        "这两项会改正文字节,AI 块标记与块修改时间会随之变化"),
    "clipLocalizeImages": ("Save images into the vault when clipping", "剪藏时把图片存进知识库"),
    "clipLocalizeImagesHint": ("After a full-page clip, article images are downloaded through that same "
                               "local proxy into clips/assets/ (deduplicated by content hash; 10 MB per "
                               "image, 30 images and 50 MB per clip), and the note is rewritten to point at "
                               "them — reading is then fully offline, and neither hotlink protection nor a "
                               "dead image host loses you a picture. Images that fail to download keep their "
                               "original link and are marked in the frontmatter.",
                               "整页剪藏落盘后,正文图片经同一本机代理下载到 clips/assets/(内容哈希去重,"
                               "单图 10MB、单次 30 张 50MB 上限),笔记改写为本地引用——此后阅读零网络,"
                               "防盗链与图床失效不再丢图。下载失败的图保留原始链接并在 frontmatter 标记。"),
    "ruleUrls": ("Blocking rule subscriptions (one URL per line)", "拦截规则订阅源(每行一个 URL)"),
    "ruleUrlsHint": ("No rule list ships inside vemry: the sources below are downloaded the first time you "
                     "open a page and cached locally. Each source is fetched on its own, so one unreachable "
                     "host costs only its own list — add a mirror as another line, or import a file you "
                     "downloaded yourself.",
                     "vemry 不内置任何规则表:下面的源在你第一次打开页面时下载,之后本地缓存。每个源独立下载,"
                     "某个源取不到只影响它自己——可以另起一行加镜像,或导入你自己下载好的文件。"),
    "updateRules": ("Update now", "立即更新"),
    "updatingRules": ("Updating…", "更新中…"),
    "rulesUpdated": ("Blocking rules updated", "拦截规则已更新"),
    "rulesPartial": ("Some sources could not be fetched: {errors}", "部分源没取到: {errors}"),
    "importRules": ("Import from file…", "从文件导入…"),
    "importRulesTitle": ("Import blocking rules", "导入拦截规则"),
    "ruleFileFilter": ("Filter list", "规则表文件"),
    "rulesImported": ("Imported {n} rule files", "已导入 {n} 个规则文件"),
    "privacyZones": ("Privacy zones", "隐私区"),
    "privacyZonesHint": ("Everything listed here is permanently invisible to the AI — never read, never "
                         "written, never in a prompt. Changes take effect immediately.",
                         "此处列出的内容对 AI 永远不可见 — 不读、不写、不进 prompt。改动立即生效。"),
    "privacyRules": ("Rules (.vemry/privacy)", "规则（.vemry/privacy）"),
    "privacyRulesHint": ("gitignore syntax, one glob per line; a folder rule covers everything inside it, "
                         "and a leading ! exempts. Examples:",
                         "gitignore 语法，每行一条 glob；文件夹规则覆盖其中所有内容，行首 ! 表示豁免。例如："),
    "deleteRule": ("Delete this rule", "删除此规则"),
    "noRules": ("(no rules yet)", "（暂无规则）"),
    "rulePlaceholder": ("secret/ or diary/**", "secret/ 或 diary/**"),
    "addRule": ("Add", "添加"),
    "privacyMarkedNotes": ("Notes marked one by one (frontmatter private: true)",
                           "逐篇标记的笔记（frontmatter private: true）"),
    "noMarkedNotes": ("(no notes marked one by one)", "（暂无逐篇标记的笔记）"),
    "unmarkNote": ("Unmark", "取消标记"),
    "openNote": ("Open this note", "打开此笔记"),
    "skills": ("AI skills", "AI Skills"),
    "skillsHint": ("The methods the AI follows for a matching task. Built-in ones ship with vemry and update "
                   "with it; a customised copy in .vemry/skills/ is your file and shadows the built-in of the "
                   "same name. Switches apply from the next turn; a file's text is read live.",
                   "AI 处理匹配任务时遵循的方法。内置的随 vemry 一起更新；.vemry/skills/ 里的自定义副本是你的文件，"
                   "会覆盖同名内置。开关自下一轮生效；文件正文每次实时读取。"),
    "skillBuiltin": ("built-in", "内置"),
    "skillCustom": ("custom", "自定义"),
    "skillOverride": ("custom · shadows the built-in", "自定义 · 覆盖内置"),
    "skillOutdated": ("built-in updated", "内置已更新"),
    "skillOutdatedHint": ("The built-in skill has changed since this copy was made — restore it to take the new "
                          "version, or customise again from it", "这份副本做好之后内置版本又改过——恢复内置即用新版，或在新版基础上再自定义"),
    "skillCustomize": ("Customise", "自定义副本"),
    "skillRestore": ("Restore built-in", "恢复内置"),
    "skillRestored": ("Restored the built-in {name}; the copy is in the trash", "已恢复内置 {name}，副本已移到废纸篓"),
    "skillNew": ("New skill", "新建 skill"),
    "skillNewPlaceholder": ("skill name", "skill 名称"),
    "edit": ("Edit", "编辑"),
    "typeExamples": ("Example block types", "示例块类型"),
    "typeExamplesHint": ("The product-document blocks: requirement, decision, question, hypothesis — installed into .vemry/types/, the editor's / menu and the AI learn them",
                         "产品文档的几种块:需求、决策、开放问题、假设——装进 .vemry/types/,编辑器 / 菜单与 AI 都认识它们"),
    "installSkill": ("Install", "安装"),
    "skillInstalled": ("Installed {path}", "已安装 {path}"),
    "skillAlready": ("Already installed", "已安装"),
    "userScripts": ("User scripts (.vemry/scripts/)", "用户脚本（.vemry/scripts/）"),
    "reloadScripts": ("Reload", "重新加载"),
    "userScriptsHint1": ("Drop a ", "放置 "),
    "userScriptsHint2": (" in place and it is live (saving hot-reloads it). Scripts run in a sandbox with "
                         "no filesystem and no network; they can only read and write this vault through the ",
                         " 即生效（保存自动热重载）。脚本在无文件系统/网络的沙箱里运行，只能通过 "),
    "userScriptsHint3": (" API. The first write asks for permission once, revocable below.",
                         " API 读写本库；首次写入会弹一次授权，可在下面撤销。"),
    "revokeGrant": ("Click to revoke write permission", "点击撤销写入授权"),
    "writable": ("writable ✕", "可写 ✕"),
    "noScripts": ("(no scripts yet)", "（暂无脚本）"),
    "scriptCommands": ("{n} commands", "{n} 个命令"),
    "scriptEvents": ("events: {list}", "事件: {list}"),
    "scriptSchedules": ("schedules: {list}", "定时: {list}"),
    "scriptNothing": ("nothing registered", "无注册项"),
})

# ---------------------------------------------------------------- settings dialog: translation
add("settings.translate.", {
    "section": ("Side-by-side translation", "对照翻译"),
    "disclosure": ("Translating sends the text of the note or page you are reading to the model below. A note in a privacy zone is never sent; private blocks are left out.",
                   "翻译会把正在读的笔记或页面正文发送给下面的模型。隐私区里的笔记从不发送,私密块会被跳过。"),
    "target": ("Target language", "目标语言"),
    "targetAuto": ("Follow the interface language", "跟随界面语言"),
    "targetCustom": ("Custom (type a tag below)", "自定义(在下方填写代码)"),
    "targetCustomPlaceholder": ("e.g. fr, de-DE, pt-BR", "如 fr、de-DE、pt-BR"),
    "model": ("Translation model", "翻译模型"),
    "modelHint": ("Empty = the chat model above. A fast, cheap model is enough here (the same endpoint and key are used).",
                  "留空 = 上面的对话模型。翻译用快而便宜的模型即可(端点与密钥沿用对话的)。"),
    "addendum": ("Extra instruction", "附加要求"),
    "addendumHint": ("One line added to the translation rules, e.g. “keep technical terms in English”",
                     "追加一条翻译规则,例如「专业术语保留英文」"),
})

# ---------------------------------------------------------------- app shell / toasts
add("app.", {
    "openBrowserHint": ("Open the browser ({key} to type an address)", "打开浏览器({key} 输入网址)"),
    # Joins the items of an inline list; the separator differs per language
    "listSep": (", ", "、"),
    "recordedIn": ("Recorded in {path}", "已记入 {path}"),
    "scriptError": ("Script {script}: {message}", "脚本 {script}: {message}"),
    "openVaultFirst": ("Open a vault first", "先打开一个知识库"),
    "pickVaultFolder": ("Pick a vault folder", "选择知识库文件夹"),
    "newVaultTitle": ("New vault", "新建知识库"),
    "newVaultDefaultName": ("My vault", "我的知识库"),
    "noHistory": ("No browsing history yet — {key} to type a URL and start", "暂无浏览历史,{key} 输入网址开始浏览"),
    "historyCleared": ("Browsing history cleared", "浏览历史已清除"),
    "noBrowserWindow": ("No browser window is open", "没有打开的浏览器窗口"),
    "nothingSelected": ("No text selected", "未选中文字"),
    "clippedInto": ("Clipped into {spot}", "已剪入 {spot}"),
    "insertFailed": ("Insert failed", "插入失败"),
    "clippedTo": ("Clipped → {path}", "已剪藏 → {path}"),
    "insertedInto": ("Inserted into {path}", "已插入 {path}"),
    "clipFailed": ("Clip failed", "剪藏失败"),
    "savedToInboxNoEditor": ("Saved to inbox (no note editor is visible)", "已存入 inbox(当前没有可见的笔记编辑器)"),
    "savedToInbox": ("Saved to inbox", "已存入 inbox"),
    "agentBusy": ("The agent is running — try again in a moment", "agent 运行中,请稍后再试"),
    "noKeyStaticTemplate": ("No API key configured — a static template was generated instead",
                            "未配置 API Key,已生成静态模板"),
    "noKeyOpenedForEdit": ("No API key configured, so there is nothing to survey with; VEMRY.md is open "
                           "for you to edit by hand",
                           "未配置 API Key,无法调研改进;已打开 VEMRY.md 供手动编辑"),
    "wroteFile": ("Wrote {path}", "已写入 {path}"),
    # A scaffold command whose file is already there opens it instead of failing
    "existsOpened": ("{path} already exists — opened it", "{path} 已存在,已打开"),
    "noReaderPageYet": ("No web page has been read yet — try clicking a citation in an AI answer",
                        "还没有打开过网页投影——点击 AI 回答里的引用试试"),
    "attachmentsAlreadyMigrated": ("Attachment folders are already bucketed by note path — nothing to migrate",
                                   "附件目录已是按笔记路径分桶,无需迁移"),
    "migratePlan": ("Move {buckets} attachment buckets and rewrite links in {notes} notes.",
                    "迁移 {buckets} 个附件桶,改写 {notes} 篇笔记的链接。"),
    "migrateMixed": ("{n} same-name mixed buckets stay where they are: {list}",
                     "同名混桶原地保留 {n} 个:{list}"),
    "migrateOrphaned": ("{n} orphaned buckets stay where they are", "无归属孤儿桶原地保留 {n} 个"),
    "migrateConflicts": ("{n} skipped because the target already exists: {list}",
                         "目标已存在跳过 {n} 个:{list}"),
    "migrateDone": ("Moved {buckets} attachment buckets, rewrote {notes} notes",
                    "已迁移 {buckets} 个附件桶,改写 {notes} 篇笔记"),
    "collapseSidebar": ("Collapse the sidebar", "收起侧栏"),
    "openSidebar": ("Open the sidebar", "打开侧栏"),
    "collapseSidebarKey": ("Collapse the sidebar ({key})", "收起侧栏 ({key})"),
    "openSidebarKey": ("Open the sidebar ({key})", "打开侧栏 ({key})"),
    # Kept identical to the README hero — the welcome screen is the first thing a
    # new user reads, and it should say what the project page said: the positioning
    # first (GLOSSARY: study workbench), the architecture line under it.
    "tagline": ("A local-first AI study workbench.", "本地优先的 AI 研读工作台。"),
    "taglineSub": ("Read papers, pages and PDFs with AI help. Everything stays as plain Markdown on "
                   "your disk, and what you mark private never reaches the model.",
                   "用 AI 读论文、网页和 PDF。一切都以纯 Markdown 留在你的磁盘上，你标为隐私的内容永远到不了模型。"),
    "openVaultFolder": ("Open a vault folder", "打开知识库文件夹"),
    "recent": ("Recent", "最近"),
    "resizeFileTree": ("Resize the file tree", "调整文件树宽度"),
    "resizeSidebar": ("Resize the research panel", "调整研究面板宽度"),
    "dragToResize": ("Drag to resize, double-click to reset", "拖动调整宽度,双击恢复默认"),
    "tabOutline": ("Outline", "大纲"),
    "tabResearch": ("Research", "研究"),
    "pageExtractFailed": ("Could not extract the page", "页面抽取失败"),
    "memoRecorded": ("Recorded in VEMRY.md · conventions", "已记入 VEMRY.md · 约定"),
    "memoFailed": ("Could not record it in VEMRY.md", "记入 VEMRY.md 失败"),
    "noVisibleEditor": ("No note editor is visible — open a note, or pin a clip target",
                        "当前没有可见的笔记编辑器——先打开一篇笔记,或用钉住目标"),
})

# ---------------------------------------------------------------- note history dialog
add("history.", {
    "title": ("Note history", "笔记历史"),
    "versions": ("Kept versions", "保留的版本"),
    "loading": ("Loading…", "加载中…"),
    "empty": ("Nothing kept yet. A version is kept before every write that replaces this note's text — an AI "
              "write, a script, a change made outside vemry — and about every five minutes of your own typing.",
              "还没有保留的版本。每次有写入替换这篇笔记的文本时——AI 写入、脚本、vemry 之外的改动——都会先保留一份;你自己的输入约每五分钟保留一份。"),
    "sameAsNow": ("This version is the same as the note is now", "这个版本与当前笔记相同"),
    "hint": ("The diff reads as what a restore would change. Restoring keeps the current text, so it can be undone here too.",
             "diff 显示恢复会改变什么。恢复前会先保留当前文本,所以恢复本身也能在这里撤销。"),
    "copy": ("Copy text", "复制文本"),
    "copied": ("Copied", "已复制"),
    "restore": ("Restore this version", "恢复到此版本"),
    "restored": ("Restored {path}", "已恢复 {path}"),
    "notANote": ("Note history is for Markdown notes", "笔记历史只对 Markdown 笔记可用"),
    "writer.editor": ("before your edit", "被你的编辑覆盖前"),
    "writer.agent": ("before an AI write", "被 AI 写入覆盖前"),
    "writer.script": ("before a script's write", "被脚本写入覆盖前"),
    "writer.clip": ("before a clip landed", "被剪藏/速记覆盖前"),
    "writer.restore": ("before a restore", "被恢复操作覆盖前"),
    "writer.app": ("before an app write", "被应用写入覆盖前"),
    "writer.external": ("before a change outside vemry", "被 vemry 之外的改动覆盖前"),
})

# ---------------------------------------------------------------- vault store toasts
add("vaultStore.", {
    "fragmentMissing": ("Opened {path}; \"{fragment}\" is not in it", "已打开 {path}，其中没有「{fragment}」"),
    "undoneMovedBack": ("Undone: moved back to {path}", "已撤销：移回 {path}"),
    "undoneImport": ("Undone: {n} files moved to the trash", "已撤销导入，{n} 个文件移入废纸篓"),
    "undoneCreate": ("Undone: moved to the trash — {path}", "已撤销新建，移入废纸篓：{path}"),
    "alreadyInTrash": ("“{path}” is already in the system trash — restore it from there",
                       "「{path}」已在系统废纸篓，请从废纸篓找回"),
    "switchWhileRunning": ("The AI is running; switching vaults cancels the current task.",
                           "AI 正在运行，切换知识库会取消当前任务。"),
    "dirtyBeforePrivacy": ("This note has unsaved changes — press {key} to save before changing its "
                           "privacy marking",
                           "当前笔记有未保存的修改，请先 {key} 保存，再更改隐私标记"),
    "stillPrivate": ("Still in the privacy zone — a broader rule covers it (Settings › Privacy zones)",
                     "仍在隐私区 — 有更宽的规则覆盖它（设置 › 隐私区）"),
    "deletedExternally": ("{path} was deleted or renamed outside vemry; unsaved changes were discarded",
                          "{path} 已被外部删除或改名,未保存的修改已丢弃"),
    "changedUnderEdit": ("{path} changed on disk while you were editing it — your buffer will be saved over "
                         "that change; the other text is kept in Note history",
                         "{path} 在你编辑时被其他写入者改动了——你的编辑内容将覆盖那次改动;被覆盖的文本保存在「笔记历史」里"),
    "unreadableTabClosed": ("{path} is no longer readable — the tab was closed", "{path} 已不可读,标签已关闭"),
    "created": ("Created {path}", "已新建 {path}"),
    "insertFailedSwitching": ("Insert into {path} failed: the editor is switching documents",
                              "插入 {path} 失败:编辑器正在切换"),
    "headingMissingTail": ("Section “{heading}” does not exist — appended to the end of the note",
                           "小节「{heading}」不存在,已追加到文末"),
    "headingMissingTailIn": ("Section “{heading}” does not exist — appended to the end of {path}",
                             "小节「{heading}」不存在,已追加到 {path} 文末"),
    "promptExpired": ("The input box expired — script {script} received a cancel",
                      "输入框已过期，脚本 {script} 收到的是「取消」"),
    "grantExpired": ("The permission request expired — the write from script {script} was treated as denied",
                     "授权请求已过期，脚本 {script} 的写入按「拒绝」处理"),
    "pinnedTargetGone": ("The pinned target {path} no longer exists — unpinned", "钉住的目标 {path} 已不存在,已解除钉住"),
    "compacted": ("Context compacted ({level})", "上下文已压缩({level})"),
    "forkedHere": ("Forked a new session from here", "已从此处分叉出新会话"),
    "forkedHereCompacted": ("Forked a new session from here (the fork point is before a context "
                            "compaction, so what came along is the summary from before it)",
                            "已从此处分叉出新会话(分叉点早于一次上下文压缩,带过去的是此前的压缩摘要)"),
    "cannotDeleteWhileRunning": ("The agent is running — sessions cannot be deleted yet", "agent 运行中,暂不能删除会话"),
    "forkedFromAnother": ("Forked from another session", "分叉自另一会话"),
})

# ---------------------------------------------------------------- editor
add("filePreview.", {
    "translate": ("Translate", "对照翻译"),
    "translateHint": ("Read this PDF beside its translation, page by page ({key})", "按页与译文并排对照阅读这份 PDF ({key})"),
    "pageOf": ("{n} / {total}", "{n} / {total}"),
    "loading": ("Loading the PDF…", "正在载入 PDF…"),
})

add("editor.", {
    "privateBlockHint": ("Private block — invisible to the AI", "私密块——对 AI 不可见"),
    "clipboardEmpty": ("Nothing to paste from the clipboard", "剪贴板没有可粘贴的内容"),
    "openLinkNamed": ("Open link {target}", "打开链接 {target}"),
    "openLink": ("Open link", "打开链接"),
    "cut": ("Cut", "剪切"),
    "copy": ("Copy", "复制"),
    "paste": ("Paste", "粘贴"),
    "deleteNote": ("Delete this note (move to trash)", "删除当前笔记(移入废纸篓)"),
    "deleteFile": ("Delete this file (move to trash)", "删除当前文件(移入废纸篓)"),
    "confirmDelete": ("Confirm delete", "确认删除"),
    "welcomeKeys": ("{open} open a note · {palette} command palette", "{open} 打开笔记 · {palette} 命令面板"),
    "copiedCode": ("Code copied", "已复制代码"),
    "copyCode": ("Copy code", "复制代码"),
    "copyCodeShort": ("Copy", "复制"),
    "image": ("image", "图片"),
    "imageNotLoaded": ("Image not loaded in Reader — click to view it in the browser · {url}",
                       "图片未在投影中加载,点击在浏览器查看 · {url}"),
    "closeEsc": ("Close (Esc)", "关闭 (Esc)"),
    "badDate": ("Unparseable date — this date marker is ignored; the format is YYYY-MM-DD",
                "日期解析失败，这个日期标记会被忽略 — 格式应为 YYYY-MM-DD"),
    "attrOutside": ("Not read — a task's attributes go at the end of the line, after its text",
                    "未读取 — 任务属性要放在行尾、正文之后"),
    "attrDuplicate": ("Not read — the same attribute appears again later on the line, and the last one counts",
                      "未读取 — 同一属性在后面又出现了一次，以最后一个为准"),
    "copied": ("Copied", "已复制"),
    "copyFailed": ("Copy failed", "复制失败"),
    "copyImage": ("Copy image", "复制图片"),
    "zoom": ("Zoom in", "放大查看"),
    "properties": ("Properties", "属性"),
    "placeholder": ("Start writing…", "开始书写…"),
    "openWithSystem": ("Open with the system app", "用系统程序打开"),
})

# ---------------------------------------------------------------- query / run blocks
add("queryBlock.", {
    "cannotParse": ("Cannot parse: {line}", "无法解析: {line}"),
    "kindMustBe": ("kind must be one of {list}", "kind 须为 {list}"),
    "stateMustBe": ("task_state must be one of {list}", "task_state 须为 {list}"),
    "fieldEmpty": ("{key} cannot be empty", "{key} 不能为空"),
    "tagEmpty": ("tag cannot be empty", "tag 不能为空"),
    "dueBeforeFormat": ("due_before must be YYYY-MM-DD or today", "due_before 须为 YYYY-MM-DD 或 today"),
    "limitPositive": ("limit must be a positive integer", "limit 须为正整数"),
    "unknownField": ("Unknown field: {key} (available: {fields})", "未知字段: {key}(可用: {fields})"),
    "fieldFormat": ("field must be `key`, `key = value`, `key != value`, `key < value`, `key > value` or `key ~ text`", "field 须为 `key`、`key = 值`、`key != 值`、`key < 值`、`key > 值` 或 `key ~ 文本`"),
    "groupFormat": ("group must be note, kind or a field key", "group 须为 note、kind 或一个字段名"),
    "sortFormat": ("sort must be note, kind, text or a field key, optionally followed by asc or desc", "sort 须为 note、kind、text 或一个字段名，可后接 asc 或 desc"),
    "none": ("None", "无"),
    "kind.paragraph": ("Paragraphs", "段落"),
    "kind.heading": ("Headings", "标题"),
    "kind.list_item": ("List items", "列表项"),
    "kind.task": ("Tasks", "任务"),
    "kind.claim": ("Claims", "断言"),
    "kind.evidence": ("Evidence", "证据"),
    "kind.code_block": ("Code blocks", "代码块"),
    "kind.private": ("Private blocks", "私密块"),
    "querying": ("Querying…", "查询中…"),
    "hits": ("vemry-query · {n} hits", "vemry-query · {n} 条"),
    # The dirty guard's wording for a write from a query row's chip
    "act.setField": ("changing the field", "修改字段"),
    "noHits": ("No matches", "无匹配"),
})

add("wikilink.", {
    "wholeNote": ("Whole note", "整篇笔记"),
    "sectionNotes": ("Notes", "笔记"),
    "sectionHeadings": ("Headings", "标题"),
    "sectionBlocks": ("Blocks", "块"),
    "sectionDates": ("Dates", "日期"),
    "sectionTaskAttrs": ("Task attributes", "任务属性"),
    "today": ("today", "今天"),
    "tomorrow": ("tomorrow", "明天"),
    "dayAfter": ("day after tomorrow", "后天"),
    "nextMonday": ("next Monday", "下周一"),
    "blockChanged": ("This block changed since it was indexed — save and try again", "块内容已变化，索引尚未更新，稍后再试"),
    "mintedNotInserted": ("Block id ^{id} was created but the cursor moved; the link was not inserted", "已创建块 id ^{id}，但光标已移动，未插入链接"),
})

add("prov.", {
    "origin.you": ("You", "你"),
    "origin.agent": ("AI", "AI"),
    "origin.clip": ("Clip", "剪藏"),
    "origin.capture": ("Capture", "速记"),
    "session": ("session {id}", "会话 {id}"),
    "sessionUnknown": ("session unknown", "会话未知"),
    "pending": ("unreviewed", "未审阅"),
    "pendingCount": ("Unreviewed {n}", "未审阅 {n}"),
    "pendingHint": ("AI blocks in this note you have not looked at yet — a marker, not a request; click to jump to the next one", "本篇你还没看过的 AI 块——只是标记，不是待办；点击跳到下一个"),
    "acceptBlock": ("Mark as reviewed", "标为已审阅"),
    "acceptRun": ("Mark these {n} blocks as reviewed", "这 {n} 个块标为已审阅"),
    "rejectRun": ("Revert this AI change ({n} blocks)", "撤回这处 AI 修改（{n} 个块）"),
    "showReplaced": ("Show what this write replaced", "查看写入前的内容"),
    "hideReplaced": ("Hide what this write replaced", "收起写入前的内容"),
    "replacedLabel": ("Before this write", "写入前"),
    "prevHint": ("Previous unreviewed AI change ({key})", "上一处未审阅的 AI 修改（{key}）"),
    "nextHint": ("Next unreviewed AI change — past this note's end, the next note ({key})", "下一处未审阅的 AI 修改，本篇看完则到下一篇（{key}）"),
    "allDone": ("No unreviewed AI blocks left", "没有待审阅的 AI 块了"),
    "acceptAll": ("Mark all in this note as reviewed", "本篇全部标为已审阅"),
    "acceptAllShort": ("✓ This note", "✓ 本篇"),
    "pendingVault": ("Unreviewed in {m} notes · {n} blocks", "待审阅 {m} 篇 · {n} 块"),
    "pendingOthers": ("Other notes {m} · {n} blocks", "其它 {m} 篇 · {n} 块"),
    "pendingListHint": ("The notes with unreviewed AI blocks — click to list them", "有未审阅 AI 块的笔记，点击列出"),
    "pendingListTitle": ("Notes with unreviewed AI blocks", "有未审阅 AI 块的笔记"),
    "acceptVaultConfirm": ("Click again to mark every note", "再点一次，全库标为已审阅"),
    "acceptVault": ("Mark every unreviewed AI block in the vault as reviewed", "全库未审阅的 AI 块全部标为已审阅"),
    "acceptedVault": ("Marked {n} blocks in {m} notes as reviewed", "已把 {m} 篇里的 {n} 个块标为已审阅"),
    "noPendingVault": ("No unreviewed AI blocks in the vault", "本库没有未审阅的 AI 块"),
    "accepted": ("Marked as reviewed", "已标为已审阅"),
    "acceptedAll": ("Marked {n} blocks as reviewed", "已把 {n} 个块标为已审阅"),
    "rejectBlock": ("Revert AI change", "撤回 AI 修改"),
    "rejectAll": ("Revert all unreviewed AI changes in this note", "撤回本篇全部未审阅的 AI 修改"),
    "rejectAllShort": ("✕ This note", "✕ 本篇"),
    "reverted": ("Reverted {n} changes", "已撤回 {n} 处修改"),
    "revertedPartly": ("Reverted {n} changes; {m} could not be located", "已撤回 {n} 处修改，{m} 处找不到原位"),
    "revertMissed": ("Cannot locate what this write replaced (the text around it has moved on); edit by hand", "找不到这处写入替换前的内容（周围文字已变动），请手动处理"),
    "revertStale": ("The note changed meanwhile; try again", "笔记刚刚有变动，请重试"),
    "noPending": ("No unreviewed AI blocks in this note", "本篇没有未审阅的 AI 块"),
    "noPendingHere": ("The block at the cursor is not an unreviewed AI block", "光标所在块不是未审阅的 AI 块"),
    "overlayOn": ("Origin overlay on", "已显示出处"),
    "overlayOff": ("Origin overlay off", "已隐藏出处"),
})

add("field.", {
    "sectionValues": ("Values", "取值"),
    "sectionKeys": ("Fields", "字段"),
    "today": ("today", "今天"),
    "tomorrow": ("tomorrow", "明天"),
    "notesWithKey": ("{n} notes", "{n} 篇笔记"),
    "fromType": ("type {name}", "类型 {name}"),
    "pickDate": ("Pick a date…", "选一个日期…"),
    "empty": ("empty", "未填"),
    "fillHint": ("Click to fill in", "点击填写"),
    "enterValue": ("Type a value", "输入取值"),
    "remove": ("Remove field", "移除字段"),
    # A state field (a `type: state` key of a block type): the menu's first entry moves to the next step
    "advance": ("Advance to {value}", "推进到 {value}"),
})

add("embed.", {
    "loading": ("Loading…", "加载中…"),
    "notFound": ("Block not found", "未找到该块"),
    "noteNotFound": ("Note not found", "未找到该笔记"),
    "open": ("Open source", "打开来源"),
    "edit": ("Edit reference", "编辑引用"),
    "recursive": ("This embed contains itself", "嵌入了自身"),
    "thisNote": ("this note", "本笔记"),
})

# The `/` menu is English in every language, on purpose: its entries are typed
# commands (`/task`, `/quote`, `/embed`), and a command name that changes with the
# interface language cannot be typed from memory. The zh-CN column therefore
# repeats the English text rather than translating it. The block menu's "turn
# into" list (`block.kind.*`) is a clicked menu and stays translated.
add("slash.", {
    "section.basic": ("Basic", "Basic"),
    "section.types": ("Types", "Types"),
    "section.blocks": ("Blocks", "Blocks"),
    "section.refs": ("References", "References"),
    "heading1.title": ("Heading 1", "Heading 1"),
    "heading1.desc": ("A top-level section heading", "A top-level section heading"),
    "heading2.title": ("Heading 2", "Heading 2"),
    "heading2.desc": ("A section heading", "A section heading"),
    "heading3.title": ("Heading 3", "Heading 3"),
    "heading3.desc": ("A subsection heading", "A subsection heading"),
    "bullet.title": ("Bulleted list", "Bulleted list"),
    "bullet.desc": ("A list item", "A list item"),
    "numbered.title": ("Numbered list", "Numbered list"),
    "numbered.desc": ("An ordered item", "An ordered item"),
    "task.title": ("Task", "Task"),
    "task.desc": ("A checkbox item", "A checkbox item"),
    "quote.title": ("Quote", "Quote"),
    "quote.desc": ("A quoted block", "A quoted block"),
    "callout.title": ("Callout", "Callout"),
    "callout.desc": ("A typed callout — pick the type next", "A typed callout — pick the type next"),
    "private.title": ("Private block", "Private block"),
    "private.desc": ("A block the AI never sees", "A block the AI never sees"),
    "claim.title": ("Claim", "Claim"),
    "claim.desc": ("A claim block other notes can support with evidence", "A claim block other notes can support with evidence"),
    "evidence.title": ("Evidence", "Evidence"),
    "evidence.desc": ("An evidence block — pick the claim it supports next", "An evidence block — pick the claim it supports next"),
    "code.title": ("Code block", "Code block"),
    "code.desc": ("A fenced code block", "A fenced code block"),
    "js.title": ("Runnable JS block", "Runnable JS block"),
    "js.desc": ("A JavaScript block with a Run button (read-only sandbox)", "A JavaScript block with a Run button (read-only sandbox)"),
    "query.title": ("Query block", "Query block"),
    "query.desc": ("A live vemry-query over the vault's blocks", "A live vemry-query over the vault's blocks"),
    "table.title": ("Table", "Table"),
    "table.desc": ("A 2×2 table", "A 2×2 table"),
    "hr.title": ("Divider", "Divider"),
    "hr.desc": ("A horizontal rule", "A horizontal rule"),
    "date.title": ("Today's date", "Today's date"),
    "date.desc": ("Insert today as YYYY-MM-DD", "Insert today as YYYY-MM-DD"),
    "link.title": ("Link to a note", "Link to a note"),
    "link.desc": ("[[ — pick a note, then a heading or block", "[[ — pick a note, then a heading or block"),
    "blockref.title": ("Block reference", "Block reference"),
    "blockref.desc": ("@ — reference a note, heading, block or date", "@ — reference a note, heading, block or date"),
    "embed.title": ("Embed", "Embed"),
    "embed.desc": ("![[ — embed a note, section or block", "![[ — embed a note, section or block"),
})

add("block.", {
    "foldedLines": ("{n} lines", "{n} 行"),
    "fold": ("Fold", "折叠"),
    "unfold": ("Unfold", "展开"),
    "title": ("Block", "块"),
    "handleTip": ("Drag to move · click for the block menu", "拖动移动 · 点击打开块菜单"),
    "insertBelow": ("Insert below", "在下方插入"),
    "copyLink": ("Copy block link", "复制块链接"),
    "copyEmbed": ("Copy block embed", "复制块嵌入"),
    "turnInto": ("Turn into", "转换为"),
    "moveUp": ("Move up", "上移"),
    "moveDown": ("Move down", "下移"),
    "indent": ("Indent", "缩进"),
    "outdent": ("Outdent", "取消缩进"),
    "duplicate": ("Duplicate block", "复制块"),
    "delete": ("Delete block", "删除块"),
    "askAgent": ("Ask AI about this block", "就此块问 AI"),
    "askPrivate": ("This note is in a privacy zone — the AI cannot read it", "这篇笔记在隐私区，AI 读不到它"),
    "changedRetry": ("The note changed while the id was being prepared — try again", "准备块 ID 时笔记已改动，请重试"),
    "noBlockHere": ("No block at the cursor", "光标处没有块"),
    "kind.paragraph": ("Paragraph", "段落"),
    "kind.heading1": ("Heading 1", "标题 1"),
    "kind.heading2": ("Heading 2", "标题 2"),
    "kind.heading3": ("Heading 3", "标题 3"),
    "kind.bullet": ("Bulleted list", "项目符号"),
    "kind.numbered": ("Numbered list", "编号列表"),
    "kind.task": ("Task", "任务"),
    "kind.quote": ("Quote", "引用"),
    "kind.callout": ("Callout", "标注"),
    "kind.code": ("Code block", "代码块"),
    "kind.private": ("Private block", "私密块"),
    "makePrivate": ("Make private", "设为私密"),
    "unmakePrivate": ("Remove privacy", "取消私密"),
})

add("runBlock.", {
    "run": ("Run", "运行"),
    "running": ("Running…", "运行中…"),
    "noOutput": ("(no output)", "(无输出)"),
    "clearOutput": ("Clear output", "清除输出"),
    # the lab of a block that called vemry.lab.*: live while its controls re-run it
    "live": ("Live", "实时"),
    "pin": ("Pin the values into the code", "把当前参数写回代码"),
    "pinDone": ("Values written into the code", "参数已写回代码"),
    "pinPartial": ("Values written; not found as literals in the code: {names}", "参数已写回;代码里找不到字面量的有:{names}"),
    "pinNothing": ("Nothing to pin: the values are unchanged, or the code does not declare them as literals", "没有可写回的值:参数没变,或代码里不是字面量"),
    "pinNoRange": ("Pin is not available for a code block inside a quote", "引用块里的代码块不支持写回"),
    "rerun": ("Run again", "重新运行"),
    "closeLab": ("Close the lab", "关闭实验"),
    "tableView": ("Show as a table", "以表格查看"),
    "chartView": ("Show as a chart", "以图表查看"),
    "experiment": ("Experiment", "实验"),
    "editCode": ("Edit the code", "编辑代码"),
    "codeChanged": ("The code changed — run again", "代码已改动,重新运行"),
    # the fallback name of a chart's value column, when the block gave the view none
    "value": ("value", "值"),
    "nextStep": ("Next", "下一步"),
    "allSteps": ("All", "全部"),
    "resetSteps": ("Reset", "重来"),
})

# ---------------------------------------------------------------- VEMRY.md prompts (sent to the model)
add("memoryPrompt.", {
    "draft": (
        "Draft VEMRY.md for this vault — it is the constitution injected into your system prompt, and "
        "you read it at the start of every turn from now on.\n\n"
        "Survey before you write: browse the folder structure and how tags are actually used, sample a "
        "few representative notes, and infer the real habits around naming, linking and organisation "
        "(do not invent what you are unsure of).\n\n"
        "Then create VEMRY.md with write_note. Requirements:\n"
        "- Short (under 50 lines), conventions and preferences only; the folder list and tag counts are "
        "already injected into the system prompt, so do not repeat them\n"
        "- Suggested sections: Structure (what each directory and key note is for), Conventions (tags, "
        "links, naming habits), AI behaviour preferences\n"
        "- Anything you are unsure about goes in a closing “To confirm” section as questions for me to answer",
        "请为这个 vault 起草 VEMRY.md——它是注入你 system prompt 的「宪法」,以后每轮对话你都会先读到它。\n\n"
        "先调研再动笔:浏览目录结构与标签的实际用法,抽样读几篇有代表性的笔记,归纳命名、链接与组织的真实习惯(不确定的不要编造)。\n\n"
        "然后用 write_note 创建 VEMRY.md,要求:\n"
        "- 简短(50 行以内),只写约定与偏好;文件夹清单和标签统计已自动注入 system prompt,不要重复罗列\n"
        "- 建议分节:结构(各目录/关键笔记的用途)、约定(标签、链接、命名习惯)、AI 行为偏好\n"
        "- 拿不准的习惯,在文末「待确认」一节用问句列出,我来补充"),
    "review": (
        "Review VEMRY.md: read it, then use your tools to check it against the vault as it is now — is "
        "anything out of date, has a new habit formed that is worth recording, is anything long-winded "
        "enough to trim? Propose changes with edit_note if there are any (keep it short); if it is still "
        "accurate, say so.",
        "请审阅 VEMRY.md:先读它,再用工具对照 vault 现状抽查——描述是否过时、有没有新形成的习惯值得补记、"
        "有没有冗长可精简的内容。需要修改就用 edit_note 提出(保持简短);没有问题就说明它仍然准确。"),
})

# A key can carry the same name in both catalogs and still be broken if the two
# translations disagree on placeholder names — the caller passes one set, so the
# other language renders a literal `{foo}`. Neither `pnpm check` nor the key-set
# test can see that, so it is caught here and again in src-tauri/src/i18n.rs.
_PLACEHOLDER = re.compile(r"\{(\w+)\}")
_bad = [k for k, (en, zh) in M.items()
        if set(_PLACEHOLDER.findall(en)) != set(_PLACEHOLDER.findall(zh))]
if _bad:
    raise SystemExit("placeholder mismatch between languages: " + ", ".join(sorted(_bad)))

# ---------------------------------------------------------------- VEMRY.md file content (Rust-owned)
add("memoryFile.", {
    "header": (
        "# VEMRY.md — conventions for this vault\n\nThis file is injected into the AI agent's system "
        "prompt: it is the constitution it reads to understand this vault.\n",
        "# VEMRY.md — 本知识库的约定\n\n这份文件会注入 AI agent 的 system prompt，是它理解这个知识库的「宪法」。\n"),
    # The section memos are appended into. Lookup accepts every name vemry has
    # ever written (vault.rs::MEMORY_SECTIONS), so switching language does not
    # strand an existing section — only creation uses the current language.
    "section": ("Conventions", "约定"),
    "template": (
        "Describe your structure and preferences in your own words, for example:\n\n"
        "## Structure\n"
        "- `inbox.md`: quick capture — unsorted thoughts land here first\n"
        "- `projects/`: work in progress, one map-of-content note per project\n"
        "- `daily/`: journal\n\n"
        "## Conventions\n"
        "- Tags: #idea #reading #todo\n"
        "- Linking: create a [[link]] the first time a concept appears\n"
        "- AI preferences: say what you intend before editing a note; put surveys in `reviews/`\n\n"
        "## Organising files\n"
        "- Where things go: clips in `clips/`, attachments in `attachments/<note path>/`, one hub note per project folder\n"
        "- Naming: notes by topic, dates as YYYY-MM-DD, no version numbers in file names\n"
        "- Every note carries `project:` when it belongs to one; tidy-ups fill it in\n"
        "- Iterations are a field on the block: `iteration:: v0.8` on tasks and requirements; the board grouped by it is the iteration view\n",
        "用你自己的话描述组织结构与偏好，例如：\n\n"
        "## 结构\n"
        "- `inbox.md`：速记收集箱，未整理的想法先进这里\n"
        "- `projects/`：进行中的项目，每个项目一个 MOC 笔记\n"
        "- `daily/`：日记\n\n"
        "## 约定\n"
        "- 标签体系：#idea #reading #todo\n"
        "- 建链原则：概念第一次出现时建 [[链接]]\n"
        "- AI 行为偏好：修改笔记前先说明意图；综述放在 `reviews/` 目录\n\n"
        "## 资料组织约定\n"
        "- 落点：剪藏进 `clips/`，附件进 `attachments/<笔记路径>/`，每个项目文件夹一篇枢纽笔记\n"
        "- 命名：笔记按主题命名，日期写 YYYY-MM-DD，文件名不带版本号\n"
        "- 属于某个项目的笔记带 `project:`，整理时补齐\n"
        "- 迭代是块上的字段：任务与需求写 `iteration:: v0.8`，看板按它分组就是迭代视图\n"),
})

# ---------------------------------------------------------------- errors surfaced from Rust
# `err.*` mirrors vemry_core::PathReject::key(); `ipc.*` are src-tauri's own.
add("err.", {
    "notFound": ("Does not exist: {path}", "不存在: {path}"),
    "notAFile": ("Not a file: {path}", "不是文件: {path}"),
    "notADirectory": ("Not a directory: {path}", "不是目录: {path}"),
    "tooLarge": ("File too large: {path}", "文件过大: {path}"),
    "notUtf8": ("Not UTF-8 text: {path}", "不是 UTF-8 文本: {path}"),
    "exists": ("Already exists: {path}", "已存在: {path}"),
    "attachmentIsMd": ("An attachment cannot be a .md file: {path}", "附件不能是 .md: {path}"),
    "mdNeedsNoteApi": (".md must be written through the note API: {path}", ".md 须通过笔记接口写入: {path}"),
    "metaNotANote": ("A file under .vemry/ is configuration, not a note: {path}", ".vemry/ 下的文件是配置,不是笔记: {path}"),
    "isTheVaultItself": ("Cannot import the vault itself: {path}", "不能导入知识库自身: {path}"),
    "folderIntoItself": ("Cannot copy a folder into itself: {path}", "不能把文件夹复制到它自身内部: {path}"),
    "reservedName": ("{path} — the name uses characters or a reserved word Windows forbids",
                     "{path}(文件名含 Windows 不允许的字符或保留名)"),
    "escapes": ("Path escapes the vault: {path}", "路径越出知识库: {path}"),
    "empty": ("Empty {path}", "{path} 为空"),
    "blockNotFound": ("No block {id}", "找不到块 {id}"),
    "blockChanged": ("Block {id} changed since it was read — try again", "块 {id} 已改动，请重试"),
    "blockNotIdable": ("A code block, table or rule cannot carry a block id", "代码块、表格或分割线不能挂块 ID"),
})

# ---------------------------------------------------------------- native window titles and vault-facing clip copy
# Rust owns these surfaces (window chrome, and the source line written into a
# clip note): the catalog is still the only place the wording lives.
add("window.", {
    "agentTitle": ("AI chat — vemry", "AI 对话 — vemry"),
    "browserTitle": ("vemry browser", "vemry 浏览"),
    "readerTitle": ("vemry reading fetch", "vemry 阅读取数"),
})

add("clip.", {
    # The `> Source:<url>` line at the top of a clipped note. It is written into
    # the vault, so it follows the interface language at the moment of clipping.
    "sourceLabel": ("Source", "来源"),
    "localizeStarted": ("Fetching the images into clips/assets — the note updates when they land", "正在把图片存进 clips/assets，存好后笔记自动更新"),
    "localizeNoSource": ("This note has no source: URL to fetch its images with", "这篇笔记没有 source: 来源地址，无法取图"),
    "localizeOff": ("Image localisation is off in Settings", "设置里关闭了剪藏图片本地化"),
})

add("ipc.", {
    "fileExists": ("A file with that name already exists: {path}", "已存在同名文件: {path}"),
    "followPrivate": ("A private note cannot be followed by the AI", "私密笔记不能让 AI 跟随"),
    "dirNotEmpty": ("That folder exists and is not empty — pick another place for a new vault, or open "
                    "the existing one with “Open vault”",
                    "目标目录已存在且不为空——新建请换一个位置,已有的库走「打开知识库」"),
    "imageFormatUnsupported": ("The system clipboard does not support this image format",
                               "图片格式不受系统剪贴板支持"),
    "copyImageUnsupported": ("Copying images is not supported on this platform yet", "当前平台暂不支持复制图片"),
    "httpOnly": ("Only http(s) links are supported: {url}", "仅支持 http(s) 链接: {url}"),
    "agentRunning": ("The agent is already running", "agent 已在运行中"),
    "noApiKey": ("No API key configured: fill one in under Settings, or set the ZHIPU_API_KEY "
                 "environment variable",
                 "未配置智谱 API key：请在设置中填写，或设置 ZHIPU_API_KEY 环境变量"),
    "nothingToRetry": ("There is no message in this session to retry", "会话里没有可重试的消息"),
    "runningNoEditResend": ("The agent is running — cannot edit and resend", "agent 运行中,无法编辑重发"),
    "sessionEmpty": ("The session is empty", "会话为空"),
    "runningNoFork": ("The agent is running — cannot fork", "agent 运行中,无法分叉"),
    "noSessionToFork": ("There is no current session to fork", "当前没有会话,无从分叉"),
    "runningNoNewSession": ("The agent is running — cannot start a new session", "agent 运行中,无法新建会话"),
    "runningNoSwitch": ("The agent is running — cannot switch sessions", "agent 运行中,无法切换会话"),
    "runningNoDelete": ("The agent is running — cannot delete the current session", "agent 运行中,无法删除当前会话"),
    "contentEmpty": ("The content is empty", "内容为空"),
    "typeNameEmpty": ("A block type needs a name", "块类型需要一个名字"),
    "typeNotFound": ("That block type no longer exists", "该块类型已不存在"),
    "typeFrontmatterBroken": ("The type's frontmatter is not valid YAML, so it cannot be switched — open the file and fix it",
                              "该类型的 frontmatter 不是有效的 YAML，无法切换，请打开文件修正"),
    "memoryExists": ("VEMRY.md already exists", "VEMRY.md 已存在"),
    "skillNotFound": ("That skill no longer exists", "该 skill 已不存在"),
    "skillNameEmpty": ("A skill needs a name", "skill 需要一个名字"),
    "skillNameBuiltin": ("A built-in skill is already called {name} — customise it instead, or pick another name",
                         "内置 skill 已经叫 {name}——请改为自定义它，或换一个名字"),
    "skillFileTaken": ("{file} in .vemry/skills/ already holds a skill of a different name — rename or remove it first",
                       ".vemry/skills/ 下的 {file} 已是另一个名字的 skill——请先重命名或移走它"),
    "trashFailed": ("Could not move it to the trash: {error}", "移入废纸篓失败: {error}"),
    "rulesUpdateRunning": ("A rule update is already running", "拦截规则更新正在进行中"),
    "noRuleSource": ("No rule subscription source is configured", "没有配置拦截规则订阅源"),
    "noRuleFile": ("No rule file was chosen", "没有选择规则文件"),
    # Rendered as one item of settings.rulesPartial's list, after "{url}: " — hence
    # the lower case, unlike every standalone message above
    "notAFilterList": ("the response is not a filter list", "返回的内容不是规则表"),
})

out = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.makedirs(os.path.join(out, "locales"), exist_ok=True)
for lang, idx in (("en", 0), ("zh-CN", 1)):
    data = {k: v[idx] for k, v in M.items()}
    p = os.path.join(out, "locales", f"{lang}.json")
    with open(p, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2, sort_keys=True)
        f.write("\n")
    print(f"wrote {p}  ({len(data)} keys)")
