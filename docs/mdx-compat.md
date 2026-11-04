# MDX 兼容性说明（M1）

本文记录 ODC 对 MDict `.mdx` 文件的支持范围、解析器选型依据与已知缺口。
实现位于 `crates/odc-core/src/mdx.rs`（适配层），底层解析器为
[`mdictlib`](https://crates.io/crates/mdictlib) 0.2.8（MIT）。

## 支持的版本与编码

| 项目 | 支持情况 |
| --- | --- |
| MDX 1.2（`GeneratedByEngineVersion` 主版本 1） | 支持，UTF-16LE |
| MDX 2.0（主版本 2） | 支持，含加密 key block 头（`Encrypted="2"`） |
| 其他主版本 | 拒绝，报 `ODC-E021` |
| 文本编码 | UTF-8、UTF-16LE、GBK/GB2312、GB18030、Big5（由头部 `Encoding` 决定） |
| 块压缩 | 无压缩、zlib、LZO（启用 mdictlib 的 `lzo` 特性） |
| 校验和 | 导入时启用（adler32 逐块校验）；MDX 2.0 的坏块可修复跳过（见「损坏块容错」），其余情况报 `ODC-E021` |
| MDD 资源文件 | 导入（主卷 + 数字卷，逐条规范化键写入 `resources` 表） |
| MDX 写出 | 不支持（M1 非目标） |

## 第 0 步：候选 crate 评测

评测在 `/tmp/mdx-spike` 中进行，样本为本机 4 个真实词典（只读）：

| 样本 | 词条数 | 版本 | Encoding | Encrypted |
| --- | --- | --- | --- | --- |
| LDOCE6.mdx（115 MB） | 147 909 | 2.0 | UTF-8 | 2 |
| OALD9EnEn.mdx（29 MB） | 77 903 | 2.0 | UTF-8 | 2 |
| 三省堂スーパー大辞林3.0.mdx（41 MB） | 497 905 | 2.0 | UTF-8 | 2 |
| COCA Frequency 60000.mdx（2 MB） | 53 967 | 2.0 | UTF-8 | 2 |

| crate | 版本 / 许可证 | 结果 |
| --- | --- | --- |
| `mdictlib` | 0.2.8 / MIT | 4/4 全部枚举成功；LDOCE6 全量枚举 0.90 s、峰值 5.9 MB；无 panic。**选用** |
| `mdict-rs` | 0.1.4 / AGPL-3.0-only | 4/4 可读，但枚举比 mdictlib 慢 2–3 倍（LDOCE6 2.76 s）；AGPL 与本项目 GPL-3.0-or-later 及 `deny.toml` 许可白名单冲突，弃用 |
| `rs-mdict` | 0.1.1 / MIT | 4/4 可打开并枚举键；无记录迭代器（只能逐键 `lookup`），重复键命中与 mdictlib 不一致（`apple` 返回另一个重复项），UTF-16 记录尾部残留 NUL；大辞林仅读键即占 80 MB，弃用 |
| `readmdict` | 0.1.0 / MIT | 4/4 全部失败（`corrupt deflate stream`），弃用 |

结论：封装 mdictlib 为 `odc_core::mdx` 适配层，上游错误统一映射为
`Error::Mdx`（`ODC-E021`），并在适配层加上记录大小上限与键冲突策略。

## 导入映射

* **payload**：`{"display": <原词形>, "raw_html": <记录 HTML>}`；
  `defs` / `phonetic` 留空，`extra` 为空。导入不做 HTML 结构化解析。
* **键归一化**：与 ODC 其他入口一致（NFKC + casefold，见 spec §4.1）。
* **重复键**：同一次导入内归一化键重复时，第一次保留原键，后续按文件顺序
  追加 `U+001F<n>`（n = 2、3…），`display` 始终保留原词形。
* **跳过规则**：空键（trim 后为空）与超过 1 MiB 的记录被跳过并计数；
  `odc import --format mdx` 会输出跳过/去重统计。

## HTML 规范化与警告

0.9.0 起，MDX 导入默认把每条记录的正文当作 HTML5 **片段**处理（`--html format`）：

1. `html5ever` 以 `<div>` 为上下文解析片段，按 HTML5 树构建规则修复未闭合、
   错配的标签，并收集解析错误；
2. 删除 `script`、`iframe`、`object`、`embed`、`on*` 事件属性以及
   `javascript:` URL；
3. `<style>` 块与 `style` 属性交给 `lightningcss` 解析并重写
   （`style` 属性始终输出为单行）；
4. 序列化后用块级结构化格式化器（`block_fmt`）排版：只在块级元素
   （`div` / `p` / `li` / `table` 等）边界换行并缩进；标签绝不拆行，文本
   节点逐字节保留；`pre` / `script` / `style` / `textarea` 与注释原样输出。

`--html check` 只做诊断，输出与输入逐字节相同；`--html off` 完全不解析，
恢复到 0.8 的行为。两条路径（打包导入与目录仓库导入）共用同一个管道。

### 诊断码

| code | 含义 |
| --- | --- |
| `html.parse_error` | HTML5 解析 / 树构建错误（未闭合标签等） |
| `html.removed_element` | 删除了危险元素 |
| `html.removed_attribute` | 删除了事件属性 |
| `html.unsafe_url` | 删除了 `javascript:` URL |
| `css.parse_error` | `lightningcss` 无法解析的 CSS |
| `format.failed` | 块级格式化器无法排版该条正文（畸形 HTML） |

### 失败策略

* 格式化失败：**原 HTML 原样入库**，计 `entries_failed`，告警
  `format.failed`，导入不中断。
* CSS 解析失败：保留该块 / 属性的原文，只告警 `css.parse_error`。
* 原始 HTML 不另存副本；源 `.mdx` 即底稿。

### 为什么不用宽度排版

`markup_fmt` 是 Prettier 的移植，会按行宽重排文本。词典正文内联标签密集，
80 列时 **47.85%** 的行被拆开；放宽到 160 列反而升到 **61.52%**（更多内联
内容被纳入同一行）。0.14.0 起改用块级排版：只在块级边界换行，200 条真实
正文的拆行率 **0.00%**、文本等价 200/200、幂等 200/200，约 0.17 ms/条
（本机 200 条实测）。
`inline_neighbours_keep_text` 回归测试继续守住「相邻内联元素之间不插空白」。

### 报告与独立命令

`MdxImportReport::html` 汇总 `entries_checked` / `entries_changed` /
`entries_failed`、按 code 的计数、每 code 最多 20 条样例（headword + message）
以及 `css_entries` / `css_changed`。CLI 导入结束打印一行汇总、每类计数和少量
样例。

* `odc check-html [PATH]`：`PATH` 为 `.mdx` 时检查该文件，省略时用
  `for_each_entry` 检查仓库全部词条；只读，不取写锁、不产生 commit。
  `--json` 输出稳定结构，`--limit N` 限制条数。
* `odc normalize`：默认 dry-run，报告将改动多少条及诊断；`--apply` 要求工作
  集干净，只改 `raw_html`，通过 `Repo::import`（`prune=false`）写工作集再
  `commit`，提交信息 `normalize HTML for N entries (odc 0.9.0)`；无改动不提交。

## 实测（release 构建，Apple Silicon，本机样本）

| 样本 | 词条数 | 导入耗时 | 导入峰值内存 | commit 耗时 | commit 峰值内存 |
| --- | --- | --- | --- | --- | --- |
| LDOCE6 | 147 909 | 3.57 s | 80 MB | 4.13 s | 2.64 GB |
| OALD9 | 77 903 | 1.25 s | 24 MB | 1.24 s | 906 MB |
| 大辞林 3.0 | 497 905 | 2.15 s | 77 MB | 3.39 s | 1.04 GB |
| COCA | 53 967 | 0.24 s | 11 MB | 0.42 s | 153 MB |

点查（LDOCE6 库，随机 1000 词，`examples/lookup_bench.rs`）：
release p50 99 µs / p95 136 µs；debug p50 597 µs / p95 941 µs。

## 损坏块容错

真实词典偶有「整条记录段里只有一个块坏了」的情况。早期实现里 `mdictlib`
读到那个块就整体失败（`ODC-E021`，见下方实测），339,937 条的《汉语大词典》
就因为 1 个块丢掉了整部词典。现在 MDX 2.0 的导入改为
「先扫描、再修复、后过滤」：

1. **扫描**（`odc-core::mdx_scan`）：按 v2 记录段布局定位记录段——头 32 字节为
   4 个 `u64` BE，随后是 16 字节/项的索引，块数据紧随其后。定位公式为
   key 段头（`4 + header_len + 4`，其中 4 字节是 v2 头部校验和）`+ 44`
   （key 段头）`+ index_comp_size + key_blocks_len`；随后用
   **`record_offset + 32 + index_len + blocks_len == 文件大小`** 做自校验；
   对不上（或任何结构对不上）就**不做容错、不猜偏移**，改为按原文件直接导入，
   并打一行 warning（见下）。每个 `comp_type = 2` 的块用 zlib 解压，核对
   解压长度与 adler32；其它 `comp_type` 只做结构校验（不猜压缩算法）。
2. **修复副本**：只要存在坏块，就在系统临时目录写一份副本：坏块之前的字节
   原样复制，坏块替换为「解压后长度**恰好等于**声明值」的哨兵内容
   （`ODC-SKIPPED-BLOCK-<n>` 重复填充后重新 zlib 压缩），并重写索引与
   32 字节记录段头（`blocks_len` 更新）；**解压偏移不变**，key 段完全不动。
   没有坏块时直接用原文件，不产生副本。

定位或自校验失败（例如记录段后面还有多余字节）**不算导入失败**：导入会退回
旧路径，直接读原文件，stderr 打一行形如
`mdx: warning: record section self-check failed: …; reading the file as-is
(corrupt-block recovery unavailable)` 的提示；只有“确实检测到坏块、但生成
修复副本失败”才会报错。
3. **过滤**：导入时丢弃 HTML 含哨兵标记的条目，计入 `skipped_entries`；
   CLI 在原有输出之后追加一行，例如
   `mdx: skipped 1 corrupt record block (123 entries)`。坏块明细（块序号、
   文件偏移、压缩/声明长度、解压区间、失败原因）打到 stderr。

公共 API（`odc_core::import_mdx`）：

```rust,ignore
pub struct MdxImportOptions {
    pub skip_corrupt_blocks: bool, // 默认 true
    pub prune: bool,
    pub limit: Option<usize>,
    pub progress: Option<Box<dyn FnMut(ImportProgress)>>,
}
pub fn import_mdx(path: &Path, repo: &mut Repo, options: MdxImportOptions)
    -> Result<MdxImportReport>; // imported_entries / written / removed /
                                // skipped_blocks / skipped_entries / corrupt_blocks
                                // （另有 dedup / empty / oversized 计数）
```

进度样本是 `ImportProgress { stage, done, total, volume_index, volume_count }`，
`done` / `total` 都是**阶段内**的条数：`Preparing` 报 MDD 预扫描到的资源总数，
`Entries` 报 `min(limit, 文件条目数)`，`Resources` 报资源条数并带卷号
（`volume_index` 从 1 起，`volume_count` 为可读卷数），`Committing` 时两者相等、
等于词条数加资源数。回调限速约 10 次/秒，阶段切换与终态样本必发；需要一条总
进度条时按 `(词条 done + 资源 done) / (词条 total + 资源 total)` 自行合成。

### 实测（release 构建，Apple Silicon，本机样本）

样本：`汉语大词典(简体精排).mdx`，72,104,480 B，MDX 2.0，339,937 条，
`Encrypted="2"`，3,186 个记录块。

* 记录段偏移 2,364,101；记录段头
  `num_blocks=3186, num_entries=339937, index_len=50976, blocks_len=69689371`；
  自校验 `2,364,101 + 32 + 50,976 + 69,689,371 = 72,104,480` 与文件大小一致。
* 3,185/3,186 个块解压长度与 adler32 全部匹配；只有 **块 #1601（0 基）**
  损坏：文件偏移 37,510,487，压缩 21,942 B，声明解压 65,419 B，
  zlib 报 `incorrect data check`，实际只产出 65,365 B。
* 修复前：`ODC-E021: MDX error: invalid data: invalid or oversized zlib block
  in record block: Adler32Mismatch`，导入在第 166,149 条（1 基）中止。
* 修复后：`imported 339814 entries (339814 written, 0 removed); working set = 339814`
  加 `mdx: skipped 1 corrupt record block (123 entries)`；
  `339,814 + 123 = 339,937`，与文件声明的条目数一致。按 `--limit` 逐点探测，
  第一个被跳过的条目正是第 166,149 条（1 基），与修复前的失败位置完全吻合。
* 耗时与内存（`/usr/bin/time -l`，import 全程）：`2.12 real 1.70 user 0.34 sys`，
  `44,662,784 maximum resident set size`（约 42.6 MiB），
  `41,697,736 peak memory footprint`。随后 `odc commit` 报告
  `339814 entries, 946 new blocks`，`odc verify` 报
  `verify: OK (946 blocks, 339814 entries, 2 commits)`。
* 抽查三条正常条目（`一`、`一丁不识`、`中国`）的 `raw_html`，已用独立
  Python 解析原始 MDX 记录段做字节比对，全部逐字节命中。

### 已知限制

* 只对 **MDX 2.0** 生效。1.2 文件保持原行为（坏块仍报 `ODC-E021`），
  因为 1.2 没有可重写的记录段索引。
* 记录段定位/自校验失败时容错不可用：导入按原文件进行（与旧行为一致），
  这类文件里的坏块仍然会让导入以 `ODC-E021` 结束，只是不会被提前拦下。
* 坏块内的**全部**条目都会被跳过（本次 123 条）；与坏块相邻的边界条目可能
  一并丢失，不会尝试从损坏的解压流里抢救部分记录。
* 有坏块时会多写一份临时副本（约等于原文件大小，本样本约 72 MB），
  导入结束（含出错路径）即删除。
* 扫描要完整解压一遍记录段（本样本约 206 MB），相当于导入前多一次解压
  开销；无坏块的大词典也要付这一遍扫描。
* CLI 暂未提供关闭容错的开关（需要严格失败时用 core API 的
  `skip_corrupt_blocks: false`）。
* 与 `--limit` 同用时，`skipped_entries` 只在导入真正读到坏块时累计；
  若条目上限先达到、坏块尚未读到，则 `skipped_blocks` 为 1 而
  `skipped_entries` 为 0（`skipped_blocks` 来自扫描，始终准确）。

## 已知缺口

* `odc commit` 仍沿用 M0 的一次性装载实现，峰值内存随 payload 增长
  （LDOCE6 约 2.6 GB），流式 commit 留到 M2。
* 需要显式 passcode 才能解密的词典暂不能导入（CLI 尚无 `--passcode`）；
  本机 4 个样本无需 passcode。
* LZO 压缩路径已编译启用，但没有真实 LZO 词典可验证。
* MDD（图片、发音、样式表）随 MDX 一起导入：`<同名>.mdd`、`<同名>.1.mdd` … 按主卷优先、
  数字升序读取，单卷损坏只记 warning，不中断 MDX 导入。阅读器把 `raw_html` 里的相对
  `src`/`href`/`url(...)` 改写为 `odc-resource:///<规范化键>`。
