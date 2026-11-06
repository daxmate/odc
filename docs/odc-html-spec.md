# ODC 词条 HTML 规范 v0.1（实验性草案）

> 状态：**v0.1 实验性草案（spike）**，未定稿。定稿前不合并 main、不发布；按本规范产出的 HTML 不承诺向后兼容。
> 范围：只定义**词条正文 HTML 层**，即 L1 归一化 DOM、L2 `odc-*` 词汇、L3 适配器接口。
> 容器格式、payload 字段、块格式、哈希与提交语义以 [`odc-spec.md`](odc-spec.md) 为唯一权威，本文件不改动其中任何一条。
> 证据：9 本 MDX 方言调研（`/tmp/odc-dialect-survey/`）与 12 个高频多义词结构深挖（`/tmp/odc-polysemy-survey/`），2026-11-04。引用格式见附录 A。
> v0.1 另据 LDOCE6++ 适配器原型的 10 条实测问题修订（200+12 条样本，2026-11-05，`/tmp/odc-ldoce6-adapter/`）；逐条对应见文末「变更记录」。

## 1. 状态与范围

### 1.1 问题

当前导入把 MDX 记录正文**原样**写入 `payload.raw_html`：能显示，但无法检索、无法统一样式、无法做结构操作。9 本词典实测有 **464 个不同 class**，跨「不同标记体系」在 3 组以上共现的只有 3 个（`entry` / `pos` / `gram`），且同名不同义；交互还绑在 `onclick`（LDOCE 三本分别 3,709 / 484 / 449 处）与内联 JS 上。

本规范的回答：**不追求「一套选择器吃下所有词典」**，而是定义一层与来源无关的归一化 DOM（L1）+ 词汇表（L2），由每本词典一个适配器（L3）把方言翻译进来；翻译不了的保留原样并标记。

### 1.2 非目标

- **不做义项对齐**：跨词典义项不合并、不建立编号映射；同一词的 LDOCE 义项 7 与 OALD 义项 3 各自独立。
- **不做逐条 AI 归类**：本地模型只用于「从样本推断映射规则」，全量执行是确定性规则（见 5.2）。
- **不改容器格式**：`payload` 的 5 个字段、块编码、`content_hash`、提交流程全部沿用 `odc-spec.md`。
- **不定义视觉设计**：只规定样式锚点与禁用项，具体排版由 ODC 自带 CSS 决定。
- **不保证旧仓库自动升级**：v0.1 不提供迁移（见 9）。

### 1.3 与现有实现的关系

导入管道已经完成「净化」，本规范在其**之后**运行：

| 阶段 | 已有实现（`docs/mdx-compat.md`） | 本规范的位置 |
|---|---|---|
| 解析 | `html5ever` 以 `div` 为上下文解析片段，按 HTML5 规则修复未闭合/错配标签 | 复用；适配器只面对已修复的树 |
| 净化 | 删除 `script` / `iframe` / `object` / `embed` / `on*` / `javascript:` | 复用；L1 额外要求删除渲染噪声（3.6） |
| 样式 | `<style>` 与 `style=` 交 `lightningcss` | 本规范**不消费**原词典 CSS（决策 D1） |
| 排版 | 块级格式化器（分支 `feat/block-html-formatter`，v0.14.0）：换行只在块级边界、标签不拆、文本逐字节复制 | L1 空白规则与之一致（3.3） |
| 归一化 | — | **本规范新增**：方言 → L1 DOM |

### 1.4 与容器层的接口（不改容器）

- 归一化产物写入 `payload.raw_html`；`raw_html` 的语义从「原始片段」变为「**当前最佳渲染片段**」（归一化成功或兜底原样）。
- 原片段不另存副本：源 `.mdx` 即底稿，与现有导入策略一致（`docs/mdx-compat.md` 失败策略）。
- `defs` / `phonetic` 是否由归一化结果回填**不在本规范范围**，列为未决（附录 B-1）。
- 归一化会改变正文 → `content_hash` 变化 → 产生新提交；不重写历史，不自动迁移旧仓库。

## 2. 术语

| 术语 | 定义 |
|---|---|
| 词条（entry） | 一个 key 对应的一条正文；一个源记录一个根。同形异条可多根（7.6），也可能同根多词性（7.6） |
| 词性段（pos-block） | 词条内按词性划分的段；**它是义项编号的作用域**（LDOCE 每个词性从 1 重新编号） |
| 义项（sense） | 带编号的最小释义单元；可含释义、译文、例句、子义项、搭配 |
| 义项作用域（scope） | 编号的归属范围：词性段 id 或短语动词 id；同一词条内可有多个作用域 |
| 子义项（subsense） | 义项下的二级单元；通常用字母（a/b/c），**与数字编号是两套独立空间** |
| 短语动词（phrasal verb） | 独立的子条目（有自己的词头、音标、编号、锚点），**不是 sense 的子节点** |
| 短语索引（phrase index） | 只有短语文本与链接、没有释义的条目清单（如 LDOCE `PHRASES FROM OTHER ENTRIES`） |
| 习语（idiom） | 带释义的固定表达块（如 OALD `idm-gs`） |
| 参考块（reference block） | 挂在词条级（非某个义项）的资料块：搭配、同义词、词族、词源、用法盒 |
| 适配器（adapter） | 每本词典一个的翻译器：指纹识别、解析、资源解析、可选样式 |
| 方言（dialect） | 某本词典特有的标记方式（class 方言 / 自定义标签 / 无标记 token） |
| raw_html 兜底 | 适配器不可用或失败时，保留净化后的原片段并在 L1 外层标记「未适配」 |
| 别名（alias / redirect） | MDX 记录正文只有 `@@@LINK=<target>`、没有独立内容的条目；应指向目标词条 |
| 块重建（block rebuild） | 按文档顺序把不闭合的源标记重建成扁平块序列的前置阶段（5.2） |
| 未映射块（unmapped） | 不在 30 槽位内但带文本的块；包进 `.odc-unmapped` 逃生口，不产生 odc 语义 |
| 同形组（homograph group） | 拼写相同、词性或来源不同的词条；组内序号用 `data-odc-homograph` 标注 |

## 3. L1 归一化 DOM

### 3.1 根元素与骨架

```html
<section class="odc-entry" data-odc-dict="ldoce6encn" data-odc-key="make" data-odc-lang="en">
  <header class="odc-entry-head">
    <h1 class="odc-hw">make</h1>
    <span class="odc-hw-alt" data-odc-kind="hyphenation">make</span>
    <span class="odc-phon" data-odc-variant="uk">/meɪk/</span>
    <span class="odc-inflection" data-odc-kind="past">made</span>
  </header>

  <nav class="odc-toc" data-odc-counted="0"><!-- 可选；可由 sense 列表重建；不参与义项统计 --></nav>

  <section class="odc-pos-block" data-odc-pos="verb" id="make-verb">
    <h2 class="odc-pos">verb</h2>
    <div class="odc-sense-group" data-odc-label="create">
      <div class="odc-sense" data-odc-n="1" data-odc-label="1" data-odc-scope="make-verb" id="make-verb-s1">
        <span class="odc-signpost">create</span>
        <span class="odc-def">to produce something
          <span class="odc-def-trans" data-odc-lang="zh">制作；制造</span>
        </span>
        <ul class="odc-examples">
          <li class="odc-example">
            <span class="odc-ex-text">Diane makes all her own clothes.</span>
            <span class="odc-ex-trans" data-odc-lang="zh">黛安娜的衣服都是自己做的。</span>
          </li>
        </ul>
        <div class="odc-subsense" data-odc-n="a" data-odc-label="a)" id="make-verb-s7-a">…</div>
      </div>
    </div>

    <section class="odc-phrasal-verbs">
      <article class="odc-phrasal-verb" data-odc-lemma="make away with" id="make-verb-pv1">
        <h3 class="odc-phrasal-verb-hw">make away with</h3>
        <div class="odc-sense" data-odc-n="1" data-odc-scope="make-verb-pv1" id="make-verb-pv1-s1">…</div>
      </article>
    </section>
  </section>

  <!-- 同形词不并入本根：第二个词条是同级 <section class="odc-entry">，见下方与 7.6 -->

  <section class="odc-refs">
    <section class="odc-ref" data-odc-kind="collocation" data-odc-source="corpus">…</section>
    <section class="odc-ref" data-odc-kind="thesaurus" data-odc-source="entry">…</section>
  </section>

  <nav class="odc-crossrefs">
    <a class="odc-crossref" href="#odc-time-noun-s1" data-odc-target="time-noun-s1">time</a>
  </nav>
</section>

<!-- 同形词：make² 是同级条目，自己的 head / refbar / pos-block -->
<section class="odc-entry" data-odc-dict="ldoce6encn" data-odc-key="make" data-odc-homograph="2" data-odc-lang="en">
  <header class="odc-entry-head">
    <h1 class="odc-hw">make</h1>
    <span class="odc-hw-alt" data-odc-kind="homograph">2</span>
  </header>

  <nav class="odc-refbar">…只含本词条的引用块，不与 make¹ 合并…</nav>

  <section class="odc-pos-block" data-odc-pos="noun" id="make-noun">
    …
  </section>
</section>
```

**语义承载只用 `class="odc-*"`**（决策 D3）：无需自定义元素，普通 HTML 元素 + 固定 class 即可；元素名按语义选（`section` / `article` / `nav` / `ul` / `li` / `span`）。

**别名（未在导入层消解时）**：正文只有 `@@@LINK=<target>` 的记录如果没被消解，必须产出空容器，不把 `@@@LINK=` 当正文：

```html
<section class="odc-entry odc-entry--redirect"
         data-odc-dict="ldoce6encn" data-odc-key="GMO" data-odc-lang="en"
         data-odc-target="GMO"></section>
```

- **导入层应优先消解别名**：LDOCE6++ 265,545 条记录中 212,797 条（80.1%）是 `@@@LINK=` 别名，只有 52,735 条是真词条；把别名写成独立正文会让全文检索重复、统计失真。推荐做法是只保留 `(key → target)` 映射，不落 HTML。
- 未消解时：`data-odc-target` 存目标 key；容器**无 sense、无编号、无正文**；不参与 8.2 的槽位统计与 8.1 文本保真比较（目标串不是正文）。
- 渲染器把 `odc-entry--redirect` 当跳转处理，不渲染空正文、不显示「未适配」标记。

**同形词（一条源记录含多个词条）**：LDOCE6++ 的 `make` 一个 key 内含 `make1` + `make2`，输出为**同级**的多个 `section.odc-entry`；每个根上的 `data-odc-homograph` 从 1 起编号，各自带独立的 `header`、`nav.odc-refbar` 与 `.odc-pos-block`。词头一律写作 `h1.odc-hw`，不再有浮在 pos-block 外的 homograph-headword。规则细节见 7.6。

### 3.2 元素与嵌套骨架（白名单）

| 元素 | 允许的父 | 允许的子 | 说明 |
|---|---|---|---|
| `.odc-entry` | 无（片段根） | `.odc-entry-head`、`.odc-toc`?、`.odc-pos-block`+、`.odc-refs`?、`.odc-crossrefs`? | 1 个词条 1 个根 |
| `.odc-entry-head` | entry | `.odc-hw`、`.odc-hw-alt`*、`.odc-phon`*、`.odc-inflection`* | 1 |
| `.odc-pos` | pos-block | 文本 | 词性标签，1 |
| `.odc-pos-block` | entry | `.odc-pos`、`.odc-sense-group`*、`.odc-sense`+、`.odc-phrasal-verbs`? | 1..N |
| `.odc-sense-group` | pos-block | `.odc-sense-group`（可嵌套）、`.odc-sense`+ | 分类头/大类 |
| `.odc-sense` | pos-block / sense-group / subsense / 短语动词 | `.odc-signpost`?、`.odc-grammar`?、`.odc-label`*、`.odc-def`、`.odc-subsense`*、`.odc-examples`?、`.odc-collocation`*、`.odc-citations`? | 1..N |
| `.odc-subsense` | sense | 同 sense（不含 def 以外的必需项） | 0..N |
| `.odc-citations` | sense | `.odc-citation`+ | 0..1 |
| `.odc-citation` | citations | `.odc-citation-text`、`.odc-citation-source`? | 0..N |
| `.odc-examples` | sense | `.odc-example`+ | 0..1 |
| `.odc-example` | examples | `.odc-ex-text`、`.odc-ex-trans`?、`.odc-ex-source`? | 0..N |
| `.odc-phrasal-verb` | `.odc-phrasal-verbs` | `.odc-phrasal-verb-hw`、`.odc-phon`*、`.odc-sense`+ | 0..N |
| `.odc-ref` | refs | 自由（块内允许 `ul/li/table/p`），但**不得含 sense 语义** | 0..N |
| `.odc-crossref` / `.odc-homograph` | crossrefs / homographs | 文本 | 0..N |
| `.odc-entry--redirect` | 根上的修饰类（与正文互斥） | 无（空元素） | 0..1/词条 |
| `.odc-unmapped` | entry / pos-block / ref | 原文子节点与文本 | 0..N |
| `.odc-entry[data-odc-homograph]` | 片段根（多条并列） | 与其他 `.odc-entry` 同级 | 0..N/记录 |


### 3.3 空白与文本保真

1. **文本节点逐字节复制**：进入 L1 的文本不做实体重编码、不做空白折叠、不做首尾 trim。依据：块级格式化器的既有保证（标签不拆、文本逐字节）。
2. **换行只允许插在块级兄弟之间**：`.odc-entry` / `.odc-pos-block` / `.odc-sense` / `.odc-examples` / `.odc-example` / `.odc-ref` / `.odc-phrasal-verbs` / `.odc-phrasal-verb` 的边界。**禁止**在行内元素（`span`、`a`、`em`）之间或内部插换行。
3. **行内空白保留**：源片段里 `word<span class="neutral"> </span>word` 的空格属于正文，必须保留；不得因缩进产生额外空格。
4. **`pre` 例外**：`pre` 内原样保留，不重排、不缩进；`pre` 内的换行计入正文。
5. **不可见源字符**：`\r\n` 统一为 `\n`（仅换行归一，不算文本改写）；其余控制字符保留。
6. **文本保真按「空白归一后」比较**：块边界插入的换行/缩进不承载语义；判据按 8.1 的抽取算法（丢弃纯空白节点、折叠空白 run、去掉片段首尾空白）比较。L1 本身仍保留原文本，不做 trim、不做折叠。

### 3.4 id / anchor / `data-odc-*` 命名与唯一性

**命名**

| 对象 | 规则 | 例 |
|---|---|---|
| 词性段 id | `<entry-slug>-<pos>` | `make-verb` |
| 义项 id | `<scope>-s<n>` | `make-verb-s1` |
| 子义项 id | `<scope>-s<n><label>` | `make-verb-s7a` |
| 短语动词 id | `<entry-slug>-<pos>-pv<n>` | `make-verb-pv1` |
| 短语动词内义项 | `<pv-id>-s<n>` | `make-verb-pv1-s2` |
| entry-slug | 归一化 key 的 URL-safe 形式：小写、空格转 `-`、非 `[a-z0-9-]` 转 `-`、压缩连续 `-`；结果为空（纯非拉丁 key）时用 `u` + 各字符 U+ 码点的十六进制（A–F 大写，`-` 分隔） | `make-up`；`打` → `u6253` |
| 词性段 pos 片段 | 词性文本小写化、空格转 `-`；缺词性时用 `x` | `verb`、`phrasal-verb` |

- `id` 字符集限定 `[a-z0-9-]`；**不含冒号、点、下划线**，避免 CSS 选择器与 URL 冒号转义问题（HTML5 允许更宽，这里自我收窄）。
- **唯一性**：同一片段内所有 `id` 必须唯一。构造时冲突（如 OALD 的 `cixing_part` 复用）按**文档顺序**追加 `-2`、`-3`；同一输入两次运行必须得到同一结果（确定性）。
- **稳定性**：`id` 只依赖 (entry key, pos, 编号路径)，不依赖类名、偏移或哈希；重新导入同一词典不得改变。
- 原文锚点（`<a name="...">`、`entry://...#...`）不进入 `id`，保存为 `data-odc-orig-anchor`，供交叉引用重写与排障。

**`data-odc-*` 属性总表**

| 属性 | 取值 | 出现位置 | 说明 |
|---|---|---|---|
| `data-odc-dict` | 词典 slug | entry | 来源标识（`ldoce6encn` …） |
| `data-odc-key` | 归一化 key | entry | 与容器 key 一致 |
| `data-odc-lang` | BCP-47 或 `zh` / `en` | entry / 译文节点 | 缺省由适配器声明 |
| `data-odc-pos` | 词性文本 | pos-block | 原样保留（`verb`、`noun`、`phrasal verb`） |
| `data-odc-n` | 整数或字母 | sense / subsense | **规范编号**，用于排序与检索 |
| `data-odc-label` | 显示文本 | sense / subsense / sense-group | 原样显示（`1`、`a)`、`①`、`（Ⅰ）`） |
| `data-odc-scope` | 作用域 id | sense | 必填；指向 pos-block 或短语动词的 id |
| `data-odc-homograph` | 整数（组内序号，从 1 起） | entry（独立同形 key）/ pos-block（同记录多词性） | 同形组标注；见 7.6 |
| `data-odc-redirect` | `1` | entry（`odc-entry--redirect`） | 别名标记；目标在 `data-odc-target` |
| `data-odc-counted` | `0` / `1` | toc 等派生视图块 | `0` = 只保文本，不计义项、不参与编号与槽位统计；缺省视为 `1` |
| `data-odc-orig-class` | 原 class 串 | 任意 | 降级保留，不参与渲染 |
| `data-odc-orig-anchor` | 原锚点 | 任意 | 排障与链接重写 |
| `data-odc-orig-tag` | 原标签名 | 任意 | 排障；仅在被替换的私有标签上写 |
| `data-odc-orig-src` | 原资源引用 | audio / img / source | 排障；解析失败时唯一线索 |
| `data-odc-orig-target` | 原链接目标 | crossref / homograph | 排障；重写链接时保留 |
| `data-odc-src` | `odc-resource://…` | audio / img / source | 见 3.5 |
| `data-odc-missing` | `1` | audio / img / crossref | 资源或目标不可解析 |
| `data-odc-kind` | 枚举：`collocation` / `thesaurus` / `word-family` / `word-origin` / `usage-note` / `hyphenation` / `homograph` / `homograph-headword` / `past` / `plural` … | ref / hw-alt / inflection | 块或变体类型 |
| `data-odc-source` | `entry` / `other-entries` / `corpus` | ref | 参考块来源 |
| `data-odc-target` | 目标 id 或 key | crossref / homograph / toc | 归一化后的链接目标 |
| `data-odc-variant` | `uk` / `us` / `gb` / `us-2` | phon / audio | 音标/读音变体 |

- **禁止**用 `data-odc-*` 承载原词典的任意属性（如 `data-odc-orig-onclick`）；需要保留的原文属性一律进 `data-odc-orig-*` 且只用于排障。
- 属性值不含 HTML，不做二次转义；序列化时按常规属性转义。

### 3.5 资源 URL 形式

归一化层统一用 `odc-resource://<资源 key>`，与容器 `resources.key` 的规范化规则一致（`docs/odc-spec.md` §9：反斜杠转 `/`、去前导 `/` 与 `./`、大小写保留）：

```html
<button class="odc-audio" data-odc-src="odc-resource://prons/make__gb_1.mp3" data-odc-variant="uk"></button>
<img class="odc-img" src="odc-resource://img/make_pic.jpg" alt="">
```

- 原文的 `sound://`、相对路径、`<audio name>`、`<img src>` 一律由适配器 `resolve()` 转成资源 key；同时保留 `data-odc-orig-src` 供排障。
- 解析不到 → 保留元素、置 `data-odc-missing="1"`、不写 `src`（或写占位），**不得**降级成 `http(s)` 外链（实测 0 例外链）。
- 禁止 `data:` URI 与 base64 内嵌（实测 0 处）。
- 音频寻址差异（LDOCE `sound://`、OALD9cn `<audio name>`、MWALED `div.play_pron`、OALD8 `a.fayin`）全部收敛到 `resolve()`。

### 3.6 禁止项与降级

**不得进入 L1**：

| 禁止项 | 证据 | 处理 |
|---|---|---|
| `<font>` | 汉典 18 处 | 去掉标签，保留文本 |
| `style=` 内联样式 | ldoce6 22 处、oald9 4 处、oald8cn 3 处 | 去掉属性 |
| `on*` 事件属性 | LDOCE 三本 3,709 / 484 / 449 处 | 由净化阶段删除；L1 不接收 |
| `<script>` / `<link rel=stylesheet>` / `<head>` | mwaled 记录自带 `head` + `link`（@142） | 丢弃 head，只取体内容 |
| 私有标签 | `l6` / `sigt` / `deft` / `exat` / `<EN>` / `<TRAN>` / `<xhtml:*>` | 解包成 `span`，语义进 `odc-*` |
| 位置类噪声 class | `wrap` / `separator` / `prefix` / `neutral` | 丢弃 class，文本不动 |
| 折叠态标记 | mwaled `m_hidden` / `collapsed` | 丢弃，展开渲染 |

降级原则：**语义保留、载体更换**。原文 class 记入 `data-odc-orig-class`；原文标签名记入 `data-odc-orig-tag`（可选）。

**未知块逃生口（v0.1 新增）**：不在白名单、但**带文本**的块（如 LDOCE 词族树 `span.wf`、搭配表行 `div.content`）不得直接丢弃，也不得硬塞给不存在的槽位——包成 `div.odc-unmapped`，原标签名/原 class 记入 `data-odc-orig-tag` / `data-odc-orig-class`，内部保留原文子节点与文本。`.odc-unmapped` **不产生任何 odc-* 语义**（不含 sense、编号、槽位），只保证内容不丢、样式可局部接管。不带文本的未知块（纯布局容器）直接解包，不留空壳。

## 4. L2 词汇表

### 4.1 分层原则

- **P0（10 个）**：没有它多义词渲染不出来（词根、词头、词性段、义项、编号、释义、译文、例句、短语动词、别名容器）。
- **P1（15 个）**：结构完整性/可检索性；有则支持，缺失不阻塞渲染（含 v0.1 新增的未映射兜底块）。
- **P2（5 个）**：增强项，v0.1 只定名不求实现。
- 合计 **30 个槽位**（v0 是 28 个；v0.1 新增 `odc-entry--redirect` 与 `odc-unmapped`）。
- 所有槽位**允许为空**（除 P0 的结构必需项）：不同词典天然缺项（汉典没有搭配/同义块，OALD 没有 LDOCE 目录）。

### 4.2 P0 槽位（10）

| 槽位 | 语义 | 允许子元素 | 基数 | 关键属性 | 证据 |
|---|---|---|---|---|---|
| `odc-entry` | 词条根，一个源记录一个根 | entry-head、toc?、pos-block+、refs?、crossrefs? | 1 | dict / key / lang | ldoce6ec/make@590；mwaled/make@142；hanyu/打@56 |
| `odc-entry--redirect` | 未消解的别名（空容器，指向目标 key） | 无 | 0..1/词条 | target、redirect | `ldoce6encn` 212,797 / 265,545 条记录（80.1%） |
| `odc-entry-head` | 词头 + 音标 + 屈折 | hw、hw-alt*、phon*、inflection* | 1 | — | ldoce6ec/make@590；mwaled/make@142；hanyu/打@84 |
| `odc-pos-block` | 词性分段，**编号作用域** | pos、sense-group*、sense+、pvs? | 1..N | pos、id | ldoce6ec/make@973；oald9cn/make@405；mwaled/make@467 |
| `odc-sense` | 编号义项 | signpost?、grammar?、label*、def、subsense*、examples?、collocation*、citations? | 1..N | n、label、scope、id | ldoce6ec/make@679559；oald9cn/make@6888；mwaled/make@1174；hanyu/打@146 |
| `odc-sense-number` | 编号载体（可省：编号也可直接在 `data-odc-n`） | 文本 | 0..1/义项 | scope、n、label | ldoce6ec/make@679587；oald9cn/make@7093；mwaled/make@973 |
| `odc-def` | 释义（可含多段，**译文是其子元素**） | 行内 + `odc-def-trans`? | 1/义项 | — | ldoce6ec/make@12373；oald9cn/make@7232；mwaled/make@1293；hanyu/打@163 |
| `odc-def-trans` | 释义译文 | 行内 | 0..1/释义 | lang | ldoce6ec/make@12467；oald9cn/make@7042 |
| `odc-example` | 单个例句 | ex-text、ex-trans?、ex-source? | 0..N | — | ldoce6ec/make@12551；oald9cn/make@7827；mwaled/make@1454；hanyu/打@239 |
| `odc-phrasal-verb` | 短语动词**子条目** | phrasal-verb-hw、phon*、sense+ | 0..N | lemma、id | ldoce6ec/make@727532；oald9cn/make@127557；mwaled/make@42812 |

说明：

- `odc-sense-number` 在 v0.1 仍是**可选**槽位：容器属性 `data-odc-n` / `data-odc-label` 已能表达编号，显式元素只用于需要独立样式或独立锚点的场合（LDOCE 的 `sensenum` 有独立 class）。

### 4.3 P1 槽位（15）

| 槽位 | 语义 | 允许子元素 | 基数 | 关键属性 | 证据 |
|---|---|---|---|---|---|
| `odc-sense-group` | 义项大类/分类头 | sense-group*、sense+ | 0..N | label | oald9cn/make@6888；hanyu/打@74 |
| `odc-subsense` | 二级义项（字母编号） | 同 sense（子集） | 0..N | n、label、id | ldoce6ec/make@693150；mwaled/make@1210 |
| `odc-signpost` | 义项导语/分类词（create / do / mark…） | 行内 | 0..1/义项 | — | ldoce6ec/make@679618；oald9cn/make@6976 |
| `odc-ex-trans` | 例句译文 | 行内 | 0..1/例句 | lang | ldoce6ec/make@12551 起；oald9cn 相邻 `chn` |
| `odc-ex-source` | 例句出处 / 语料标识 | 行内 | 0..1/例句 | — | hanyu/打@239（《梦赋》）；ldoce6ec 语料块 header |
| `odc-idiom` | 习语条目（**带释义**） | hw、sense+ | 0..N | lemma | oald9cn/make@96777；ldoce6ec/make@521716 |
| `odc-phrase-index` | 短语索引（**只有链接，无释义**） | 链接项+ | 0..N | source | ldoce6ec/make@521716（go 632 条） |
| `odc-grammar` | 语法模式 | 行内/模式串 | 0..N | — | ldoce6ec/make@679713；oald9cn/make@36169；mwaled/make@1085 |
| `odc-collocation` | 搭配（可分组、多来源） | 行内/子列表 | 0..N | source | ldoce6ec/make@688928；oald9cn/make@7944 |
| `odc-usage-note` | 用法说明/学习盒 | 段落、列表 | 0..N | kind（HELP/SYN/NOTE/OPP/ORIGIN/GRAMMAR） | oald9cn/make@15152；mwaled/make@33724；ldoce6ec/back（grambox） |
| `odc-crossref` | 交叉引用（带目标锚点） | 文本 | 0..N | target、missing? | ldoce6ec/make@682700；oald9cn/make@1505；mwaled/make@35223 |
| `odc-label` | 语域/学科标签（literary 等） | 文本 | 0..N | kind | mwaled/make@33724；ldoce6ec `registerlab` |
| `odc-citation` | 书证（中文特有） | 引文、出处 | 0..N | source | hanyu/打@239 |
| `odc-homograph` | 同形词/同形异条链接 | 文本 | 0..N | target、homograph-id | ldoce6ec 独立 key（`Time` vs `time`）；ldoce6 `back1/back2` |
| `odc-unmapped` | 未映射但带文本的块（逃生口，不产生 odc 语义） | 原文子节点与文本 | 0..N | orig-tag、orig-class | 原型样本：`span.wf`、`div.content` |

### 4.4 P2 槽位（5）

| 槽位 | 语义 | 允许子元素 | 基数 | 关键属性 | 证据 |
|---|---|---|---|---|---|
| `odc-toc` | 词条目录（派生视图；保文本、不计义项） | 链接项+/文本 | 0..1 | counted=0 | ldoce6ec/make@1679（仅 LDOCE 有）；LDOCE `entrymenu` |
| `odc-thesaurus` | 同义/联想词 | 词条列表 | 0..N | source | ldoce6ec/make@19014；oald9cn/make@15152 |
| `odc-word-family` | 词族 | 词列表 | 0..1 | — | ldoce6ec/make@678840 |
| `odc-word-origin` | 词源 | 段落 | 0..1 | — | ldoce6ec/make@678000 附近；oald9cn `ORIGIN` 盒 |
| `odc-pron-history` | 反切/中古音/注音 | 文本 | 0..N | kind | hanyu/打@84 后（`〔《广韵》德冷切…〕`） |

### 4.5 命名对照（易混项）

| 语义 | 规范化类名 | 不使用 |
|---|---|---|
| 例句列表 | `ul.odc-examples` | `ul.odc-ex` |
| 单个例句 | `li.odc-example` | `li.odc-ex`（旧草案缩写） |
| 例句正文 | `.odc-ex-text` | `.odc-ex-src`（会与「出处」混淆） |
| 释义 | `.odc-def` | `.odc-def-text`（不建中间层） |
| 参考块容器 | `.odc-ref[data-odc-kind]` | 每类一个顶层类名 |
| 词条头 | `header.odc-entry-head` | v0 短名（已废弃，不兼容） |
| 短语动词列表 | `section.odc-phrasal-verbs` | v0 缩写（已废弃） |
| 短语动词条目 | `article.odc-phrasal-verb` | v0 缩写（已废弃） |

## 5. L3 适配器接口

### 5.1 接口契约

每本词典一个适配器；接口与实现语言无关，以下为语义契约。

| 方法 | 输入 | 输出 | 约束 |
|---|---|---|---|
| `probe(fragment)` | 净化后的片段（字符串或已解析树） | `{ dict, confidence, signals[] }` 或 `null` | 只读、确定性、不得改写输入；`confidence ∈ [0,1]`；`signals` 列出命中的 class/标签及计数 |
| `parse(fragment, ctx)` | 净化片段 + 上下文（key、lang、字典元数据） | L1 DOM（字符串或树） | 纯函数；不读网络、不读时钟；同一输入必须逐字节同输出 |
| `resolve(ref, ctx)` | 原始引用（`sound://…`、`img/…`、`<audio name>`） | 资源 key 或 `null` | 只做映射，不读资源内容；`null` 触发 `data-odc-missing` |
| `styles()` | 无 | 带作用域的 CSS 字符串或 `null` | 作用域固定为 `.odc-entry[data-odc-dict="<slug>"]`；不得含 `@import`、外链、`javascript:` |

- **调用顺序**：`probe` → `parse` → `resolve`（对 parse 产物中的引用）→ `styles`（可选）。
- **失败即回退**：任一步抛错或超时，整条走 raw_html 兜底（第 6 节），不得产出「半归一化」结果。
- **无状态**：适配器不得依赖上一条词条的解析残留（并行导入时尤其重要）。

### 5.2 推断型适配器（AI 辅助，决策 D4）

本地模型**只**用于从 20–50 条样本推断「映射规则 JSON」；规则入库、可人工读改、可手写覆盖；全量执行是确定性规则，不经过模型。

**规则文件最小形态**（示意，字段待实现定稿）：

```json
{
  "adapter": "ldoce6encn",
  "version": 1,
  "match": { "classes": { "entry": 1.0, "entryhead": 0.98, "sensenum": 0.6 } },
  "rules": [
    { "slot": "odc-entry", "select": "span.entry", "card": "1" },
    { "slot": "odc-pos-block", "select": "span.entry", "idFrom": "span.pos", "posFrom": "span.pos" },
    { "slot": "odc-sense", "select": "span.sense", "n": { "select": "span.sensenum", "type": "int" } },
    { "slot": "odc-def", "select": "span.def" },
    { "slot": "odc-example", "select": "span.example" }
  ],
  "drop": ["span.neutral", "script", "link"],
  "resources": { "audio": { "select": "a[href^=\"sound://\"]", "op": "strip-prefix" } }
}
```

**前置阶段：流式块重建（v0.1 新增，实测必需）**

LDOCE6++ 的 `span.entryhead` / `span.buttons` 在 MDX 里**永不闭合**，后续义项与短语动词会被解析成它们的子节点（`make2` 的义项就挂在 `span.buttons` 下）。因此「选择器 → 槽位」不能直接依赖源文档的父子关系：规则执行前必须有一个按**文档顺序**重建块骨架的流式阶段。

| 哨兵 | 作用 |
|---|---|
| `div.chwd` / `span.entry` 开始 | 开启新词条；同一条记录内再次出现则进入同形组下一段（7.6） |
| `div.at-link` | 进入/退出弹层（参考块） |
| `popheader` 文本 | 决定参考块类型（COLLOCATIONS / THESAURUS / WORD FAMILY…） |
| `span.buttons` / `span.entryhead` | 只作标记，不提供嵌套 |

该阶段输出**扁平的块序列**（entry → pos-block → sense → 短语动词），再由规则文件映射到槽位。

**硬约束**：

1. 规则只描述「**选择器 → 槽位**」与属性传递，**不得包含改写文本的指令**（无 replace / regex 文本替换）。
2. 规则条数上限 50（20–50 条样本量级下，超出视为过拟合）。
3. 规则必须可人工编辑；手写规则优先级高于推断规则。
4. 规则变更必须重跑 5.3 的全部校验，未过不进产线。
5. 规则只作用于流式块重建后的序列；**禁止**写依赖源文档父子嵌套的组合选择器（如 `span.entry > span.sense`）。

### 5.3 校验三件套与阈值

| 校验 | 定义 | v0.1 阈值（沿用 v0 提议值，待实测校准） | 失败处理 |
|---|---|---|---|
| 样本覆盖率 | 20–50 条样本上，规则命中的 P0 槽位数 / 应命中数（口径见 8.2） | **P0 = 100%** | 拒绝启用；回到手写规则或扩大样本 |
| 文本（空白归一） | 两份文档按 8.1 抽取、并按 8.1 归一空白后的纯文本逐字符比对 | **100%，零容忍** | 拒绝启用；定位到首个差异片段与字符 |
| 未覆盖条目比例 | 全量/抽样中 parse 失败或 P0 缺失的条目占比 | **≤ 2%** | 超阈值不发布；低于阈值则这些条目走 raw_html 兜底 |

- 三项均为**准入条件**，不是评分项。
- 阈值沿用 v0 的提议值，需 200 条渲染对比后校准（附录 B-6）；覆盖率分子/分母按 8.2 定义。
- 校验产物（报告 + 首批差异样本）随适配器一起入库，便于复核。

### 5.4 适配器与 L1 的边界

- 适配器**不改写文本**：只搬运节点、改标签、加 class/属性。文本内容任何差异都算失败。
- 适配器**不决定样式**：`styles()` 只在允许的定制范围内追加，不覆盖 ODC 基础样式（决策 D1）。
- 适配器**不填 `payload.defs`**：结构化字段的维护不属于本规范（附录 B-1）。

## 6. raw_html 兜底

决策 D2：未知词典或适配器失败时，**原样保留并标记**，不强行套结构。

```html
<section class="odc-entry odc-entry--raw"
         data-odc-dict="unknown"
         data-odc-key="someword"
         data-odc-unadapted="1"
         data-odc-reason="no-adapter">
  <!-- 净化后的原片段，原样嵌入，不再解析 -->
</section>
```

| 字段 | 取值 | 说明 |
|---|---|---|
| `data-odc-unadapted` | `1` | 渲染器据此显示「未适配」标记 |
| `data-odc-reason` | `no-adapter` / `probe-low-confidence` / `parse-failed` / `coverage-low` / `alias` | 排障与统计（`alias` 仅用于未消解别名，见 3.1） |
| `data-odc-dict` | 词典 slug 或 `unknown` | 未知词典也必须写 |

规则：

1. 兜底内容仍是**净化后**的片段（script/on*/iframe 已删），不允许把危险内容带回来。
2. 兜底词条不参与结构检索（不建 sense 索引），但仍参与全文检索（文本来自同一片段）。
3. 兜底是**正常状态**而非错误：UI 显示「未适配」，不弹错、不阻塞。
4. 后续加适配器时可离线重跑该词典，产出的新 HTML 与旧 HTML 走正常 diff（决策 D6 的收口方式）。

## 7. 结构规则

### 7.1 编号作用域

- 编号**按词性段重置**：LDOCE `make` 动词 1–27、名词 1–2；`time` 名词 1–68、动词 1–3；`way` 名词 1–59、副词 1–3。
- 每个 `.odc-sense` 必须带 `data-odc-scope`，值为其 pos-block（或短语动词）的 `id`；**没有 scope 的编号视为非法**。
- `data-odc-n` 存规范编号（整数）；`data-odc-label` 存显示文本（`1`、`a)`、`①`、`（Ⅰ）`）。两者可不同，**检索一律用 `data-odc-n`**。
- 跨词性不比较编号：文档中任何「第 7 义」的表述必须带 scope。

### 7.2 子义项编号空间

- 子义项用**字母**（`a)` / `a` / `A.`），与数字编号是两套独立空间：LDOCE `make` 的 `make it` 下是 `a)`，不是 `7.1`。
- `.odc-subsense` 的 `data-odc-n` 存字母本身（`a`），`data-odc-label` 存原样显示（`a)`）。
- OALD 没有字母级：同一义项下并列的 `<def>` 块**不强行造子义项**，保持同级（多义项并列）；只有源词典明确有层级时才建 `.odc-subsense`。依据：OALD9cn `sn-g` 下并列 `def`。
- 嵌套深度上限：`sense > subsense` 之后不再嵌套（实测只到一层，多出来的层级按同级展开并记告警）。

### 7.3 signpost（义项导语）

- 语义：义项前的分类词/导语（`create`、`do`、`mark`…），帮助用户在同词多义中定位。
- 位置：**义项的第一个子元素**；不得提升为 sense-group 的唯一依据（OALD 的 `shcut` 同时出现在 `sn-gs` 与义项内）。
- 允许与 `sense-group` 同时存在：group 是大类头，signpost 是义项导语。

### 7.4 TOC

- TOC 是**派生视图**，不是事实来源：可由 sense 列表重建（`.odc-toc` 可缺省）。
- LDOCE `entrymenu` 必须映射为 `nav.odc-toc`：**保留全部文本**（满足 8.1 文本保真），并置 `data-odc-counted="0"`；内部条目只是文本/链接，**不计义项、不参与编号**，也不计入 `odc-sense` 槽位统计。
- 为什么不是「排除」：v0 要求「排除 entrymenu 里重复的 sensenum」，与 8.1 的文本保真直接冲突（排除即丢文本）。v0.1 的口径是「**文本在、语义不在**」：`make` 的义项数仍是 29，不会因 entrymenu 虚增到 67。
- TOC 项只放链接 + 标签，`data-odc-target` 指向义项 id；不放释义文本。

### 7.5 交叉引用

- 原文 `entry://target#anchor` 必须重写：`data-odc-target` = 目标词条 key（+ 归一化锚点），`href` = `#<id>`（同文档）或 `<key>#<id>`（跨词条）。
- 目标不在当前包内 → `data-odc-missing="1"`，**保留文本与链接语义**，不得删节点。依据：LDOCE `PHRASES FROM OTHER ENTRIES` 的链接目标可能不在抽取范围。
- `help:` 协议（仅 oald9cn，371 次）视为不可解析，统一走 missing。
- 原目标串保存在 `data-odc-orig-target`。

### 7.6 同形词

- 同形异条**不合并**，两种形态都用「一个词条一个根」的同一口径：
  - **独立 key**（`Time` vs `time`；`ldoce6` 的 `back1` / `back2` / `back3`）：各自一个 `.odc-entry` 根，根上 `data-odc-homograph="<n>"` 标注组内序号；组内互链放 `.odc-crossrefs` 下的 `.odc-homograph`。
  - **同一 key 内多个词条**（`ldoce6encn` 的 `make`：`make1` verb + `make2` noun；`come`：verb + 44 个短语动词 + noun）：输出**多个同级** `section.odc-entry`，每个根各带自己的 `header`（`h1.odc-hw` + 音标 + POS）、`nav.odc-refbar` 与 `.odc-pos-block` 子树；根上 `data-odc-homograph` 从 1 起编号，`data-odc-key` 仍用源 key。
- **引用块按词条分组**：每个 entry 的 popup / at-link 引用块只进它自己的 `nav.odc-refbar`，不得串到相邻 entry。
- **词性切换条**（源 `div.chwd`）保留在该 entry 自己的 header 附近，链接指向同组其他 entry。
- **id 唯一**：多个 entry 的 HTML id 不得冲突（同词性同形词由适配器加序号区分）。
- **口径必须点名到文件**：`ldoce6`（按词性拆条）与 `ldoce6encn`（整条，**一条记录可含多个词性**）是两套口径；同一库内只允许一种，由适配器声明。§2/§3 中任何「一词性一条」的假设只对 `ldoce6` 成立，对 `ldoce6encn` 不成立。
- 混用两套 LDOCE（`ldoce6` 拆条 + `ldoce6encn` 整条）会产生重复词条：**同一库内只允许一种口径**，由适配器声明。

### 7.7 短语动词 / 习语 / 短语索引

- **短语动词是子条目**，与义项同级（`.odc-phrasal-verbs > .odc-phrasal-verb`），有自己的词头、音标、编号作用域；**不是 sense 的子节点**。依据：LDOCE `phrvbentry` 自带 `entryhead`/`pos`；MWALED `sblock_c > pva`；OALD `pv-gs`。
- **去重**：`make up` 既可能是 `make` 词条内的 PV，也可能是独立 MDX 条目（key `make up`）；抽取按 `(lemma, pv_text)` 去重，规则待定（附录 B-4）。
- **短语索引与习语分离**：
  - `.odc-phrase-index`：只有短语文本 + 链接，**没有释义**（LDOCE `PHRASES FROM OTHER ENTRIES`，`go` 632 条）；不得当义项渲染。
  - `.odc-idiom`：**带释义**的习语块（OALD `idm-gs`，`way` 112 个）。
- 量级差异必须容忍：OALD `way` 112 习语、`make` 仅 12；空块合法。

### 7.8 参考块

- 参考块挂**词条级**（`.odc-refs`），不挂义项；义项级搭配仍用 `.odc-collocation`。
- **可空**：没有就不建 `.odc-refs`。
- **同类型多来源**：用 `data-odc-source` 区分 `entry` / `other-entries` / `corpus`（LDOCE 三种 COLLOCATIONS 并存）。
- 块内允许再分条（`ul/li`、`table`），但**不得含 sense 语义**（不放编号、不放 `data-odc-n`）。
- 块标题（`popheader`）文本保留在块首（`h3.odc-ref-title`），不参与槽位统计。

### 7.9 超大规模条目

- 已知规模：`go` 1.48 MB / 2744 例句；LDOCE `take` 2356、`get` 2420。
- 规则：例句数 > 200 时按 `data-odc-lazy="1"` 分组懒加载；DOM 仍完整（服务端渲染全量），前端按视口展开。
- 禁止在 L1 阶段截断例句；截断只能发生在渲染层，并明确标注「已折叠 N 条」。
- 单条正文 > 1 MiB 与容器导入上限冲突（`ODC-E022`），列为未决（附录 B-2）。

## 8. 验收标准

### 8.1 文本保真（空白归一后比较）

- 方法：对同一条词条，分别从**源净化片段**与**L1 产物**抽取纯文本，两份文档用同一套算法。
- **抽取算法**：
  1. 按文档顺序深度优先遍历，只取文本节点；不取属性、注释、`<script>` / `<style>` 内容。
  2. `pre` 子树整体原样取出（不折叠空白），作为单个不可分割的片段。
  3. 丢弃**纯空白文本节点**（只含 U+0020 / U+0009 / U+000A / U+000D）。`&nbsp;`（U+00A0）**不算空白**，必须保留。
  4. 其余文本节点：把连续空白 run 折叠为单个 U+0020，再去掉片段首尾空白。
  5. 按文档顺序用单个 U+0020 连接相邻片段；空片段跳过。
- 判定：两份结果**逐字符相等**；不相等即失败，并给出首个差异位置与两侧各 40 字符上下文。
- 为什么按空白归一：块级格式化器会在块边界插入换行与缩进，源片段自身的缩进也不稳定，这些空白不承载语义。实测 LDOCE6++ 212 条：严格逐字节 **212/212 不相等**，按本算法归一后 **212/212 相等**。
- 覆盖：200 条渲染对比样本 + 12 个多义词全量。
- 已知边界：`&nbsp;` 按 U+00A0 保留，不参与折叠，也不转成 U+0020。

### 8.2 结构槽位覆盖

- LDOCE6（参考实现）：**P0 = 100%**（10 个槽位全部有落点），**P1 ≥ 80%**（15 个中至少 12 个）。
- 其他词典：P0 100% 为适配器发布门槛；P1 记录实测值，不做硬性要求。
- **统计单位与分母（v0.1 明确）**：
  - **有效词条**：抽样中源片段能成功归一化、且**不是** `odc-entry--redirect` 别名、**不是** `odc-entry--raw` 兜底的词条。分母 = 有效词条数。
  - **命中词条**：该词条内至少出现 1 个该槽位节点（多节点不重复计）。分子 = 命中词条数。
  - **槽位出现率** = 命中词条数 / 有效词条数，每个槽位各算一次（例：`odc-example` 122/212 = 57.5%）。
  - **按源证据覆盖率** = 命中词条数 / **源片段中确实具备该语义**的有效词条数（源证据按人工标注或源选择器判定）。§8.2 的 P0 = 100% 指这个口径；它通常高于槽位出现率，因为分母排除了源里本就没有该语义的词条。
- 别名与兜底**单独计数**：`redirect` 计数既不入分子也不入分母；`unadapted` 单列，不拉低覆盖率。
- 空槽位不扣分：词典天然缺项（见 4.1）。

### 8.3 拆行率 0

- 定义：L1 序列化结果中，**任何标签不得被换行拆开**（`<span\n class=…>` 计为违规）。
- 判定：对 200 条样本扫描标签字节区间，违规数 = 0。
- 与块级格式化器的既有保证一致（`feat/block-html-formatter`）。

### 8.4 幂等

- 判据保持 **100%**：`parse(parse(x)) == parse(x)`，对已是 L1 的输入再跑归一化，输出深比较相等（含属性顺序）。
- `id` 稳定：同一输入跨运行、跨机器（不同 CPU/并发）生成相同 id。
- 序列化稳定：属性顺序固定（按 3.4 总表顺序），避免无意义 diff。
- **已知缺陷（v0.1 记录，本文不改代码）**：块级格式化器 `format_blocks` 对**嵌套列表**不幂等——对已格式化的 L1 再跑一次，`</li>` 的缩进会从 6 空格漂移到 12 空格（样本 `-ware`，首个差异在 99,858 字节处）；简单样例 `<div><ul><li>a</li></ul></div>` 幂等，触发条件是「列表内含块级子元素」。
- 在该缺陷修复前：`parse` 对识别为 L1 的输入**原样返回**（no-op），幂等判据按「`parse` 两次结果深比较相等」执行；缩进漂移属于格式化器缺陷（单独立项修复，见 B-12），不降低本规范的 100% 要求，也不拿「重新格式化」当幂等测试。

### 8.5 200 条渲染对比

- 样本：9 本各 200 条抽样中取重叠词 + 12 个多义词。
- 结构断言：P0 槽位存在、编号 scope 正确、文本保真通过。
- 视觉：用户真机目检（不在 CI 中做像素对比）。

### 8.6 兜底可见

- 构造 3 类失败（无适配器 / probe 低置信 / parse 抛错），确认输出带 `data-odc-unadapted="1"` 与正确 `reason`，且文本仍可检索。

### 8.7 性能预算

- 单条 1.5 MB / 2744 例句的归一化耗时应可测量；目标值待实测后写入（附录 B-7），v0.1 只要求**不超时、不 OOM**。

## 9. 版本与迁移

- **文档版本**：v0.1（实验性）。与容器格式版本（`odc-spec.md` 的 `0.1`）**独立编号**；本规范不改变容器版本。
- **v0.1 不做迁移**：旧仓库的 `raw_html` 保持原样，不重写、不批量转换。
- **渲染器双模**：必须同时支持 `odc-*` 归一化词条与 `odc-entry--raw` 兜底词条；不能假设库里全是归一化数据。
- **重新归一化**：未来以显式命令（如 `odc html normalize`）重跑指定词典 → 产生新提交（`content_hash` 变化），走正常 diff/合并流程，**禁止静默重写历史**。
- **规范升级**：L2 槽位只增不改语义；改名必须保留旧类名一个 MINOR 周期（渲染器同时认）。
- **v0.x 例外**：v0 是未定稿 spike、无产线数据，**不承诺向后兼容**；v0.x 之间的改名不要求过渡期（v0.1 即把词条头/短语动词统一为与槽位同名的类）。**v1.0 定稿后**才执行上面的 MINOR 过渡规则。
- **定稿条件**：LDOCE6 参考实现 + 韦氏推断适配器 A/B + 200 条渲染对比通过 → 才考虑从 spike 分支合入。

## 附录 A：证据索引

### A.1 引用格式

`<词典目录>/<词>@<字节偏移>` 指向 2026-11-04 调研产物：

- 多义词样本：`/tmp/odc-polysemy-survey/samples/`（`ldoce6ec` / `oald9cn` / `mwaled` / `hanyu`；每词一个 HTML 文件，偏移为该文件内字节位置）
- 方言样本：`/tmp/odc-dialect-survey/samples/`（9 本各 200 条中的 3 条）
- 报告：`/tmp/odc-polysemy-survey/report.md`、`/tmp/odc-dialect-survey/report.md`

上述产物是 spike 证据，**不入库**；本文件引用它们是为了可复核。定稿前应把关键样片裁剪后随文档入库（附录 B-9）。

### A.2 槽位 × 词典落点（30 槽位）

| 槽位 | LDOCE6++/6 | OALD9 英汉 | MWALED | 汉语大词典 |
|---|---|---|---|---|
| entry | `span.entry` | `top-g` | `div.all-entry` | 控制码 `1` |
| entry-head | `entryhead`/`hwd`/`pron` | `hkey`/`h`/`pron-gs` | `hw_d`/`hw_infs_d` | `1` + `6` |
| pos-block | `span.entry` + `span.pos` | `div.cixing_part[id]` | `span.fl` | —（无词性段） |
| toc | `entrymenu` | — | — | — |
| sense-group | — | `sn-gs` + `shcut` | — | `（Ⅰ）（Ⅱ）` |
| sense | `span.sense` | `sn-g`/`li` | `div.sense` | 控制码 `2` |
| sense-number | `sensenum` | `xhtml:ol[start]` | `sn_block_num` | `2` 内数字 |
| subsense | `span.subsense` | 并列 `def`（无字母级） | `sn_letter` | — |
| signpost | `span.signpost` | `shcut` | — | — |
| def | `span.def` | `def` | `def_text` | 控制码 `3` |
| def-trans | `TRAN` | `chn` | — | — |
| example | `span.example`/`EXAEN` | `x-g-blk` | `li.vi` | 控制码 `5` 书证 |
| ex-trans | `EXAMPLE` | 相邻 `chn` | — | — |
| ex-source | 语料块 header | — | — | `5` 前书名 |
| phrasal-verb | `phrvbentry`/`phrvbhwd` | `pv-gs`/`pv` | `sblock_c`/`pva` | — |
| idiom | 归入 phrase 列表 | `idm-gs`/`idm` | — | — |
| phrase-index | `span.phrase`/`phrasetext` | — | — | — |
| grammar | `span.gram` | `gram` | `sgram` | — |
| collocation | `collo`/`colloexa` | `cf-blk`/`cf` | — | — |
| thesaurus | `thespropform` | `SYN` 盒 | — | — |
| usage-note | `usagebox`/`errorbox`/`grambox` | `unbox`+`boxtag` | `ssla` | ldoce6ec `grambox` 整块留在义项流内，kind=GRAMMAR |
| word-family | `WORD FAMILY` 块 | — | — | — |
| word-origin | `WORD ORIGIN` | `ORIGIN` 盒 | — | — |
| crossref | `crossref`/`refhwd` | `xr-gs` | `dx`/`dx_link` | TODO(evidence)：2.3 摘要列为 `entry://`，落点待核 |
| label | `registerlab` | `label-g` | `ssla` | — |
| citation | — | — | — | `5` + `<table>` 行 |
| pron-history | — | — | — | `6` + `〔《广韵》…〕` |
| homograph | 独立 key（`Time`/`time`）、`back1/back2` | — | `sup.homograph`（@142） | — |

> `odc-audio` / `odc-img` 不在 30 槽位内，但 L1 必须表达：LDOCE `a[href^="sound://"]`、OALD9cn `<audio name>`、MWALED `div.play_pron`、OALD8 `a.fayin`；图片均为 `img`（各本 0.5%–99.5% 覆盖）。

### A.3 已核实的规模与覆盖率

| 事实 | 数值 | 来源 |
|---|---|---|
| 词典数 | 9（汉典部分可读：止于第 166,148 条） | 方言 §2 |
| 不同 class | 464 | 方言 §5.3 |
| 跨 ≥3 个标记体系共用的 class | 3（`entry` / `pos` / `gram`） | 方言 §5.3 |
| `span.hwd` 覆盖率 | 100% / 21.5% / 36%（ldoce6latest / ldoce6encn / ldoce6） | 方言 §4 |
| `span.sensenum` 覆盖率 | 27% / 5.5%；OALD `sn-g` 66%；`sn-g > sn-blk` 29% | 方言 §4 |
| `onclick` 处数 | 3,709 / 484 / 449（LDOCE 三本） | 方言 §5.3 |
| 最大条目 | 1,481 KB（LDOCE `go`） | 多义词 §3.5 |
| 最大例句数 | 2,744（LDOCE `go`） | 多义词 §3.4 |
| 最大嵌套深度 | 24（OALD `make`/`get`/`go`） | 多义词 §3.5 |

### A.4 样本清单

| 目录 | 内容 |
|---|---|
| `samples/ldoce6ec/` | make / take / get / run / set / go / come / turn / work / time / way |
| `samples/oald9cn/` | 同上 |
| `samples/mwaled/` | 同上 |
| `samples/hanyu/` | `打`（`水` 因解压校验失败未导出） |

## 附录 B：未决问题

| # | 问题 | 现状 / 下一步 |
|---|---|---|
| B-1 | `defs` / `phonetic` 是否由归一化结果回填 | 本规范不涉及；需容器层决定 |
| B-2 | 单条 > 1 MiB 与 `ODC-E022` 冲突（`go` 1.48 MB） | 容器层问题；本规范只标注，不改上限 |
| B-3 | 汉语大词典 `水` 解压失败、控制码语义仅由 `打` 推断 | 换 reader 复验；`7` 是否总为块结束待证 |
| B-4 | `make up` 词条内 PV 与独立条目去重规则 | 待定；当前只记录 `(lemma, pv_text)` |
| B-5 | MWALED 编号含子条目（`get` 129），跨词典不可比 | 不比较；仅作本词典内顺序 |
| B-6 | 5.3 三项阈值需实测校准 | 200 条渲染对比后回填 |
| B-7 | 8.7 性能目标值 | 需在 1.5 MB 条目上实测 |
| B-8 | 词典级定制 CSS 的作用域语法 | 决策 D1 的后期能力；本规范只定义 `styles()` 边界 |
| B-9 | 证据样片是否随文档入库 | 建议裁剪关键 5–10 条；当前引用 `/tmp` 产物 |
| B-10 | 音频/图片去重策略（同一资源多词引用） | 由容器 `resources` 表承担；本规范只保证 key 形式 |
| B-11 | 流式块重建的哨兵是否覆盖所有 MDX 方言 | 目前只对 LDOCE6++ 实测；其他词典按各自方言补哨兵（5.2） |
| B-12 | `format_blocks` 嵌套列表缩进漂移（不幂等） | 单独立项修复；修复前 `parse` 对 L1 输入 no-op（8.4） |
| B-13 | 别名是否在导入层消解 | 推荐消解；未消解时走 `odc-entry--redirect` 槽位（3.1） |

## 附录 C：与讨论纪要的差异

| 项 | 纪要 | 本文件 | 理由 |
|---|---|---|---|
| 例句类名 | `odc-ex` | `odc-example`（`li`）+ `odc-examples`（`ul`） | 槽位名与类名一致，避免「example 是槽位还是元素」歧义 |
| 编号元素 | 未定 | `odc-sense-number` 可选，编号可只在 `data-odc-n` | LDOCE 有独立元素，OALD/MW 没有；强制元素会造空节点 |
| P2 数量 | 纪要 10.2 列 6 项（含 `example-lazy`） | 5 项，`lazy` 降为 `data-odc-lazy` 属性 | 与多义词报告 §4 的 28 槽位表对齐；lazy 是渲染属性不是结构槽位；v0.1 另加 redirect / unmapped，共 30 |

## 变更记录

### v0.2（2026-11-06）

| # | 问题 | 修改 | 位置 |
|---|---|---|---|
| 1 | 同记录多同形词条被塞进一个根 | `come` / `make` 改为输出多个**同级** `section.odc-entry`：各自 head、refbar、pos-block；根上 `data-odc-homograph` 从 1 编号 | 3.1 / 3.2 / 7.6 |
| 2 | 浮动的 `homograph-headword` | 取消 `span.odc-hw-alt[data-odc-kind="homograph-headword"]`；第 2 个及以后的词头同样用 `h1.odc-hw` | 3.1 / 3.2 / 7.6 |
| 3 | 引用块跨词条混串 | refbar 改为按 entry 分组，只收本词条的 popup / at-link | 7.6 |

### v0.1（2026-11-05）

据 LDOCE6++ 适配器原型（`/tmp/odc-ldoce6-adapter/findings.md`，200+12 条样本）的 10 条实测问题修订：

| # | 问题 | 修改 | 位置 |
|---|---|---|---|
| 1 | 80.1% 记录是 MDX 别名，规范无落点 | 新增 `odc-entry--redirect` + `data-odc-target`；导入层应先消解 `@@@LINK=` | 3.1 / 3.4 / 4.2 / 6 |
| 2 | 槽位名与骨架不一致 | 统一为 `odc-entry-head` / `odc-phrasal-verb`（含容器与词头类名） | 3.1 / 3.2 / 4.5 / 7.7 |
| 3 | entrymenu 与文本保真冲突 | entrymenu → `nav.odc-toc`，保文本、`data-odc-counted="0"`、不计义项不编号 | 3.4 / 4.4 / 7.4 |
| 4 | 同记录多同形词条无槽位 | 合并一根 + 多 `odc-pos-block`；补 `data-odc-homograph`；词头用 `hw-alt[data-odc-kind=homograph-headword]` | 3.1 / 3.2 / 3.4 / 7.6 |
| 5 | 未映射带文本块无兜底 | 新增 `div.odc-unmapped` + `data-odc-orig-tag/class` | 3.2 / 3.6 / 4.3 |
| 6 | 文本保真口径不可判定 | 改为「空白归一后比较」并给出抽取算法 | 3.3 / 5.3 / 8.1 |
| 7 | `format_blocks` 不幂等 | 维持幂等 100%；记录嵌套列表缩进漂移缺陷，修复前 `parse` 对 L1 no-op | 8.4 / B-12 |
| 8 | 「LDOCE6 按词性拆条」不成立 | 口径点名到文件：`ldoce6` 拆条、`ldoce6encn` 整条可含多词性 | 2 / 7.6 |
| 9 | 源文档不闭合 | 5.2 增加「流式块重建」前置阶段与哨兵规则 | 5.2 |
| 10 | 覆盖率口径不明 | 定义有效词条、分母/分子、出现率与按源证据覆盖率 | 8.2 |

词汇表：P0 9→10、P1 14→15、P2 5（合计 28→30）。
