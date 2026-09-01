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
    "reading": ("Reader", "阅读投影"),
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
        ("Open in the built-in knowledge browser", "在内置知识浏览器中打开"),
    ),
    "open-browser": (
        ("Open browser", "打开浏览器"),
        ("Focus the browser window, or reopen the last page", "聚焦浏览窗口,或重开上次浏览的页面"),
    ),
    "clear-browser-history": (
        ("Clear browsing history", "清除浏览历史"),
        ("Empty the built-in browser's history (bookmarks are untouched)",
         "清空内置浏览器的历史记录(收藏不受影响)"),
    ),
    "show-browser": (
        ("Browser controls", "浏览控制"),
        ("Now part of the research panel: the current page card and clip actions",
         "已并入研究面板:当前页面卡 + 剪藏入口"),
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
    "init-research-skill": (
        ("Install research skill", "初始化研究方法论 skill"),
        ("Write .vemry/skills/research.md: search the vault before the web, citation "
         "discipline, how to record what you learn",
         "写入 .vemry/skills/research.md:先查库再搜索、引用纪律、沉淀约定"),
    ),
    "reveal-in-tree": (
        ("Reveal current note in file tree", "在文件树定位当前笔记"),
        ("Expand the left panel and scroll to the current note", "展开左栏并滚到当前笔记"),
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
    "view-reading": (
        ("View: Reader", "视图:阅读投影"),
        ("Return to the most recently opened web page", "回到最近打开的网页阅读投影"),
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
    "check-updates": (
        ("Check for updates", "检查更新"),
        ("Ask GitHub Releases for a newer version (progress shows in Settings)",
         "向 GitHub Releases 查询新版本(设置页可看进度)"),
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
})

# ---------------------------------------------------------------- command palette
add("palette.", {
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
})

# ---------------------------------------------------------------- shared view chrome
add("viewChrome.", {
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
    "summary.create": ("Create {path}", "创建新笔记 {path}"),
    "summary.overwrite": ("Overwrite {path}", "覆盖写入 {path}"),
    "summary.edit": ("Edit {path}", "编辑 {path}"),
    "summary.move": ("Move {path} → {toPath}", "移动 {path} → {toPath}"),
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
    "field.scheduled": ("start date", "开始日期"),
    "overdue_one": ("{n} day overdue", "逾期 {n} 天"),
    "overdue_other": ("{n} days overdue", "逾期 {n} 天"),
    "toggleDone": ("Toggle done", "切换完成状态"),
    "editDate": ("Change {field}", "修改{field}"),
    "editDateWith": ("Change {field} ({date})", "修改{field} {date}"),
    "setDate": ("Set {field}", "设定{field}"),
    "count_one": ("{n} task", "{n} 个任务"),
    "count_other": ("{n} tasks", "{n} 个任务"),
    "filterPlaceholder": ("Filter: task / note", "过滤:任务 / 笔记"),
    "empty1": ("No tasks yet. Write ", "没有任务。用 "),
    "empty2": (" in a note to create one; ", " 语法在笔记里创建；"),
    "empty3": (" means in progress, and ", " 表示进行中，"),
    "empty4": (" is a start date.", " 是开始日期。"),
    "noMatch": ("No tasks match “{query}”.", "没有匹配「{query}」的任务。"),
    "dirtyGuard": ("This note has unsaved changes — press {key} to save before {action}",
                   "当前笔记有未保存的修改，请先 {key} 保存再{action}"),
    "act.toggle": ("checking it off", "勾选任务"),
    "act.editDate": ("changing the date", "修改日期"),
    "act.move": ("moving the task", "移动任务"),
    "act.state": ("changing its state", "修改状态"),
    "act.remove": ("removing the task", "移除任务"),
})

add("agenda.", {
    "overdue": ("Overdue", "已过期"),
    "today": ("Today", "今天"),
    "week": ("Next 7 days", "未来 7 天"),
    "later": ("Later", "更晚"),
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
    "jumpToSource": ("Jump to the source line", "跳到原文"),
    "markDoing": ("Mark in progress", "标记进行中"),
    "markBackTodo": ("Mark back to to-do", "标回待办"),
    "markDone": ("Mark done", "标记完成"),
    "reopen": ("Reopen", "重新打开"),
    "removeTask": ("Remove task", "移除任务"),
    "removeConfirm": ("Remove it? The line is deleted from the note.", "确认移除？将从笔记原文删除"),
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
    "count_one": ("{n} claim", "{n} 断言"),
    "count_other": ("{n} claims", "{n} 断言"),
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
    "compactSummary": ("older conversation summarized", "旧对话摘要化"),
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
    "edit_note": ("Edit", "编辑"),
    "move_note": ("Move", "移动"),
    "propose_plan": ("Propose a plan", "提出计划"),
    "mixed": ("Tool calls", "工具调用"),
})

add("planDecision.", {
    "approve": ("Plan approved", "计划已确认"),
    "approve_edited": ("Plan approved with edits", "计划修改后确认"),
    "reject": ("Plan rejected", "计划被否决"),
})

add("decision.", {
    "allow": ("Allowed", "允许"),
    "allow_session": ("Allowed for the session", "会话内允许"),
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
    "pageBrowserSub": ("Current web page", "当前网页"),
    "pageReadingSub": ("Current reader page", "当前阅读页"),
    "heading": ("Add context", "添加上下文"),
    "headingWithQuery": ("Add context: {query}", "添加上下文:{query}"),
    "keys": ("↑↓ move · ↵ confirm · esc close", "↑↓ 选择 · ↵ 确认 · esc 关闭"),
    "empty": ("No match — Enter sends the text as-is", "无匹配——回车按原文发送"),
    "extracting": ("Extracting…", "抽取中…"),
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

# ---------------------------------------------------------------- clip bar (browser window)
add("clipBar.", {
    "label": ("Clip bar", "剪藏工具条"),
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
    "pickTargetHint": ("Clip the selection into a chosen note or section (picking pins it; one click after that)",
                       "选区剪入指定笔记/小节(选定即剪并钉住,之后单击直达)"),
    "unpinHint": ("Unpin and go back to the default target", "解除钉住,恢复默认落点"),
    "clipPage": ("Page→clips", "整页→clips"),
    "clipPageHint": ("Extract the article, convert to Markdown, save in clips/", "整页抽正文转 markdown,存 clips/"),
    "clipLink": ("Link→note", "链接→笔记"),
    "clipLinkHint": ("Insert this page as a reference in the main window's current note",
                     "当前页链接作为参考条目插入主窗当前笔记"),
    "openClippedHint": ("Open the local clip note {path} (back in the main window)",
                        "打开本地剪藏笔记 {path}(回主窗)"),
    "undock": ("Detach", "分离"),
    "undockHint": ("Detach the window and place it freely", "分离窗口,自由摆放"),
    "dock": ("Dock", "吸附"),
    "dockHint": ("Dock to the right of the main window", "吸附到主窗右侧"),
})

# ---------------------------------------------------------------- research panel
add("agentPanel.", {
    "newSession": ("New session", "新会话"),
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
    "currentModel": ("Current model {model} (changeable in Settings)", "当前模型 {model}(设置里可换)"),
    "searchInactive": ("Search inactive", "搜索未生效"),
    "searchInactiveHint": (
        "Web search is on, but this endpoint is not api.deepseek.com, so this session will not search. "
        "Change the endpoint in Settings, or turn the switch off to hide this.",
        "已开启「AI 联网搜索」,但当前端点不是 api.deepseek.com,会话不会联网——可在设置里换端点,或关掉开关消除此提示"),
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
    "turns": ("{n} turns", "{n} 轮"),
    "confirmDelete": ("Confirm delete", "确认删除"),
    "openClippedNote": ("Open the local clip note", "打开本地剪藏笔记"),
    "adblockOn": ("Ad blocking active · {n} rules", "去广告已生效 · {n} 条规则"),
    "adblockFailed": ("Ad blocking failed: {error}", "去广告失败: {error}"),
    "reload": ("Reload", "刷新"),
    "redock": ("Dock to the right of the main window again", "重新吸附到主窗右侧"),
    "closeBrowser": ("Close the browser window", "关闭浏览窗口"),
    "selectionToTargetHint": ("Clip the browser selection into {path}; {key}-click to change target",
                              "浏览器选区剪入 {path};{key} 点击换目标"),
    "selectionDefaultHint": ("Turn the browser selection into a quote block at the cursor in the current "
                             "note; {key} or ▾ picks a target note",
                             "浏览器里选中的文字变引用块,插入当前笔记光标处;{key} 或 ▾ 选择目标笔记"),
    "clipPageHint": ("Extract the article, convert to Markdown, save in clips/ ({key})",
                     "整页抽正文转 markdown,存 clips/ ({key})"),
    "clipLinkHint": ("Insert this page as a reference in the current note", "当前页链接作为参考条目插入当前笔记"),
    "attachContextHint": ("Extract this page and hand it to the AI as reference for your next message",
                          "抽取当前页正文,作为下一条消息的参考资料递给 AI"),
})

# ---------------------------------------------------------------- agent chat
add("chat.", {
    "skillInstalled": ("Research skill installed (.vemry/skills/research.md — disable it in Settings)",
                       "已安装研究方法论 skill(.vemry/skills/research.md,可在设置中停用)"),
    "planRejectedByUser": ("Rejected by the user", "用户否决"),
    "planApproved": ("Approved ✓", "已确认 ✓"),
    "planApprovedEdited": ("Approved with edits ✓", "已修改并确认 ✓"),
    "planCancelled": ("Cancelled", "已取消"),
    "planRejected": ("Rejected", "已否决"),
    "usageCache": (" · cache {pct}%", " · 缓存 {pct}%"),
    "usageStep": ("Step {n}/{max} · context {tokens}", "步 {n}/{max} · 上下文 {tokens}"),
    "answerSignature": ("AI research session", "AI 研究会话"),
    "copiedSelection": ("Selected excerpt copied", "已复制选中片段"),
    "copiedAnswer": ("Answer copied as Markdown", "已复制回答 Markdown"),
    "researchNoteName": ("Research {date}", "研究 {date}"),
    "savedAs": ("Saved as {path}", "已存为 {path}"),
    "forked": ("Forked into a new session (the original stays in the list)", "已分叉出新会话(原会话保留在列表里)"),
    "continueWord": ("continue", "继续"),
    "clipped": ("Clipped → {path}", "已剪藏 → {path}"),
    "starter1": ("Go through my recent notes and find a theme worth digging into",
                 "梳理我最近的笔记,找出值得深入的主题"),
    "starter2": ("Research a topic online and write a survey with citations",
                 "联网调研一个话题,给出带引用的综述"),
    "starter3": ("Around the current note, find related notes and outside material",
                 "围绕当前笔记,找相关笔记与外部资料"),
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
    "deepseekNoImages": ("The DeepSeek endpoint has no image support and will error on send; "
                         "switch to glm-4.6v / qwen-vl in Settings",
                         "DeepSeek 端点不支持图片,发送会报错;可在设置里换 glm-4.6v / qwen-vl 等多模态模型"),
    "unreadableFile": ("{name} is in the vault, but it is not text or PDF — the model cannot read this format",
                       "{name} 已存入 vault,但不是文本/PDF,模型读不了这种格式"),
    "fileImages": ("Images", "图片"),
    "fileText": ("Text", "文本"),
    "fileAll": ("All files", "所有文件"),
    "effortMenuTitle": ("Thinking depth (this session)", "思考深度(本会话)"),
    "effortLabel": ("Thinking · {level}", "思考·{level}"),
    "effortThisSession": (" (this session)", "(本会话)"),
    "effortFollowGlobal": ("Follow the global setting ({level})", "跟随全局({level})"),
    "copySelection": ("Copy the selected excerpt", "复制选中片段"),
    "copy": ("Copy", "复制"),
    "insertAsQuote": ("Insert as a quote block at the cursor in the current note", "作为引用块插入当前笔记光标处"),
    "insert": ("Insert", "插入"),
    "sendToTarget": ("Send the quote block to {name}", "引用块送到 {name}"),
    "sendToTargetPlain": ("Send to {name}", "送到 {name}"),
    "pinFirst": ("Pin a note first with “▾ pick target”", "先用「▾ 选目标」钉住一篇笔记"),
    "pinWord": ("pin", "钉住"),
    "planProgress": ("Plan · {done}/{total}", "计划 · {done}/{total}"),
    "welcomeTitle": ("Research panel", "研究面板"),
    "welcomeBody": ("Ask, and the AI searches your notes (and the web) and answers with citations. "
                    "Click a citation to read the article; clip what is worth keeping back into the "
                    "vault, and the next answer uses it.",
                    "提问,AI 检索你的笔记(与网络)后给出带引用的回答;点击引用读正文,值得留的剪藏回知识库,下次回答会直接用上。"),
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
    "sourcesCount": ("Sources · {n}", "来源 · {n}"),
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
    "usageHint": ("Context {prompt} tokens · cache hits {cache}", "上下文 {prompt} tokens · 缓存命中 {cache}"),
    "usageLastTurn": ("Last turn: context {prompt} tokens · cache hits {cache} · output {completion}",
                      "上一轮:上下文 {prompt} tokens · 缓存命中 {cache} · 输出 {completion}"),
    "stop": ("Stop", "停止"),
    "backToBottom": ("↓ Back to the bottom", "↓ 回到底部"),
    "queued": ("Queued: {text}", "已排队:{text}"),
    "cancelQueued": ("Cancel the queued message", "取消排队消息"),
    "runningQueueHint": ("Running — type the next one and press Enter to queue it",
                         "运行中——输入下一条,回车排队…"),
    "composerAria": ("Ask a question or give a task", "提问或下达任务"),
    "addContextHint": ("Add context: a note, folder, selection or web page, or a file from your computer "
                       "(typing @ works too; images and files can be pasted straight in)",
                       "添加上下文:笔记 / 文件夹 / 选区 / 网页,或从电脑添加文件(输入 @ 亦可;图片和文件也可直接粘贴进输入框)"),
    "effortHint": ("Thinking depth: click to set it for this session (the global default is in Settings)",
                   "思考深度:点击调整本会话档位(全局默认在设置)"),
    "popOutHint": ("Press {key} again to pop out a separate window", "再按 {key} 弹出独立窗口"),
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
    "endpointDeepseek": ("DeepSeek (web search needs this endpoint)", "DeepSeek(联网搜索需此端点)"),
    "endpointOllama": ("Ollama, local", "Ollama 本机"),
    "endpointHint": ("Zhipu BigModel by default; any OpenAI-compatible endpoint works (Ollama, DeepSeek…)",
                     "默认智谱 BigModel；可换任意 OpenAI 兼容端点（Ollama、DeepSeek…）"),
    "model": ("Model", "模型"),
    "modelZhipuFlash": ("Zhipu · free, supports tool calls", "智谱 · 免费,支持工具调用"),
    "modelZhipu": ("Zhipu · flagship", "智谱 · 旗舰"),
    "modelDeepseekFlash": ("DeepSeek · light", "DeepSeek · 轻量"),
    "modelDeepseekChat": ("DeepSeek · chat", "DeepSeek · 对话"),
    "modelDeepseekReasoner": ("DeepSeek · reasoning (thinking)", "DeepSeek · 推理(thinking)"),
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
    "webSearchHint": ("Uses the DeepSeek Responses API's server-side web_search, active only when the "
                      "endpoint is api.deepseek.com. The provider bills searches separately from chat, "
                      "and it is off by default. Sources show up in the AI timeline and open in the "
                      "built-in browser.",
                      "走 DeepSeek Responses API 的服务端 web_search,仅端点为 api.deepseek.com 时生效;"
                      "搜索由 provider 按次计费,与本地功能无关,默认关闭。来源引用会显示在 AI 时间线,可点开内置浏览器。"),
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
    "downloadInstall": ("Download and install {version}", "下载并安装 {version}"),
    "restartToFinish": ("Restart to finish the update", "重启以完成更新"),
    "checking": ("Checking…", "检查中…"),
    "upToDate": ("Up to date", "已是最新版本"),
    "updateAvailable": ("Version {version} is available", "新版本 {version} 可用"),
    "installedRestart": ("Installed — takes effect after a restart", "已安装,重启后生效"),
    "autoCheck": ("Check for updates at startup", "启动时自动检查更新"),
    "autoCheckHint": ("Off by default (offline-first: without it, the app only goes online when you click "
                      "“check for updates”). On, it asks GitHub Releases once at startup and only tells "
                      "you about a new version; downloading and installing always need your confirmation, "
                      "and the package signature is verified.",
                      "默认关闭(离线优先:不开启则只有点「检查更新」才联网)。开启后启动时向 GitHub Releases "
                      "静默查询一次,发现新版只提示;下载与安装始终需要你确认,安装包经签名校验。"),
    "adblock": ("Ad blocking (subscription rules, native WebKit blocking, macOS only)",
                "去广告(订阅规则,WebKit 原生拦截,仅 macOS)"),
    "adblockActive": ("Active · {n} rules", "已生效 · {n} 条规则"),
    "adblockCompiling": ("Compiling rules…", "规则编译中…"),
    "adblockDownloading": ("Downloading rules…", "规则下载中…"),
    "adblockNoRules": ("No rules yet — they download when you open the first page, or fetch them now.",
                       "还没有规则——打开第一个页面时会自动下载,也可以现在就取。"),
    "openLinksIn": ("How links and citations open", "链接与引用的打开方式"),
    "openLinksReading": ("Reader (main window, centre pane — default)", "阅读投影(主窗中栏,默认)"),
    "openLinksInternal": ("Built-in knowledge browser", "内置知识浏览器"),
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
    "skills": ("AI skills (.vemry/skills/)", "AI Skills（.vemry/skills/）"),
    "skillsHint1": ("Instruction-level skills: the summary lives in the AI's system prompt, the body is "
                    "read on demand. Drop a ", "指令级技能:摘要常驻 AI 的 system prompt,正文按需读取。放置 "),
    "skillsHint2": (" in place and it is live; disable one by editing its `disabled` field. Changes take "
                    "effect from the next AI session.",
                    " 即生效;停用改写文件的 `disabled` 字段。改动自下一个 AI 会话生效。"),
    "editWithSystemApp": ("Open in the system default app to edit", "用系统默认应用打开编辑"),
    "edit": ("Edit", "编辑"),
    "noSkills": ("(no skills yet — opening the research panel for the first time installs the research "
                 "template)", "（暂无 skill——首次打开研究面板会自动安装研究方法论模板）"),
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

# ---------------------------------------------------------------- app shell / toasts
add("app.", {
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
    # new user reads, and it should say what the project page said.
    "tagline": ("Your knowledge stays in Markdown. Every view grows from it.",
                "知识留在 Markdown 里，一切视图由此生长。"),
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

# ---------------------------------------------------------------- vault store toasts
add("vaultStore.", {
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
    "updateAvailable": ("vemry {version} is out — update it in Settings", "vemry {version} 已发布,可在设置里更新"),
    "updateCheckFailed": ("Update check failed: {error}", "检查更新失败: {error}"),
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
    "querying": ("Querying…", "查询中…"),
    "hits": ("vemry-query · {n} hits", "vemry-query · {n} 条"),
    "noHits": ("No matches", "无匹配"),
})

add("runBlock.", {
    "run": ("Run", "运行"),
    "running": ("Running…", "运行中…"),
    "noOutput": ("(no output)", "(无输出)"),
    "clearOutput": ("Clear output", "清除输出"),
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
        "- AI preferences: say what you intend before editing a note; put surveys in `reviews/`\n",
        "用你自己的话描述组织结构与偏好，例如：\n\n"
        "## 结构\n"
        "- `inbox.md`：速记收集箱，未整理的想法先进这里\n"
        "- `projects/`：进行中的项目，每个项目一个 MOC 笔记\n"
        "- `daily/`：日记\n\n"
        "## 约定\n"
        "- 标签体系：#idea #reading #todo\n"
        "- 建链原则：概念第一次出现时建 [[链接]]\n"
        "- AI 行为偏好：修改笔记前先说明意图；综述放在 `reviews/` 目录\n"),
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
    "clipbarTitle": ("Clip", "剪藏"),
    "browserTitle": ("vemry browser", "vemry 浏览"),
    "readerTitle": ("vemry reading fetch", "vemry 阅读取数"),
})

add("clip.", {
    # The `> Source:<url>` line at the top of a clipped note. It is written into
    # the vault, so it follows the interface language at the moment of clipping.
    "sourceLabel": ("Source", "来源"),
})

add("ipc.", {
    "fileExists": ("A file with that name already exists: {path}", "已存在同名文件: {path}"),
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
    "memoryExists": ("VEMRY.md already exists", "VEMRY.md 已存在"),
    "researchSkillExists": ("The research skill already exists", "research skill 已存在"),
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
