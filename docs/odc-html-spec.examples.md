# ODC 词条 HTML 规范 v0.1 — 样例集

> 配套 [`odc-html-spec.md`](odc-html-spec.md)。所有「源」代码块均为 `2026-11-04` 调研样片的**节选**（省略号表示删节，未改动保留部分）；「L1」为按规范手写的目标形态，**不是**任何实现的真实输出。
> 证据路径见规范附录 A.1。

## 例 1：LDOCE6++ 英汉 `make`（class 方言）

### 源（`ldoce6ec/make@590`、`@12373`、`@12551` 节选）

```html
<span class="entry">
  <span class="entryhead">
    <span class="hwd">make</span>
    <span class="hyphenation frequent">make</span>
    <span class="homnum frequent">1</span>
    <span class="proncodes"><span class="neutral">/</span><span class="pron">meɪk</span><span class="neutral">/</span></span>
    <span class="level tooltip">●●●</span> <span class="freq">S1</span> <span class="freq">W1</span>
    <span class="pos">verb</span>
    <span class="inflections">…made, making…</span>
  </span>
  <!-- …中间省略 content 包装与锚点… -->
  <span class="sense">
    <span class="sensenum">1</span>
    <span class="def"><EN>used about things you make yourself, or things that are made in a factory</EN><TRAN>制作﹔制造</TRAN></span>
    <deft></deft><l6></l6>
    <span class="example"><EXAEN><span class="neutral">·</span> Diane makes all her own clothes.</EXAEN><EXAMPLE>黛安娜的衣服都是自己做的。</EXAMPLE></span>
  </span>
</span>
```

要点：`sensenum` 只是文本节点；译文混在 `def` 里（`<TRAN>`）；例句译文是 `<EXAMPLE>`；`<deft>` / `<l6>` 是不闭合的空标签。

### L1

```html
<section class="odc-entry" data-odc-dict="ldoce6encn" data-odc-key="make" data-odc-lang="en">
  <header class="odc-entry-head">
    <h1 class="odc-hw">make</h1>
    <span class="odc-hw-alt" data-odc-kind="hyphenation">make</span>
    <span class="odc-phon" data-odc-variant="uk">/meɪk/</span>
    <span class="odc-meta" data-odc-orig-class="freq">S1 W1</span>
  </header>
  <section class="odc-pos-block" data-odc-pos="verb" id="make-verb">
    <h2 class="odc-pos">verb</h2>
    <div class="odc-sense" data-odc-n="1" data-odc-label="1" data-odc-scope="make-verb" id="make-verb-s1">
      <span class="odc-def">used about things you make yourself, or things that are made in a factory
        <span class="odc-def-trans" data-odc-lang="zh">制作；制造</span>
      </span>
      <ul class="odc-examples">
        <li class="odc-example">
          <span class="odc-ex-text">Diane makes all her own clothes.</span>
          <span class="odc-ex-trans" data-odc-lang="zh">黛安娜的衣服都是自己做的。</span>
        </li>
      </ul>
    </div>
  </section>
</section>
```

注：`S1 W1` 频率标记不属于 28 槽位，样例用 `.odc-meta` 兜住并保留原文 class；若要进词汇表需另行立项。

## 例 2：OALD9 英汉 `make`（自定义标签方言）

### 源（`oald9cn/make@405`、`@6888`、`@7232` 节选）

```html
<div id="verb" class="cixing_part">
  <sn-gs>
    <sn-g>
      <sn-blk><xhtml:ol start="1"><xhtml:li>1</xhtml:li></xhtml:ol></sn-blk>
      <def>to create or produce something</def>
      <chn>制造；制作</chn>
      <x-g-blk>
        <x>She made a cake.</x>
        <chn>她做了一个蛋糕。</chn>
      </x-g-blk>
    </sn-g>
  </sn-gs>
</div>
```

要点：词性在 **`id` 属性**（`div#verb.cixing_part`），不是 class 文本；编号靠 `ol[start]` + 单个 `li`；例句译文是**相邻的** `chn`，不是嵌套。

### L1

```html
<section class="odc-entry" data-odc-dict="oald9cn" data-odc-key="make" data-odc-lang="en">
  <header class="odc-entry-head"><h1 class="odc-hw">make</h1></header>
  <section class="odc-pos-block" data-odc-pos="verb" id="make-verb">
    <h2 class="odc-pos">verb</h2>
    <div class="odc-sense" data-odc-n="1" data-odc-label="1" data-odc-scope="make-verb" id="make-verb-s1">
      <span class="odc-def">to create or produce something
        <span class="odc-def-trans" data-odc-lang="zh">制造；制作</span>
      </span>
      <ul class="odc-examples">
        <li class="odc-example">
          <span class="odc-ex-text">She made a cake.</span>
          <span class="odc-ex-trans" data-odc-lang="zh">她做了一个蛋糕。</span>
        </li>
      </ul>
    </div>
  </section>
</section>
```

注：源里 `chn` 与 `x` 是兄弟节点，L1 必须把「后一个相邻 `chn`」判定为例句译文——这是适配器规则，不是通用启发式。

## 例 3：汉语大词典 `打`（无标记 token 方言）

### 源（`hanyu/打@56`、`@146`、`@239` 节选）

```html
`1`打<font size=+1>（I）</font>`6`<br>
ㄉㄚˇ
〔《广韵》德冷切，上梗，端。〕
`2`&nbsp;1.&nbsp;`3`&nbsp;敲击；撞击。`7``7`
<table>
<tr VALIGN="Top"><td>&nbsp;●</td><td>汉王延寿《梦赋》：`5`“捎魍魉，拂诸渠，撞纵目，打三颅。”`7`</td></tr>
</table>
```

要点：语义靠反引号数字控制码（`1` 词头组、`6` 注音、`2` 义项号、`3` 释义、`5` 书证、`7` 块结束）；书证是表格行。控制码语义目前**只由 `打` 一个字推断**（附录 B-3）。

### L1

```html
<section class="odc-entry" data-odc-dict="hanyu" data-odc-key="打" data-odc-lang="zh">
  <header class="odc-entry-head">
    <h1 class="odc-hw">打</h1>
    <span class="odc-sense-group" data-odc-label="（I）"></span>
    <span class="odc-phon" data-odc-variant="zhuyin">ㄉㄚˇ</span>
    <span class="odc-pron-history" data-odc-kind="fanqie">〔《广韵》德冷切，上梗，端。〕</span>
  </header>
  <section class="odc-pos-block" data-odc-pos="verb" id="u6253-verb">
    <h2 class="odc-pos">verb</h2>
    <div class="odc-sense" data-odc-n="1" data-odc-label="1" data-odc-scope="u6253-verb" id="u6253-verb-s1">
      <span class="odc-def">敲击；撞击。</span>
      <ol class="odc-citations">
        <li class="odc-citation"><span class="odc-citation-text">捎魍魉，拂诸渠，撞纵目，打三颅。</span><span class="odc-citation-source">汉 王延寿《梦赋》</span></li>
      </ol>
    </div>
  </section>
</section>
```

注：`（I）` 在源里是 font 包裹的大类标记，L1 提升为 sense-group，`font` 丢弃（3.6）；`&nbsp;` 必须保留为 U+00A0。

## 例 4：MWALED 短语动词（子条目）

### 源（`mwaled/make@142`、`@42812` 节选）

```html
<div class="all-entry">
  <div class="entry entry_v2 boxy">
    <div class="hw_d hw_0 boxy m_hidden">
      <span class="hw_txt gfont"><sup class="homograph">1</sup> make</span>
      <span class="hpron_word ifont">/<span class="smark">ˈ</span>meɪk/</span>
    </div>
    <div class="sense"><strong class="sn_block_num">1</strong> …</div>
    <span class="sblock_c"><span class="pva">make away with (something)</span></span>
  </div>
</div>
```

### L1

```html
<section class="odc-entry" data-odc-dict="mwaled" data-odc-key="make" data-odc-lang="en">
  <header class="odc-entry-head">
    <h1 class="odc-hw">make</h1>
    <span class="odc-hw-alt" data-odc-kind="homograph" data-odc-homograph="1">1</span>
    <span class="odc-phon" data-odc-variant="us">/meɪk/</span>
  </header>
  <section class="odc-pos-block" data-odc-pos="verb" id="make-verb">
    <h2 class="odc-pos">verb</h2>
    <div class="odc-sense" data-odc-n="1" data-odc-scope="make-verb" id="make-verb-s1">…</div>
    <section class="odc-phrasal-verbs">
      <article class="odc-phrasal-verb" data-odc-lemma="make away with" id="make-verb-pv1">
        <h3 class="odc-phrasal-verb-hw">make away with (something)</h3>
        <div class="odc-sense" data-odc-n="1" data-odc-scope="make-verb-pv1" id="make-verb-pv1-s1">…</div>
      </article>
    </section>
  </section>
</section>
```

## 例 5：raw_html 兜底

```html
<section class="odc-entry odc-entry--raw"
         data-odc-dict="unknown" data-odc-key="serendipity"
         data-odc-unadapted="1" data-odc-reason="no-adapter">
  <div class="entry"><h2>serendipity</h2><p>…原片段，净化后原样嵌入…</p></div>
</section>
```

## 例 6：词典级样式作用域（`styles()` 产物）

```css
/* 只作用于本词典，不覆盖 ODC 基础样式 */
.odc-entry[data-odc-dict="hanyu"] .odc-citation-source { display: block; }
.odc-entry[data-odc-dict="hanyu"] .odc-def { letter-spacing: 0.02em; }
```

## 例 7：适配器校验报告（5.3 产物形态）

```json
{
  "adapter": "ldoce6encn",
  "version": 1,
  "samples": 40,
  "coverage": { "p0": 1.0, "p1": 0.93 },
  "textFidelity": { "checked": 40, "normalizedEqual": 40, "mode": "whitespace-normalized", "firstDiff": null },
  "uncovered": { "entries": 331, "ratio": 0.0012, "strategy": "raw_html" },
  "verdict": "pass"
}
```

> 数值为形态示意，非实测结果。

## 例 8：MDX 别名（未在导入层消解）

### 源（`ldoce6encn/GMO`，正文只有链接）

```html
@@@LINK=GMO
```

### L1

```html
<section class="odc-entry odc-entry--redirect"
         data-odc-dict="ldoce6encn" data-odc-key="GMO" data-odc-lang="en"
         data-odc-redirect="1" data-odc-target="GMO"></section>
```

要点：推荐在导入层直接消解 `@@@LINK=`（LDOCE6++ 有 212,797 / 265,545 条，80.1%），只留 `(key → target)` 映射、不落 HTML；未消解时用这个空容器兜底——无 sense、无编号、无正文，不计入覆盖率分母。

## 例 9：未映射但带文本的块（逃生口）

### 源（`ldoce6ec/make` 词族树节选）

```html
<div class="content">
  <span class="wf">maker</span> <span class="wf">making</span>
</div>
```

### L1

```html
<section class="odc-ref" data-odc-kind="word-family" data-odc-source="entry">
  <h3 class="odc-ref-title">WORD FAMILY</h3>
  <div class="odc-unmapped" data-odc-orig-tag="div" data-odc-orig-class="content">
    <span class="wf">maker</span> <span class="wf">making</span>
  </div>
</section>
```

要点：`.odc-unmapped` 是唯一允许保留原文标签与 class 的容器（内部不再做 odc 映射）；它不产生 sense/编号/槽位语义，只为不丢内容。纯布局容器（无文本）直接解包。

## 例 10：entrymenu → TOC（保文本、不计义项）

### 源（`ldoce6ec/make@1679` 节选）

```html
<span class="entrymenu">
  <span class="sensenum">1</span> create
  <span class="sensenum">2</span> do
</span>
```

### L1

```html
<nav class="odc-toc" data-odc-counted="0">
  <a href="#make-verb-s1" data-odc-target="make-verb-s1">1 create</a>
  <a href="#make-verb-s2" data-odc-target="make-verb-s2">2 do</a>
</nav>
```

要点：文本全部保留，8.1 文本保真可过；`data-odc-counted="0"` 表示不参与义项统计与编号，`make` 的义项数仍是 29，不会因目录里的编号虚增到 67。

## 例 11：同一记录里的同形词条

### 源（`ldoce6encn/make` 节选：`make1` verb + `make2` noun）

```html
<span class="entry"><span class="entryhead"><span class="hwd">make</span>…</span>
  <span class="pos">verb</span>…
</span>
<span class="entry"><span class="entryhead"><span class="hwd">make</span><span class="homnum">2</span>…</span>
  <span class="pos">noun</span>…
</span>
```

### L1

```html
<section class="odc-entry" data-odc-dict="ldoce6encn" data-odc-key="make" data-odc-lang="en">
  <header class="odc-entry-head">
    <h1 class="odc-hw">make</h1>
    <span class="odc-phon" data-odc-variant="uk">/meɪk/</span>
  </header>

  <section class="odc-pos-block" data-odc-pos="verb" data-odc-homograph="1" id="make-verb">…</section>

  <section class="odc-pos-block" data-odc-pos="noun" data-odc-homograph="2" id="make-noun">
    <span class="odc-hw-alt" data-odc-kind="homograph-headword">make<sup>2</sup></span>
    <div class="odc-sense" data-odc-n="1" data-odc-scope="make-noun" id="make-noun-s1">…</div>
  </section>
</section>
```

要点：一条源记录含多个 `span.entry` 时合并为一个根；每个词性一个 pos-block，`data-odc-homograph` 从 1 起编号；第 2 个词头用 `hw-alt[data-odc-kind="homograph-headword"]` 放在该 pos-block 开头。
