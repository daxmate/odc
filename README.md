# ODC — Open Dictionary Container

[![CI](https://github.com/daxmate/odc/actions/workflows/ci.yml/badge.svg)](https://github.com/daxmate/odc/actions/workflows/ci.yml)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

**一个为词典而生的、内容寻址的容器格式与工具链。**

ODC 想把词典制作从「一堆需要专家维护的 MDX 脚本」变成「任何人都能参与校对和发布的开放格式」：

* 词条是结构化的（释义、词性、例句、标签），不是一整段 HTML；
* 存储与传输按**内容切块**（content-defined chunking），改一个词只传一个块；
* 每次提交是一棵 **BLAKE3 Merkle 树**，可校验、可增量、可审计；
* 一条命令就能把 Excel / CSV / JSONL 变成可发布的词典库。

> M0 范围：格式规范 + Rust 内核 + 命令行。macOS SwiftUI 编辑器在后续里程碑。

## 快速开始

```bash
cargo build --release
./target/release/odc --repo ./demo init
./target/release/odc --repo ./demo import words.jsonl --format jsonl
./target/release/odc --repo ./demo commit -m "初次导入"
./target/release/odc --repo ./demo log
./target/release/odc --repo ./demo verify
```

`words.jsonl` 每行一个词条：

```json
{"word":"apple","display":"apple","phonetic":["ˈæp.əl"],
 "defs":[{"pos":"noun","sense":"a round fruit","examples":["an apple a day"],"labels":[]}],
 "raw_html":null,"extra":{}}
```

CSV 也可（表头 `word,display,definition[,pos]`）：

```bash
odc --repo ./demo import words.csv --format csv --prune
```

## 非技术用户的工作流（目标形态）

1. 在 Excel / Numbers / 在线表格里维护 `word / 释义 / 词性 / 例句`；
2. 导出 CSV，拖进「词典编辑器」（M1 的 macOS App）；
3. App 实时显示：新增 / 修改 / 删除各多少条、哪些字段变了；
4. 一键提交，自动生成变更说明；
5. 发布：导出单个 `.odc` 库或生成增量包，阅读器按需下载。

命令行版本已经完整支持 1、4、5 的前半段（导出与增量包在 M3）。

## 命令

| 命令 | 作用 |
| --- | --- |
| `odc init` | 新建仓库（含 genesis 提交） |
| `odc import FILE --format jsonl\|csv [--prune]` | 导入 / 更新工作区 |
| `odc commit -m MSG` | 提交工作区 |
| `odc log [-n N] [--json]` | 历史 |
| `odc show WORD [--json]` | 查询词条 |
| `odc diff A B [--json]` | 词条级差异（added / removed / modified） |
| `odc status` | 工作区状态 |
| `odc verify` | 全量校验块、索引与提交 |
| `odc patch create\|apply` | 增量包（M3，M0 返回 ODC-E017） |

全局参数 `--repo DIR` 指定仓库目录（默认当前目录，库文件 `odc.db`）。

## 仓库结构

```
docs/odc-spec.md          规范性文档（格式的唯一权威）
spec/testvectors/         机器可读测试向量
crates/odc-core/          切块、编码、哈希、SQLite、commit/diff/verify
crates/odc-cli/           odc 命令行
scripts/e2e.sh            10k 词条端到端演示
```

## 设计要点

* **payload = canonical CBOR**：结构化字段 + `raw_html` 兜底 + 未知字段保留；
* **block = 不可变字节串**：`ODB1` 魔数，记录按 key 排序，尾部偏移索引；
* **CDC**：gear 表由 BLAKE3 导出，min 16 KiB / max 256 KiB，哈希命中断块；
* **哈希**：`block_hash` 基于未压缩字节，换压缩算法不影响互认；
* **commit**：`ts/author/message` 不进入哈希，同一内容树永远同一 commit `hash`。

详见 [docs/odc-spec.md](docs/odc-spec.md)。

## 路线图

| 里程碑 | 内容 |
| --- | --- |
| **M0（本仓库）** | 规范 + Rust 内核 + CLI + 测试向量 + CI |
| M1 | macOS SwiftUI 编辑器（表格导入、逐条校对、字段级 diff） |
| M2 | 合并 / 冲突表、并发编辑、多语言词条 |
| M3 | patch 生成与应用、增量发布、签名 |
| M4 | 三方合并、冲突表、字段级合并 |
| M5 | gc / shallow 仓库、Swift 与 Kotlin 绑定 |

## 非技术用户工作流（目标形态）

面向词典编辑者，最终只需要四步，全部在图形界面里完成：

1. **导入**：把 Excel / CSV / JSONL 词表拖进应用，字段自动映射成「词条 / 词性 / 释义 / 例句 / 标签」；
2. **校对**：表格里逐条改，应用实时显示「新增 X 条 / 修改 Y 条 / 删除 Z 条」；
3. **提交**：点「提交」，写一句变更说明，得到一串内容哈希（相当于 Git 提交号）；
4. **发布 / 分享**：导出一个 .odc 库给阅读器，或只生成几十 KB 的增量包给协作者。

M0 阶段这些能力由 CLI 提供（odc import / commit / diff / patch），M1 起有 macOS 图形编辑器。

## 开发

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
./scripts/e2e.sh
```

## 许可证

GNU General Public License v3.0 or later（GPL-3.0-or-later），见 [LICENSE](LICENSE)。

选择 GPL-3.0-or-later 的原因：ODC 的目标是让词典内容长期可被任何社区维护，要求分发修改版时同样开放格式实现，避免出现「私有分支格式」。
