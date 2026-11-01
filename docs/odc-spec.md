# ODC 格式规范 v0.1（Open Dictionary Container）

> 状态：M0 冻结草案。本文件是互操作性的唯一权威来源；参考实现见 `crates/odc-core`。
> 格式版本：`0.1`（MAJOR = 0，MINOR = 1）。

## 1. 目标与非目标

ODC 是一个面向词典内容的、内容寻址（content-addressed）的容器格式，目标是让**不懂技术的词典爱好者**也能制作、校对、增量同步词典。

非目标（M0）：图形界面、网络协议、加密、DRM、全文检索引擎。

## 2. 术语

| 术语 | 含义 |
| --- | --- |
| entry（词条） | 一个查询键对应的一份词典内容 |
| key | 归一化后的查询键，UTF-8 字符串，词条的主键 |
| record | 块内的 `(key, payload)` 二元组 |
| payload | 词条内容的规范 CBOR 编码 |
| block | 一组按 key 排序的 record 的不可变字节串 |
| commit | 指向一棵 Merkle 词条树与一组块的状态快照 |
| repository | 一个 SQLite 数据库（实现细节）承载的提交历史 |

## 3. 分层模型

```
word (display)  →  payload (canonical CBOR)  →  record (key + payload)
record…         →  raw block ("ODB1" …)      →  blocks 集合
(entry_root, block_root, count, parents)     →  commit_hash
```

* **payload** 是内容的最小单位：只有 payload 变化才会改变 `content_hash`。
* **record** 是切块的最小单位：一条 record 永不被拆分到两个块中。
* **block** 是不可变的存储与传输单位。
* **commit** 是历史的单位：包含父提交指针、两个 Merkle 根与词条计数。

逻辑格式（本规范）与物理容器（SQLite）分离：任何能重建上述逻辑对象的存储都合规。

## 4. 词条与 payload

### 4.1 key 归一化

`key = NFKC(display_input)` 再做 Unicode case folding。参考实现采用 `str::to_lowercase` 加以下特例：

* `ß` / `ẞ` → `ss`
* `ς` → `σ`
* 连字（如 `ﬁ`）由 NFKC 完成分解

其余完整 case folding 映射见 Unicode CaseFolding.txt；MINOR ≥ 2 时若实现完整映射，会产生不同的 key 空间，因此**归一化算法版本必须与库版本一并声明**（见 §13）。

### 4.2 payload 字段

payload 是一个 CBOR map，M0 定义 5 个字段，**全部字段必须出现**（空值用空数组 / null）：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `display` | text | 展示词形（可含大小写、变音） |
| `phonetic` | array of text | 读音，IPA 或自定义 |
| `defs` | array of map | 释义，见下 |
| `raw_html` | text 或 null | 无法结构化时的 HTML 兜底 |
| `extra` | map | 未知字段保留区 |

`defs` 每项为 map，字段：`pos`（text）、`sense`（text）、`examples`（array of text）、`labels`（array of text），四者均必须出现。

### 4.3 前向兼容

* 解码器遇到**未知顶层字段**必须原样保留（写入 `extra`，或按实现选择保留在原位置），并在重新编码时输出。
* 编码器输出字段顺序由 canonical CBOR 决定，与源码顺序无关。

### 4.4 content_hash

`content_hash = BLAKE3-256(canonical_cbor(payload))`，32 字节。

### 4.5 field_hashes

`field_hashes` 是 CBOR map：字段名（text）→ BLAKE3-256(`canonical_cbor(该字段的值)`)。用于字段级 diff 与未来的块级合并。

## 5. 字节编码约定

### 5.1 varint

无符号 LEB128，**必须最短编码**（拒绝 `0x80 0x00` 这类 overlong 形式），最大 10 字节，超出 u64 范围视为错误。

### 5.2 canonical CBOR 子集

允许的类型：unsigned / negative integer、byte string、text string、array、map、`true`、`false`、`null`。

禁止：indefinite length、tag、float、bignum、simple value（除 true/false/null）。

确定性规则（RFC 8949 §4.2.1 core deterministic encoding）：

* 所有长度使用定长头；
* 整数使用最短头；
* map 的 key **按编码后字节的字典序**排序；
* 重复 key 或乱序 key 视为解码错误。

## 6. 块格式（`ODB1`）

```
raw_block = "ODB1"                      ; 4 字节
          | flags                       ; u8
          | count                       ; varint
          | record * count              ; 见下
          | offset[count]               ; 可选，varint 数组
record    = varint(key_len) | key       ; key = UTF-8 字节
          | varint(payload_len) | payload
```

* `flags`：bit0 = 尾部偏移索引存在；其余位保留，必须为 0，否则报错。
* **规范编码器必须置 bit0 = 1**（写出偏移索引），保证同一内容只有一种规范字节串。
* `offset[i]` = 第 i 条 record 起始位置相对 **raw_block 起点**的字节偏移。
* record 必须按 key 的 UTF-8 字节序**严格升序**，不允许重复 key。
* 块内 record 数可以为 1（超大词条独占块）。
* `block_hash = BLAKE3-256(raw_block 字节)`，**不是压缩后字节**：换 codec 不改变哈希，patch 可跨 codec 互认。

## 7. 内容定义切块（CDC）

### 7.1 gear 表

```
gear[i] = u64::from_le_bytes( BLAKE3("odc-gear-v1" || [i as u8])[0..8] )
```

i = 0..255。任何实现都可独立重算该表。

### 7.2 滚动哈希

对每条 record 的字节流 `key || 0x00 || content_hash`（content_hash 为 32 字节）逐字节滚动：

```
h = (h << 1) + gear[byte]        // u64 环绕
```

每个块开始时 `h = 0`。

### 7.3 边界规则

按 key 升序处理 record，维护当前块字节数 `size`（只计 record 字节，不含块头与偏移索引）：

1. 若加入下一条 record 会超过 `max = 1 MiB` 且当前块非空 → **先断块**，再开始新块；
2. 加入该 record，对该 record 的 `key || 0x00 || content_hash` 逐字节更新 `h`；
3. 若 `size ≥ min = 16 KiB` 且该 record 内**任一字节**使 `(h & 0xFFFF) == 0` → 在该 record 末尾断块（record 是原子单位，断点不落在 record 中间）；
4. 否则若 `size ≥ max` → 断块；
5. 单条 record 本身超过 max 时独占一块（规则 1 保证它单独成块）。

min = 16 384 字节，max = 1 048 576 字节（1 MiB），掩码 `0xFFFF`。

> `max` 在 0.1 格式里是 256 KiB；提高到 1 MiB 让一次断块覆盖更长的压缩窗口（每块是独立的 zstd
> 帧），代价是 dedupe 粒度变粗：一个 1 MiB 块内改动任一字节，整个块都要重写。

> 由于块头与偏移索引不计入 `size`，raw_block 长度可比 max 多出块头与每条记录约 1–3 字节的偏移条目（约 \`2 × 记录数 + 16\` 字节）。

### 7.4 性质

* 插入 / 删除 / 修改一条词条只影响其所在块及其后的一个边界区间（平均约 1–2 块），这是增量同步的成本基础。
* 切块结果与文件切分、压缩方式无关。

## 8. 哈希与 Merkle

### 8.1 词条树

对按 key 升序排列的 `(key, content_hash)`：

```
leaf  = BLAKE3(0x00 || varint(len(key)) || key || content_hash)
node  = BLAKE3(0x01 || left || right)
```

* 奇数个节点时最后一个**直接上提**（不复制）。
* 空集合根 = 32 字节 0（`0x00…00`）。
* 结果记为 `entry_root`。
* `content_hash` 永远是**完整 32 字节**的 `BLAKE3-256(canonical_cbor(payload))`。SQLite 索引里只保留它的
  8 字节前缀（§9）是存储细节，不改变本节的树；同一内容在布局版本 1 与 2 上必须得到同一个 `entry_root`。

### 8.2 块根

`block_root` = 对所有**被引用的块哈希**按字节升序排列后做 Merkle：叶子即 32 字节的 `block_hash` 本身，内部节点规则同 §8.1。空集合为 32 字节 0。

### 8.3 commit_hash

```
commit_hash = BLAKE3(
    "odc-commit-v1"
  || parent            ; 32 字节，无父时全 0
  || parent2           ; 32 字节，无第二父时全 0
  || entry_root        ; 32 字节
  || block_root        ; 32 字节
  || LE64(entry_count)
)
```

`ts`、`author`、`message` **不参与哈希**，单独存储：同一内容树在不同时间提交应得到相同 commit_hash（保证可重放、可校验）。

### 8.4 空库 genesis

空库 commit 的 `parent = parent2 = entry_root = block_root = 0…0`，`entry_count = 0`。其哈希由 §8.3 唯一确定，见测试向量 `genesis.json`。

## 9. 存储（SQLite 参考实现）

SQLite 布局是实现细节，但参考实现固定如下（布局版本 2）：

```sql
CREATE TABLE meta(key TEXT PRIMARY KEY, value BLOB NOT NULL);
CREATE TABLE blocks(id INTEGER PRIMARY KEY, block_hash BLOB NOT NULL UNIQUE,
                    codec INTEGER NOT NULL, raw_size INTEGER NOT NULL,
                    data BLOB NOT NULL);
CREATE TABLE entries(word TEXT PRIMARY KEY, display TEXT NOT NULL,
                     block_id INTEGER NOT NULL, offset INTEGER NOT NULL,
                     length INTEGER NOT NULL, content_hash BLOB NOT NULL,
                     field_hashes BLOB NOT NULL);
CREATE TABLE commits(commit_hash BLOB PRIMARY KEY, parent BLOB NOT NULL,
                     parent2 BLOB NOT NULL, entry_root BLOB NOT NULL,
                     block_root BLOB NOT NULL, entry_count INTEGER NOT NULL,
                     ts INTEGER NOT NULL, author TEXT NOT NULL,
                     message TEXT NOT NULL);
CREATE TABLE commit_index(commit_hash BLOB PRIMARY KEY, codec INTEGER NOT NULL,
                          raw_size INTEGER NOT NULL, data BLOB NOT NULL);
CREATE TABLE refs(name TEXT PRIMARY KEY, commit_hash BLOB NOT NULL);
CREATE TABLE conflicts(id INTEGER PRIMARY KEY, word TEXT NOT NULL, detail TEXT NOT NULL);
CREATE TABLE staging(word TEXT PRIMARY KEY, display TEXT NOT NULL,
                     payload BLOB, content_hash BLOB NOT NULL);
CREATE TABLE resources(key TEXT PRIMARY KEY, block_id INTEGER NOT NULL,
                       size INTEGER NOT NULL);
```

* `entries` 是 HEAD 的物化索引；`block_id` 指向 `blocks.id`（块号），`offset`/`length` 指向块内该
  record 的字节区间。块号是**本库局部**的句柄：跨库搬运块必须按 `block_hash` 复制、由目标库重新发号
  （`pack` 即如此），不能直接复制 `id`。
* `entries.content_hash` 与 `field_hashes` 内每个字段摘要都只存 BLAKE3 的**前 8 字节**（64 bit）。
  词条树的叶子仍用完整 32 字节摘要（提交时由 payload 现算），因此提交哈希与格式 0.1 完全一致；
  索引只做"内容是否变化"的比较，64 bit 前缀的意外碰撞概率是 2⁻⁶⁴。
* `meta.schema_version = 2` 标记 SQLite 布局版本。版本 1 的库（`entries.block_hash`、32 字节摘要、
  `entries_block` 索引）仍可读、可查、可导出；第一次写入（import / commit / patch / merge / gc）时
  在原位重写为版本 2 并写入该键。`format_version` 不变：两种布局描述同一个 block/commit 模型。
* 块用 zstd 存储，参考实现的写入档位是 **level 9**（读路径与档位无关：帧头自带参数）。
* 提交成功后 `staging` 被整体清空，`meta.workset_clean = "1"` 表示工作区等于 HEAD；
  下一次写入前按需从 `entries` 重建（只复制 word / display / content_hash，payload 由块按需取回）。
  这样工作区不必在任何时候都保留一份索引的副本。
* `commit_index` 存 zstd 压缩的 CBOR 条目，用于历史 diff。条目是 6 元数组
  `["odc-idx", version, kind, depth, entry_count, body]`：

  | 字段 | 含义 |
  | --- | --- |
  | `"odc-idx"` | 固定文本帧，用来与旧版条目区分 |
  | `version` | 条目格式版本，当前为 `1` |
  | `kind` | `0` = full（全量快照），`1` = delta（相对父提交的增量） |
  | `depth` | 到最近 full 锚点之间的 delta 个数；full 为 `0` |
  | `entry_count` | 重建后索引的条目数 |
  | `body` | full：按 key 升序的 `[[word, content_hash], …]` 数组；delta：按 word 严格升序的操作数组 |

  delta 的操作为 `[word, op]`（`op = 2` 删除）或 `[word, op, content_hash]`
  （`op = 0` 新增、`1` 修改）。读一个 delta 时沿 first parent 回溯到最近的 full，
  再按顺序回放：两侧都是升序流，合并只保留每条流的一个当前项，不物化整表。
  每 32 个提交写一次 full 锚点（`snapshot::ANCHOR_INTERVAL`），把回溯深度限制在 31 层；
  除锚点外，新增提交只在它与父提交的差异不超过索引一半时写 delta，否则退化为 full。

  旧库无需迁移：没有 `"odc-idx"` 帧的条目就是旧版的全量快照（顶层是
  `[[word, content_hash], …]` 数组），读路径直接按 full 处理。
  `raw_size` 记录该条目自身的未压缩字节数；`verify` 会把一条链上所有条目的
  `raw_size` 之和与实际解码字节数比对。
* `staging` 是工作区（未提交内容）。它不属于互操作格式。已提交词条只保留指针（`word`、`display`、`content_hash`），`payload` 为 NULL；未提交词条必须携带 payload。需要正文时按 `content_hash` 从 `entries` 定位块、解压并校验。提交成功后整张表被清空，见上文 `meta.workset_clean`。
* codec：`0` = 原始字节，`1` = zstd（参考实现写入用级别 9，实现可调，不影响任何哈希）。
* `conflicts` 保存未完成的合并中每个冲突词条的决定（见 §10.1 与 `docs/merge.md`）。
* `meta.shallow = 1`（ASCII `"1"`）标记由 `pack --shallow` 导出的文件：HEAD 的父提交可以缺失，`log` 在边界结束、`verify` 视其为正常终点。缺省 `0`；这是可选元数据，不改变 `format_version`，也不改写提交对象。详见 `docs/shallow.md`。
* `resources` 保存 MDX 配套 MDD 里的图片、发音等载荷：`key` 是规范化后的资源路径（反斜杠转 `/`、去前导 `/` 与 `./`，大小写保留、查找时大小写不敏感），`block_id` 指向 `blocks.id`（`codec = 0` 原样存储，载荷本身已是压缩媒体）。导入旧库时按需建表，不改变 `schema_version`；`pack` 一并导出该表及其块。
* `pack` 的产物是一个独立的 SQLite 数据库文件，可以直接作为 `--repo` 打开；它不包含 `staging`、`conflicts` 或 `meta.merge`。

## 10. 命令语义

| 命令 | 语义 |
| --- | --- |
| `init` | 建库并写入 genesis commit（空树） |
| `import --format jsonl\|csv [--prune]` | 将词条写入工作区；`--prune` 删除文件中不存在的词条 |
| `import --format mdx [--limit N] [--progress]` | 流式导入 MDX 词典，映射规则见 §16 |
| `commit -m …` | 工作区与 HEAD 不同才允许；生成新块（已存在的块复用）、重建索引、提交 |
| `log` | 从 HEAD 沿 first parent 逆序输出 |
| `show <word>` | 按键查词条，返回 payload |
| `lookup <word>` | 同 `show`，词条不存在时退出码 4 |
| `search --prefix P [-n N]` | 归一化前缀查询，走 entries 索引，按键升序 |
| `export --format jsonl\|csv [-o FILE]` | 导出全部已提交词条（display + raw_html 等） |
| `pack [-o FILE] [--shallow] [--force]` | 导出为单个 .odc 文件：full 写 HEAD 可达的全部提交与快照，`--shallow` 只写 HEAD；`--force` 覆盖已存在的文件 |
| `diff A B` | 词条级 added / removed / modified；`--json` 输出结构化结果 |
| `status` | 工作区相对 HEAD 的 added / modified / removed |
| `verify` | 校验块哈希、块解码、索引偏移、content_hash、field_hashes、commit 哈希链、HEAD 根一致性；逐块流式校验，内存不随词典规模增长 |
| `gc --keep-last N [--dry-run] [--vacuum]` | 保留每个 ref 及其 first-parent 链上最近 N 个提交，删除其余 commit、commit_index 与当前索引不再引用的块（保留链上最老的 delta 会先重写成 full 锚点，避免父索引被删后无法回溯）；`--dry-run` 只报告将释放的条数与字节，`--vacuum` 事后重建数据库文件 |
| `patch create \| apply \| verify` | 见 §11 与 `docs/patch.md`；`apply` 在精确 base 上应用，HEAD 为 base 后代时转为三方合并 |

`search` 只做前缀匹配；核心另提供 `Repo::search_substring`（`LIKE ... ESCAPE` 包含匹配，短词优先），FFI 与 Apple 端用它作为前缀无结果时的回退，CLI 尚未暴露。

revision 语法：`HEAD`、`HEAD~N`、commit 哈希前缀（≥ 4 个十六进制字符，需唯一）。

## 10.1 合并语义

`merge` 在 `parent` / `parent2` 组成的 DAG 上工作，第一个参数必须解析为 HEAD。

* **merge base**：按两个提交到各祖先的距离做分层 BFS；候选先比较 `max(distance)`，
  再比较距离和，最后比较 `commit_hash` 字典序（最小者胜），结果确定。没有公共
  祖先时按空树作为 base。
* **快进**：A 是 B 的祖先时 HEAD 直接指向 B，不产生合并提交；B 是 A 的祖先时为无操作。
* **自动合并**：逐 word 比较 base / ours / theirs 的 `content_hash`；只有一方变化取变化方，
  双方变成相同值取该值；双方都变且不同时比较 `field_hashes`，改动字段互不重叠则按字段
  取值并重新计算 `content_hash`。
* **冲突**：写入 `conflicts` 表（`detail` 为规范 CBOR，含 base / ours / theirs 的
  `content_hash`、各方字段哈希、冲突字段与时间），不产生合并提交，退出码 5；`--json`
  输出冲突清单。
* **解决**：`resolve <word> --ours|--theirs|--union|--file <path>` 只记录决定；
  `merge --continue` 在全部冲突解决后写入合并提交（`parent` = 原 HEAD，`parent2` = B）。
* **union**：ours 保留原 key，theirs 存为第一个空闲的 `<key>~N`（N 从 2 起）；后缀作用于
  已归一化的 key。
* **patch 联动**：HEAD 是 patch base 的后代时，`patch apply` 不再报 `ODC-E025`，而是在安装
  target 之后做三方合并（base = patch base，ours = HEAD，theirs = target）；冲突时 HEAD 与
  工作集不变，退出码 5。
* 未完成的合并记录在 `meta` 表的 `merge` 键下。
## 11. patch 格式（`ODP1`）

M0 不实现生成与应用，但字节格式冻结如下：

```
patch = "ODP1" | version:u8 (=1) | flags:u8 | base:32B | target:32B
      | varint(base_block_count) | base_block_hash[base_block_count]
      | varint(block_count) | block_entry[block_count]
      | varint(index_count) | index_entry[index_count]
      | checksum:32B

block_entry = varint(key_len)? … 见下
            = varint(raw_size) | varint(codec) | varint(data_len) | data
index_entry = varint(len(word)) | word | content_hash:32B
```

* `flags` bit0：1 = 携带完整索引增量（M0 语义），0 = 仅块差。
* `checksum = BLAKE3("odc-patch-v1" || 上述除 checksum 外的全部字节)`。
* 应用方必须校验：`base` 存在、`checksum` 正确、每个块的 `BLAKE3(raw)` 与 `target` 引用的 `block_root` 一致，然后原子地写入块、索引与提交。

## 12. 错误码

| 码 | 含义 |
| --- | --- |
| ODC-E001 | varint 畸形 / 非最短编码 |
| ODC-E002 | 截断 |
| ODC-E003 | 尾随多余字节 |
| ODC-E004 | 魔数或 flags 非法 |
| ODC-E005 | CBOR 不合规（含非 canonical / 重复 key / 未知类型） |
| ODC-E006 | record 未排序 |
| ODC-E007 | 块内重复 key |
| ODC-E008 | 哈希不匹配 |
| ODC-E009 | 块不存在 |
| ODC-E010 | 未知 codec |
| ODC-E011 | 未知 commit / revision |
| ODC-E012 | 无变更可提交 |
| ODC-E013 | 仓库已初始化 |
| ODC-E014 | 不是 ODC 仓库 |
| ODC-E015 | 不支持的格式版本（MAJOR 不符） |
| ODC-E016 | 参数非法 |
| ODC-E017 | 已规范但未实现（M0 的 patch） |
| ODC-E018 | 索引不一致 |
| ODC-E019 | I/O 错误 |
| ODC-E020 | SQLite 错误 |
| ODC-E021 | MDX 解析错误（头部、key block、记录块、压缩流或加密） |
| ODC-E022 | 单条 MDX 记录超过 1 MiB 上限 |
| ODC-E023 | patch 校验和错误 |
| ODC-E024 | patch 基线不符（base 不存在或不是 HEAD） |
| ODC-E025 | 需要三方合并（HEAD 是 base 的后代，属 M4） |
| ODC-E026 | patch 已应用（target 提交已存在） |
| ODC-E027 | patch 字节流结构非法 |
| ODC-E028 | 合并已在进行中 |
| ODC-E029 | 合并仍有未解决的冲突 |
| ODC-E030 | 没有进行中的合并 |

## 13. 版本兼容

* 版本字符串 `MAJOR.MINOR`。
* **未知 MAJOR**（≠ 0）：必须拒绝读取，报 `ODC-E015`。
* **未知 MINOR**：必须可读；未知字段原样保留（§4.3），写回时不丢失。
* 新 MINOR 只能增加字段或放宽约束，不能改变既有字段的字节语义。
* 同一 MAJOR 内，`entry_root` / `block_root` / `commit_hash` 的计算方式不可改变。

## 14. 测试向量

`spec/testvectors/` 下为机器可读 JSON：

| 文件 | 内容 |
| --- | --- |
| `payload_encoding.json` | 若干词条 → canonical CBOR 十六进制、content_hash、field_hashes |
| `block_encoding.json` | record 列表 → raw_block 十六进制、偏移、block_hash |
| `cdc_boundaries.json` | 确定性生成的 record 集 → 每个块的 record 区间与字节数 |
| `commit_hash.json` | 各分量 → commit_hash（含 parent 场景） |
| `genesis.json` | 空库 genesis commit 哈希 |

生成方式：`cargo run -p odc-core --example gen_testvectors`（算法由本规范完全确定，任何独立实现都可复算并比对）。测试 `crates/odc-core/tests/vectors.rs` 会读取并断言。

## 15. 已知限制

* case folding 为近似实现（§4.1），未覆盖全部 Unicode 特殊映射。
* 合并（`parent2`）、patch、索引压缩策略调优、并发写入控制留待后续里程碑。
* `conflicts` 表暂未使用。
* 未对大块做内存映射；verify 会解压所有块，超大库需分片执行。
* `commit` 一次性装载全部工作区 payload，峰值内存随内容线性增长（MDX 大词典可达 GB 级），流式 commit 留待后续里程碑。
* MDX 导入不解析 HTML 结构，`defs` 为空；需要结构化词条时应由外部工具生成 JSONL。

## 16. MDX 导入映射

`odc import --format mdx` 将 MDict `.mdx` 词典（1.2 / 2.0）映射为 ODC 词条：

* **读取**：底层解析器 mdictlib 0.2.8，逐块解码并校验 adler32；头部、key block、
  记录块或压缩流损坏时报 `ODC-E021`。
* **payload**：`display` = 词典中的原始词形，`raw_html` = 记录正文；
  `defs`、`phonetic` 留空，`extra` 为空。导入不做 HTML 结构化解析。
* **键**：按 §4.1 归一化（NFKC + casefold）。
* **重复键**：同一次导入内归一化键重复时，第一次保留归一化键本身，后续出现
  按文件顺序追加 `U+001F<n>`（n = 2、3…）；`display` 保留各次的原词形。
* **上限**：单条记录正文超过 1 MiB（1 048 576 字节）时报 `ODC-E022`；
  导入命令跳过该词条并计数。trim 后为空的键同样跳过并计数。
* **流式**：词条按 1024 条一批写入工作区，进程内存与会话大小无关；
  `--limit N` 在读取 N 条后停止，`--progress` 每 10 000 条向 stderr 报告一次。
* **后续命令**：词条在 `commit` 后进入 `entries` 索引，`lookup`、`search`、
  `export` 只读取已提交索引。

兼容范围、实测数据与已知缺口见 [mdx-compat.md](mdx-compat.md)。
