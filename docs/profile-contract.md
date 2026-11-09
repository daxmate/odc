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
  --source <raw.jsonl> [--report out.json] [--dump <dir>] [--limit N] [--strict]
````

- 输入格式与 `--raw` 相同：每行 `{'key': ..., 'text': <源片段>}`；`@@@LINK` 记录按 alias 跳过。
- `--dump` 落每条 L1 片段，文件名取词头的 percent-encoding（`/` / `:` / 空白都不再炸路径），
  同目录写 `index.json`（原词头 → 文件名），保证可回查。
- 默认模式只测量，**退出码恒为 0**；`--strict` 才是门禁，不达标 `exit 6`（规则见 §4）。

输出：

````json
{
  "mode": "source-contract",
  "summary": { "total": 202, "body": 45, "alias": 157, "adapted": 45, "unadapted": 0,
               "equal": 21, "bag_ok": 23, "order_ok": 45, "structure_ok": 41,
               "clean_ok": 45, "clean_exempt_raw": 0 },
  "reason_counts": [ { "code": "bag.missing", "count": 46 }, ... ],
  "failures": [ { "key": "...", "bag_ok": false, "order_ok": true,
                  "structure_ok": true, "clean_ok": true, "clean_exempt_raw": false,
                  "reasons": [ { "code": "bag.missing", "detail": "\"sets\" x1 absent from L1" } ],
                  "first_diff": "bag.missing: \"sets\" x1 absent from L1" } ]
}
````

`summary` 的四个 `*_ok` 都是**在 adapted 记录上**计数（这里是 45 条全部适配）；`equal` 是四条全过的条数。
`clean_exempt_raw` 与 `unadapted` 同义，保留旧键名以免破坏既有报告脚本。

## 3. 四条判定（精确规则）

两侧都用 `profile::exec::parse_fragment` 解析——和执行器同一个解析器、同一套实体解码，
所以判定与「引擎看到的树」一致，不依赖外部工具。

### 3.1 文本词袋 `bag_ok`

- 词袋在**同一份文本的两种切分口径**下各算一次，任一通过即通过（见下面的「为什么两种口径」）：

  **口径 A —— 连续文本（默认，报告用这一口径）**：
  - **inline 边界不切词**：`span` / `b` / `i` / `em` / `a` / `u` / `strong` 等行内元素两侧的文字
    处于同一渲染位置，先拼接再切词 —— `sandwich<span>es</span>` 与 `sandwiches` 都得到 `sandwiches`；
  - **块级边界切词**：`div` / `p` / `li` / `section` / `td` / `br` / `h1..h6` 等（以及 `data-odc-ui` 注入的 chrome）
    结束当前文本段，绝不把两个段落粘成一个词；

  **口径 B —— 逐文本节点**：每个文本节点各自切词，被元素隔开的词不会拼接。

  两口径共用词法：拉丁/数字连成一段 → 一个 token（`don't` → `don` + `t`）；
  中日韩/假名/谚文**每字符一个 token**；标点与空白只作分隔，不比较；**大小写保留**（改写词头也算失真）。
- 两个多重集必须相等；差异按 `bag.missing` / `bag.extra`（漏词 / 多词）报告。

**为什么两种口径**：源片段与 L1 是两棵不同的树。源用行内 `span` 承载视觉上的块
（OALD9 的 `span.heading` + `span.p` 被 presentation 提升为 `p.odc-ref-panel__title` + `div.odc-ref`），
引擎又会把源里的元素 unwrap 掉（`.ndv` → 子节点），于是**同一个词在两棵树里的切分点不同**，
这在标签层面不可比。两种口径都对块边界切词，所以「L1 把一个词切成两块」「丢词 / 造词」
在两边都判失败；只有切分差异被容忍。报告只在口径 A 下记录失败项。

### 3.2 块内顺序 `order_ok`

对 L1 中每个语义块（`odc-sense`、`odc-subsense`、`odc-examples`、`odc-ref-block`、
`odc-pos-block`、`odc-head`），块内 token 序列必须是**源 token 序列的子序列**。
跨块的排列（引用条提前、词性块分组）不在判定范围内——那是 P1 的职责，由结构检查管。

### 3.3 结构不变量 `structure_ok`

1. 顶层（剥掉 `html` / `head` / `body` 包装后）存在 `section.odc-entry`；
2. 词条内有词性标记（`odc-pos`）时，必须有 `section.odc-pos-block[data-odc-pos]`，
   且它是词条的**直接子元素**（P1 骨架）。**引用条 `nav.odc-refbar` 内的标记不算**：
   那些面板引用的是别的词条；嵌套 `section.odc-entry` 的标记也不算；
3. 义项（`odc-sense` / `odc-subsense`）不得**跟在**词性块之后与它同级：义项必须在词性块内部。
   例外：**前置义项**——出现在词条内第一个词性块**之前**的无词性义项块是允许的
   （有些词典在词性容器之前先印一条 run-in 义项）；判定按文档顺序追踪「是否已遇到词性块」，
   一旦遇到过，后续任何同级义项仍然报 `structure.sense_sibling`；
4. `nav.odc-refbar` 的直接子元素必须都是引用类块（类名以 `odc-ref` 开头）。

### 3.4 源标签残留 `clean_ok`

- 每个 `class` token 必须以 `odc-` 开头（源类名一律算残留）；
- 每个属性必须属于：`class` / `id` / `href` / `src` / `srcset` / `alt` / `title` /
  `lang` / `dir` / `colspan` / `rowspan` / `width` / `height`，或以 `data-odc-` 开头
  （含 `data-odc-orig-class`——那是引擎**有意**记录原类名的地方）；
- 标签名不判：`h2` / `li` / `b` 这类中性标签允许保留，只要求类名与属性归引擎。
- **raw 兜底豁免**（规范 §6）：L1 根节点是 `section.odc-entry--raw`（或带 `data-odc-unadapted="1"`）时，
  该词条子树跳过 clean 检查——兜底按设计保留净化后的源结构，把它的源类名当成泄漏只是噪声；
  豁免**必须计数**，报告里 `summary.clean_exempt_raw` 记条数，不许静默。
  `bag` / `order` / `structure` 三条照旧不豁免 —— 记录层面的判定不变，但**统计口径**里未适配记录不进四维率分母（见 §4）。

### 3.5 不计入比较的文本

- 带 `data-odc-ui` 的元素（D2：文案由客户端注入）；
- `script` / `style`。

## 4. coverage 与四维率的适用域

`--source` 的输出只有一行：`coverage <adapted>/<total> (<pct>%) | mapped <pass>/<adapted> four-dim | unadapted <n>`。

- **coverage** = 真实适配条数 / 总条数（`adapted / total`）。「适配」指 profile 真的改写了这条记录；
  走 raw 兜底（`section.odc-entry--raw` 或 `data-odc-unadapted="1"`）的记录算**未适配**。
  alias 记录只是跳转指针、没有正文，不进分母。
- **四维率**（`bag` / `order` / `structure` / `clean`）的分母是 **adapted**，不是 total。
  未适配记录是「把源 HTML 原样包一层」，拿它和源比等于「源自比源」，四维必然全过；
  把它算进通过率就是把「没适配」读成「通过」。OALD10 用 OALD9 profile 跑出 `500/500`
  正是这个陷阱：51,310 条里 51,309 条是 raw 兜底 —— 旧口径给出 99.998% 的空转数字。
- **unadapted** 单独成列，不掺进通过率：它度量「这本词典还差多少」，四维率度量
  「已适配的部分做得好不好」，两个问题分开回答。
- `mapped <pass>/<adapted> four-dim`：已适配记录里四维全过的条数。
- 百分比保留三位小数：`59644/59645` 显示 `99.998%`，不会被四舍五入成 `100.0%`
  而掩盖掉未适配记录。
- verdict 层面 `ok()` 仍是四维判定（原有语义不变）；`adapted_ok() = ok() && !unadapted`
  才是「既适配又干净」。

`--strict` 退出码：

| 条件 | 退出码 |
| --- | --- |
| 默认模式（不带 `--strict`） | `0`（尺子只测量，不判定） |
| `--strict`：coverage 100% 且 unadapted 0 且已适配记录四维全过 | `0` |
| `--strict`：coverage < 100%，或 unadapted > 0，或已适配记录有四维失败 | `6` |

````
cargo run --release -q -p odc-core --example profile_compare -- \
  --profile profiles/oald9.profile.json --source /tmp/oald10-stratified.jsonl
# coverage 0/500 (0.000%) | mapped 0/0 four-dim | unadapted 500
````

### 4.1 P0 规则必须显式声明 `cardinality.min`

- **规则**：`emit.classes` 与 `P0_SLOTS`（`odc-entry` / `odc-entry-head` / `odc-pos-block` /
  `odc-sense` / `odc-sense-number` / `odc-def` / `odc-def-trans` / `odc-example` /
  `odc-phrasal-verb`）有任何交集的规则，**必须**在 `match` 里写出 `cardinality`；
  `min` 可以为 `0`，但不许省略。子规则（`children`）同样适用。
- **为什么**：运行时「未声明」与「`min: 0`」完全等价，而 P0 判定只在
  `hits < min` 时把记录判成 unadapted。全部 P0 规则都不声明 ⇒ 没有任何记录会因为
  P0 缺失被判 unadapted ⇒ coverage 退化成一个常量函数，`--strict` 的失败分支永远不可达。
  OALD10 实测就是这样：83 条规则里 `cardinality` 出现 0 次，51,310 条全量 coverage 100%。
- **加载期校验**：`Profile::from_json` 在解析成功后立刻检查，缺声明直接报错，
  不进入执行阶段。错误文案点名规则与槽位：

```
rule "root.entry" emits P0 slot(s) [odc-entry]; declare match.cardinality.min (use 0 for optional slots)
```

- **JSON Schema**：`docs/odc-profile.schema.json` 里 `$defs/rule` 的 `allOf` 条件分支
  做同一件事（draft 2020-12：`emit.classes` 命中任一 P0 ⇒ `match` 必须含 `cardinality`），
  第三方工具校验 profile 时能提前发现同样的遗漏。
- **`min: 0` 的合法用途**：槽位确实可选时（这本词典某些词条没有该结构）写 `0`，
  并在规则 `rationale` 里说明；显式写 0 表示「实测过、确实可能为 0」，与「忘了写」区分开。

## 5. 可复现性

- 无随机、无外部依赖、无网络：同一输入两次运行输出逐字节相同（`serde_json::to_string_pretty`）。
- 词条级判定互不影响；`--limit` 只截断输入，不改变判定。
- 每条记录的 `reasons` 每种 code 最多 3 条（`MAX_PER_CODE`），避免单个记录刷屏；
  `reason_counts` 是**记录数**维度的计数（一条记录同一 code 记一次）。

## 6. LDOCE6 实测（2026-11-06，基线 0254ed9 + 本任务）

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

## 7. 已知局限（判不了的情况）

1. **文案类的「改写」判不出来**：只要两侧 token 多重集相同，把 "Word Sets" 改成
   "Word Sets" 之外的任何同词重排都不会被发现（顺序只在块内判）。
2. **跨块顺序不判**：D1 的决定，块间排列由 P1 结构规则兜底；校验器只保证块内不乱。
3. **标签名残留不判**：源里用 `<h2>`、L1 里也用 `<h2>`，只要类名是 `odc-*` 就放行。
   要判这一层需要一份「中性标签」封闭清单，规范目前没有（也无法穷举）。
4. **删掉的源内容看不出来**：若 profile 把整块源内容删干净，词袋会缺词 → 能发现；
   但若被删的文本和别处重复（同一词出现两次），缺一次仍可能多重集相等。
5. **只判「源 → L1」一个方向的结构**：L1 里凭空多出的 `odc-pos-block` 结构只要文本
   在源里找得到，不会被结构规则拦（属 profile 语义问题，不在 D1 范围）。
6. **折叠条文案已按 B 取自源（2026-11-07 定案）**：profile 把 `span.popup-button`
   保留为 `span.odc-ref-caption`，chrome 把它搬进 `summary.odc-refbar__label` 的文本；
   源里没有 caption 时留空。引擎不写任何英文文案，`data-odc-ui` 退化成 label 里的
   一个空槽位 `<span data-odc-ui="<kind>"></span>`：标在整个 label 上会把源文本判成
   chrome，让尺子漏掉它。LDOCE6 45 条实测：`bag` / `order` / `structure` / `clean`
   全部 45/45。

## 8. 已知差异（2026-11-08）

OALD9 全量 59,645 条 body：`structure` / `clean` 都是 59,645（另计 1 条 raw 兜底豁免），
`bag` / `order` 是 59,621 —— 剩下的 **24 条 bag/order 失败全部是同一类 ref-panel 切分口径差异**，
不是文本丢失，也不打算修：

- key：`angling` / `blood sport` / `blues` / `breakfast` / `cartoon` / `comic` / `comic strip` /
  `dinner` / `field sports` / `fishing` / `gcse` / `hunting` / `jazz` / `lunch` / `meal` / `nct` /
  `oxbridge` / `religion` / `shooting` / `supper` / `tea` / `the church of england` /
  `the national curriculum` / `union`
- 形态：topic / wordfinder 折叠框。源里框标题与正文是两个**紧邻的行内 `span`，中间没有空白**，
  例如 `<span class="heading">More About</span><span class="body"><span class="h1">meals</span>…`；
  口径 A（inline 边界不切词）把它们拼成一个 token，报告里因此出现 `bag.missing "Aboutmeals"`。
- L1 把同一个框提升成块级结构：`p.odc-ref-panel__title`（`More About`）+ `div.odc-ref`（`meals`…），
  块边界把同一个词分开，L1 侧是两个 token `About` / `meals`。
- 结论：**字符零丢失**——24 条记录逐条比对，源文本与 L1 文本的字母数字字符多重集完全相等（24/24）。
  这只是切分点不同（源用行内 `span`、L1 用块级标题 + 正文），渲染上「标题一行 + 正文一段」更合理。
  **不修**，也**不放宽尺子的块边界守卫**：放宽会让真正的「把一个词拆成两块」漏判。

复核命令（样本重建见 [`dialect-survey.md`](dialect-survey.md)）：

````
profile_compare --profile profiles/oald9.profile.json \
  --source /tmp/odc-oald9-full/all-body.jsonl --report /tmp/all.json
````

同一天结构规则 3 的放宽（前置义项豁免，见 §3.3 第 3 条）把这批之外的 6 条
`structure.sense_sibling`（`ayrshire` / `dagwood` / `detroit` / `hereford` / `lancashire` / `said`）
从失败名单里消掉了：`structure` 由 59,639 升到 59,645，`bag` / `order` 不变。

## 9. 导入时使用 profile

`odc import` 可以直接用 profile 产出 L1，并把契约测量写进导入报告；
macOS App 的转换链路走 FFI 的同名能力。

````
odc import dictionary.mdx --format mdx --profile profiles/ldoce6.profile.json [--json]
odc_import_mdx_to_odc_with_profile(mdx, out, mdd_paths_json, profile_path, &report, &error)
```

- `--profile` 只对 `--format mdx` 生效，其他格式给了会报错退出；
  同理，`--json`（导入报告 JSON）也只在 MDX 上可用。
- 顺序：profile 先跑，输入是**源记录体**。`Profile::apply` 自己会做 repair/sanitize
  （`repair_sanitize` + `raw_fallback`），所以 profile 输出不再经过第二遍净化；
  如果 profile 不把记录变成正文（`@@@LINK=` 别名），该记录回落到原来的
  `process_html` 路径，行为与不带 profile 时一致。
- 回退语义：P0 规则未命中的记录用 `raw_fallback`（带 `data-odc-unadapted`，
  `odc-entry--raw`）落盘，计入 `contract.unadapted`，导入不中断；
  别名记录不进 contract 统计（它是指针，不是记录）。
- 报告字段（人类输出一行；`--json` 与 FFI 报告同名）：

````
imported 2 records | coverage 1/2 (50.0%) | mapped four-dim 1/1 | unadapted 1
```

| 字段 | 含义 |
| --- | --- |
| `contract` | 无 profile 时为 `null`；否则 `total` / `adapted` / `unadapted` / `coverage`（0–1 小数）/ `bag_ok` / `order_ok` / `structure_ok` / `clean_ok` / `equal` / `line` |
| `p0_failed` / `p0_failed_samples` | P0 未命中的记录数与样本 key（前若干条）；P0 命中但记录本身已带 `odc-entry` 时仍计入这里 |
| `imported_entries` | 只统计真正落盘的记录（空 key、超限记录不计入 `contract.total`） |

不带 `--profile` 时报告与旧版本逐字段一致（`contract` 为 `null`，`p0_failed` 为 0）。
