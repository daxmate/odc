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

词条容器识别：`--entry-selector` > class 里含 `entry` token 的元素（取最外层，
一条记录可以有多个）> 退化启发式（同签名重复出现、含标题、文本最长的容器）。
识别不到任何容器 → 退出码 2。

难度档位（核心口径，写死，不可调）：

- **easy**：core kinds ≤ 12 且 clusters_core ≤ 20 且离群词条占比 ≤ 5%
- **medium**：core kinds ≤ 25 且 clusters_core ≤ 60
- **manual**：其它

阈值来自「一本能手工写完 profile 的词典该有多少骨架」这一判断，**不随词典调**；
调阈值凑档位等于把尺子掰弯。

## 输出

| 文件 | 内容 |
| --- | --- |
| `survey.md` | 人读：核心 kinds 表、装饰块表、clusters_core 全表、最大 200 簇的 layout + 样本 key、结构统计 |
| `survey.json` | 机器读：`kinds.core / kinds.decor / clusters_core / clusters_all / coreless / coverage / decor / stats / verdict` |
| `samples/<cluster-id>/<safe-key>.html` | 每簇最多 5 条原始 HTML（单条上限 256 KB，全局预算 64 MB） |

`coverage` 给出覆盖 50% / 80% / 95% **已聚类词条**所需的簇数；分母不含 coreless。

退出码：`0` 产出成功（**不代表词典好适配**）；`2` 识别不到词条容器；`1` 输入/参数错误。

## 实测（2026-11-07，release 构建，全量）

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
