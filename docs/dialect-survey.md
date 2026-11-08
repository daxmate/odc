# 方言盘点（`odc survey`）

一句话：**不写任何规则**，直接从 MDX 的 HTML 量出「核心槽位种类 / 词条模板簇 / 装饰块 / 离群占比」，
给「这本词典要多少人工」一个可复现的档位。它是写 profile 之前的「量词」，不是适配器。

```bash
odc survey book.mdx --limit 2000 --out ./odc-survey
odc survey book.mdx --limit 0            # 全量，注意耗时与内存
odc survey book.mdx --entry-selector '.entry'
odc survey book.mdx --core-threshold 0.01  # 核心种类门槛（默认 1% 词条）
odc survey book.mdx --json               # 报告打到 stdout
```

## 怎么算（没有词典特判）

| 概念 | 定义 |
| --- | --- |
| 槽位种类 kind | 词条容器内一个元素的 `(tag, 首个 class token)` |
| 折叠 | 同一父元素下连续的同类兄弟合并为一次，并记录重复次数 |
| 核心种类 core | 出现在 ≥ core threshold 词条里的 kind（默认 1%） |
| 装饰块 decor | 低于门槛的 kind：照常计数，但不参与模板签名 |
| 模板 template | 词条**直接子级 + 一层嵌套**上出现的**核心** kind 的**集合**（不是序列） |
| 簇 cluster | 共享同一核心模板的词条集合 |
| coreless | 一个核心 kind 都没有的词条，单独计数，不参与聚类 |
| 离群 outlier | 占已聚类词条 < 0.5% 的簇 |
| 别名 | `@@@LINK=` 开头的记录只计入 alias，不参与模板统计 |

旧口径（E5）用「全部 kind 的集合」当签名，装饰块有没有出现会直接分裂模板，簇数里混着噪声；
`clusters_all` 保留这个数字，只作对照，不参与档位判定。

## 成本模型（E7：种类 ≈ 规则数）

前两版都拿「组合」当人工量：E5 用全部 kind 的集合，E6 只留 ≥1% 的核心 kind。两者在
LDOCE6 上都失败（11632 / 11186 簇），而事实是 LDOCE6 用 `profiles/ldoce6.profile.json`
里的 47 条规则就适配成功，OALD9 一共只有 15 种槽位。

**结论：人工量跟着「槽位种类数 + 结构怪癖类型数」走，不跟组合走。**可选块的有无
不增加规则，只增加渲染分支。

```
estimated_rules = structural_kinds + leaf_kinds_used   # 每种一个映射规则
estimated_ops   = 命中的结构怪癖类型数                  # 每类一个算子
verdict:
  easy   : estimated_rules <= 20 且 estimated_ops <= 2
  medium : estimated_rules <= 60 且 estimated_ops <= 4
  hard   : 其它
```

种类分两类（`kinds.core[].shape` / `kinds.decor[].shape` 之外，表格里也有 `shape` 列）：

| 类别 | 判据 | 成本 |
| --- | --- | --- |
| `structural` | 出现过元素子节点（容器） | 1 条映射 + 子级自身的规则 |
| `leaf` | 子节点只有文本或为空 | 1 条映射 |

结构怪癖（`quirks`，只检测计数，不自动修）：

| 怪癖 | 判据 | 需要的算子 |
| --- | --- | --- |
| `multi_entry` | 一条记录里有多个词条容器 | split |
| `nested_entry` | 词条容器内又出现同 kind 容器 | demote / promote |
| `long_run` | 同一父节点下同类兄弟 > 10 | group |
| `floating_block` | 同一 kind 在 ≥2 个深度出现 | hoist |

簇数从此只描述骨架分布：`clusters_core` / `clusters_all` 仍在报告里，但不参与定档。

词条容器识别：`--entry-selector` > class 里含 `entry` token 的元素（取最外层，
一条记录可以有多个）> 退化启发式（同签名重复出现、含标题、文本最长的容器）。
识别不到任何容器 → 退出码 2。

参考档位（旧核心口径，写死，可调不了，但**不再定档**）：

- **easy**：core kinds ≤ 12 且 clusters_core ≤ 20 且离群词条占比 ≤ 5%
- **medium**：core kinds ≤ 25 且 clusters_core ≤ 60
- **manual**：其它

阈值来自「一本能手工写完 profile 的词典该有多少骨架」这一判断，**不随词典调**；
调阈值凑档位等于把尺子掰弯。E7 起它只作参考，`survey.md` 里标为
`## Cluster verdict (reference only)`；定档看上面的成本模型。

## 输出

| 文件 | 内容 |
| --- | --- |
| `survey.md` | 人读：顶部先给成本估算（规则数 / 算子数 / 结论）与怪癖表，再给 kinds 表（含 `shape`）、装饰块表、clusters_core 全表、最大 200 簇的 layout + 样本 key、结构统计 |
| `survey.json` | 机器读：`estimate{rules,ops,verdict,basis} / quirks[] / kinds.core / kinds.decor / clusters_core / clusters_all / coreless / coverage / decor / stats / verdict` |
| `samples/<cluster-id>/<safe-key>.html` | 每簇最多 5 条原始 HTML（单条上限 256 KB，全局预算 64 MB） |

`coverage` 给出覆盖 50% / 80% / 95% **已聚类词条**所需的簇数；分母不含 coreless。

退出码：`0` 产出成功（**不代表词典好适配**）；`2` 识别不到词条容器；`1` 输入/参数错误。

## 分层抽样导出（`--dump-jsonl`）

手挑的 60 条证明不了「整本词典」，全量复验又太贵。`--dump-jsonl` 把整本词典压成一份
**可复现**的小样本，直接喂给 `examples/profile_compare.rs --source`：

```bash
odc survey 'OALD9.mdx' --out /tmp/odc-oald9/survey \
  --dump-jsonl /tmp/odc-oald9/stratified.jsonl --dump-limit 500

cargo run --release -q -p odc-core --example profile_compare -- \
  --profile profiles/oald9.profile.json \
  --source /tmp/odc-oald9/stratified.jsonl \
  --report /tmp/oald9-stratified.json [--strict]
```

`--strict` 是门禁：coverage 100% 且 unadapted 0 且已适配记录四维全过才 `exit 0`，
否则 `exit 6`；不带 `--strict` 时 `exit 0`，只输出测量行。coverage 与四维率的口径见
`profile-contract.md` §4。

抽样规则（写死、不含随机数，同一个 MDX 永远得到同一份样本）：

1. 按 headword 首字符分桶：`A`–`Z` 各一桶，数字、符号和非 ASCII 首字母统一进 `#` 桶；
2. 每桶配额 = 该桶正文条数占全量的比例 × `--dump-limit`，不足 5 条按 5 条补
   （桶本身不够、或总预算撑不起时除外：先削配额最大的桶）；
   **`--dump-limit 0` = 全量**（每桶配额即该桶全部正文，用于整本复验）；
3. 桶内按 `--dump-stride` 等距取（默认 `桶大小 / 配额`，至少 1），取满配额为止；
4. `@@@LINK=` 别名记录和空正文一律不写。

输出两个文件：

| 文件 | 内容 |
| --- | --- |
| `<path>` | JSONL，每行 `{"key": …, "text": …}`。`text` 是原始记录 HTML，也正是 `--source` 的输入契约（见 `profile-contract.md` §2） |
| `<path>.keys` | 抽样清单，一行一个 headword，便于人工复核与二分排查 |

各桶的 `取到/配额/可用/步长` 会打到 stderr，方便直接看出抽样是否覆盖均匀。


| 词典 | 口径 | records | entries | 核心 kinds | 装饰 kinds | clusters_core | clusters_all | 离群词条占比 | 覆盖 80% | verdict | 耗时 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| OALD9 | 旧（全部 kind） | 77903 | 65615 | – | – | 31 | 31 | 2.2% | 3 | medium | 3.6 s |
| OALD9 | 核心 1% | 77903 | 65615 | 7 | 8 | 24 | 31 | 2.2% | 3 | medium | 3.5 s |
| LDOCE6 | 旧（全部 kind） | 52749 | 57142 | – | – | 11632 | 11632 | 67.2% | 2144 | manual | 15.4 s |
| LDOCE6 | 核心 1% | 52749 | 57142 | 42 | 14 | **11186** | 11632 | 66.5% | 1940 | manual | 15.6 s |

## 校准结论：口径修正只在 OALD9 成立，在 LDOCE6 不成立

OALD9：15 个 kind 里只有 7 个达到 1%，装饰块少而稀，模板从 31 收敛到 24 —— 修正有效。

LDOCE6：56 个 kind 里有 **42 个都 ≥ 1%**（`span.def` 98.8%、`span.pos` 84.8%、
`span.proncodes` 70.8%、`span.gram` 62.7%、`span.example` 43.1%、`span.neutral` 41.8%…）。
它们的**有无组合**才是簇的来源：11186 个核心簇里 7182 个是单例，4 个超过 1000 条，
最大的簇只占 6.2%。也就是说，LDOCE6 的长尾**不是装饰噪音**，而是「同一骨架 + 不同可选块」的
正常组合爆炸 —— 集合签名天生会把它数成簇。

阈值扫描（仅诊断，默认值未改）：

| core threshold | 核心 kinds | clusters_core | 覆盖 80% | 离群词条占比 | verdict |
| --- | --- | --- | --- | --- | --- |
| 1% | 42 | 11186 | 1940 | 66.5% | manual |
| 2% | 35 | 10077 | 1559 | 64.7% | manual |
| 5% | 29 | 7841 | 1023 | 62.4% | manual |
| 10% | 22 | 2303 | 248 | 49.7% | manual |
| 20% | 16 | 202 | 29 | 11.9% | manual |
| 50% | 10 | 22 | 3 | 1.2% | medium |

要到 O(10²) 得把门槛抬到 20%，而那时 42 个真实槽位已被砍到 16 个 —— 拿丢信息换档位，
不是修正口径。已知 LDOCE6 用 47 条**按 selector/kind** 写的规则就能适配，说明人工工作量
跟着「种类 + 结构规则」走，不跟「组合」走。下一个口径要解决的是**模板身份**（例如只按必有骨架
聚类、可选块作为簇内属性），而不是阈值高低。本次不调阈值凑结果。

读法：核心 kinds 少说明词条骨架稳定；clusters_core 仍大且单例多，说明集合签名把可选块组合
也数成了模板 —— 先看 `clusters_all - clusters_core` 判断装饰噪音，再看 `coverage` 判断真实长尾。

## 成本模型校准（E7）

真实规则数从 `profiles/ldoce6.profile.json` 的 `rules` 数组数出来，单测
`the_estimate_stays_within_a_quarter_of_the_ldoce6_rule_count` 直接读该文件，不写死 47。

| 词典 | 实测槽位种类 | estimated_rules | 真实规则数 | 误差 | 判定 |
| --- | --- | --- | --- | --- | --- |
| LDOCE6 | 56（42 核心 + 14 装饰） | 56 | 47（profile JSON） | +19.1% | 通过（≤25%） |
| OALD9 | 15（7 核心 + 8 装饰） | 15 | 尚无 profile | – | 规则数已落在 easy 上界内 |

误差 = `|estimate - real| / real`。上限 25% 是硬判据：**超了说明模型又错了，要停手报模型，
不许调系数**。

边界说明：本机没有 LDOCE6 / OALD9 的原始 `.mdx`，上表「实测槽位种类」引用 2026-11-07
全量实测（见上一节）；`estimated_ops` 需要原始语料才能数怪癖，本机未复测，因此 verdict
只写规则数能支撑的部分，不猜算子数。
