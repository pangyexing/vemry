<div align="center">

# vemry

**读透一篇论文，留住每一点理解。**

本地优先的 AI 研读工作台。论文、网页、PDF 都进来变成你自己磁盘上的纯 Markdown，
在这个文件夹里干活的 AI 只看得见你允许的部分。

[**下载**](https://github.com/pangyexing/vemry/releases) · [快速上手](#快速上手) · [文档](#进一步了解) · [Discussions](https://github.com/pangyexing/vemry/discussions)

[![Release](https://img.shields.io/github/v/release/pangyexing/vemry?include_prereleases&sort=semver)](https://github.com/pangyexing/vemry/releases)
[![CI](https://github.com/pangyexing/vemry/actions/workflows/ci.yml/badge.svg)](https://github.com/pangyexing/vemry/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-FSL--1.1--ALv2-blue)](#许可)
![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)

[English](README.md) · [简体中文](README.zh-CN.md)

</div>

![vemry — 研读中的一章论文，原文按块嵌入](docs/assets/hero.png)

<br>

**你的笔记就是文件。** 纯 Markdown，放在你自己选的文件夹里。没有数据库，没有账号，
没有同步服务。用任何编辑器打开这个文件夹，看到的和 vemry 看到的一模一样。

**AI 写入前先问你。** 每一条修改都停在一份 diff 上，由你允许、拒绝，或者改完再应用。
你标为隐私的内容，模型永远看不见。

**离线照常工作。** 除非你做了需要联网的事，否则什么都不会碰网络。无遥测、无账号；
不配 API key，它仍然是一个完整的本地编辑器。

## vemry 是什么

vemry 是一个 Markdown 文件夹的编辑器，加上一个按你的规则在这个文件夹里干活的 AI。
你看到的一切——编辑器、每个视图、AI 的工具、你自己的脚本——读的都是从你的文件建出来的
同一份块级索引。你的知识在任何地方都没有第二份副本。

它不是云服务，也不是一个挂着笔记文件夹的聊天窗。AI 是你知识库里的一个参与者：每次写入前
有一道权限门，隐私边界它看不过去。

**操心更少，懂得更透，记得更久。** 读完一篇论文，你能用自己的话讲出来；哪里讲偏了，当场
看到；快忘的时候，卡片把它带回来。剪成 Markdown、对照翻译、整理归档、追出处、做卡片，这些
繁琐的活不用你动手，每一次写入仍先停在 diff 上等你点头。

## 你能用它做什么

### 一章一章读一篇论文

贴一个 arXiv 链接、拖进一份 PDF，或者指一篇剪藏，选「研读论文」。原文变成一篇能随手翻的
笔记——每个小节、公式、图、表都是独立的块——然后一章一章读下去。每一章都从你库里已经知道的
东西讲起，标出哪些是新的，每一句都链到原文出处。

### 看看自己到底读懂了没有

用你自己的话把一章复述一遍，你会看到自己和原文偏在哪里。想动手试试这个想法？要一个实验，
章节里就多出一个可运行的块。没完全读懂的地方变成卡片，复习视图会在你快忘的时候把它们端回来。

### 两种语言并排读

把一篇笔记、一个网页或一份 PDF 和它的译文并排打开，逐段对齐。读 PDF 时，点一段译文，
页面上对应的原文段落就会亮起来。

### 不离开笔记就剪藏网页

内置浏览器就开在笔记旁边。剪一整页或一段选区，它带着来源落进笔记，图片存进你的知识库；
去广告是内置的。一个还不知道该放哪的念头，一条命令就送进收件箱笔记。

### 同一份笔记，按你的问题去看

今天该做什么，看日程；每件事推进到哪一步，看看板；这个月做了些什么，看时间线。哪些笔记互相
连着，看图谱；哪个论点有证据撑着、哪个还没有，看断言和证据；哪些卡片该复习了，看复习。它们
都不是另存的数据，全是对同一份索引的查询；在笔记里写一个 `vemry-query` 围栏，同样的查询就
直接渲染在正文里。

![看板——全库任务按状态分列](docs/assets/kanban.png)

![图谱——同一份索引画成链接；那三篇隐私笔记你看得见，模型看不见](docs/assets/graph.png)

### 提问，然后核对答案

每个答案都引用它读过的那个块，点引用就能打开。开着联网搜索时，每个来源都标明验证过没有；
想留下的答案可以作为一个块放进笔记。按 `@` 把一篇笔记、一个目录或当前选区交给 AI。

### 把杂活交出去，按你的规矩

十六个内置技能覆盖阅读之外的工作：整理文件夹、整理速记、重整一篇笔记、会议记录变行动项、
从一周的笔记起草周报（由你收尾）、今日收尾。杂活交给 AI；读、复述和裁决留给你。往 `.vemry/skills/`
里放一个 Markdown 文件就是你自己的技能。每次写入都是一份
diff，一整轮的写入可以整体撤销，每场会话都是一份审计日志，在会话回放视图里逐条回放。

![写入门——这条编辑是 agent 提的；你不作答，它就不落盘](docs/assets/permission-diff.png)

### 用脚本自动化，不用装插件

往 `.vemry/scripts/` 丢一个 `.js` 文件，保存即生效。脚本可以注册命令面板条目、订阅
vault 事件、按 cron 定时运行，跑在一个无文件系统、无网络、无进程的沙箱里。应用的每一项
能力，包括你自己写的，都在同一个可搜索的命令面板里（⌘P）。

## 快速上手

1. **安装。** 到 [Releases](https://github.com/pangyexing/vemry/releases) 下载
   `.dmg`、`.exe`、`.AppImage`、`.deb` 或 `.rpm`，或者[从源码构建](#从源码构建)。
2. **打开示例知识库。** 先 clone 本仓库，再 ⌘P →「打开其他知识库…」→ 选
   `examples/demo-vault`。这是一位学习者 2026 年 9 月的一周，和官网讲的是同一个故事：一篇
   分章研读的论文，附两个能直接运行的实验，以及复述核对后留下的卡片；一个带任务的项目；一条
   带断言与证据的研究线；一页网上剪藏和一份用来对照翻译的讲义；一个收着电脑和手机速记的
   收件箱；几天日志、三段录下的 AI 会话，以及一个隐私目录。从「Start Here」这篇开始。
3. **配一个 AI key**（可选）。设置 → API Key。任何 OpenAI 兼容端点都能用，包括本机的
   Ollama；大陆默认端点是智谱，它的 `glm-4.7-flash` 免费且支持工具调用。也可以把提供方
   切成「vemry 托管」，用邮箱登录：模型随账户而来，请求经 vemry 中继原样转给模型厂商。
4. **研读一篇论文。** 打开 AI 面板，选「研读论文」，贴一个 arXiv 链接或拖进一份 PDF。
   原文笔记、章节、卡片都以 diff 的形式到来，由你确认。

这就是整个闭环。它做出来的每一样东西，都是你文件夹里的一个 Markdown 文件。

Windows 和 Linux 上，把 ⌘ 读作 Ctrl。

### 先试试这几句

打开 AI 面板（在任何地方按 ⌘J），输入其中一句：

```
研读这篇：https://arxiv.org/abs/1706.03762
这是我对第 3 章的复述：…… 哪里和原文偏了？
给我一个 3.2 节的实验，能在这里直接跑。
用今天到期的卡片考考我。
这个库这周改了什么？
整理一下 research/ 目录。
```

示例知识库里的 `private/` 已经是隐私区：让 AI 列出你的笔记，它看不见这个目录，也不会知道少了什么。

## 什么留在你的机器上

**隐私即不存在。** 把一个目录、一篇笔记或单独一个 `> [!private]` 块标为隐私，agent 就永远
不读、不写、也不会在 prompt 里见到它。这在 Rust 层的每个读写卡点上强制执行，而且一个隐私
文件返回的错误与一个从来不存在的文件逐字节相同。

**每次写入都停在一份 diff 上。** 允许、拒绝，或者改完再应用。这道门就在掌握文件访问能力的
那一层 Rust 里，界面上没有任何东西能绕过它。

**五个网络出口，每一个都由你触发。** LLM 信道、你打开的网页、阅读视图的图片代理、去广告
规则下载、检查更新。它们列在 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 里，其余一切
断网照常。

**任意代码永不自动执行。** 打开一篇笔记不等于运行它。每条执行路径都始于你的某个动作，
而 agent 根本没有能执行代码的工具。

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

## 现状

截至 2026 年 9 月，`main` 上已经能用：编辑器与实时预览、日程/看板/时间线/图谱/断言/证据/复习
七个视图、块索引与查询 API、工具写入过门的 agent、隐私区、脚本沙箱、内置浏览器与剪藏、笔记/网页/PDF 的对照翻译，
以及一套中英双语界面（日、韩、法、德、西班牙语随翻译逐步加入）。

接下来，按研读闭环需要的次序：落进原文笔记的 PDF 批注、原文笔记上的文献管理（DOI、BibTeX
导出）、跨论文阅读（一条断言，每一篇与之相关的来源）。之后才是：语义检索、可安装的扩展与主题、
主窗内的同窗分屏浏览。每一项从哪条接缝长出来，列在 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

## 进一步了解

- [docs/SYNTAX.zh-CN.md](docs/SYNTAX.zh-CN.md) —— Markdown 方言及其降级形态
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) —— 分层、关键流程、安全边界与网络出口清单
- [docs/GLOSSARY.md](docs/GLOSSARY.md) —— 本项目的术语表（中英对照）
- [CONTRIBUTING.md](CONTRIBUTING.md) —— 怎么构建，以及 PR 评审依据的设计原则

## 社区与反馈

| 你想…… | 去哪里 |
|---|---|
| 报一个 bug 或提问 | [Discussions](https://github.com/pangyexing/vemry/discussions) |
| 动手改 vemry 的什么 | 先在 Discussions 说一声，再看 [CONTRIBUTING.md](CONTRIBUTING.md) |
| 给自己的知识库做自动化 | 往 `.vemry/scripts/` 写个脚本，vemry 本体不用动 |

vemry 由一个人开发，精力有限，外部 PR 暂时关闭。

## 从源码构建

```bash
git clone https://github.com/pangyexing/vemry.git
cd vemry
pnpm install          # 只用 pnpm：锁文件和 pnpm-workspace.yaml 里的供应链设置都是 pnpm 的
pnpm tauri dev        # 需要先 source ~/.cargo/env
```

需要 Rust（stable）与 Node 22+。打包版的更新经 minisign 签名，用编译进应用的公钥校验；
下载与安装始终需要你确认，启动时自动检查默认关闭。

## 许可

[Functional Source License 1.1，附 Apache-2.0 未来许可](LICENSE)（FSL-1.1-ALv2）。

你可以随意使用、编译、修改，也可以在公司内部部署。唯一不能做的，是把它或者实质相似
的东西当成竞品拿去商业售卖或提供服务。每个版本发布满两年，自动转为 Apache-2.0。
