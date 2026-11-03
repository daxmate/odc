# 浅导出（`odc pack --shallow`）

`odc pack` 把一个仓库导出成**一个可独立打开的 .odc 文件**（SQLite 单文件，沿用
[§9](../docs/odc-spec.md) 的参考布局）。导出只写从 HEAD 可达的对象，未提交的工作区
（`staging`）、未完成的合并（`conflicts` / `meta.merge`）和历史垃圾都不会进入产物。

```bash
odc --repo ./demo pack -o demo.odc            # 完整历史
odc --repo ./demo pack --shallow -o head.odc  # 只有 HEAD
odc --repo head.odc log                       # 单文件可直接作为 --repo
```

`-o` 省略时写在仓库旁边：目录仓库为 `<目录名>.odc`，单文件仓库为
`<文件名>.packed.odc`。目标已存在时报错，`--force` 才会覆盖。

## 写入了什么

| 对象 | full | shallow |
| --- | --- | --- |
| `commits` | HEAD 可达的全部提交（沿 `parent` / `parent2` 追溯） | 只有 HEAD |
| `commit_index` | 上述每个提交的条目（full 与 delta 原样复制） | 只有 HEAD 的条目；若它是 delta，导出时重建成 full |
| `entries` | HEAD 的物化索引 | 同 full |
| `blocks` | 存储中的全部块 | 仅 `entries` 引用的块 |
| `refs` | `HEAD` | `HEAD` |
| `meta` | `format_version`、`created_at` 等（丢弃 `merge`） | 同 full，另写 `shallow = 1` |

块的取舍：shallow 只保留 HEAD 的 `entries` 引用的块；full 保留存储中的所有块，因为
这些块都由 HEAD 可达的提交产生（`gc` 负责删除不再被任何保留修订需要的块）。
`commit_index` 只保存 `word → content_hash`，不保存历史块位置；浅导出没有父提交可回溯，
所以 HEAD 的条目必须是 full（delta 会在导出时重建成 full）。完整导出虽然保留了
旧提交的块字节，但无法从快照把某个历史修订的词条重新定位到块内偏移；历史修订仍可用
`diff` 比较，但正文以 HEAD 的物化索引为准。

`meta.shallow` 存 ASCII `"1"`，缺省视为 0。它只是可选元数据，**不改变
`format_version`**，也不改写 HEAD 提交的内容或哈希：HEAD 的 `parent` 仍然指向一个
不存在的提交。

## 边界语义

- `log`：沿 first parent 遍历到缺失的父提交时正常结束；CLI 在 stderr 输出
  `(shallow)`。
- `verify`：`meta.shallow = 1` 时缺失父提交是预期边界，不报错；仍然逐块校验
  哈希/解码、索引偏移、`content_hash`、`field_hashes`、提交字段哈希以及 HEAD 与
  索引的一致性。非浅仓库行为不变。
- `gc`：只按当前 `entries` 保留块，HEAD 可达对象不会被误删。
- `patch apply`：以浅仓库的 HEAD 为 base 的补丁可以正常安装并生成新的提交。
- `lookup` / `search` / `show` / `export` / `diff`：与普通仓库一致；`diff` 能用的
  历史范围就是文件里保留下来的提交。

## 与 git shallow 的差异

| 维度 | `git clone --depth` | `odc pack --shallow` |
| --- | --- | --- |
| 产物 | 工作树 + 截断的提交图，另有 `.git/shallow` | 单个只读分发文件 |
| 边界标记 | `.git/shallow` 列出 graft 点 | `meta.shallow = 1` |
| 提交改写 | 不重写提交对象 | 同样不重写，父哈希悬挂 |
| 后续补历史 | `git fetch --unshallow` | `odc fetch --depth`（见下） |
| 用途 | 克隆加速 | 分发阅读器、控制下载体积 |

## 体积基线

`scripts/bench-shallow.sh` 用合成数据跑两个场景，并区分**硬指标**与**信息性指标**：

| 场景 | 构造 | 硬指标 | 信息性指标 |
| --- | --- | --- | --- |
| append | 100,000 词条分 10 批提交，每批一次 commit | `shallow ≤ 1.10 ×` 同 HEAD 的单提交仓库 | full/shallow（实测 1.365） |
| churn | 同一 20,000 键用不同 `gen-entries --seed` 重写 5 次 | 无 | full/shallow（实测 1.452） |

硬指标不达标时脚本退出非零；信息性指标只打印数字。full/shallow **不是**代码质量门槛：
append 场景旧块共享度高，churn 场景又被固定的 `entries` 索引体积主导，比例由数据形状和
churn 程度决定。M5 的硬体积指标只有「shallow 不超过同 HEAD 全新仓库的 1.10 倍」。

## 补齐历史：`odc fetch --depth`

浅导出可以用 `odc fetch` 从**另一个 ODC 仓库**离线补齐历史。它不联网，也不涉及
任何远程协议：源就是一个普通的仓库目录或另一个 `.odc` 文件。

```bash
odc --repo head.odc fetch --from ./demo --depth 50      # HEAD 起保留 50 个提交
odc --repo head.odc fetch --from demo.odc --depth 999 --json
```

- `--from` 必填，可以是目录仓库或单文件 `.odc`；源必须包含 HEAD 和足够的第一父
  提交链，缺一个就整体失败。
- `--depth N` 是**补齐后**从 HEAD 沿 first-parent 链可见的提交总数（含 HEAD）。
  `N = 1` 是合法空操作，`N = 0` 报用法错（退出码 2）。
- 目标必须是浅仓库（`meta.shallow = 1`），否则直接报错（退出码 1）。

### 写入了什么

| 对象 | 变化 |
| --- | --- |
| `commits` | 插入缺失的提交行，只沿 first-parent 链 |
| `commit_index` | 随提交一起复制；新边界那个提交原本是 delta 时改写成 full 快照 |
| `blocks` | 按 `block_hash` 去重补入源仓库的块，写入前重新校验 `BLAKE3(raw)` |
| `entries` / `staging` / `refs` / `resources` / HEAD / 工作区 | 不动 |

补到 genesis（或源历史不够 `N` 个提交）时删除 `meta.shallow`，`log` / `verify`
回到 full 语义，提交图与块存储此时和一次不带 `--shallow` 的 `pack` 一致。没补到
genesis 时保留 `meta.shallow = 1`，可以再执行一次 `fetch` 继续往下补。

### 失败语义

整次补齐在**一个事务**里完成。提交行的哈希与自身字段对不上、快照条目数与提交声明
不符、块的 `BLAKE3(raw)` 与 `block_hash` 不符、或目标与源的共享前缀不一致——任意
一条都回滚全部写入并报错，不会留下半套历史。

### 限制

- 只沿 **first-parent** 链补齐；merge 提交的第二个父提交若落在边界之外，仍然悬挂。
- 历史修订的正文以 `commit_index` 为准（`diff` 可用），不会重新物化工作区。
