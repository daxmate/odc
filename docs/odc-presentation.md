# ODC 呈现层规范 v0

> 状态：已定稿（2026-11-06）。`docs/odc-html-spec.md` §3 引用本文件；引擎实现在 `crates/odc-core/src/presentation/`。
> 由来：2026-11-06 讨论 —— 用户验收过的 LDOCE6 对比页，内容与 profile 输出一致，但「之前手工改过的那些呈现」复现不了。

## 0. 一句话

**profile 只回答「这块是什么」，呈现层回答「怎么摆」。** 摆法对所有词典通用，不写进任何一本词典的规则包。

## 1. 证据：问题出在哪

8 条人工验收基准（`/Users/dax/odc-golden/ldoce6-demo-2026-11-06`）实测：

| 维度 | 基准 | profile 输出 | 结论 |
|---|---|---|---|
| 文本内容（词袋） | — | 5/8 完全一致，其余 3 条差 1–4 个词 | **内容没丢** |
| 文本顺序 | — | 1/8 一致 | 顺序来自呈现决定 |
| `data-odc-orig-class` | 66,453 个 | 12 个 | 派生属性缺失 |
| `id` 锚点 | 1,089 个 | 0 个 | 锚点生成缺失 |
| `<details>` 折叠 | 81 个 | 0 个 | 折叠结构缺失 |

而用户手工改动的 6 次提交，每次都同时改 `assets/odc.css`（共享）与 `ldoce6.rs`（专用）—— 说明呈现决定当时被塞进了适配器。

## 2. 三层职责

| 层 | 输入 | 输出 | 谁写 |
|---|---|---|---|
| **语义层** | 源 HTML | 槽位标注（def / example / grammar / ref-block / sense …） | profile（AI 推断 + 人审）或适配器 |
| **呈现层** | 槽位树 | 固定骨架、chrome、折叠、顺序 | **引擎，共享，不可被 profile 覆盖** |
| **派生化** | 转换过程 | `orig-*` 属性、锚点 id、资源 URL | **引擎，转换时顺手盖章** |

## 3. 规范条目

### P1 文档骨架与顺序（固定）

```
section.odc-entry[data-odc-dict, data-odc-key, data-odc-lang, data-odc-homograph?]
├── header.odc-head                 ← 词头（hw / hw-alt / phon / inflection / label）
├── nav.odc-refbar                  ← 折叠条：Entry menu + 词条级引用块
└── section.odc-pos-block[data-odc-pos, id]   ← 每个词性一块，按源顺序
    └── div.odc-sense[data-odc-n] …（含 odc-usage-note 语法框、odc-subsense）
```

- 词性块之间按**源文档出现顺序**；块内义项按源顺序。
- **词条根唯一**：弹窗里引用的其它词条（`span.entry`）会随骨架重建变成嵌套 `section.odc-entry`，呈现层把它们降级为 `div.odc-entry-inner`（属性、子节点、文本一律保留），保证一个片段只有一个词条根。
- 引用块（搭配/同义词/词族/词源/用法盒）**不留在义项流内**，一律进 `nav.odc-refbar`（P3）。
- 例外：语法框（`odc-usage-note[data-odc-kind=grammar]`）留在所属义项流内（c6f0934 定案）。

### P2 chrome 由引擎生成（profile 不写）

| 元素 | 生成规则 |
|---|---|
| Entry menu | 词条开头一个 `details.odc-refbar__item[data-odc-kind=toc]`，内容由义项列表重建 |
| 引用条容器 | `nav.odc-refbar` 恒存在；无引用块时为空 |
| 词头 | `header.odc-head` 由 `odc-hw/phon/pos/inflection/label` 槽位装配 |

理由：这三样 8 条基准全都一样，与词典无关；让 profile 复述只会让规则臃肿、让模型更容易推错。

### P3 引用块折叠

- 每个引用块 → `details.odc-refbar__item[data-odc-kind=<种类>]`
- `<summary class="odc-refbar__label">` 只带标记 `data-odc-ui="<kind>"`，**不写文案**（D2）：引擎不翻译、不改写、也不发明英文
- 内容包在 `div.odc-ref-panel` 内
- 种类枚举：`toc | thesaurus | collocation | word-family | word-origin | usage | other`

### P4 UI 文案与内容分离

- 「Entry menu」是 **UI 文案**，不是词典内容。
- L1 中写成 `<summary class="odc-refbar__label" data-odc-ui="entry-menu"></summary>`（空文本 + 标记），由客户端按语言注入。
- **理由**：现状把英文硬编码进正文，导致 (a) 中文用户看到英文；(b) 文本保真统计把 UI 文案算成「差异」。

### P5 派生属性（引擎盖章）

转换过程中，凡发生 `rename` / `unwrap` / `move` 的元素，自动保留源属性：

| 属性 | 来源 | 说明 |
|---|---|---|
| `data-odc-orig-class` | 源 `class` | 溯源与二次加工的唯一依据（基准 66,453 个） |
| `data-odc-orig-src` | 源 `src` | 资源原始路径 |
| `data-odc-orig-anchor` | 源 `id` / `name` | 锚点保留 |
| `data-odc-orig-target` | 源 `href` | 交叉引用原目标 |

**规则**：这三个属性不由规则包声明，引擎无条件产出；`data-odc-rule=<id>` 同时盖在被该规则处理过的节点上（现有行为保留）。

### P6 锚点 id（引擎生成）

```
{key}-{pos}            词性块
{key}-{pos}-s{n}       第 n 个义项
{key}-{pos}-s{n}{a..}  子义项
```

- 只对 `odc-pos-block` 与 `odc-sense` 生成；同一文档内必须唯一。
- 依据：基准 1,089 个 id，profile 输出 0 个；交叉引用（`odc-crossref`）依赖它跳转。
- **源锚点别名**：源记录用 `<a id="uuid">` + `entry://#uuid` 互相引用，L1 用生成的 id。capture 在锚点元素本身与它后面的块上记账，装配期按文档顺序解析成 `{key}-{pos}[-s{n}]`，并改写页内 `#…` 链接（含 `data-odc-target`）。
- **不发明 id**：解析不到生成 id 的源锚点保持原样，只记一条 `notes`；给元素补 id 会改 P6 契约。

### P7 资源 URL

- `sound://x.mp3` → `odc-resource://x.mp3`，双写 `data-odc-src` + `data-odc-orig-src`
- 图片相对路径同理：`img[src]`、`source[src]` 走同一套解析，写 `src` + `data-odc-orig-src`
- 解析只看 scheme，不看词典：`sound://` 与普通/绝对路径进 pack；`data:`、`http(s)://`、`mailto:`、协议相对 `//host/x` **原样保留**并记一条 `notes`
- 依据：基准 2,687 处音标按钮

### P8 呈现层自检（装配后必跑）

1. 骨架顺序符合 P1
2. `nav.odc-refbar` 存在；`details.odc-refbar__item` 的 summary 要么带源文案、要么带 `data-odc-ui`
3. `id` 唯一
4. 每个 `odc-sense` 有 `data-odc-n`
5. 无 `data-odc-orig-*` 缺失的 rename 节点

五项之外的诊断（保留原样的资源引用、解析不到 id 的源锚点）走 `CheckReport.notes`，不计入通过/失败，也不改变「检查项恒为五条」。

## 4. 与现有 `docs/odc-html-spec.md` 的偏差（必须合并）

| 项 | 文档写的 | 实际产出（基准） | 处理 |
|---|---|---|---|
| 词头 | `header.odc-entry-head` | `header.odc-head` | 以实际为准，改文档 |
| 义项菜单 | `nav.odc-toc` | `details.odc-refbar__item[data-odc-kind=toc]` | 以实际为准，改文档 |
| 义项容器 | 未明确 | `div.odc-sense[data-odc-n]` | 补进文档 |
| 折叠 | 未提 | `details/summary` | 补 P3 |
| 引用块 | `nav.odc-refbar` | 同名，但内部结构未定义 | 补 P3 |

## 5. 实现顺序与验收

| 步 | 内容 | 预期效果 |
|---|---|---|
| 1 | P5/P6/P7 引擎派生 | 吃掉 36/45 首差（属性、id、音频） |
| 2 | P2/P3/P4 chrome 装配 | 吃掉顺序差异（Entry menu、refbar 位置） |
| 3 | P1 + 文档合并 | 规范自洽 |
| 4 | 重跑 8 条基准 | 目标：结构 + 文本全等，允许 UI 文案按 P4 差异 |

## 6. 已拍板（2026-11-06，大象）

| 编号 | 决策 | 结论 |
|---|---|---|
| D1 | 文本保真口径 | **A**：保真 = 词袋 + 块内顺序；跨块顺序由呈现层决定 |
| D2 | UI 文案 | **A**：L1 只留 `data-odc-ui` 标记，文案由客户端注入 |
| D3 | 呈现层可覆盖性 | **A**：结构不可覆盖；只允许词典级 CSS 换皮 |

### D1 落地表述（替换 odc-html-spec.md §8.1）

> **文本保真**：源片段与 L1 产物在去标签后的文本**词袋**上必须完全一致（同一多重集）；每个语义块（`odc-sense` / `odc-ref-block` / `odc-examples` …）**内部**的文本顺序必须与源一致。跨块排列顺序由呈现层规范（P1）决定，不参与保真判定。
>
> **落地判定**：`profile_compare --source`（实现 `profile/contract.rs`），规则、实测与局限见 [`profile-contract.md`](profile-contract.md)。

### D2 落地表述

> UI 文案（Entry menu 等）不进入 L1 文本：`<summary class="odc-refbar__label" data-odc-ui="entry-menu"></summary>`。客户端按语言注入；文本保真统计排除带 `data-odc-ui` 的元素文本。

### D3 落地表述

> 词典包不得改变 L1 结构；只能通过统一样式入口提供 CSS 变量/选择器覆盖视觉样式。
