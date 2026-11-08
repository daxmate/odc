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

增量发布（详见 [`docs/patch.md`](docs/patch.md)）：

```bash
# 发布方：A、B 两个修订号都取自 `odc log`
./target/release/odc --repo ./demo patch create <base> <target> -o fix.odp
# 接收方：HEAD 必须正好是 <base>
./target/release/odc --repo ./mirror patch verify fix.odp
./target/release/odc --repo ./mirror patch apply fix.odp
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

MDict `.mdx` 词典（1.2 / 2.0）可直接导入；词条 HTML 默认在导入时修复、清洗并格式化（`--html off` 关闭，`--html check` 只告警）：

```bash
odc --repo ./demo import LDOCE6.mdx --format mdx
odc --repo ./demo commit -m "import LDOCE6"
odc --repo ./demo check-html            # 只读检查已提交词条
odc normalize --repo ./demo             # dry-run：报告将改动多少条
odc normalize --repo ./demo --apply     # 写回 raw_html 并提交
odc --repo ./demo lookup apple --json
odc --repo ./demo search --prefix app
odc --repo ./demo export --format jsonl -o dump.jsonl
```

兼容范围、实测数据与已知缺口见 [docs/mdx-compat.md](docs/mdx-compat.md)。

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
| `odc import FILE --format jsonl\|csv\|mdx [--prune] [--limit N] [--progress] [--html format\|check\|off]` | 导入 / 更新工作区（MDX 支持限量、进度与 HTML 模式） |
| `odc check-html [FILE] [--json] [-n N]` | 只读检查 MDX 文件或仓库词条的 HTML / CSS |
| `odc normalize [--apply] [--json]` | 规范化工作区 HTML（默认 dry-run，`--apply` 写回并提交） |
| `odc commit -m MSG` | 提交工作区 |
| `odc log [-n N] [--json]` | 历史 |
| `odc show WORD [--json]` | 查询词条 |
| `odc lookup WORD [--json]` | 点查已提交词条，缺失时退出码 4 |
| `odc search --prefix P [-n N] [--json]` | 前缀查询（走索引） |
| `odc export --format jsonl\|csv [-o FILE]` | 导出全部已提交词条 |
| `odc pack [-o FILE] [--shallow] [--force]` | 导出为单个 .odc 文件（`--shallow` 只含 HEAD，见 `docs/shallow.md`） |
| `odc fetch --from SRC --depth N [--json]` | 浅仓库沿 first-parent 链离线补齐历史（见 `docs/shallow.md`） |
| `odc diff A B [--json]` | 词条级差异（added / removed / modified） |
| `odc status` | 工作区状态 |
| `odc verify` | 全量校验块、索引与提交 |
| `odc patch create A B -o FILE [--json]` | 生成增量包（只含目标新增块与索引增量） |
| `odc patch verify FILE [--json]` | 校验增量包与本地基线 |
| `odc patch apply FILE [--sig PATH] [--trust-new] [--json]` | 应用增量包；给了 `--sig` 时先验签与信任校验（失败不动仓库），精确 base 直接前进，后代 HEAD 转三方合并，失败退出码 4 |
| `odc merge <A> <B> [--strategy auto\|ours\|theirs\|union] [--json]` | 三方合并（A 必须是 HEAD）：快进 / 自动合并 / 冲突退出码 5 |
| `odc conflicts [--json]` | 列出未完成合并的冲突 |
| `odc resolve <word> --ours\|--theirs\|--union\|--file FILE [--json]` | 记录一个冲突的解决方式 |
| `odc merge --continue [--json]` | 冲突全部解决后写入合并提交 |
| `odc keygen [--out DIR] [--name NAME] [--force]` | 生成 Ed25519 私钥（0600）与公钥文件（0644） |
| `odc sign FILE [--key PATH] [--signer NAME] [--out PATH] [--force]` | 生成旁挂签名 `<FILE>.sig` |
| `odc verify-signature FILE [--sig PATH] [--pub PATH \| --pubkey HEX] [--json]` | 验签；无效退出码 1，用法/IO 错误退出码 2 |
| `odc publish --out DIR --label LABEL [--name NAME] [--base COMMIT] [--key PATH] [--signer NAME] [--notes TEXT] [--force] [--json]` | 生成签名发布目录（pack + 可选 patch + `release.json`，见 `docs/release.md`） |
| `odc verify-release DIR [--pub PATH \| --pubkey HEX] [--json]` | 校验清单、工件大小 / BLAKE3 与全部签名；无效退出码 1，用法/IO 错误退出码 2 |
| `odc trust add --pub PATH \| --pubkey HEX [--name NAME] [--json]` | 把发布者公钥写入仓库信任库（`manual`） |
| `odc trust list [--json]` | 列出信任库中的发布者 |
| `odc trust remove PUBLIC_KEY_HEX [--json]` | 从信任库移除发布者 |

全局参数 `--repo DIR` 指定仓库目录（默认当前目录，库文件 `odc.db`）。

### 合并示例

```console
# 两人从同一 base 出发，各自提交后再汇合（A 必须是 HEAD）
odc merge HEAD 3f2a9c1d --json

# 有冲突时：退出码 5，HEAD 不变，冲突记录在 conflicts 表
odc conflicts --json
odc resolve alpha --ours        # 或 --theirs / --union / --file payload.cbor
odc merge --continue --json     # 写入 parent2 = 对方的合并提交
```

自动合并按词条比较 `content_hash`：一方改动直接取该方；双方都改且字段不重叠时按
字段合并（例如一方改音标、一方改释义），否则记为冲突。详见 `docs/merge.md`。

## 签名与发布

发布方（一次性密钥 + 每次发布签名）：

```sh
odc keygen --name release            # 私钥 ~/.config/odc/keys/release.key (0600)
odc sign dict.odc --key ~/.config/odc/keys/release.key --signer alice
odc publish -o release/ --key ~/.config/odc/keys/release.key --signer alice
```

接收方：

```sh
odc trust add --pub release.pub --name alice     # 记住发布者
odc verify-signature dict.odc --pubkey <HEX>     # 单独验签
odc patch apply update.odp --sig update.odp.sig  # 验签 + 信任校验后再应用
odc verify-release release/ --pubkey <HEX>       # 校验整个发布目录
```

Apple 客户端：导入带 `.sig` 的 `.odc` 时会先验签，首次信任需用户确认，未知发布者会被拒绝；库条目上的「应用更新…」用于应用带签名的 `.odp` 增量包，完成后刷新词典与编辑器。布局、清单与信任规则见 `docs/release.md`。

## 仓库结构

```
docs/odc-spec.md          规范性文档（格式的唯一权威）
spec/testvectors/         机器可读测试向量
crates/odc-core/          切块、编码、哈希、SQLite、commit/diff/verify
crates/odc-cli/           odc 命令行
crates/odc-testkit/       测试用合成 MDX 夹具（不发布）
scripts/e2e.sh            10k 词条端到端演示
scripts/bench-1m.sh       100 万合成词条基准（import/commit/diff/gc/verify 的时间与峰值 RSS）
scripts/bench-shallow.sh  pack 体积基线：append（100k/10 提交）+ churn（20k/5 次重写）
apps/apple/               iOS / macOS SwiftUI 客户端（ODict / ODict Desktop，只读 v0）
```

## 设计要点

* **payload = canonical CBOR**：结构化字段 + `raw_html` 兜底 + 未知字段保留；
* **block = 不可变字节串**：`ODB1` 魔数，记录按 key 排序，尾部偏移索引；
* **CDC**：gear 表由 BLAKE3 导出，min 16 KiB / max 1 MiB，哈希命中断块；
* **哈希**：`block_hash` 基于未压缩字节，换压缩算法不影响互认；
* **commit**：`ts/author/message` 不进入哈希，同一内容树永远同一 commit `hash`；
* **索引**：SQLite 索引存块号与 8 字节摘要前缀（布局版本 2，`meta.schema_version = 2`），
  提交哈希仍按完整 32 字节摘要计算；版本 1 的库可读可查，第一次写入时原位升级。

详见 [docs/odc-spec.md](docs/odc-spec.md)。

## 路线图

| 里程碑 | 内容 |
| --- | --- |
| **M0（本仓库）** | 规范 + Rust 内核 + CLI + 测试向量 + CI |
| **M1**（已完成，0.13.0） | macOS SwiftUI 编辑器（表格导入、逐条校对、字段级 diff） |
| M2 | 合并 / 冲突表、并发编辑；多语言词条 **deferred**（0.13 不做） |
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

## Apple 客户端（v0）

`apps/apple/` 下是 **ODict**（iOS）与 **ODict Desktop**（macOS）两个 SwiftUI
target，共享 `ODictKit`（`ODCFFI` 封装、状态与视图）。当前范围：打开 .odc →
前缀搜索 → 词条渲染；macOS 另支持新建 / 示例词典、导入和编辑（iOS 仍只读）。
构建步骤见 `apps/apple/README.md`：

```bash
bash scripts/build-odc-xcframework.sh   # Rust C ABI -> ODCFFI.xcframework
cd apps/apple && xcodegen generate      # project.yml -> ODict.xcodeproj
```

一条命令构建两端：`./scripts/build-app.sh`（根目录 `./build-app.sh` 为兼容别名，旧入口 `scripts/build-apple.sh` 仍可用）。包测试：
`swift test --package-path apps/apple/ODictKit`。

打包安装 macOS（覆盖安装并启动）：`./scripts/build-app.sh --install`；安装目录用
`ODC_INSTALL_DIR` 覆盖（默认 `/Applications`），`ODC_SKIP_LAUNCH=1` 跳过启动。
iOS 端尚未开发，`ios --install` 会报错退出（状态码 2）。细节见 `apps/apple/README.md`。

**导入 MDX（仅 macOS）**：`Open Dictionary or MDX…`（⌘O）选一个 `.mdx`，
App 在后台调用 `odc_import_mdx_to_odc` 转换成独立 `<name>.odc`（与源文件同
目录，不可写时回退 `~/Documents`，已存在则加 `-1`、`-2` 后缀），完成后自动
打开并提示导入条数与跳过的损坏块。损坏记录块沿用 Phase 1 的「跳过坏块」策略；
iOS target 不暴露入口，只保证编译。细节见 `apps/apple/README.md`。

## 开发

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
./scripts/e2e.sh
./scripts/bench-1m.sh
./scripts/bench-shallow.sh
```

## 许可证

GNU General Public License v3.0 or later（GPL-3.0-or-later），见 [LICENSE](LICENSE)。

选择 GPL-3.0-or-later 的原因：ODC 的目标是让词典内容长期可被任何社区维护，要求分发修改版时同样开放格式实现，避免出现「私有分支格式」。
