# 方言盘点（`odc survey`）

一句话：**不写任何规则**，直接从 MDX 的 HTML 量出「槽位种类 / 词条模板簇 / 离群占比」，
给「这本词典要多少人工」一个可复现的档位。它是写 profile 之前的「量词」，不是适配器。

```bash
odc survey book.mdx --limit 2000 --out ./odc-survey
odc survey book.mdx --limit 0            # 全量，注意耗时与内存
odc survey book.mdx --entry-selector '.entry'
odc survey book.mdx --json               # 报告打到 stdout
```

## 怎么算（没有词典特判）

| 概念 | 定义 |
| --- | --- |
| 槽位种类 kind | 词条容器内一个元素的 `(tag, 首个 class token)` |
| 折叠 | 同一父元素下连续的同类兄弟合并为一次，并记录重复次数 |
| 模板 template | 词条**直接子级 + 一层嵌套**上出现的 kind 的**集合**（不是序列） |
| 簇 cluster | 共享同一模板的词条集合 |
| 离群 outlier | 占比 < 0.5% 的簇 |
| 别名 | `@@@LINK=` 开头的记录只计入 alias，不参与模板统计 |

词条容器识别：`--entry-selector` > class 里含 `entry` token 的元素（取最外层，
一条记录可以有多个）> 退化启发式（同签名重复出现、含标题、文本最长的容器）。
识别不到任何容器 → 退出码 2。

难度档位（写死，不可调）：

- **easy**：kinds ≤ 12 且 clusters ≤ 20 且离群词条占比 ≤ 5%
- **medium**：kinds ≤ 25 且 clusters ≤ 60
- **manual**：其它

## 输出

| 文件 | 内容 |
| --- | --- |
| `survey.md` | 人读：kinds 表、clusters 全表、最大 200 簇的 layout + 3 个样本 key、结构统计 |
| `survey.json` | 机器读：kinds / clusters / stats / verdict 全量 |
| `samples/<cluster-id>/<safe-key>.html` | 每簇最多 5 条原始 HTML（单条上限 256 KB，全局预算 64 MB） |

退出码：`0` 产出成功（**不代表词典好适配**）；`2` 识别不到词条容器；`1` 输入/参数错误。

## 实测（2026-11-07，release 构建）

| 词典 | 范围 | records | entries | kinds | clusters | 离群词条占比 | 覆盖 80% 所需簇 | verdict | 耗时 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| OALD9 | limit 2000 | 2000 | 1715 | 7 | 17 | 1.5% | 3 | easy | 0.13 s |
| LDOCE6 | limit 2000 | 2000 | 2102 | 51 | 872 | 64.0% | 452 | manual | 0.50 s |
| OALD9 | 全量 | 77903 | 65615 | 15 | 31 | 2.2% | 3 | medium | 3.6 s |
| LDOCE6 | 全量 | 52749 | 57142 | 56 | 11632 | 67.2% | 2144 | manual | 15.2 s |

耗时是进程内 elapsed；全量 LDOCE6 峰值内存 166 MB（采样预算 64 MB）。全量 OALD9 比 2000 条样本多出一倍的
kind（7 → 15）和近一倍的簇（17 → 31），说明**抽样会低估长尾**：估算工作量要用全量。

读法：kinds 少说明词条骨架稳定；clusters 与离群占比同时膨胀，说明可选块（例句、语法框、
用法框……）在模板里制造长尾，这些才是人工工作量所在。「覆盖 80% 所需簇」是最接近
「要写多少条规则」的数字。
