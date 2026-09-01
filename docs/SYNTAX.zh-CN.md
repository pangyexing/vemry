# vemry Markdown 方言

vault 的唯一真源是磁盘上的纯 Markdown 文件。本文定义 vemry 在 **CommonMark + GFM** 之上额外识别的全部记号，以及每一条在**不认识它的标准 Markdown 渲染器**里的降级形态。

这份文档同时是 [CONTRIBUTING.md](../CONTRIBUTING.md) 「零锁定」原则的评审判据：新语法必须先进入本文，并在这里写明降级行为——判据见文末[「新增语法的门槛」](#新增语法的门槛)。

> English: [SYNTAX.md](SYNTAX.md)。本文是它的中文版，两份逐节对应，改动请同步。

实现真源：`crates/vemry-core/src/parser.rs`（块级扫描器，行级 + 内联，刻意不用完整 Markdown AST）、`crates/vemry-core/src/note.rs`（frontmatter）、`src/lib/editor/`（渲染侧）。

## 基线

CommonMark + GFM（表格、删除线、任务列表）+ 文首 YAML frontmatter。凡本文未列出的记号，vemry 一律按标准 Markdown 处理，不做额外解释。

## Frontmatter

```yaml
---
title: 可选标题
tags: [研究, 睡眠]        # 也接受单个字符串
aliases: [别名一, 别名二]  # 参与 wikilink 的裸名解析
created: 2026-08-20
private: true             # 进隐私区：对 AI 永远不可见
vemry:                    # vemry 自己写的溯源信息，与用户字段隔离
  origin: agent           # agent | capture
  agent_session: <id>
---
```

未知字段原样保留（`Frontmatter.extra` 展平透传），往返读写不会吃掉你自己的元数据。

**降级**：标准 YAML frontmatter，绝大多数 Markdown 工具原生识别；不识别的渲染器把它显示为顶部一段 `---` 包裹的文本，正文完整。

## 链接与嵌入

| 写法 | 含义 |
|---|---|
| `[[目标]]` | 指向笔记 |
| `[[目标\|别名]]` | 显示别名 |
| `[[笔记#小节]]` | 指向标题 |
| `[[笔记#^块ID]]` | 指向块 |
| `[[#小节]]` / `[[#^块ID]]` | 目标为空 = 本篇自身 |
| `![[图片.png]]` | 内联嵌入图片 |
| `![[图片.png\|300]]` | 指定显示宽度（px） |

解析规则（`scan_inline`）：`[[…]]` 不跨行、内容非空，行内代码 `` ` `` 之间不解析。

目标解析（`BlockIndex::resolve_target`）：目标为空 = 本篇；含 `/` 按完整相对路径匹配；裸名匹配文件名主干或 `aliases`，**大小写不敏感**，歧义时路径最短者胜。

**降级**：显示为字面文本 `[[目标]]`——可读，但不是链接。**这是本方言里降级最弱的一条**：语义（链接图、反向链接）在外部工具中丢失，文本不丢。

## 标签

行内 `#标签`，与 frontmatter `tags` 合并去重。

- 允许的字符：字母、数字、`_`、`-`、`/`（支持 `#研究/睡眠` 层级）
- 前一个字符不能是字母、数字、`#` 或 `&`（避开 `&#8203;` 一类实体）
- 纯数字不算标签（`#1` 是普通文本，不是标签）
- 行内代码之间不解析

**降级**：普通文本。行首的 `#标签` 不会被误当成标题——ATX 标题要求 `#` 后必须有空格，vemry 与 CommonMark 在这点上一致。

## 任务

```markdown
- [ ] 待办
- [/] 进行中
- [x] 已完成
- [ ] 带日期的任务 📅 2026-08-20 ⏳ 2026-08-15
- [ ] 等价的 ASCII 写法 due::2026-08-20 scheduled::2026-08-15
```

- 列表符 `-`、`*`、`+` 均可，任意缩进
- `[?]` 中方括号后必须是空格或行尾
- `x`/`X` 都算完成；未知状态字符按待办处理
- 日期一律 `YYYY-MM-DD`；到期 = `📅` 或 `due::`，计划 = `⏳` 或 `scheduled::`

**降级**：`[ ]`/`[x]` 是标准 GFM 任务列表，直接就是复选框；`[/]` 不是 GFM 的任务标记（GFM 只认空格与 `x`/`X`），退化为普通列表项加字面文本 `[/]`，仍然可读；日期是行内纯文本。

## Callout 与断言/证据

```markdown
> [!warning] 可选标题
> 后续以 > 开头的行都并入这个 callout

> [!claim] 睡眠影响记忆巩固 ^c1

> [!evidence] Walker 2017 的实验数据
> supports:[[睡眠研究#^c1]], [[另一笔记#^c9]]
```

- `[!类型]` 大小写不敏感、类型内不能有空白；后面可跟 `+`/`-` 折叠记号
- 识别的类型：`note`/`info`/`todo`/`abstract`/`summary`/`tldr`、`tip`/`hint`/`important`、`success`/`check`/`done`、`question`/`help`/`faq`、`warning`/`caution`/`attention`、`danger`/`error`/`failure`/`fail`/`missing`/`bug`、`example`、`quote`/`cite`，以及 vemry 自有的 `claim`/`evidence`
- 只有 `claim` 与 `evidence` 进块索引成为独立块类型，其余按段落索引
- `evidence` 用 `supports:` 声明它支撑哪些断言：**写了 `supports:` 就只取其后的链接**，没写则整块的链接都算

**降级**：标准 blockquote，`[!warning]` 作为首行文字照常可见。这是本方言里降级最干净的一条。

## 块 ID

行尾 ` ^id`（空格 + `^` + 标识符）给该块一个稳定锚点，供 `[[笔记#^id]]` 引用。没有显式 ID 的块由索引按序号自动分配临时 ID，**不写回磁盘**。

**降级**：行尾的普通文本。

## 行内高亮

`==高亮文字==`。CommonMark 与 GFM 都没有这条；实现是仿 `@lezer/markdown` 自带删除线写法的成对定界符扩展。

**降级**：字面 `==高亮文字==`，可读。

## 查询块

````markdown
```vemry-query
kind: task
task_state: todo
due_before: today
limit: 20
```
````

YAML 体，**七个平铺标量**，全部可选：

| 字段 | 取值 |
|---|---|
| `kind` | `paragraph` / `heading` / `task` / `claim` / `evidence` / `code_block` |
| `note` | 笔记路径或名字的子串 |
| `tag` | 标签名（前导 `#` 会被去掉） |
| `text` | 正文子串 |
| `task_state` | `todo` / `doing` / `done` |
| `due_before` | `YYYY-MM-DD` 或 `today`（按本地时区求值） |
| `limit` | 正整数 |

不嵌套、不支持表达式——刻意做成一个安全子集：查询块只读、不能写盘、不能取网络。

**降级**：**普通代码块**，原样显示查询条件。这是有意设计——查询结果是派生数据，永远不写进磁盘，所以外部工具看到的和磁盘上的完全一致。

## 可运行代码块

` ```js ` / ` ```javascript ` 代码块（语言标注大小写不敏感）在编辑器里带一个「运行」按钮，点击把块内容送进 Rust 侧的一次性只读沙箱（2 秒熔断、无写 API、无网络），输出渲染在闭合围栏之后，只存在于编辑器状态，**永不落盘**，关掉文档即消失。

**永不自动执行**：打开笔记 ≠ 运行代码；整条链路唯一入口是这个按钮，AI 也没有任何执行代码的工具。

**降级**：普通 `js` 代码块。

## 隐私标记

三条独立入口，任一命中即进隐私区（对 AI 不读、不写、不进 prompt）：

- 笔记 frontmatter `private: true`
- `.vemry/privacy` 里的 glob 规则（gitignore 语法，大小写不敏感）
- 任何以 `.` 开头的路径段（内置，无需配置）

**降级**：`private: true` 是一个普通 frontmatter 键；`.vemry/privacy` 是一个普通文本文件。隐私隔离是 vemry 运行时的行为，不改变文件本身。

## 新增语法的门槛

往这份方言里加东西之前，三条都要满足：

1. **写进本文**，连同它的降级形态一起；
2. **在不认识它的标准 Markdown 渲染器里优雅降级**——退化成可读的纯文本或标准结构即可，但不允许让内容消失、让后续结构错乱（例如吃掉整段、破坏列表或围栏嵌套）；
3. **不依赖边车文件还原语义**——把 vault 目录整个拷走、只用一个普通编辑器打开，写下的东西必须还在。

判据是可证伪的：拿一个纯 CommonMark 渲染器跑一遍，对照本文写明的降级形态。
