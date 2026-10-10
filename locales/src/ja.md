# Japanese (`ja`) translation conventions

The rules `ja.json` is written to. Concept words come from
[docs/GLOSSARY.md](../../docs/GLOSSARY.md); these settle everything else two
translators would otherwise do differently. A new string follows them, and a
changed rule changes the catalog with it.

## Style

- Register: every full sentence (hints, descriptions, toasts, errors, empty states, dialog text) is です・ます. Buttons, menu items, command titles, tabs, column headers, chips and placeholders are noun phrases or a noun + を + dictionary-form verb, with no です・ます and no 。 (保存, 名前を変更, サイドバーを開く, ノートを選択…).
- No imperative: never plain commands (〜しろ, 〜せよ, bare 〜して). Hints, placeholders and empty states state a possibility or a result instead (〜できます, 〜すると〜します, 値を入力). 〜してください is allowed only in an error or warning that names the one action needed to continue, at most once per string.
- Punctuation: full-width 、。「」（）：？ inside Japanese text; English “…” and ‘…’ become 「…」 (nested 『…』); never ！. End with 。 only when the string is a full sentence (the English ends with a period, or the Japanese holds two or more sentences); labels never end with 。. Half-width punctuation stays inside code, paths, URLs, vault syntax (`key:: value`, `^id`, `[[…]]`), times and ranges like {done}/{total}.
- Em dash "—": do not carry it into Japanese. Rewrite as 。 (two sentences) or 、, as "：" for a label followed by its explanation (プライバシーゾーン：AI からは見えません), or as （…） for an aside. Only window titles keep it: 「クイック記録 — vemry」.
- Ellipsis: one U+2026 "…" with no space before it, exactly where the English has one (items that open a dialog or picker, progress such as 翻訳中…, placeholders). Never "...", "・・・" or "……".
- Separators and arrows stay exactly as in English, spacing included: " · ", "→", "›", " / ". Do not turn "·" into "・". Settings paths are written 「設定 › AI」 with the translated tab names inside 「」.
- Spacing: one half-width space between Japanese and Latin letters, digits or {placeholders} (AI の変更, {n} 件, Markdown ファイル, ブロック ID, vemry の); no space next to full-width punctuation (「{title}」, （{key}）) and none between a number and %. Never full-width letters or digits (ＡＩ, １).
- Katakana: loanwords ending in -er/-or/-ar take the long mark (フォルダー, フィルター, ユーザー, ブラウザー, プロバイダー, エディター, サーバー); words ending in -y do not (メモリ, カテゴリ, エントリ, ディレクトリ). No ・ between the parts of a compound (ユーザースクリプト, クレジットパック).
- The person using the app: leave the subject out and drop "your" ("your notes" → ノート). Never あなた. When a label has to name them (a block's origin "You", "attached by the user" in Session Replay) use 自分 (自分で指定). ユーザー appears only inside the term ユーザースクリプト.
- The AI is a tool: the subject is AI or モデル with plain action verbs (読み取る, 書き込む, 生成する, 提案する, 検索する, 実行する): "AI が書き込みました". Never 考える, 思う, 理解する, 覚える, 判断する, 〜してくれる or お手伝い, and no honorific or humble forms for what the AI does. The model's thinking setting and output are 推論 (推論の深さ, 推論中…, 推論を展開), never 思考中 or 考え中.
- Words with two senses take the rendering of their context: Plan → プラン (account tier) / 計画 (the AI's multi-step plan, whose items are ステップ); Review → 復習 (flashcards; "Study ahead" → 先取り復習) / 確認 (AI writes: 未確認, 確認済みにする, 確認を開始); Later → 後で (defer, priority) / 今後 (the agenda bucket after the next 7 days); Note → ノート (a file) / 注記 (the callout type); Translate → 翻訳 (verb, button) / 対訳 (the view and the mode); cancelled → 中止 (task state) / キャンセル (dialog button, a cancelled plan or request).
- The undo family is kept apart: 元に戻す = undo (⌘Z, and "Undo this turn's writes"); 破棄 = revert an AI change already on disk (AI の変更を破棄, {n} 件の変更を破棄しました); 拒否 = Deny in the permission dialog; 却下 = Reject a plan; 許可 = Allow. 取り消す is used for none of them (it is the undo command on macOS).
- 確認 is reserved for reviewing AI writes, so that 未確認 has one meaning: the permission dialog's title is 書き込みの許可, and Confirm buttons use 確定 or the action verb itself (削除を確定). "unverified" (a link no web search confirmed) is 未検証. Fold → 折りたたむ / Unfold → 展開; Collapse → たたむ / Expand → 展開.
- Counters: notes, tasks, changes, writes, sessions, links and generic items → 件; blocks → 個; cards → 枚; lines → 行; words → 文字 (the count takes CJK characters one by one); days → 日; plan steps → ステップ; credits → {n} クレジット. Labels and chips put the noun first (ノート {n} 件); sentences put the count first ({n} 件のノートを…). Japanese has only the _other plural form: no たち/達, no "(s)".
- Keys: every key.* value stays exactly as English (Ctrl, Shift, Alt, Backspace, Win), as do Enter, Esc, Tab, Space and ⌘ ⌥ ⇧ ⌃. Combos are joined with "+" and no spaces (Ctrl+Shift+P). Add キー only after a single key named alone in a sentence (Esc キーで戻ります), never after {key}/{hotkey} or inside a combo.
- Messages: success is past polite 〜しました (コピーしました, 保存しました → {path}); failure is 〜できませんでした, then "：" and the cause (保存できませんでした：{error}); no apologies (申し訳ありません, すみません) and no "エラー：" prefix. Progress is 〜中…; an empty state is まだ〜はありません。. A confirmation asks 〜しますか？ and states the consequence in the next sentence; its button repeats the verb (削除), never はい/いいえ.
- Command patterns: New X → 新しい X (新しいノート, 新しい保管庫, 新しいタブ, 新しいセッション); Open X → X を開く; Toggle X → X の表示を切り替え; Copy X → X をコピー; Mark as X → X にする (確認済みにする, 中止にする).
- Never translated: vemry (lower-case Latin, even at the start of a sentence), AI, PDF, URL, Markdown, VEMRY.md, file and folder paths (clips/, research/), frontmatter keys, slash.* commands, priority:: values, callout type names as written on disk, {placeholders} and their spelling. Use the half-width forms. diff → 差分 (Raw diff → 生の差分), token → トークン, web search → ウェブ検索.
- Supporting words with one rendering: AI session → セッション, turn → ターン, conversation → 会話, model → モデル, API key → API キー, source (a web or clip source) → 出典, citation → 引用 (block citation → ブロック引用), close reading → 精読, browser → ブラウザー, selection → 選択範囲, backlinks → バックリンク.
- Glossary words are used verbatim and never varied for style: no ワークスペース/リポジトリ/ライブラリ for 保管庫, プロパティ/属性/メタデータ for フィールド, バッジ/タグ for チップ, ダッシュボード/ウィジェット for 投影ビュー, 承認フロー for 許可ゲート, 非表示/伏せ字 for プライベート, 締め切り for 期日, ステータス for 状態フィールド, お気に入り for ブックマーク, メモ/受信箱 for クイック記録, 保存先 for クリップ先, ポイント/円 for クレジット, 上限 for 利用枠.

## Words decided outside the glossary

Interface words the glossary does not cover, fixed while the catalog was written
so that every string uses the same one.

| English | Japanese |
|---|---|
| follow (the note the AI works in), working note | 作業ノート |
| capture (noun) / capture target | 記録 / 記録先 |
| phone | スマートフォン |
| inbox (the file `inbox.md`) | inbox (Latin letters) |
| check for updates | アップデートをチェック |
| chat / conversation | チャット / 会話 |
| action items; keep / discard one | アクションアイテム; 採用 / 不採用 |
| decline (a page read) | 見送る |
| discarded turn (Session Replay) | 除外 |
| revoke | 解除 |
| method (a skill, as the picker calls it) | 方法 |
| thinking | 推論 |
| Properties (frontmatter) / task attributes (📅 …) | フィールド / 属性 |
| review (an AI write) — 確認 is reserved for it; the permission dialog | 確認; 書き込みの許可 |
