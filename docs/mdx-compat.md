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
| 校验和 | 导入时启用（adler32 逐块校验），损坏即报 `ODC-E021` |
| MDD 资源文件 | 不导入（M1 非目标） |
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

## 实测（release 构建，Apple Silicon，本机样本）

| 样本 | 词条数 | 导入耗时 | 导入峰值内存 | commit 耗时 | commit 峰值内存 |
| --- | --- | --- | --- | --- | --- |
| LDOCE6 | 147 909 | 3.57 s | 80 MB | 4.13 s | 2.64 GB |
| OALD9 | 77 903 | 1.25 s | 24 MB | 1.24 s | 906 MB |
| 大辞林 3.0 | 497 905 | 2.15 s | 77 MB | 3.39 s | 1.04 GB |
| COCA | 53 967 | 0.24 s | 11 MB | 0.42 s | 153 MB |

点查（LDOCE6 库，随机 1000 词，`examples/lookup_bench.rs`）：
release p50 99 µs / p95 136 µs；debug p50 597 µs / p95 941 µs。

## 已知缺口

* `odc commit` 仍沿用 M0 的一次性装载实现，峰值内存随 payload 增长
  （LDOCE6 约 2.6 GB），流式 commit 留到 M2。
* 需要显式 passcode 才能解密的词典暂不能导入（CLI 尚无 `--passcode`）；
  本机 4 个样本无需 passcode。
* LZO 压缩路径已编译启用，但没有真实 LZO 词典可验证。
* MDD（图片、发音、样式表）不导入，`raw_html` 中的相对资源引用需阅读器自行处理。
