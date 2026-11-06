# 源片段 ↔ L1 语义契约校验（D1 尺子）

> 实现：`crates/odc-core/src/profile/contract.rs`；命令行入口：`examples/profile_compare.rs --source`
> 决策依据：`odc-presentation.md` §6 D1（文本保真 = 词袋 + 块内顺序）。

## 1. 为什么需要它

黄金面板只能证明「和 LDOCE6 适配器一致」，判不了第二本词典——它没有适配器。
唯一有效的真值是**源片段本身**：L1 必须承载源的全部文本，且不得携带源的结构痕迹。
这个校验器就是这把尺子：把 profile 跑出来的 L1 和喂进去的源 HTML 直接比对。

## 2. 用法

````
profile_compare --profile profiles/<dict>.profile.json \
  --source <raw.jsonl> [--report out.json] [--dump <dir>] [--limit N]
````

- 输入格式与 `--raw` 相同：每行 `{'key': ..., 'text': <源片段>}`；`@@@LINK` 记录按 alias 跳过。
- `--dump` 落每条 L1 片段（`<key>.html`），便于定位。
- 退出码：全部 body 记录四条全过才 0；否则 1（尺子只测量，不放过半绿）。

输出：

````json
{
  "mode": "source-contract",
  "summary": { "total": 202, "body": 45, "alias": 157, "equal": 21,
               "bag_ok": 23, "order_ok": 45, "structure_ok": 41, "clean_ok": 45 },
  "reason_counts": [ { "code": "bag.missing", "count": 46 }, ... ],
  "failures": [ { "key": "...", "bag_ok": false, "order_ok": true,
                  "structure_ok": true, "clean_ok": true,
                  "reasons": [ { "code": "bag.missing", "detail": "\"sets\" x1 absent from L1" } ],
                  "first_diff": "bag.missing: \"sets\" x1 absent from L1" } ]
}
````

`summary` 的四个 `*_ok` 都是**在 45 条 body 上**计数；`equal` 是四条全过的条数。

## 3. 四条判定（精确规则）

两侧都用 `profile::exec::parse_fragment` 解析——和执行器同一个解析器、同一套实体解码，
所以判定与「引擎看到的树」一致，不依赖外部工具。

### 3.1 文本词袋 `bag_ok`

- 取两棵树的文本节点，按文档顺序切词：
  - 拉丁/数字连成一段 → 一个 token（`don't` → `don` + `t`）；
  - 中日韩/假名/谚文**每字符一个 token**（这些文字不用空格分词）；
  - 标点与空白只作分隔，不比较；**大小写保留**（改写词头也算失真）。
- 两个多重集必须完全相等；差异按 `bag.missing` / `bag.extra`（漏词 / 多词）报告。

### 3.2 块内顺序 `order_ok`

对 L1 中每个语义块（`odc-sense`、`odc-subsense`、`odc-examples`、`odc-ref-block`、
`odc-pos-block`、`odc-head`），块内 token 序列必须是**源 token 序列的子序列**。
跨块的排列（引用条提前、词性块分组）不在判定范围内——那是 P1 的职责，由结构检查管。

### 3.3 结构不变量 `structure_ok`

1. 顶层（剥掉 `html` / `head` / `body` 包装后）存在 `section.odc-entry`；
2. 词条内有词性标记（`odc-pos`）时，必须有 `section.odc-pos-block[data-odc-pos]`，
   且它是词条的**直接子元素**（P1 骨架）。**引用条 `nav.odc-refbar` 内的标记不算**：
   那些面板引用的是别的词条；嵌套 `section.odc-entry` 的标记也不算；
3. 义项（`odc-sense` / `odc-subsense`）不得与词性块同级：义项必须在词性块内部；
4. `nav.odc-refbar` 的直接子元素必须都是引用类块（类名以 `odc-ref` 开头）。

### 3.4 源标签残留 `clean_ok`

- 每个 `class` token 必须以 `odc-` 开头（源类名一律算残留）；
- 每个属性必须属于：`class` / `id` / `href` / `src` / `srcset` / `alt` / `title` /
  `lang` / `dir` / `colspan` / `rowspan` / `width` / `height`，或以 `data-odc-` 开头
  （含 `data-odc-orig-class`——那是引擎**有意**记录原类名的地方）；
- 标签名不判：`h2` / `li` / `b` 这类中性标签允许保留，只要求类名与属性归引擎。

### 3.5 不计入比较的文本

- 带 `data-odc-ui` 的元素（D2：文案由客户端注入）；
- `script` / `style`。

## 4. 可复现性

- 无随机、无外部依赖、无网络：同一输入两次运行输出逐字节相同（`serde_json::to_string_pretty`）。
- 词条级判定互不影响；`--limit` 只截断输入，不改变判定。
- 每条记录的 `reasons` 每种 code 最多 3 条（`MAX_PER_CODE`），避免单个记录刷屏；
  `reason_counts` 是**记录数**维度的计数（一条记录同一 code 记一次）。

## 5. LDOCE6 实测（2026-11-06，基线 0254ed9 + 本任务）

样本：`/tmp/odc-dialect-survey/raw/ldoce6encn.jsonl`（202 行 = 45 条正文 + 157 条别名）。

| 指标 | 结果 |
|---|---|
| total / body / alias | 202 / 45 / 157 |
| bag_ok | 23 / 45 |
| order_ok | 45 / 45 |
| structure_ok | 41 / 45 |
| clean_ok | 45 / 45 |
| 四条全过 | 21 / 45 |

失败原因 top：`bag.missing` 46、`bag.extra` 26、`structure.pos_block` 4。

**已定位的两类真实问题**（都不是 profile 的文本覆盖问题）：

1. **chrome 注入英文标签文案（P4/D2 违规）**：`presentation/chrome.rs` 的 `label()`
   往 `summary.odc-refbar__label` 写死 "Thesaurus" / "Verb Table" 等，而源里的按钮文案
   是 "Word Sets" / "Verb Table"。只有 `toc` 分支带 `data-odc-ui`。
   实测：把 L1 的 `.odc-refbar__label` 文本与源的 `.popup-button` 文本同时排除后，
   **45/45 条词袋完全相等**——即 22 条 bag 失败 100% 来自这句标签，正文一个字没丢。
2. **引用面板被提升成第二个 `section.odc-entry`**：`balance` / `world` / `slushy` /
   `proceeds` 四条里，源弹窗内的 `span.entry` 被规则标成 `odc-entry`，于是词条里出现
   第二个顶层词条（内含 `odc-pos` 标记却无词性块）→ `structure.pos_block`。

## 6. 已知局限（判不了的情况）

1. **文案类的「改写」判不出来**：只要两侧 token 多重集相同，把 "Word Sets" 改成
   "Word Sets" 之外的任何同词重排都不会被发现（顺序只在块内判）。
2. **跨块顺序不判**：D1 的决定，块间排列由 P1 结构规则兜底；校验器只保证块内不乱。
3. **标签名残留不判**：源里用 `<h2>`、L1 里也用 `<h2>`，只要类名是 `odc-*` 就放行。
   要判这一层需要一份「中性标签」封闭清单，规范目前没有（也无法穷举）。
4. **删掉的源内容看不出来**：若 profile 把整块源内容删干净，词袋会缺词 → 能发现；
   但若被删的文本和别处重复（同一词出现两次），缺一次仍可能多重集相等。
5. **只判「源 → L1」一个方向的结构**：L1 里凭空多出的 `odc-pos-block` 结构只要文本
   在源里找得到，不会被结构规则拦（属 profile 语义问题，不在 D1 范围）。
6. **chrome 文案只解决了一半**：引擎已按 D2 把标签写成 `data-odc-ui` + 空文本，
   L1 一侧不再有引擎文案；但源侧「按钮文案」（LDOCE6 的 `span.popup-button`）仍在源 token 里，
   识别它需要词典级选择器（禁止词典特判）。实测 LDOCE6 45 条：引擎文案清空后 `bag` 全部剩余差异
   都是这一类（`bag.missing`，源有 L1 无）。要把 `bag` 收到 45/45，只能二选一：
   ① D1 口径承认「profile 自己声明为 chrome 的源文本不算正文」（改判定规则）；
   ② profile 不再丢弃源按钮文案、让 caption 取自源（改 profile）。两者都超出 D2 的范围。
