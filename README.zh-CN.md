# vemry

**本地优先的 AI 研读工作台。**

论文、网页、PDF 都进来变成你自己磁盘上的纯 Markdown。你和 AI 一起读、对照翻译、提问、
做卡片；它引用的是读过的那个块，写入前先停在一份 diff 上，你标为隐私的内容它看不见。

[![CI](https://github.com/pangyexing/vemry/actions/workflows/ci.yml/badge.svg)](https://github.com/pangyexing/vemry/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/pangyexing/vemry?include_prereleases&sort=semver)](https://github.com/pangyexing/vemry/releases)
[![License](https://img.shields.io/badge/license-FSL--1.1--ALv2-blue)](#许可)
![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)
![No telemetry](https://img.shields.io/badge/telemetry-none-success)

[English](README.md) · [简体中文](README.zh-CN.md)

![vemry — 编辑器、文件树与大纲](docs/assets/hero.png)

---

vemry 把你的知识存成自己磁盘上的纯 Markdown 文件。没有数据库，没有账号，没有同步服务。

在这个文件夹之上，它只建一样派生物：一份块级索引。你看到的每个视图——日程、看板、
时间线、图谱、断言、证据、复习、会话回放——都是对它的一次查询。你的脚本查的是它，AI 的工具
查的也是它。没有第二条数据通道，这才是「AI 原生」的实指，而不是外挂一个聊天框。

这条边界反过来也成立：你标为隐私的内容在 Rust 层被滤掉，就在掌握文件访问能力的那一层。
不是「无权访问」，是不存在。

## 为什么是 vemry

### 读透一篇论文，留住每一点理解

贴一个 arXiv 链接、拖进一份 PDF，或者指一篇剪藏，说「研读这篇」。论文就成了知识库里一篇
能随手翻的笔记——每个小节、公式、图、表都是独立的块——然后按你的节奏一章一章读下去。每一章
都从你库里已经知道的东西讲起，标出哪些是新的；每一句都链到原文出处，随时一点就能回去核对。

用你自己的话把一章复述一遍，你会看到自己和原文偏在哪里。想动手试试这个想法？要一个实验，
章节里就多出一个可运行的块。没完全读懂的地方变成卡片，复习视图会在你快忘的时候把它们端回来。

读外文吃力？原文和译文并排打开——笔记、网页、PDF 都行，逐段对齐。一路上产出的一切——原文、
你的章节、你的卡片、译文——都是你文件夹里的纯 Markdown，走到哪里都打得开。

### 它就是个文件夹，你随时能走

你的知识库是你自己也写得出来的纯 Markdown。vemry 认的是标准 Markdown 的一个超集，
它每加一条记号，都在别的渲染器里降级成读得懂的东西——方言定义与逐条降级形态写在
[docs/SYNTAX.zh-CN.md](docs/SYNTAX.zh-CN.md)，所以「零锁定」是一份可以核对的文档，不是一句承诺。
索引是派生的、可重建的，删掉它什么也不会丢。

### 一份索引，长出所有视图

```
磁盘上的 Markdown  →  块索引      →  日程 · 看板 · 时间线 · 图谱
（唯一真相源）        （唯一派生层）   断言 · 证据 · 复习 · 会话回放
                                     你的 JS 脚本 · agent 的工具
```

七个投影，全是查询。在笔记里写一个 `vemry-query` 围栏，它就渲染成随索引更新的实时结果。
因为 agent 查的是你的视图查的同一个 `query()`，你能看到的它都算得出来，而不存在只为它
准备的数据。

![看板——全库任务按状态分列](docs/assets/kanban.png)

![图谱——同一份索引画成链接；那两篇隐私笔记你看得见，模型看不见](docs/assets/graph.png)

### 一切皆命令，一切可编程

应用的每一项能力都注册在同一个可搜索的命令面板里（⌘P），没有藏在菜单深处的功能。往
`.vemry/scripts/` 丢一个 `.js` 文件，保存即生效：它跑在 Rust 侧的 QuickJS 沙箱里，
无文件系统、无网络、无进程，只能经 `vemry.*` API 触到知识库。脚本可以注册命令面板条目、
订阅 vault 事件、按 cron 定时运行。

任意代码永不自动执行。打开一篇笔记不等于运行它：每条执行路径都始于你的某个动作，而
agent 根本没有能执行代码的工具。

### 标为隐私的东西，不存在

把一个目录、一篇笔记或单独一个 `> [!private]` 块标为隐私，agent 就永远不读、不写、也不会在
prompt 里见到它。这在 Rust 层的每个读写卡点上强制执行，而且一个隐私文件返回的错误与一个从来
不存在的文件**逐字节相同**——模型连「有东西被藏起来了」都推断不出来。

AI 的每一次写入都停在一个 diff 上，等你允许、拒绝，或者改完再应用。每一场会话——每次
读、每次写、每个权限决定、每次隐私过滤——都是知识库里的一份 JSONL 审计日志，可以在应用
里逐条回放。

![写入门——这条编辑是 agent 提的；你不作答，它就不落盘](docs/assets/permission-diff.png)

### 默认离线，除非你开口

只有五件事能碰网络，每一件都由你的某个动作触发，每一件都列在
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 里：LLM 信道、你打开的网页、阅读投影的
图片代理、去广告规则下载、检查更新。无遥测，无账号。其余一切断网照常——编辑器、所有
视图、搜索、你的脚本。

AI 是可选的。不配 key，vemry 是一个完整的本地编辑器。

## 安装

到 [Releases](https://github.com/pangyexing/vemry/releases) 下载 macOS（`.dmg`）、
Windows（`.exe`）或 Linux（`.AppImage` / `.deb` / `.rpm`）。更新包经 minisign 签名，
用编译进应用的公钥校验；下载与安装始终需要你确认，启动时自动检查默认关闭。

macOS 尚未公证，首次打开需右键 →「打开」，或执行 `xattr -cr /Applications/vemry.app`。

### 从源码运行

```bash
git clone https://github.com/pangyexing/vemry.git
cd vemry
pnpm install          # npm 装不了：vite 的 devDependencies 用了 link: 协议
pnpm tauri dev        # 需要先 source ~/.cargo/env
```

需要 Rust（stable）与 Node 22+。

## 快速上手

1. **打开示例知识库。** 先 clone 本仓库，再 ⌘P →「打开知识库…」→ 选
   `examples/demo-vault`。这是一个不大的真实库，特意配得让每个投影都有东西可看。
2. **按 ⌘P。** vemry 能做的每一件事都在那一个列表里。
3. **配一个 AI key**（可选）。设置 → API Key。默认端点是智谱，它的 `glm-4.7-flash`
   免费且支持工具调用；任何 OpenAI 兼容端点都能换，包括本机的 Ollama。
4. **研读一篇论文。** 打开 AI 面板，选「研读论文」，贴一个 arXiv 链接或拖进一份 PDF。
   原文笔记、章节、卡片都以 diff 的形式到来，由你确认——这也是最快看清权限门长什么样的一条路。
   示例库的 `papers/` 下已经有一篇读完的论文。

## Markdown 方言

语义用别的工具本来就认识的普通 Markdown 表达：

```markdown
- [ ] 一个任务 📅 2026-08-20          ← 截止日期
- [/] 进行中
- [x] 已完成

> [!claim] 睡眠巩固记忆 ^c1                      ← 断言，带块 ID

> [!evidence] Walker 2017 的剥夺实验
> supports:[[睡眠与记忆#^c1]]                    ← 证据，指向那条断言

![[睡眠与记忆#^c1]]                               ← 只读嵌入那个块
- 再读 [author:: 勒古恩] [status:: reading]      ← 行内字段：可查询、可分组
```

上面每一条在别处都渲染成一个列表项或一段引用。完整方言与逐条降级形态见
[docs/SYNTAX.zh-CN.md](docs/SYNTAX.zh-CN.md)。

## 文档

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) —— 分层、关键流程、安全边界与网络出口清单
- [docs/SYNTAX.zh-CN.md](docs/SYNTAX.zh-CN.md) —— Markdown 方言及其降级形态
- [docs/GLOSSARY.md](docs/GLOSSARY.md) —— 本项目的术语表（中英对照）
- [CONTRIBUTING.md](CONTRIBUTING.md) —— 怎么构建，以及 PR 评审依据的设计原则

## 现状

`main` 上已经能用：编辑器与实时预览、七个投影视图、块索引与查询 API、带领域工具且
写入过门的 agent、隐私区、脚本沙箱、内置浏览器与剪藏及阅读投影、笔记/网页/PDF 的对照翻译，
以及一套中英双语界面。

**已发布的安装包早于本文。** 双语界面尤其还没随包发布——最新的发布版只有中文。在 0.9.0
发布之前，想要本文描述的形态请从源码构建。

接下来，按研读闭环需要的次序：落进原文笔记的 PDF 批注、原文笔记上的文献管理（DOI、BibTeX
导出）、跨论文阅读（一条断言，每一篇与之相关的来源）。之后才是：语义检索、可安装的扩展与主题、
主窗内的同窗分屏浏览。每一项从哪条接缝长出来，列在 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

## 参与贡献

vemry 由一个人开发。源码公开是为了让你能审计、能自己编译；外部 PR 默认关闭，bug
反馈走 [Discussions](https://github.com/pangyexing/vemry/discussions)。想动手改什么，
先在那里说一声——[CONTRIBUTING.md](CONTRIBUTING.md) 里有一份五分钟的「改什么去哪里」
导览。如果你只想给自己的库做点自动化，那连 vemry 本体都不用碰：往
`.vemry/scripts/` 里写个脚本就行。

## 许可

[Functional Source License 1.1，附 Apache-2.0 未来许可](LICENSE)（FSL-1.1-ALv2）。

你可以随意使用、编译、修改，也可以在公司内部部署。唯一不能做的，是把它或者实质相似
的东西当成竞品拿去商业售卖或提供服务。每个版本发布满两年，自动转为 Apache-2.0。
