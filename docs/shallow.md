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
| `commit_index` | 上述每个提交的快照 | 只有 HEAD 的快照 |
| `entries` | HEAD 的物化索引 | 同 full |
| `blocks` | 存储中的全部块 | 仅 `entries` 引用的块 |
| `refs` | `HEAD` | `HEAD` |
| `meta` | `format_version`、`created_at` 等（丢弃 `merge`） | 同 full，另写 `shallow = 1` |

块的取舍：shallow 只保留 HEAD 的 `entries` 引用的块；full 保留存储中的所有块，因为
这些块都由 HEAD 可达的提交产生（`gc` 负责删除不再被任何保留修订需要的块）。
`commit_index` 只保存 `word → content_hash`，不保存历史块位置，所以 full 虽然保留了
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
| 后续补历史 | `git fetch --unshallow` | 暂不支持（见下） |
| 用途 | 克隆加速 | 分发阅读器、控制下载体积 |

## 未来 `odc fetch --depth`（占位设计，未实现）

本版本**没有** `fetch` 子命令，也不做任何网络或远程操作。若以后补齐，建议：

1. 接收方在 `meta.shallow = 1` 且父提交缺失时，允许请求“补齐到 depth N”。
2. 传输沿用 `patch` 的块集合差分（`base_blocks` + 内联新块），避免重传 HEAD 块。
3. 补到 genesis 后删除 `meta.shallow`，让 `log` / `verify` 回到 full 语义。

在此之前，需要完整历史请从源头 `odc pack`（不带 `--shallow`）。
