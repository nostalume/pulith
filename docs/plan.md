# pulith 开发计划与 Stage 报告

状态：**PS5 已冻结并执行中**。用户 2026-11-10 明确「可以开始开发 stage-5」，据此冻结 PS5 范围
与首个任务 `s5-1`；`s5-1` 已执行并回填 §10.3，随后按同一授权执行 `s5-2` 并回填 §10.3.1。
本文位于 **pulith 仓库内部**，是 pulith 唯一的
开发计划文档；tuckin 只在 `README.md` / `docs/architecture.md` 引用本文，不复制正文。

证据 revision `f989578`（测量开始时工作区干净，`0.1.1`，Rust 2024，MSRV 1.88，开发 toolchain
1.99.0），日期 2026-11-10。本地 `.agents/plan/` 是私有执行层，被 `.gitignore:4:.agents/` 忽略，
不是交付物；其中 `stage-5-*.md` 与 `s5-*-task.md` 自带步骤、门与回滚。

## 0. 权威、边界与编号命名空间

| 项 | 内容 |
| --- | --- |
| 权威 | `docs/AGENTS.md`、`docs/architecture.md`、`CONTRIBUTING.md`、`.github/workflows/ci.yml`、`deny.toml`、本文 |
| 用户授权范围 | **PS5 的开发**。不授权发布、不授权上传、不授权外部服务 |
| 依赖形态 | tuckin 以 `path` 依赖引用本地 pulith 工作树；**不发布 0.2.0**，边确定边开发 |
| 发布时机 | 用户决定（UD-6）：「直到我们认为完善再上传」。当前不发布、不打 tag、不做 crates.io 操作 |
| 目标 | 提高性能、易用性、人体工程；P1–P6 特性开发按共开发需要逐项落地 |
| 非目标 | 为 tuckin 打补丁；交互执行 / stdio 继承；registry / factory / manager 对象 |
| 本文排除 | 具体 API 设计（类型签名、trait 形状、模块划分、错误类型字段）。历史 API 草案是内部记录，非权威 |

### 0.1 两套 stage 不共用编号

用户指出此前把 pulith 的 stage 与 tuckin 的 stage 混在一起。现固定命名空间：

| 前缀 | 属于 | 指什么 |
| --- | --- | --- |
| `TS0`–`TS4` | **tuckin** | tuckin 的需求阶段机（概念 → 规格 → 技术栈 → 切片 → 内容层）。见 `tuckin/docs/architecture.md` |
| `PS1`–`PS7` | **pulith** | 本文的 pulith 开发 stage。PS1–PS3 已落地，PS4 验收未收口，PS5 已收口（§10.3.9） |
| `s<n>-<m>` | **pulith** | PSn 下的任务编号，沿用既有文件名 |
| `VC1`–`VC9` / `K1`–`K4` | tuckin | 形式化核对 / CLI 参数规则，与 stage 无关 |
| `P1`–`P6` | pulith | 设计评审条目，**不是** stage |
| `UD-1`–`UD-6` | 用户 | 具名的用户决定，见 §8；旧 `V1`–`V5` 已废弃 |

**为什么从 PS4 出现、而不是从 PS1 开始写计划**：PS1–PS3 是既成历史，任务文件均为 `LANDED`
（见 `.agents/plan/README.md` 的 `s3-*` / `priority-*` 记录），不需要重新计划。PS4 是唯一未收口的
既有阶段，所以本文把它登记为外部前置；**新增的工作从 PS5 开始**，这也是用户本次授权的对象。
两套编号不换算、不对齐、不互相引用顺序。

## 1. 决策锥

| 前提 | 分类 | 依据 / falsifier |
| --- | --- | --- |
| `f989578`、干净、`0.1.1` | observed | `git rev-parse` / `git status --porcelain` |
| Windows 去根包数 `local,sha2` 18 / `archive` 62 / default 62 / all 172 | observed | §10.3 W1；旧基线只给了前两项 |
| Linux 静态图去根 19 / 63 / 63 / 176 | observed（静态解析） | `cargo tree --target x86_64-unknown-linux-gnu`；本机无 Linux runner |
| warm check 中位数 0.785 / 1.073 / 1.754 s | observed | 三次 `time cargo check`；单次本机观测，不是 benchmark |
| `cargo deny --offline check advisories` = 17 error，全部 crate 在 `local,sha2` 图零命中 | observed | §10.3 归属表 + 逐条 `cargo tree -i` 等价检查 |
| 10 ms 轮询在 `process.rs:328,843,1361,1374`；等价探针 200 ms → 22–24 次、2 s → 190–192 次 | observed（模型层） | **不是 pulith 内部插桩**；`T/10ms` 精确系数被证伪，见 §10.6 D3 |
| 114 处 `LocalTarget` 构造、23 处 `.inspect(())` | observed | `grep -rn`，含测试与 examples |
| `LocalTarget` 仅持 `PathBuf`，无句柄 / custody / 代际标记 | observed | `src/local.rs:483`；s5-2 的 U1 前置已通过 |
| 16 KiB 摘要缓冲、512 B sniff、`take(limit+1)`、8 KiB capture cap 等有界 I/O 已正确 | observed | §10.3 W3；G2 规定不改 |
| `LocalTarget` 只持 `PathBuf`，`Clone` 不复制 authority | observed（原 unknown，`s5-2` 已闭合） | `src/local.rs` 复核 + `s5-2` 两个契约测试，见 §10.3.1 |
| P1–P6 的公开形状 | **user policy / unknown** | 边确定边开发；不再有 0.2.0 兼容性边界这个冻结门 |
| 「更快」的具体预算 | **partially known** | W1 中位数已产出；W2 唤醒预算仍需内部插桩，见 §10.6 D3 |

成熟度：PS5 的基线与 law 修正为 `Settled`（概念层面）；等待循环为 `Candidate`；默认 feature 为
`Blocked`（UD-2）；PS6 公开类型重构为 `Exploratory`，按共开发需要逐项推进。

## 2. 三条轴的具名定义

不带具名工作负载的「性能优化」会写出互不相关的改动，所以先固定指称：

| 轴 | 定义 | 具名工作负载 / 使用者 |
| --- | --- | --- |
| 性能 | 构建成本、运行时唤醒次数、内存上界 | W1 冷/热 `cargo check` 的 feature 组合；W2 子进程运行 T 秒；W3 单次摘要 / 归档探测 |
| 易用性 | 不看源码能否正确使用；错误信息是否误导 | `LocalTarget` 调用者、rustdoc 读者、只写 `pulith = { path = ... }` 的新用户 |
| 人体工程 | 类型形状是否让正确用法成为唯一自然用法；效应与不可用性是否显式 | 实现 P1/P2/P3 的维护者、写「plan 零写入」的下游调用者 |

冲突顺序：**正确性 / law 一致性 → 人体工程 → 易用性 → 性能**。

## 3. 统一 Stage 序列与拓扑

| Stage | 名称 | 性质 | 状态 |
| --- | --- | --- | --- |
| PS1–PS3 | 历史垂直切片与行为目录 | 已落地 | `LANDED`，不重新计划 |
| PS4 | 既有全局设计收口 | 外部前置 | `ACCEPTANCE OPEN`，非本计划产出 |
| PS5 | 质量基线：性能 / 易用性 / 人体工程 | 加法，不改公开类型形状 | `LANDED`（stage 级，§10.3.9）；`s5-1`–`s5-6` 6/6 任务级完成；G6 双平台实测关闭（§10.3.3） |
| PS6 | 特性开发：P1–P6 能力落地 | 按共开发需要渐进 | `IN PROGRESS`：P1（`Occupancy` / `Admission`）与 `s6-1`（单文件发布入口）已落地，见 §10.4.1 / §10.4.2；P2–P6 未生成 |
| PS7 | 下游集成验证（tuckin v0.1 文件相） | 消费者验收 | `NOT PLANNED`，仅登记依赖 |

```text
PS4 (ACCEPTANCE OPEN, pulith 内部记录) ── 不再是发布门（不发布）

PS5  [FROZEN / IN PROGRESS；G6 双平台已实测验收]
  s5-1 baseline .................. LANDED (task-level)
    ├→ s5-2 law + local ergonomics ......... LANDED (task-level, 双平台)
    │    └→ s5-3 bounded wait ............. LANDED (有界退避；D3 系数证伪后不再等插桩)
    └→ s5-4 dependency gate ............... LANDED (公告 17→0；zip 9 迁移见 §10.3.5)
         └→ s5-5 default feature .......... LANDED (UD-2 解除)
  s5-6 test isolation ..................... LANDED (双平台；默认直连修复见 D12)

PS6  P1–P6 按 tuckin 共开发需要逐项落地，不再等 0.2.0
PS7  tuckin TS3 文件相（属于 tuckin 阶段机，不是 pulith 的执行范围）
```

边与无环性：`s5-1` 是 PS5 唯一公共前置；`s5-2` 与 `s5-4` 相互独立；`s5-3` 依赖 `s5-2` 的 law，
`s5-6` 依赖 `s5-2` 的测试口径；`s5-5` 原依赖 `s5-4` 加 UD-2，用户 2026-11-10 解除 UD-2 后
随 feature 修订一并完成。PS5 与 PS6 无实现依赖：一个加法，一个可能改公开形状，不混进同一
交付单元。无环。

## 4. 任务清单与证据映射

### 4.1 PS5（私有执行层已有任务文件）

| 任务 | 一句话可复核结果 | 前置 | 状态 |
| --- | --- | --- | --- |
| `s5-1` 质量基线 | W1–W3 有可复现数值，每个热点确认或撤销；零源码改动 | 无 | `LANDED (task-level)`，见 §10.3 |
| `s5-2` law 与局部人体工程 | 公开表面如实说明效应；同一目标「先观察再使用」不需构造两次 | `s5-1` | `LANDED (task-level, 双平台)`，见 §10.3.1 / §10.3.3 |
| `s5-3` 有界等待效率 | 等待循环改为有界退避，`ProcessEnd` / timeout / capture cap 语义不变 | `s5-1`、`s5-2` | `LANDED (task-level, 双平台)`，见 §10.3.2 |
| `s5-4` 依赖门 | 公告 17→0，策略未放宽；`local,sha2` 图无可达公告 | `s5-1` | `LANDED (task-level)`，见 §10.3.5 |
| `s5-5` 默认 feature | 默认集合与用户选定策略一致 | 原 `s5-4` + UD-2 | `LANDED`：`default = ["local"]`，UD-2 已解除 |
| `s5-6` 测试隔离 | 脏代理环境下完整套件通过，测试不读宿主代理 / 不假设端口空闲 | `s5-2` | `LANDED (task-level, 双平台)`，见 §10.3.2 / §10.3.3 |
| `s5-7` 单后端化 | 删异步镜像与 `reqwest`/`tokio`；同步语义、证据、限额不变 | `s5-2`、§10.3.10 判据 | `LANDED (task-level, Windows 实测)`，见 §10.3.11 |

### 4.2 证据项到任务的映射

| 来源 | 编号 | 内容 | 去向 |
| --- | --- | --- | --- |
| 性能 | F1 | `default = ["archive"]` 使本地语义用户拉进 62 包 | `s5-5`（UD-2） |
| 性能 | F2 | 17 条 advisory 使 `cargo deny` 失败 | `s5-4`，已清零（§10.3.5） |
| 性能 | F3 | 重复版本集中在 all-features 图（71 组），default 图无重复 | `s5-4`，已降至 15 组 |
| 性能 | F4 | 10 ms 轮询；线性关系成立，精确系数被证伪 | `s5-3`，预算待内部插桩 |
| 性能 | F5 | CI 主循环 16 个 feature 逐个 `cargo check` | 验证门，不裁 |
| 性能 | F6 | `hex 0.4.3` 在 `hash` feature；`DigestValue` 是字符串型 | Note，不单列任务 |
| 易用性 | U1 | `LocalTarget` 无 `Clone`，114 处构造 | `s5-2` |
| 易用性 | U2 | `.inspect(())` 23 处，无信息量 | `s5-2` |
| 易用性 | U3 | `Link` rustdoc 提到不存在的 `policy` 参数（`lib.rs:84`） | `s5-2` |
| 易用性 | U4 | `AlreadyPublished` 的 `Display` 把「存在既有条目」说成「pulith 发布过」（`local.rs:249`） | `s5-2` |
| 易用性 | U5 | `ManagedProcess` 名字暗示构造即受管，实际取决于 `stop_within` | `s5-2` |
| 易用性 | U6 | `stage()` 会 `create_dir_all(parent)`，law 里没写（`apply.rs:35`） | `s5-2` |
| 人体工程 | E1–E5 | 维度事实模型、证据分层、no-replace、能力不可用表达、只比字节数 | PS6 |
| 人体工程 | E6 | 进程层 headless / bounded 未写成 law | `s5-2` |
| 设计评审 | P1–P3 / P6 | `Occupancy` + `Admission`；按维度的 `LocalFacts`；`StageContents` / `CommitOutcome` 分离；权限轮廓与能力不可用 | PS6 |
| 设计评审 | P4 / P5 | headless / bounded law 与交互 non-goal；`stage()` 副作用写进 law | `s5-2` |

**明确不改**：16 KiB 摘要缓冲、512 B 归档 sniff、record 的 `take(limit+1)`、8 KiB capture cap
等有界 I/O 已经正确（G2）。

## 5. 全局硬约束

| 编号 | 约束 |
| --- | --- |
| G1 | **先测量后优化**。`s5-1` 之前不改源码；没有具名预算不声称「更快」 |
| G2 | **有界 I/O 不改**。禁止放大缓冲、改 `read_to_end`、放宽 capture cap |
| G3 | PS5 只做加法：`Clone` + 一个便捷入口；行为 trait 仍消费 `self`，不删除或改名公开类型 |
| G4 | 不引入交互执行、stdio 继承、policy 对象、registry、factory、manager |
| G5 | 平台能力不可用必须显式拒绝；Windows 不把只读位当 `OwnerOnly` |
| G6 | Linux 与 Windows 是必需语义对等方，单平台结果不算验收 |
| G7 | MSRV 1.88 是下限；每个任务跑现有 CI feature matrix |
| G8 | advisory remediation 按 feature 作用域处理；约束内无解时停下记录，不放宽版本约束、不抬 MSRV、不加 ignore 条目 |
| G9 | 刻画证据随它所证明的改动一起走，不单独造 benchmark 套件 |
| G10 | PS5 与 PS6 不混在同一交付单元；P1–P3 不进质量计划 |

## 6. 验证门与命令

沿用 `.github/workflows/ci.yml`，不新造门：

```text
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo doc --locked --all-features --no-deps
cargo check --locked --no-default-features
for feature in <CI 主循环 16 项> ; do cargo check --locked --no-default-features --features $feature ; done
cargo test --locked --all-features
cargo test --locked --all-features -- --test-threads=1
cargo +1.88.0 check --locked --all-features
cargo deny check advisories bans sources licenses
cargo tree -d
```

双平台：Windows 与 Linux 各跑一遍。用户 2026-11-11 从宿主程序黑名单移除 `wsl.exe` 后，
Linux 侧通过 `wsl.exe -d tumbleweed` 在 WSL2 内实跑，脚本与结果见 §10.3.3；**G6 已按实测关闭**。
环境代理不再是测试输入：两个 `Default` 显式直连，脏代理场景由 `tests/ambient_proxy.rs` 在测试进程内自持；CI 的 `hostile-env` job 已撤回（§10.3.4），开发者直接 `cargo test` 即可覆盖。本机 WSL 结果仍不等同于 CI runner 结果。
网络只允许 `s5-4` 里那一次有界的 advisory-db 只读更新（不可用时用 `--offline` 并记入范围记录）。

## 7. 本地共开发与发布边界

| 项 | 规定 |
| --- | --- |
| 当前版本 | `0.1.1`；不因本计划改动版本号 |
| 依赖形态 | tuckin 用 `pulith = { path = "../pulith", default-features = false, features = ["local","sha2"] }`，不写版本约束 |
| 能力缺口 | 0.1.1 的 `publish` 只提交到空目标、无权限轮廓；D24 / D27 所需能力在共开发中逐项补齐 |
| 兼容成本 | 本地 path 依赖只有一个消费者 tuckin，公开形状变更可以当场同步；**不再有「集中一次 0.2.0 破坏」的理由** |
| 发布纪律 | 不发布、不上传、不打 tag、不做 crates.io 操作。何时发布由 UD-6 决定 |
| 交付单元 | PS5 完成后是一个可独立回滚的本地改动集（law / 人体工程修正 + 等待循环），不需要版本号 |

## 8. 用户拥有的决定（旧 `V1`–`V5` 的具名替代）

旧稿用 `V1`–`V5` 五个编号，但没有说明每个编号是什么，用户无法据此判断。现改成具名决定，
并给出本次用户指令后的状态：

| 具名决定 | 原来叫 | 内容（平实说明） | 当前状态 |
| --- | --- | --- | --- |
| `UD-1` 范围冻结 | V1 | 计划只有在用户明确「冻结范围与首个任务」后才可执行 | **已解除**：用户 2026-11-10 说「可以开始开发 stage-5」，PS5 与 `s5-1` 已冻结 |
| `UD-2` 默认 feature | V2 | `default` 取哪个最小能力集 | **已解除**：用户 2026-11-10 明确「没有前置兼容负担，直接修改」；已改为 `default = ["local"]`，`s5-5` 任务级 `LANDED` |
| `UD-3` 公开类型设计 | V3 | P1–P6 的公开形状是否冻结、有没有 0.2.0 兼容性边界 | **0.2.0 兼容边界已撤销**（不发布）；公开形状改为边确定边开发，不再是 PS6 的前置门 |
| `UD-4` PS4 状态口径 | V4 | `ACCEPTANCE OPEN` 与 README / 任务文件状态互相矛盾的处置 | 降为 pulith 内部记录一致性问题；不发布后不再阻塞任何发布型工作 |
| `UD-5` 单文件分发 | V5 | 若「单文件分发」成为硬约束，是否重新裁决 tuckin 的外部依赖策略 | 仍归用户；不发布后不迫近 |
| `UD-6` 发布时机 | 新增 | 何时把 pulith 上传 / 发布 | **新增**：用户明确「完善再上传」，当前明确不发布 |

`tuckin/docs/architecture.md` §8.3 有同一张表，两边状态必须一致。

## 9. 停止点

用户 2026-11-10 追加授权「直接修改、更新，没有前置兼容负担」，本轮解除 `UD-2`，并把
`s5-3` / `s5-4` 按「轻量、简洁、减少开发负担」重新定价：`s5-3` 改为有界退避，不再等待
内部插桩；`s5-4` 改为按 feature 作用域归责，清零工作放到发布前；新增 `s5-6` 处理测试
状态泄露。`s5-1` / `s5-2` / `s5-3` / `s5-5` / `s5-6` 任务级 `LANDED`。用户 2026-11-11 移除
`wsl.exe` 黑名单后，Linux 侧在 WSL2 实跑 28 门全绿（含脏代理下的并行与单线程完整套件），
**G6 双平台门按实测关闭**，见 §10.3.3；D5 / D8 的「缺 Linux 执行入口」限制已解除（D13）。
`s5-4` 的公告清零工作未执行，归 `UD-6` 发布前。PS6 在共开发中按需逐项推进。本文不授权 PS4 状态裁决，
也不授权任何发布动作。

---

## 10. Stage 报告（执行后回填）

### 10.1 报告契约

| 项 | 规定 |
| --- | --- |
| 何时写 | 一个 stage 的任一任务完成、出现偏差、或触发 reopen 条件时；stage 收口时写一次总结论 |
| 必须包含 | 实际观测（命令 + 输出要点）、验收门逐条结果、与预期的偏差、结论、reopen 触发与否 |
| 什么不算结论 | 只写「已改」「应该更快」「测试通过」而无命令与输出的；不得复制计划的预期值充当实测值 |
| 偏差如何记 | 写进 §10.6 全局偏差表；偏差一旦出现就占一行，即使已修复也保留 |
| 报告与计划矛盾时 | 以**实际观测**为准，并把矛盾写进偏差表 + §1，不静默改字 |

状态记号：`PLANNING ONLY` / `NOT STARTED` / `IN PROGRESS` / `ACCEPTANCE OPEN` / `BLOCKED` /
`LANDED` / `REOPENED`。`LANDED` 必须有对应的实际观测与验收门结果。

### 10.2 PS4 报告 — 既有全局设计收口（外部前置）

| 字段 | 值 |
| --- | --- |
| 状态 | `ACCEPTANCE OPEN`；**非本计划产出**，本文只记录它作为 pulith 内部事实 |
| 权威 | `.agents/plan/stage-4.md`、`s4-1`..`s4-5`、`.agents/plan/README.md` |

**已知证据（observed，`f989578`）**：`stage-4.md` 为 `ACCEPTANCE OPEN`，S4-5 在 Windows 与 WSL
落地，受限 Windows SCM 验收未完；README 把 `s4-1` 记为 `READY`，而任务文件自述
`IMPLEMENTED, but not complete`；README 把 `s4-3` / `s4-4` 记为 `IN PROGRESS`，`stage-4.md`
记为 `S4-3 LANDED` / `S4-4 SCM ACCEPTANCE OPEN`（两处状态不一致，**保留**）。

**偏差记录**：空。**结论**：未产生执行结论。由于本计划不再发布，PS4 状态不阻塞 PS5 / PS6；
归 UD-4 作为内部记录一致性问题。**Reopen 条件**：出现新的 PS4 权威记录，或用户要求统一口径。

### 10.3 PS5 · `s5-1` 报告 — 质量基线（已执行）

| 字段 | 值 |
| --- | --- |
| 执行时间 / revision | 2026-11-10，`f989578`，测量开始时 `git status --porcelain` 为空 |
| 权威 | `.agents/plan/s5-1-quality-baseline-task.md`；本文 §1、§5 |
| 前置 | UD-1（本次用户指令已解除） |
| 状态 | `LANDED (task-level)`；PS5 整体为 `IN PROGRESS` |

**W1 — 依赖图**（`cargo +1.99.0 tree --prefix none --target <triple> -e normal --format "{p}"`，
去重后计数；`--format {p}` **包含**根包 `pulith v0.1.1`，下表为去根值）：

| feature 组合 | Windows 去根 | Linux 去根（静态图） |
| --- | --- | --- |
| `local,sha2` | 18 | 19 |
| `archive` | 62 | 63 |
| default | 62 | 63 |
| `--all-features` | 172 | 176 |

**W1 — warm 编译**（`time cargo +1.99.0 check --offline --locked`，三次，单位秒）：

| feature 组合 | 三次观测 | 中位数 | 旧基线（单次） |
| --- | --- | --- | --- |
| `local,sha2` | 0.765 / 0.857 / 0.785 | **0.785** | 0.737 |
| default | 0.981 / 1.087 / 1.073 | **1.073** | 0.908 |
| `--all-features` | 1.854 / 1.754 / 1.488 | **1.754** | 4.133 |

全部为 warm、单机、单次会话，不是 benchmark。all-features 与旧基线的差异记为偏差 D2，
**不声称性能提升**。

**W2 — 等待循环**：静态确认 10 ms 轮询在 `src/process.rs:328`（sync `stop_within`）、`:843`
（async `CancelToken::cancelled`）、`:1361`（`wait`）、`:1374`（`wait_cancellable`）；`:1789` 是
timeout 定时，不是轮询。仓库外等价轮询探针（不改 pulith、不插桩）三次运行：

| T | polls（三次） | elapsed（三次，ms） | `T/10ms` 预测 |
| --- | --- | --- | --- |
| 200 ms | 22 / 24 / 23 | 232 / 238 / 250 | 20 |
| 2000 ms | 192 / 191 / 190 | 2041 / 2059 / 2040 | 200 |

结论：「唤醒次数与 T 成正比」成立；**`T/10ms` 作为精确值被证伪**（200 ms 偏多、2 s 偏少，
`thread::sleep(10ms)` 在本机实际 >10 ms 且 `try_wait` 有开销）。探针是等价模型，
**不是 pulith 内部唤醒计数**，见偏差 D3。F4 改为「线性成立、系数待内部插桩」，`s5-3` 的
唤醒预算不得直接使用本表数值。

**W3 — 有界 I/O**（源码复核，全部未改动）：`hash.rs:535` 16 KiB 缓冲；`archive.rs:454`
512 B sniff；`local/record/native.rs:56` `take(limit+1)`、`:102` `take(limit)`；
`process.rs:1831` `Vec::with_capacity(cap.min(8192))`、`:1833` 8 KiB chunk、`:1849` async
capture。G2 的「不改」前提成立。

**供应链状态**：`cargo deny --offline check advisories` = 17 error，逐条归属：

| ID | crate / 版本 | ID | crate / 版本 |
| --- | --- | --- | --- |
| 0385 | `ctutils 0.4.2` | 0346/0347/0348/0349 | `rustls 0.23.45` |
| 0382 | `rustls-webpki 0.103.15` | 0386 | `tokio 1.53.1` |
| 0381 | `webpki-roots 1.0.8` | 0383 | `webpki-root-certs 1.0.8` |
| 0367/0368/0369/0429 | `zip 8.6.0` | 0391 | `zstd 0.13.3` |
| 0398/0403 | `zstd-safe 7.2.4` | 0410 | `zstd-sys 2.0.16+zstd.1.5.7` |

以上 10 个 crate 在 `--no-default-features --features local,sha2` 的依赖图中**逐条零命中**，
「集中在 archive / net / http 分支、local 不可达」成立。

**重复版本**：`cargo tree -d`（default features）输出为空；`--all-features` 下同名多版本 71 组
（`base64`、`digest`、`getrandom`、`hashbrown`、`sha2`、`syn`、`windows-sys` 等）。即重复是非默认
feature 图的属性，不是默认构建的属性，见偏差 D4。

**调用摩擦**：`LocalTarget` 构造 114 处、`.inspect(())` 23 处，与基线一致（含测试与 examples）。

**verdict 表**：

| 编号 | 判定 | 说明 |
| --- | --- | --- |
| F1 | confirmed | 18 vs 62 包差异可复现；处置仍归 UD-2 |
| F2 | confirmed | 17 条公告可复现，且 local 分支不可达 |
| F3 | **revised** | 重复只在 all-features 图出现；default 图无重复 |
| F4 | **partially confirmed** | 线性成立，`T/10ms` 系数证伪；系数待内部插桩 |
| F5 | confirmed（不裁） | CI 16 feature 主循环是验证门 |
| F6 | confirmed（Note） | `hex 0.4.3` 与字符串型 `DigestValue`，量级很小 |
| U1–U6 | confirmed | U1 的 `Clone` 前置已过：`LocalTarget(PathBuf)` 无 custody |
| E1–E5 | deferred | 属 PS6 设计评审，不是本任务可关闭项 |
| E6 | confirmed | headless / bounded 未写成 law，归 `s5-2` |

**验收门逐条结果**：

| 门 | 结果 |
| --- | --- |
| No tracked file changes；`git status --porcelain` 为空 | **通过**：测量阶段结束时输出为空 |
| 无 `Cargo.lock` 改写；命令用 `--locked` | **通过**：所有 check 带 `--locked`，未执行 `cargo update` |
| 无新工具安装；不做 advisory-db 在线更新 | **通过**：只用了已有 cargo / deny / rustc；`--offline` |
| 测量记录写明 revision、target triple、toolchain、warm/cold | **通过**：见本节表头与 W1 |
| 每个热点确认或撤销 | **通过**：F1–F6 / U1–U6 / E1–E6 逐条判定，F3 / F4 有修正 |

注：本轮**同时**进行的文档重组（新增本文、修改 `.agents/plan/` 私有文件、tuckin 侧文档）
不属于 `s5-1` 的执行效果；`s5-1` 的零改动结论以测量阶段结束时的 `git status` 为准。

**偏差记录**：D1–D5 见 §10.6。**结论**：`s5-1` 任务级 `LANDED`；PS5 未收口。
**Reopen 条件**：revision 变化、具名工作负载变化、内部插桩给出新唤醒计数、用户改动范围政策。

### 10.3.1 PS5 · `s5-2` 报告 — law 真实性与本地 API 人体工程（已执行）

| 字段 | 值 |
| --- | --- |
| 执行时间 / revision | 2026-11-10，基线 `f989578`；实现与验证在同一工作树 |
| 权威 | `.agents/plan/s5-2-law-and-local-ergonomics-task.md`；本文 §1、§5、§6 |
| 前置 | `s5-1` `LANDED`；步骤 1 最小证伪门通过 |
| 状态 | `LANDED (task-level, Windows-verified)`；**G6 双平台验收门仍 OPEN**（D5），PS5 整体 `IN PROGRESS` |

**最小证伪门（步骤 1）**：复核 `src/local.rs:485` 与 `src/local/apply.rs`，`LocalTarget` 只是
`PathBuf` 的 newtype，不含句柄、`TempDir` custody 或代际标记；custody 由 `StagedTree` 持有，
不被 `LocalTarget` 携带。因此 `Clone` 不复制 authority，U1 保留在本任务内。

**实际改动**（tracked `git diff --stat`：6 个文件，49 insertions / 7 deletions；另有 1 个新增
测试文件与本文报告层，均为 untracked）：

| 文件 | 改动 |
| --- | --- |
| `src/lib.rs` | `Link::link` rustdoc 删除不存在的 `policy` 断言；改为「按观测到的条目种类分类被占用的 view，只有目录符号链接 view 才切换，切换 law 记在 view adapter」，不再提任何 selector |
| `src/local.rs` | `AlreadyPublished` 的 `Display` 改为「发现既有前驱条目，目标未提交」，与 rustdoc 一致；`LocalTarget` 加 `#[derive(Clone)]`；加无参 `observe(self)` 委托 `Inspect<()>`；`stage()` rustdoc 写入祖先目录副作用 |
| `src/local/apply.rs` | 模块 law 写入「staging 不是零写入操作，`stage` 会先建目标的祖先目录」 |
| `src/process.rs` | `ManagedProcess` rustdoc 写入 headless law（三路 stdio 为 null、进程组 / Job Object）、显式界限（`stop_within` 由调用者给 deadline）、non-goals（交互执行、继承 stdio、隐式等待策略） |
| `README.md` | Safety model 增补 staging 的写入效应与 managed process 的 headless 口径 |
| `tests/local.rs` / `tests/local/observation.rs` | 新增 `observation` 模块与两个契约测试 |

**两个契约测试**（`cargo +1.99.0 test --locked --all-features --test local observation::`）：

| 测试 | 断言 | 结果 |
| --- | --- | --- |
| `cloned_local_target_observes_the_same_facts_without_mutating_the_target` | clone 与原值的观察结果、evidence 相等；目标字节未被改写 | ok |
| `observe_convenience_matches_inspect_with_the_same_evidence` | `observe()` 与 `inspect(())` 的观察与 evidence 完全相等 | ok |

`observe()` 直接调用 `self.inspect(())`，没有第二条观察路径；`Inspect` 仍是 law 载体。命名冲突
检查：`f989578` 下只有 `pub(crate) fn observe_path`（`local.rs:775`）与 `ProcessSession::observe`
（`process.rs:279`，不同类型），`LocalTarget::observe` 无冲突。

**验收门逐条结果**：

| 门 | 结果 |
| --- | --- |
| 无公开类型删除 / 改名 / 改形状；行为 trait 签名不变 | **通过**：`Link` 只改 rustdoc；`Clone` 与 `observe` 均为加法 |
| 无新依赖；`Cargo.lock` 不变 | **通过**：全部命令带 `--locked`，`git status` 无 `Cargo.lock` |
| 无新状态对象、无第二观察权威 | **通过**：`observe` 委托既有 `Inspect<()>` |
| `git diff --stat` 限于指定文件 | **通过**：tracked 改动仅 `README.md`、`src/lib.rs`、`src/local.rs`、`src/local/apply.rs`、`src/process.rs`、`tests/local.rs`；新增 `tests/local/observation.rs` 为测试文件。`docs/plan.md` 是 untracked 报告层，不计入实现 diff |
| `cargo +1.99.0 fmt --all --check` | **通过** |
| `cargo +1.99.0 clippy --locked --all-targets --all-features -- -D warnings` | **通过** |
| `cargo +1.99.0 doc --locked --all-features --no-deps` | **通过**，无 broken intra-doc link |
| `cargo +1.99.0 test --locked --all-features` | **通过（关闭本机代理后）**：131 / 1 / 0 / 9 / 12 / 9 / 43 / 20 / 32 / 9 / 2 全绿；默认环境下首次失败，见 D6 |
| `cargo +1.99.0 test --locked --all-features -- --test-threads=1` | **通过（关闭本机代理后）**：全部测试二进制全绿 |
| `cargo +1.88.0 check --locked --all-features` | **通过**，MSRV 1.88 未变 |
| 16 个 feature 逐个 `--no-default-features --features <f>` | **通过**：16 / 16 |
| grep 复核：`lib.rs` 无 `policy`；`apply.rs` 有祖先目录 law；`process.rs` 有 non-goals | **通过** |
| Windows 与 Linux CI 均通过 | **OPEN**：本机只有 Windows，Linux 实跑受 D5 限制，未声称通过 |

**偏差记录**：D6、D7 见 §10.6。**结论**：`s5-2` 任务级 `LANDED`（Windows 验证），G6 双平台
门未关闭，PS5 未收口。**Reopen 条件**：Linux CI 报红、`observe()` 被证明产生第二观察路径、
或 PS4 收口改动 `local.rs`。
### 10.3.2 PS5 · 2026-11-10 追加修订报告 — feature / cfg / 测试隔离

| 字段 | 值 |
| --- | --- |
| 执行时间 / revision | 2026-11-10，基线 `f989578` + `s5-2` 工作树改动 |
| 权威 | 用户追加指令「直接修改、更新，没有前置兼容负担」；`s5-3` / `s5-4` / `s5-5` / `s5-6` 任务文件 |
| 状态 | `s5-3`、`s5-5`、`s5-6` 任务级 `LANDED`（Windows 验证）；`s5-4` 重定价；G6 Linux 门 OPEN |

**实际改动**（本轮 tracked `git diff --stat`：16 个文件，330 insertions / 109 deletions；其中
`tests/local.rs` 与 `src/lib.rs` 等含 `s5-2` 的既有改动，`docs/plan.md`、
`tests/local/observation.rs` 仍为 untracked）：

| 范畴 | 改动 |
| --- | --- |
| 默认 feature | `default = ["local"]`；`archive` 显式写为 `zip + tar + gzip + xz + zstd`；README 按能力族 / 摘要 / 格式 / 传输分组并写明所有隐含边 |
| serde 泄漏 | 去掉 `url` 的 `serde` feature；`cargo tree -i serde --features net` 实测为空；`Cargo.lock` 只删 1 行 `serde_derive` 依赖边，无版本变化 |
| tokio | 去掉未使用的 `net` feature；`process-tokio` / `http-reqwest` 编译门仍全绿 |
| cfg | 47 处 HTTP `any(...)` 谓词统一为 1 种写法；9 处 `cfg(any(test, ...))` 改为无条件编译（`allow(dead_code)`），测试态与产品态一致；6 处 `cfg(not(windows))` 统一为 `cfg(unix)` |
| platform 口径 | `examples/toolhost/service_host.rs` 的 systemd 模块本就由 `cfg(target_os="linux")` 门控，dev-deps 作用域一致；macOS job 保持 best-effort，未改 |
| s5-3 | `src/process.rs` 的 `wait` / `wait_cancellable` 改为有界退避 10→20→40→50 ms；`stop_within` 与 `CancelToken::cancelled` 保持 10 ms（短停止与取消响应需要低延迟） |
| 测试隔离 | 新增 `UreqResources::without_proxy()` / `ReqwestResources::without_proxy()`；4 处 `bind→drop` 改为测试自持的 `closed_endpoint()`；`HttpFixture` 加读超时与容错读；6 处固定 300/600 ms sleep 改为 `assert_marker_stable`；41 个真实进程测试加静态串行锁 |
| CI | 新增 5 个对角 feature 组合；新增 `hostile-env` job，启动回 502 的本地代理并在脏环境下跑完整套件 |

**未做及原因**：`hash` 与 `local` 不解耦（D9，源码证伪上一轮批评）；`s5-4` 不做在线
advisory-db 刷新与 `cargo update` 清零（重定价为发布前）；`s5-3` 不做内部插桩 / 唤醒计数
（机制上界替代数值基线）；平台 `cfg` 只统一 `src/`，`examples/` 的 `not(windows)` 写法保留。

**验收门**（全部在**默认环境**执行：本机 `HTTP_PROXY=http://127.0.0.1:62558`、无 `no_proxy`；
完整输出在 `pulith-s5-6-gates.txt`，39 个门逐条 `exit=0`）：

| 门 | 结果 |
| --- | --- |
| fmt / clippy `-D warnings` / doc / MSRV 1.88 check / 默认 check / 空 feature check | 6 / 6 通过 |
| 16 个单 feature + 5 个对角组合（`local,sha2`、`zip,gzip`、`http-ureq,http-reqwest`、`process,process-tokio`、`local,blake3,serde`） | 21 / 21 通过 |
| 并行测试 target：lib 131 / archive 1 / hash 12 / lifecycle 9 / local 43 / net 20 / process 32 / process_async 9 | 全绿 |
| 单线程：lib / net / process / process_async / examples 9 | 全绿 |
| `--test process` 并行连续两次 | 32 / 32、32 / 32 全绿（串行锁生效） |
| `--test examples` 单线程 + 300 s 超时保护 | 通过（并行版本见 D11） |

**偏差记录**：D8–D11 见 §10.6。**结论**：`s5-3` / `s5-5` / `s5-6` 任务级 `LANDED`；
`s5-4` 重定价；G6 双平台门仍 OPEN（D5 收窄见 D8），Linux 侧验收脚本已放在
`\\wsl$\tumbleweed\home\nostal\pulith-verify\verify.sh`，需 WSL 侧执行一次或由 CI 关闭。
**Reopen 条件**：Linux 验收报红、`hostile-env` CI job 失败、或 `UD-6` 打开发布前清理门。
（本节上表 CI 行的 `hostile-env` job 已由 §10.3.4 撤回；此处保留当时记录，不改写。）

### 10.3.3 PS5 · 2026-11-11 G6 收口报告 — WSL 实跑与默认直连修复

| 字段 | 值 |
| --- | --- |
| 执行时间 | 2026-11-11 08:42–08:45（WSL2 实跑）；Windows 回归 09:39–09:47 |
| 权威 | 用户 2026-11-11「wsl 黑名单已去除并关闭完全权限，请你处理」；此前「直接修改、更新，没有前置兼容负担」 |
| 状态 | **G6 双平台门按实测关闭**；D5 / D8 的「缺 Linux 执行入口」限制解除；`s5-6` 由设计补完 |

**通道与环境**【实测】：用户从宿主程序黑名单移除 `wsl.exe` 后，
`wsl.exe -d tumbleweed` 可执行。WSL2 内核 `6.18.58.1-microsoft-standard-WSL2`；
`rustup` 装好 `1.99.0` 与 `1.88.0`；`gcc` / `make` / `pkgconf-pkg-config` 已存在
（初次探测报 "missing" 是跨 WSL 传参时 `$c` 被宿主展开成空造成的假阴性，不是缺包）。

**第一次 Linux 验收报红，并定位根因**：在 502 脏代理下，`--lib` 的 HTTP mock 测试大面积
失败 / 挂起；单测复现的错误是 `CONNECT proxy failed: proxy server responded 502/502`。
根因不是平台差异：`unwrap_or_default()` 会落到 `UreqResources::default()` /
`ReqwestResources::default()`，两者原本用 `ureq::Agent::new_with_defaults()` 与
`reqwest::Client::new()`，**隐式读取进程代理环境变量**；`s5-6` 只给 4 个 transport 测试
注入了 `without_proxy()`，没有覆盖默认资源路径。Windows 侧恰好未暴露该前提，所以
§10.3.2 的「脏环境全绿」结论在 Linux 上被证伪。

**修复（效应显式，不是补丁）**：把两个 `Default` 改为显式直连——ureq 用
`Agent::config_builder().proxy(None).build().new_agent()`，reqwest 用
`Client::builder().no_proxy()`；代理必须由调用方通过 `from_agent` / `from_client` 显式注入，
`without_proxy()` 退化为 `Self::default()` 的语义别名。这是公开行为变更，按用户「没有前置
兼容负担」的授权执行，已写入 README safety model。修复后 Windows 侧 `lib 131` / `net 20`
在带代理默认环境下全绿。

**验收门**：

| 平台 | 门 | 结果 |
| --- | --- | --- |
| WSL2 Linux | `verify.sh`：fetch / fmt / clippy `-D warnings` / doc / 完整并行套件 / 单线程套件 / MSRV 1.88 / 16 单 feature / 5 对角组合 | **28 / 28 PASS，0 FAIL**（原始输出 `\\wsl$\tumbleweed\home\nostal\pulith-verify\g6-result.txt`，备份 `g6-result-20261111.txt`） |
| WSL2 Linux | 完整并行套件 11 个 target | lib 127、archive 1、common 0、examples 9、hash 13、lifecycle 9、local 43、net 20、process 32、process_async 9、doc-tests 2，全部 `0 failed` |
| WSL2 Linux | 单线程套件 | 同样 11 个 target 全部 `0 failed` |
| Windows | 修复后回归：fmt / clippy / doc / 8 个测试 target / examples 单线程 + 300 s 超时 / 完整单线程套件 | **13 / 13 `exit=0`**（输出 `pulith-postproxy-win.txt`） |

Linux 验收在 hostile proxy `http://127.0.0.1:62558`（探针实测回 502）下执行，没有
`no_proxy` 豁免；这正是 §10.3.2 设定的「脏环境反转门」。WSL 副本与工作树的
`Cargo.toml` / `Cargo.lock` / `src/process.rs` / `src/net.rs` / `src/lib.rs` /
`tests/common.rs` / `tests/process.rs` / `tests/process_async.rs` / `tests/net/http.rs`
逐个 SHA256 一致。

**验收边界（不夸大）**：① Linux 实跑对象是 WSL 直接路径上的**代码副本**（哈希一致），不是
git 工作树本身；② 当时 CI 的 `hostile-env` job 已配置但**尚未在远端 runner 触发**（未 push），
不能把本机 WSL 结果说成 CI 全绿；该 job 已由 §10.3.4 撤回；③ Windows 侧 `--test examples` 并行仍会卡在子
`cargo build` 的 `cl.exe`（D11），改以单线程 + 超时保护验收，WSL 侧 examples 并行正常通过；
④ `s5-4` 的公告清零仍归 `UD-6` 发布前，未执行。

**偏差记录**：D12、D13 见 §10.6。本节取代 §9、§10.3.2 与 §10.7 中「G6 OPEN」的**当前状态**；
这些历史段落的当时观测不改写。**Reopen 条件**：`tests/ambient_proxy.rs` 回归门报红，或
`UD-6` 打开发布前清理门后发现 `local,sha2` 图新增可达公告。

### 10.3.4 PS5 · 2026-11-11 修订报告 — `hostile-env` 撤回

| 字段 | 值 |
| --- | --- |
| 执行时间 | 2026-11-11 |
| 权威 | 用户当次指令：「hostile-env 的设计太糟糕了，我需要轻量简洁内聚的设计……开发者就应该直接运行测试就够了」 |
| 状态 | `LANDED`；§10.3.2 的 CI 行、§10.3.3 的 ② 与 Reopen 条件按本节取代 |

**问题**：`hostile-env` job 用 `python3` socket 在固定端口 `62558` 起一个回 502 的代理，
`sleep 1` 等待就绪，导出 8 个代理变量、清 `no_proxy`，再重复跑一遍完整套件。它引入
CI 专属运行时、固定端口与启动竞态、进程级全局环境、以及开发者本地无法复现的第二条执行路径。
D12 修复后两个 `Default` 已显式直连，环境代理根本不是产品代码的输入；这个 job 测的是
「一个不存在的输入」，且只在远端运行。

**改动**（不新增依赖）：撤回上一轮加入的 `hostile-env` job（31 行），`ci.yml` 相对 HEAD
只剩对角 feature 组合的 3 行新增；新增 `tests/ambient_proxy.rs`，
作为独立测试二进制承载 1 个测试——进程级环境变量必须与其他测试进程隔离。测试用进程内
`TcpListener` 同时起 502 代理与 200 源站，自己安装并恢复 8 个变量，断言默认资源仍返回
200、显式注入同一代理时请求不再到达源站。无固定端口、无外部服务、无启动延时；
`cargo test` 自动运行该 target。

**反向验证（防恒绿）**：临时把两个 `Default` 改回隐式读环境代理，该测试报红并复现 D12 原文
`CONNECT proxy failed: proxy server responded 502/502`；恢复后连跑 3 次全绿。

**验收门**：`--all-features` / `http-ureq` / `http-reqwest` / `net` 四种 feature 组合下该
target 通过；`cargo fmt --all --check` 与
`cargo clippy --locked --all-targets --all-features -- -D warnings` 通过；Windows
`cargo test --locked --all-features -- --test-threads=1` 完整套件 `exit=0`（耗时 2m44s）；
RUSTDOCFLAGS `-D warnings -D missing-docs` 的 `cargo doc` 与 MSRV 1.88
`cargo check --locked --all-features --tests` 均 `exit=0`。
`tests/ambient_proxy.rs` 由 Cargo 自动发现，与其他 `tests/*.rs` 一样进入普通套件。
历史报告 §10.3.2 / §10.3.3 的当时观测不改写，偏差新增 D14。

---

### 10.3.5 PS5 · 2026-11-11 s5-4 执行报告 — 公告清零与 zip 9 迁移

| 字段 | 值 |
| --- | --- |
| 执行时间 | 2026-11-11 11:08–11:46（Windows 验证 + WSL2 Linux 复验） |
| 权威 | 用户「继续 s5-4」→「同意，你可以升级 zip 或其他依赖主版本」→「使用 zip 原生、严格控制特征；涉及大改先评估报告」 |
| 状态 | `LANDED`（任务级，双平台实测）；未提交、未推送 |

**触发**：`s5-4` 在 §10.3.2 被重定价，其 Reopen 条件之一是「用户显式要求执行清理门」。本次即该条件。

**策略边界（未破）**：`deny.toml` 未改，`advisories.ignore` 仍为 `[]`；未抬 MSRV；未用
`patch`/fork 覆盖上游；除 `zip` 8→9（用户批准的主版本迁移）外，所有升级都在原声明约束内。

**升级（均取最小修复版本，`cargo update --precise`）**：

| crate | 前 | 后 | 公告 |
| --- | --- | --- | --- |
| `ctutils` | 0.4.2 | 0.4.4 | 0385 |
| `rustls` | 0.23.45 | 0.23.46 | 0346 / 0347 / 0348 / 0349 |
| `rustls-webpki` | 0.103.15 | 0.103.16 | 0382 |
| `webpki-roots` | 1.0.8 | 1.0.10 | 0381 |
| `webpki-root-certs` | 1.0.8 | 1.0.10 | 0383 |
| `tokio` | 1.53.1 | 1.53.3 | 0386 |
| `zstd` | 0.13.3 | 0.13.4 | 0391；连带 `zstd-safe` 7.2.4→7.4.0（0398/0403）、`zstd-sys` 2.0.16→2.1.2（0410） |
| `zip` | 8.6.0 | 9.5.1 | 0367 / 0368 / 0369 / 0429 |

**zip 9 迁移（唯一主版本）**：

1. **特征收敛**：`zip = { version = "9", default-features = false, features = ["deflate-flate2"] }`。
   默认集合里的 `aes-crypto`、`bzip2`、`deflate64`、`lzma`、`ppmd`、`time`、`zstd`、`xz`
   全部关闭，只留 `stored`（内置）与 `deflate`。相对默认，all-features 图少 11 个包
   （`zopfli`、`time`、`zstd 0.14.3`、`zstd-safe 8.2.0`、`bzip2`、`lzma-rust2`、`ppmd-rust`、
   `aes`、`pbkdf2`、`sha1 0.11`、`sha2 0.11`）。代价是一段明确的口径：zip 内用 bzip2 / lzma /
   ppmd / AES / zstd 压缩的条目会以「不支持的压缩方法」失败，`stored` 与 `deflate` 不受影响。
   README Formats 行已写明。
2. **不引入第二个 zlib 后端**：`zip = ["local", "dep:zip", "dep:flate2"]`，让 `zip` 单选时复用
   pulith 已有的 `flate2`（`rust_backend`）。不加这一条时 `--no-default-features --features zip`
   会报 `You need to choose a zlib backend`（实测）。
3. **API 适配**：`ZipFile::name()` 在 9 返回 `Result<Cow<'_, str>, ZipError>`；先取 owned 名，
   再 `enclosed_name()`，错误信息里保留条目名。
4. **错误分类修正**：解码期 `io::ErrorKind::InvalidData`（归档损坏、实际超出声明尺寸）从
   `Io` 改判为 `InvalidPreparation`，并带条目路径。理由：缺陷在归档，不在宿主 I/O；调用方
   不应把它当作可重试的文件错误。
5. **原生校验**：启用 `ZipArchive::validate_header_consistency()`（zip 9 提供），拒绝重名条目、
   局部头与中央目录不一致、重叠条目、中央目录缺失的局部头；源码核对
   `is_pending_data_descriptor` 后确认走 data descriptor 的流式 zip 仍被接受。新增
   `zip_prepare_rejects_local_header_mismatch`：只改局部头的未压缩尺寸、中央目录保持 8 字节，
   旧循环会读到真实 8 字节并接受，现在被原生校验拦下。
6. **行为变化**：zip 9 在读取时自行拒绝「实际大于声明」的条目，因此
   `zip_prepare_enforces_observed_byte_limit` 更名 `zip_prepare_rejects_entry_larger_than_declared_size`，
   断言 `InvalidPreparation` + 提取工作区无残留。「失败不留残留」这一契约未变，变的是错误归属。

**原生 vs 自建的评估（只评估，未执行替换）**：

| 能力 | zip 原生 `extract_with_options` | pulith 现实现 | 能否替换 |
| --- | --- | --- | --- |
| 路径收敛 | `enclosed_name` | 同一原语 | 已用原生 |
| symlink | `SymlinkExtractAction::Omit` 静默跳过 | 整档拒绝（`UnsupportedArchiveEntry`） | 否：静默跳会改变产物内容 |
| 限额 | 单个 `size_limit` | `max_entries` / `max_entry_bytes` / `max_total_bytes` / `max_decoded_bytes` | 否：表达不了 |
| `strip_components` | 只有 `root_dir_filter` | 丢弃前 N 段并按剩余段计数 | 否：语义不同 |
| 证据 | 返回 `()` | 返回 `ArchiveEvidence`（公开类型） | 否：会丢公开契约 |
| 失败残留 | 直写目标目录 | 私有工作区 + 失败重置 | 否：原生无此保证 |

结论：把 `extract_zip` 换成 `archive.extract_with_options()` 等于重写公开契约，属大改，按用户
指令只评估不执行。已采纳的原生部分：`validate_header_consistency`、`enclosed_name`、
`by_index`、`unix_mode`、`size`。自己保留的部分只剩原生无法表达的那几层。

**结果**：

- `cargo deny --offline check advisories`：17 error → `advisories ok`；同轮 `bans` / `licenses` /
  `sources` 均 `ok`；`deny.toml` 的 `ignore` 仍为空。
- 重复版本：all-features 71 → 15 组；default 图仍是 0 组。剩余 15 组为生态分裂
  （`syn 2/3`、`windows-sys 0.52/0.61`、`base64 0.22/0.23`、`getrandom 0.2/0.4`、
  `hashbrown 0.16/0.17`、`constant_time_eq 0.4/0.6`、`cpufeatures 0.2/0.3`），统一需
  `patch` 覆盖上游，不做。
- `local,sha2` 图对 10 个受影响 crate 逐条 `cargo tree -i` 仍为 0 命中。
- 测试增量：新增 1（`zip_prepare_rejects_local_header_mismatch`），更名 1，无删除。

**验收门**：

| 平台 | 门 | 结果 |
| --- | --- | --- |
| Windows | `cargo fmt --all --check` / `clippy --all-targets --all-features -D warnings` / strict `cargo doc` / MSRV 1.88 `check --all-features --tests` | 全部 `exit=0` |
| Windows | `cargo test --locked --all-features -- --test-threads=1` | `exit=0`，12 个 target 全 `0 failed`（lib 132 / examples 9 / local 43 / net 20 / process 32 / process_async 9 / 其余 5 个），4m33s |
| Windows | 归档 feature 单点：`zip`、`tar`、`gzip`、`xz`、`zstd`、`archive` 各自 `cargo check` | 6 / 6 无 error |
| WSL2 Linux | 同一工作树的哈希一致副本（`Cargo.toml` / `Cargo.lock` SHA256 与宿主一致）完整单线程套件 | `exit=0`，12 个 target 全 `0 failed`（lib 128，差值为 4 个 Windows 专属测试） |
| WSL2 Linux | MSRV 1.88 `check --locked --all-features --tests` | `exit=0` |

**验收边界**：① 未提交、未推送、未发布，CI 未触发；② Linux 实跑对象是复制到
`~/pulith-s54` 的哈希一致副本，不是 git 工作树；③ 归档口径收窄（仅 `stored` + `deflate`）
是公开行为变化，按用户「没有前置兼容负担 + 严控特征」授权执行。

**偏差记录**：D15 见 §10.6。**Reopen 条件**：`UD-6` 发布前 advisory-db 刷新出现新公告、
`local,sha2` 图新增可达公告、或需要在 zip 内支持 `deflate` 以外的压缩方法。

---

### 10.3.6 PS5 · 2026-11-11 archive 设计评估（`ASSESSED / NOT EXECUTED`）

| 字段 | 值 |
| --- | --- |
| 触发 | 用户：「archive 的设计是否值得保留，同依赖的关系如何组织；只关闭 feature 不会让它更通用」 |
| 结论 | 保留**策略层**；正交化**格式层**；暂不做扩展 trait。列为待裁决，未改代码 |
| 权威 | 本次只读复核 `src/archive.rs`（2520 行，非测试 1056 行）、`src/local/materialize.rs`、`Cargo.toml` feature 图 |

**结构事实（复核所得）**：

1. `ArchiveKind` 是 6 变体的封闭枚举（`Plain` / `Zip` / `Tar` / `TarGz` / `TarXz` / `TarZstd`），
   在 `src`、`tests`、`examples` 共 91 处引用；`Plain` 在 `prepare` 里被显式拒绝，只用于证据标记。
2. `sniff` 的每个分支都是「`cfg` 命中就返回变体、否则返回 `unsupported_format`」，5 组重复；
   `unsupported_format` 这个函数存在的唯一理由就是消除该重复。
3. 四个 tar 家族函数（`extract_tar_plain` / `_gzip` / `_xz` / `_zstd`）各约 14 行，差异只有
   「打开消息 + 解码器构造 + 格式标签」；真正的内核 `extract_tar_reader<R: Read>` 已经与编码无关。
4. feature 图把实现假设写成了依赖事实：`gzip = ["tar", "dep:flate2"]`、`xz = ["tar", ...]`、
   `zstd = ["tar", ...]`，而 `flate2` / `xz2` / `zstd` 都不依赖 `tar`。后果是裸 `.gz` / `.xz` /
   `.zst` 流无法处理，且 `--features gzip` 会意外拉进 `tar`。
5. `max_decoded_bytes` 的语义按格式漂移：tar 家族计解码容器流，zip 计条目解码材料（已写入
   文档）。一个字段承载两种不可比语义。

**为什么「关 feature」不产生通用性**：feature 只删除枚举里的变体，轴的维度没变。加一种编码
或容器仍要改：变体 + sniff 分支 + dispatch 分支 + feature + 测试，成本随格式数线性增长。

**目标形状（若执行）**：把两条轴拆开，组合作为数据而非变体。

```text
Container { Zip, Tar }            // 条目如何列举
Codec     { Identity, Gzip, Xz, Zstd }   // 字节如何压缩
ArchiveFormat { container, codec }       // 组合；sniff 用静态表 magic -> format
```

`prepare` 收敛为 `decode(codec) -> reader` + `extract(container, reader, policy)` 两步；
`ArchiveEvidence.format` 改为 `ArchiveFormat`；feature 只表示「该解码器是否编入」
（`gzip = ["dep:flate2"]` 等），`archive` 仍是便利包但不再蕴含 `tar`。由此自然获得
tar × 4 codec 的组合与裸流入口，无需新增变体。

**成本（可复核）**：改 1 个公开枚举为 2 枚举 + 1 结构体、91 处引用中的构造点、`sniff` 约 40 行
改为查表、5 个 dispatch 分支并为 2 个、4 个 tar 包装函数并为 1 个（净减约 40 行）。
未发布，无兼容负担。

**不做**：`trait ArchiveFormatImpl` 扩展点（无第三方格式需求，先不引入对象安全与文档成本）；
整体换成 `zip::ZipArchive::extract_with_options`（见 §10.3.5 评估）。

### 10.3.7 PS5 · 2026-11-11 archive 接缝重做（`EXECUTED`）

| 字段 | 值 |
| --- | --- |
| 触发 | 用户：「我们不应该包装功能！而是接驳功能，如果我们只能包装，那相关部分应该遗弃」。§10.3.6 的方案 B（拆 `Container` / `Codec` 两轴）被判为「加轴仍是包装」，作废 |
| 结论 | 删除格式层，保留策略内核；接缝由「路径 + 猜格式」改为「流 + 本库契约」；`xz2` / `zstd` 离开依赖图，`flate2` 只作为 `zip` 的 deflate 后端保留 |
| 权威 | 本次改动 `src/archive.rs`、`src/local/materialize.rs`、`Cargo.toml`、`examples/vtool/{manifest,realize,main}.rs`、`tests/{common,lifecycle/*,local/*}.rs` |

**逐段判定（复核所得，非印象）**：

| 原实现 | 判定 | 证据 |
| --- | --- | --- |
| `is_zip_symlink(mode)` | 包装 | `zip` 9 已有 `ZipFile::is_symlink()`；本库手抄 `unix_mode & 0o170000`，比原生更弱（只读 mode 位） |
| `extract_tar_{plain,gzip,xz,zstd}` | 包装 | 4 × 14 行，差异只有 `GzDecoder::new(file)` / `XzDecoder::new(file)` / `zstd::Decoder::new(file)` 一处构造 |
| `ArchiveKind` + `sniff` + `read_head` + `unsupported_format` | 包装 | 8 个魔数、5 组「有 feature 就返回变体、否则报错」的 cfg 对；`unsupported_format` 的存在理由只是消除该重复 |
| `Plain` 变体 | 残渣 | 被自己的 `prepare` 拒绝，只当证据标签；却让「归档枚举」里混进一个非归档 |
| `ArchivePolicy` 四级限额 | 接驳 | `tar` 全无；`zip` 9 只有单个 `size_limit`，无 entry-count / per-entry / total 三档 |
| 失败原子性 + `ArchiveCleanupFailed` | 接驳 | 两者都留残留；`zip` 的 `extract` 直写目标目录 |
| `ArchivePathIndex` | 接驳 | `zip` 的 header 校验只查重名；`a`(file) 后接 `a/b`、大小写折叠冲突都查不出；`tar` 全无 |
| Windows 保留名、符号链接整档拒绝、`strip_components`、`ArchiveEvidence` | 接驳 | 两个依赖都不表达这些契约 |

**新接缝**：

```text
prepare_tar<R: Read>(reader, root, policy)          // 编码由调用方决定
prepare_zip<R: Read + Seek>(reader, root, policy)   // 容器由 zip 解析，策略由本库决定
```

`crate::archive` 不再出现任何格式名、魔数或解码器；`crate::local::materialize` 只剩「material →
托管副本」，不再 sniff、不再返回提取证据。

**净效果（可复核）**：

- `src/archive.rs` 2520 → 2046 行（−474）；`src/local/materialize.rs` 255 → 134 行（−121）。
- feature：`archive = zip + tar`；`gzip` / `xz` / `zstd` 三个 feature 删除。`zip` / `tar` 不再蕴含
  `local`（`--features zip` 与 `--features tar` 各自 `cargo check` 无 error）。
- 依赖：`xz2`、`zstd` 从 `Cargo.lock` 移除（两者都是 C 构建）；`flate2` 降为 `zip` 的 deflate
  后端，不再由本库直接引用；`flate2` 同时进 `[dev-dependencies]`，供 gzip 组合的测试使用。
- 公开面：`ArchiveKind`、`ArchiveEvidence::format`、`MaterializeError`、`PreparationEvidence`
  删除；`LocalMaterial::prepare` 去掉 policy 与证据返回。
- 测试：270 → 261（删掉 3 个 sniff 测试、1 个格式证据测试、1 个 `Plain` 拒绝测试、2 个 xz、2 个
  zstd）。xz/zstd 的能力断言改由「tar + 调用方解码器」这一条覆盖，内核与编码无关。
- 判定口径收窄的后果：本库不再处理 `.tar.gz`。vtool 在清单里声明 `format = "zip" | "tar"`，
  由调用方把流交进来——清单本来就知道发布了什么，不需要猜魔数。

**验收**：`cargo fmt --all --check`；`cargo clippy --locked --all-targets --all-features -D warnings`
（0 warning）；`--features zip|tar|archive|local|(none)` 单点 check 全 0 error；`cargo deny --offline
check advisories bans sources licenses` 四项 ok；strict `cargo doc` 通过。Windows
`cargo test --locked --all-features -- --test-threads=1` 12 个 target 全 `0 failed`（261 tests，
2m42s）。WSL2 Linux 同哈希副本（`Cargo.toml` / `Cargo.lock` / `src/archive.rs` SHA256 一致）
12 个 target 全 `0 failed`（258 tests），MSRV 1.88 `check --all-features --tests` `exit=0`。

**历史段落不改写**：§4.x 的「`archive` = `zip + tar + gzip + xz + zstd`」与 §10.3.5 的归档 feature
矩阵是当时观测，由本节更正；`README.md` 的 formats 表与 feature 边已同步为现状。

### 10.3.8 PS5 · 2026-11-11 「接驳 / 包装」横向审计（net / process / hash / local）

判据沿用 §10.3.7，并补第三条（把同一把尺子用于仓库内部）：

| 编号 | 判据 | 结论 |
| --- | --- | --- |
| J1 | 依赖已提供同一语义 | 我们写的是包装 |
| J2 | 依赖不表达该语义（契约、证据、限额、原子性、平台机制） | 接驳，保留 |
| J3 | 同一语义在本仓写了两遍，且第二份没有消费者 | 拆掉第二份 |

**net（生产 2538 行：同步适配 399 = 1196–1594，异步适配 410 = 1596–2005，其余为类型与共享策略）**

| 部分 | 性质 | 依据 |
| --- | --- | --- |
| `RemoteUrl` 与错误层 | 接驳 | 把 `url` 收窄为「绝对 http/https、无凭据」 |
| `RetryPolicy` + `retry_delay` / `planned_retry_delay` | 接驳 | ureq 与 reqwest 都不重试 |
| `RateAdmission` / `ByteRatePacer` | 接驳 | governor 只提供 GCRA 引擎；「尝试次数」与「解码字节」是本库的两个语义单位 |
| `ResumePolicy` / `Validator` / `ResponseFacts::admit` | 接驳 | `If-Range`、`ETag`/`Last-Modified` 选择的续传语义，两个依赖都不表达 |
| `copy_response_body` | 接驳 | `max_bytes` 上限 + 字节配速 + 逐次证据，三合一 |
| `UreqResources` + `Inspect`/`Acquire` | 接驳 | 每次调用只把既有策略翻成 ureq 调用 |
| `ReqwestResources` + `AsyncInspect`/`AsyncAcquire` | **J3** | 410 行，控制流与同步版逐行镜像（同一重试循环、同一 `admit`、同一证据累积），另一半是 `AsyncSleep` 与 `chunk()` |

证据：`examples/` 里 `tokio`/`block_on`/`async fn` 命中 0；`http-reqwest` 只被 `tests/net/http.rs` 与
`tests/ambient_proxy.rs` 使用。

**process（生产 2122 行）**

| 部分 | 性质 | 依据 |
| --- | --- | --- |
| Job Object、进程组、`CREATE_SUSPENDED`、`stop_tree` | 接驳 | std 无对应能力 |
| `StagedInput`、输出 custody、`Diagnostics` cap、`CancelToken` | 接驳 | 无依赖表达「输入闭包 + 有界诊断 + 粘滞取消」 |
| 同步 `ChildSession` 轮询（1282–1446） | 接驳（被迫） | std 没有定时等待，只能轮询 |
| `Awaited` / `AsyncTreeGuard` / `acquire_async_inner`（1667–1955，289 行） | **J3** | `tokio::select!` 天然给出有界等待；模块文档自述「only the orchestration is duplicated」 |

**hash（生产 553 行）**

| 部分 | 性质 | 依据 |
| --- | --- | --- |
| `DigestAlgorithm` 两个实现（450–475） | 接驳 | blake3 与 sha2 无公共 trait，也都不返回已读字节数 |
| 64-hex 法、`DigestValue`、证据与对账 | 接驳 | 清单词汇，依赖不表达 |
| `DigestAlgorithmKind` + `digest_opened` 数据分派 | 保留 | 2 个变体、2 行 match，算法由清单**声明**；与已删除的 `ArchiveKind` 不同：那里格式靠魔数推断，是控制流化的格式知识 |

**local（`local.rs` 876 + `apply.rs` 374 + `record.rs` 311 + `view.rs` 342 = 1903 行）**

全部接驳：提交原子性与 no-replace、记录锁与有界记录、目录链接切换、按维度观察。没有依赖提供这些。
它的真实问题不是包装，是公开形状偏多（`LocalObservation` / `LocalArtifactObservation<T>` /
`LocalEntryKind` / `LocalInspectEvidence` / `LocalExpectation` / `LocalReconciliation` /
`LocalReconcileEvidence` …），属 PS6 的 P2/P3 议题，不在本判据内，不混进来判。

**本轮顺带修的三处（都是 §10.3.7 的后果，不是新设计）**

1. `ci.yml` 两处 feature 循环仍含已删除的 `gzip` / `xz` / `zstd`，配对里还有 `zip,gzip`。CI 自
   §10.3.7 起会在 `cargo check --features gzip` 直接报「feature 不存在」；因为未推送、
   CI 没跑，本地 `cargo test --all-features` 也不经过这条循环，所以一直没红。改为
   `… archive zip tar serde` 与 `zip,tar`，并逐条 `cargo check --no-default-features --features` 复核 0 error。
2. `src/hash.rs:137` 的「mirrors `archive::ArchiveKind`」失效；`examples/vtool/README.md:87` 仍写
   `gzip` feature 与 `ArchiveKind`。两处已改。
3. `.agents/plan/stage-4.md` 的 archive 冻结律加了 superseded 指针（见 §10.3.7）。

**改造建议（R1–R3，均未执行）**

| 编号 | 内容 | 预计变化 | 重开条件 |
| --- | --- | --- | --- |
| R1 | 单后端化：删 `http-reqwest`、`ReqwestResources`、异步 `AsyncInspect`/`AsyncAcquire` | −410 生产行、−`reqwest`、feature 13 → 12 | 出现必须 `await` 的消费者 |
| R2 | 同 R1 的决定：删 `process-tokio`、`Awaited`、`AsyncTreeGuard` | −289 生产行、−`tokio`（全图） | 同上 |
| R3 | 保留 `DigestAlgorithmKind`；不动 local 形状 | 0 | — |

R1+R2 合计：生产 8177 → 7478（−699，−8.6%），测试约 −500 行（`tests/net/http.rs` 异步半部、
`tests/process_async.rs` 340 行），依赖图失去 `reqwest` 与 `tokio` 两个重包，CI 的
`http-ureq,http-reqwest` 与 `process,process-tokio` 两组对角随之消失。

反证与代价（不隐藏）：async 是唯一事件驱动等待，同步侧是 10→50 ms 有界退避轮询。若将来出现
并发下载或与 tokio 运行时共存的消费者，R1/R2 需回滚；因为两份实现是镜像的，回滚成本≈删除成本。
判定：R1/R2 属大改（约 700 生产行 + 500 测试行 + 依赖图），按规则只出建议。

### 10.3.9 PS5 收口报告（stage 级）

| 字段 | 值 |
| --- | --- |
| 状态 | `LANDED`（stage 级）；任务 `s5-1`–`s5-6` 6/6 任务级 `LANDED` |
| 产出 | 测量基线（W1–W3）、law 与 rustdoc 修正、有界退避等待、依赖公告 17→0、`default = ["local"]`、测试自包含、archive 接缝重做 |
| 最终验收 | Windows 261 tests / 12 target `exit=0`；WSL2 Linux 同哈希副本 258 tests `exit=0`；fmt、clippy `-D warnings`、strict doc、MSRV 1.88、`cargo deny` 四项 |
| 显式未闭合 | W2 唤醒预算（D3 撤销内部插桩）；本轮审计为 self-review，无 fresh-context 独立复核；`s5-4` 的 advisory-db 刷新仍归 `UD-6` 发布前 |
| 不授权 | 发布 / tag / push；PS4 状态裁决 |

结论：PS5 的产出是「测量、law 修正、依赖清零、测试自包含、接缝重做」，不含新能力。新能力仍归
PS6，触发条件不变：tuckin 提具体缺口，或用户点名某个 P 项。

### 10.3.10 PS5 · 2026-11-11 判据澄清：接驳 / 包装是两条轴，不是三类代码

§10.3.8 把「这块代码与依赖的关系」和「这块代码在本仓是第几份」写进同一列「性质」，于是
`UreqResources + Inspect`/`Acquire`（记为接驳）与 reqwest 侧（记为 J3）读起来像两种不同的代码。
它们是同一种代码：都是接驳。差别只在第二条轴。本节把轴拆开，逐块重判，不修改 §10.3.8 的历史行。

**轴 A：这块代码与依赖的关系**

| 判定 | 操作性测试（删掉这块代码之后） | 含义 |
| --- | --- | --- |
| 接驳 | 某个依赖能力断线，但 pulith 的**语义仍在别处**，只需重写这一个转换面 | 依赖提供能力，pulith 提供语义 |
| 包装 | 依赖能力断线，且 pulith **不损失任何语义** | 只是把依赖的 API 换个名字再暴露一次 |
| 自持 | 删除不影响任何依赖（只用 std 与系统调用） | 轴 A 对它无定义；称它「接驳」是空话 |

浓缩判据：**包装把依赖的形状带上公开面；接驳把 pulith 的语义压在依赖之上，公开面上只剩 pulith
的词汇。** 唯一例外是注入点（`from_agent(ureq::Agent)`、`from_client(reqwest::Client)`）：接缝
必须命名它要接的那个客户端，这是凭证参数，不是能力暴露。archive 的 `ArchiveKind` 之所以是包装，
正因为它把「格式」变成公开 enum 与控制流（靠魔数推断），而不是留在一个转换面后面。

**轴 B：这块代码在本仓有几份**

唯一 / 第二份（仓内无消费者）/ 死重（零调用点）。

| 代码 | 轴 A | 轴 B | 依据 |
| --- | --- | --- | --- |
| `net::RemoteUrl` + 错误层 | 接驳 | 唯一 | `url` 收窄为「绝对 http/https、无凭据」；`url` 自己不表达该契约 |
| `RetryPolicy` + `retry_delay` / `planned_retry_delay` / `should_retry_status` / `parse_retry_after` | 接驳 | 唯一（两侧共用） | 两个客户端都不重试；纯函数，`cfg(any(ureq, reqwest))`，无 transport 门控 |
| `RateAdmission` / `ByteRatePacer` | 接驳 | 判定唯一、等待循环两份 | governor 只给 GCRA 引擎；「尝试次数」与「解码字节」是 pulith 的两个单位；`enter_sync`/`enter_async` 各约 10 行，只差 sleep 提供者 |
| `ResumePolicy` / `Validator` / `ResponseFacts::admit` | 接驳 | 唯一 | `If-Range`、`ETag`/`Last-Modified` 选择权两个依赖都不表达；`admit` 在 1438 与 1854 各调一次，判定律是单源的 |
| `copy_response_body`（ureq）与 reqwest 的 chunk 循环 | 接驳 | 两份 | `max_bytes` 上限 + 字节配速 + 逐次证据三合一；这是唯一真正按 transport 分叉的 body 面 |
| `UreqResources` + `Inspect`/`Acquire`（1187–1595） | 接驳 | 第一份 | 循环、证据累积、状态分类都在 pulith；ureq 只提供 `head`/`get`/`call` |
| `ReqwestResources` + `AsyncInspect`/`AsyncAcquire`（1596–2004） | 接驳 | 第二份，仓内消费者只有测试 | 与上一行同构，差别是 `send().await`、`chunk()`、`AsyncSleep` |
| `process` 平台机制（Job Object、`process_group`、`CREATE_SUSPENDED`、`stop_tree`） | 接驳 | 唯一 | std 无对应能力 |
| `process` 同步 `ChildSession`（1304–1446） | 自持 | 唯一 | std 没有限时等待，只能有界退避轮询 |
| `process` `Awaited` / `AsyncTreeGuard` / `acquire_async_cancellable_inner`（1667–1955） | 接驳 | 第二份 | tokio 提供限时等待与 pipe 读取；注意观察机制本身不同：同步侧重定向到 `stdout.log`，异步侧 pipe 进内存 |
| `hash` 的 `DigestAlgorithm` 两实现 | 接驳 | 唯一（每算法一份） | `blake3`/`sha2` 无公共 trait，也都不返回已读字节数；实现体各 8 行，共用 `copy_into_hasher` |
| `hash` 的 64-hex 法 / `DigestValue` / 证据 | 自持 | 唯一 | 清单词汇，无依赖可接 |
| `archive::prepare_tar` / `prepare_zip` | 接驳 | 唯一（每格式一份） | 格式库解析条目，pulith 决定路径、限额、冲突、symlink 拒绝、失败原子性 |
| `archive` 两个 entry 循环里的政策序列 | 自持 | 两份（已接受） | zip 随机访问 + 声明尺寸，tar 流式；合流要给两套无关 entry 类型造新 trait，那是新词汇不是接缝 |
| `local` 的 `apply` / `view` / `record` / `materialize` | 自持 | 唯一 | 提交原子性、no-replace、记录锁、链接切换都在 std / `rustix` / `windows-sys` 原语上自己实现 |
| `local` 用 `tempfile` / `walkdir` | 接驳 | 唯一 | `tempfile` 管唯一命名与 drop 清理，`walkdir` 管遍历；放置与准入是 pulith 的 |
| `same-file` | — | 死重（零调用点） | 既非接驳也非包装：没有任何代码碰它，只在 `local` 的 feature 列表里挂着 |

**结论（对 §10.3.8 的修正）**

1. 现有代码里**没有一处包装**。§10.3.8 的 J1 在 net / process / hash / local 上命中 0。
2. `local` 的「全部接驳」在轴 A 上几乎是空转：它的主体是自持 law，只有 `tempfile` / `walkdir`
   两个使用点是接驳。§10.3.8 那句话读起来像「包装」的反面，实际是「无定义」。
3. R1 / R2 的依据必须写成轴 B，而不是轴 A：异步侧不是包装，是**同一套接驳的第二份**，且仓内
   消费者只有测试（`tests/` 5 个文件；`examples/` 命中 0）。「拆」的理由是重复，不是包装。
4. 因此「删掉 reqwest / tokio」不损失任何 pulith 语义——这才符合「只能包装的部分应遗弃」的
   原意：遗弃的是第二份实现，不是语义。

**本轮顺带发现并修复**

| 编号 | 内容 | 处置 |
| --- | --- | --- |
| F1 | `same-file` 零调用点却挂在 `local` feature 上（图里由 `walkdir` 传递进来，删掉不改图，但声明是假的） | 从 feature 与 `[dependencies]` 移除；`cargo check --all-features` / `--features local` 均 0 error |
| F2 | `docs/AGENTS.md:42-43` 仍写 `same-file` 与 `xz2, zstd`（后者已于 §10.3.7 离开依赖图） | 改为 `tempfile, walkdir` 与 `zip, tar, flate2` |

**建议（E1 未执行；R1/R2 已由 §10.3.11 执行）**

| 编号 | 内容 | 收益 | 风险 |
| --- | --- | --- | --- |
| E1 | 把 `net.rs`（2888 行，其中测试 930）与 `process.rs`（1895 / 测试 76）按接缝拆成 `net/{mod,ureq}.rs`、`process/{mod,sync}.rs` | 接缝在文件树上可见；单文件阅读成本降一半 | 纯移动、`rustdoc` 路径不变；需一次 `cargo doc` 复核模块文档引用。R1/R2 已于 §10.3.11 执行，拆分对象只剩单后端，优先级下调 |

### 10.3.11 PS5 · 2026-11-11 s5-7 单后端化（执行 R1/R2）

| 字段 | 值 |
| --- | --- |
| 触发 | 用户 2026-11-11：「既然是重复设计，依照你的分析去简化，优化即可」——批准 §10.3.10 的 R1/R2 |
| 判据 | 轴 B（同一套接驳的第二份，仓内无消费者），不是轴 A（包装）；所以删的是重复，不是语义 |
| 状态 | `LANDED`（任务级）；PS5 任务数 6 → 7 |

**删除（生产行为零变化，同步语义一字未改）**

| 对象 | 变化 |
| --- | --- |
| `net`：`ReqwestResources` / `inspect_reqwest` / `acquire_reqwest` / reqwest 侧 `StagedDownload` / `AsyncSleep` / `enter_async` / `before_chunk_async` | `net.rs` 4187 → 2888（生产 2537 → 1958，测试 1650 → 930） |
| `process`：`AsyncAcquire` 实现 / `Awaited` / `AsyncTreeGuard` / `acquire_async*` / 异步 capture 辅助 / `CancelToken::cancelled` | `process.rs` 2199 → 1895（生产 2122 → 1818） |
| `lib`：`AsyncAcquire` / `AsyncInspect` 两个 trait | `lib.rs` 146 → 122 |
| 测试：`tests/process_async.rs` 整个 target、`tests/net/http.rs` 异步半部、`net.rs` 内 31 个 `http-reqwest` 块、`ambient_proxy` 与 `lifecycle` 的异步块 | 261 → 213 tests，12 → 11 target；`http.rs` 598 → 372，`ambient_proxy.rs` 203 → 165，`process_async.rs` 340 → 0 |
| 依赖与 feature | feature 13 → 11（删 `http-reqwest` / `process-tokio`）；`reqwest`、`tokio` 离开依赖图；CI 的 2 组对角组合随之消失 |

**保留（不是重复，是单份 law）**

`RetryPolicy` 与 `retry_delay` / `planned_retry_delay` / `should_retry_status` / `parse_retry_after`、
`ResponseFacts::admit`、`copy_response_body`、`RateAdmission` / `ByteRatePacer` 的判定、`ResumePolicy`
与 `Validator`、四级限额、证据类型、同步 `ChildSession` 的有界退避轮询、平台树停止、Job Object、
`CREATE_SUSPENDED`。这些的 `cfg` 由 `any(http-ureq, http-reqwest)` 收窄为 `http-ureq`，语义不变。

**验收（Windows 实测）**

| 门 | 结果 |
| --- | --- |
| `cargo test --all-features -- --test-threads=1` | `exit=0`，11 target / 213 tests 全 `0 failed`，2m11s（改前 3m52s） |
| `cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings` | 通过 |
| `RUSTDOCFLAGS="-D warnings -D missing-docs" cargo doc --all-features --no-deps` | 通过 |
| feature 边界：空集 + 11 个单点 + 3 组对角（`local,sha2` / `zip,tar` / `local,blake3,serde`），`RUSTFLAGS=-D warnings` | 15 / 15 `0 error` |
| `cargo +1.88.0 check --all-features` | 通过 |
| `cargo deny check` | advisories / bans / licenses / sources 全 `ok` |

**同轮发现并修（都不是本次删除引入的）**

| 编号 | 内容 | 处置 |
| --- | --- | --- |
| D18 | `--no-default-features --features hash` 在 `-D warnings` 下失败：`ArtifactDescriptor::from_digest` 只在 `local` 组合里被调用，`hash` 单点触发 dead_code。CI 的 feature 循环带 `RUSTFLAGS=-D warnings`，但从未在远端跑过（未推送），属 D17 同族 | 给 `from_digest` 补 `#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]`；`hash` 单点 0 error |
| D20 | 满载下取消用例偶发 `WorkspaceCleanup`（error 32）：停止后的后代仍短暂持有工作区 cwd | `fail_cleanup` 改为 `keep()` + 有界重试（6 次，10→80 ms）；仍是 best-effort，不把主结果转成功 |
| D19 | `cargo deny` 出现新公告：`rustix 1.1.5` 的 xattr 零长度缓冲区，要求 ≥ 1.1.6；来源是 `tempfile` 与 `tar → xattr`，与本轮改动无关 | `cargo update -p rustix` → 1.1.6（仍在 `1` 约束内，MSRV 1.88 兼容）；deny 4/4 `ok` |

**边界与回滚**

- 仓内已无异步入口：`examples/` 与 tuckin 的 `path` 依赖都只用同步面；`AsyncAcquire` / `AsyncInspect`
  不再是公开 API。
- 重开条件：出现必须 `await` 的消费者（并发下载、与 tokio 运行时共存、或用 tokio 定时器替换 10→50 ms
  轮询）。回滚成本 ≈ 删除成本：两份实现曾是镜像，可从 git 历史整段取回，但需重新迁移 feature 与 CI 对角。
- 不再声称「异步与同步语义等价」——现在只有一份语义，这句话失去对象。

### 10.4 PS6 报告 — 特性开发

| 字段 | 值 |
| --- | --- |
| 目标 | P1–P6 能力在共开发中按 tuckin 的 D24 / D27 需要逐项落地 |
| 状态 | `IN PROGRESS`；P1 已落地（§10.4.1），`s6-1` 单文件发布入口已落地（§10.4.2），P2–P6 未开始 |
| 权威 | 用户 + 本文 §4.2；内部设计评审稿只读，不作为执行输入 |

**task 文件按需生成**：P1–P3 / P6 会改公开类型形状。旧稿把「先冻结 0.2.0 兼容性边界」
当作前置；不发布后这个前置撤销，但公开形状仍需与 tuckin 同步确定，按「边确定边开发」在
真正落地时生成任务，而不是预先把未定的形状写进可执行步骤。目前只有
`.agents/plan/s6-1-single-file-publish-task.md` 一份（私有执行层）。

**偏差记录**：空。**结论**：未产生执行结论。**Reopen 条件**：用户指定要落地的 P 项、或
tuckin TS3 提出具体能力缺口。

### 10.4.2 PS6 · 2026-11-12 `s6-1` 单文件发布入口（`LANDED`）

| 字段 | 值 |
| --- | --- |
| 触发 | tuckin 报 `apply` 300 文件全写 2.1 s 太慢；归因与对照测量在 `tuckin/docs/plan.md` §6.1.31 与 DL-38 |
| 归因 | 慢的不是实现，是**计量单位**：`StagedTree` 的单位是一棵树，tuckin 每文件用一个 |
| 交付 | `LocalTarget::write_atomic(bytes, admission)`；纯加法，不改任何既有签名与证据类型 |
| law | 与 `StagedTree::commit` 同一条：同目录临时文件 → 提交前重分类 → 一次换名；覆盖沿用目标 mode；失败删临时文件、目标原状不变 |
| 数字（300 个目标，同一进程对照） | `StagedTree` 全套 **1.36–1.98 s**、`write_atomic` **0.60–0.73 s**、手写 `write` + `rename` **0.56–0.74 s** |
| 平台 | Windows 实测（`fmt` / `clippy --all-targets` / 56 条测试绿）；Linux 未跑，WSL 可用时补 |
| 偏差 | 无 |

**为什么是加而不是改**：树入口对「临时目录 + 整棵树一次可见」仍是正确答案，删掉它等于用第二个
消费者换掉一条 law。两个入口共享 `occupancy` / `rename_no_replace` / `rename_replace` /
`inherit_mode`，平台差异仍然只有一份——这是「不留第四种换名实现」的同一条理由。

**两处实现细节值得记下来**：

1. 临时名 `.<目标名>.pulith-<pid>-<序号>`，`create_new` 才算数——名字是提议，不是保证。进程号分开
   并发写者，计数器分开顺序写者；两个进程撞名的概率交给内核判。
2. 父目录不预先 `create_dir_all`。先 open，只有 `NotFound` 才建链。这是把一个「每文件两次系统
   调用」的常数改成「每个新子树一次」。**这一条是量出来的**：预建父目录的对照比不建还慢
   （0.73 → 0.98 s），因为 `create_dir_all` 对一个已存在的目录也要先失败再 `is_dir`。

**五条契约测试**（`tests/local/materialization.rs`）：发布一笔并确认邻居只有目标本身；替换既有字节；
`Admission::Absent` 遇到前身时拒绝且字节不变；目标是目录时拒绝且不留临时文件；替换沿用 mode
（unix only）。

**falsifier**：单文件入口与树入口在某平台上产生不同的可观察结果（mode、ACL、reparse 分类任一项）；
或某消费者需要「同目录多个文件一起可见」的整树原子性——那时回树入口，而不是给单文件入口加批量
语义。

### 10.4.1 PS6 · P1 设计稿（待批）

2026-11-11。用户批准 P1 落地，但要求先报设计。**本节只写设计，未写实现代码。**

**缺口（唯一来源）**：tuckin `F4` / `DL-03` 要「覆盖既有常规文件、沿用其 mode、失败保原状」；
pulith 现在只有 no-replace（`renameat_with(NOREPLACE)` / `MoveFileExW` 无
`REPLACE_EXISTING`），目标存在即 `AlreadyPublished`。tuckin `t1-4` 的提交契约没有第二条路。

**现状事实（不重复造词）**

| 行为 | 现在的 law |
| --- | --- |
| `LocalTarget::stage` | 建父目录 + 目标旁私有暂存树（`StagePlacement::Destination`） |
| `StagedTree::publish` | no-replace rename；目标存在 → `AlreadyPublished`；跨目录先重落到目标父目录 |
| `LocalTarget::observe` | `LocalObservation`：`Missing` / `File{bytes}` / `Directory` / `SymlinkTo*` / `Other`，**会解析链接目标** |
| `LocalEntryKind` | no-follow 分类：`Missing` / `File` / `Directory` / `Symlink` / `Reparse` / `Other` |
| 删除 | `Remove`，与提交正交 |

仓内已存在两套「路径上有什么」的词汇。P1 不再引入第三套，只在 no-follow 词汇上加一个**准入投影**。

**设计：2 个类型 + 1 个错误变体 + 1 个方法，零新依赖**

```rust
/// What occupies a target path, restricted to what publication can admit.
pub enum Occupancy {
    Absent,
    RegularFile { bytes: u64 },
}

impl LocalTarget {
    /// Classifies the target without following links.
    pub fn occupancy(&self) -> Result<Occupancy, LocalError>;
}

/// The occupancy precondition a commit is allowed to proceed under.
pub enum Admission {
    /// Commit only when the target is absent.
    Absent,
    /// Commit when the target is absent or a regular file; a regular file is replaced in place
    /// and keeps its mode.
    ReplaceRegularFile,
}

impl StagedTree {
    pub fn commit(self, target: LocalTarget, admission: Admission) -> Result<ApplyEvidence, LocalError>;
    /// Equivalent to `commit(target, Admission::Absent)`; the immutable law is unchanged.
    pub fn publish(self, target: LocalTarget) -> Result<ApplyEvidence, LocalError>;
}
```

新增错误：`LocalError::OccupiedTarget { path: PathBuf, kind: LocalEntryKind }`——拒绝时把
no-follow 事实原样报出（`Directory` / `Symlink` / `Reparse` / `Other`），不解析、不改动。

**为什么 `Occupancy` 不复用 `LocalObservation`**：`observe` 为了区分 `SymlinkToDirectory` /
`SymlinkToFile` 必须解析链接目标，而准入判据恰恰是「不解析」。用 `observe` 做准入等于先跟随再
拒绝，与 F4「符号链接目标不被跟随」的 falsifier 直接冲突。`LocalEntryKind` 是唯一不解析的词汇，
所以拒绝时用它，成功时用两变体的 `Occupancy`。

**`Admission` 是 syscall 形状，不是口味**：`Absent` 走 `RENAME_NOREPLACE` / `flags = 0`；
`ReplaceRegularFile` 走 `rename(2)` / `MOVEFILE_REPLACE_EXISTING`。两者不是同一调用的策略参数，
所以不退化成 `bool`，也不引入配置面。

**逐平台提交语义**

| 步 | POSIX | Windows |
| --- | --- | --- |
| 1 分类 | `symlink_metadata`，只接受 `Absent` / `RegularFile` | 同左，另拒 `FILE_ATTRIBUTE_REPARSE_POINT` |
| 2 只读 | 无此概念（`0400` 仍可 rename 覆盖） | 只读常规文件 → `OccupiedTarget`，**不先清位** |
| 3 mode | 读目标 `mode & 0o7777`，`set_permissions` 写在暂存件上（std，`cfg(unix)`） | 不做 mode 保留 |
| 4 提交 | `rename(2)` 原子覆盖 | `MoveFileExW(REPLACE_EXISTING)` |

ACL：暂存目录建在目标父目录内，新文件按父目录继承，覆盖后权限与同级文件一致——这是「同目录暂存」
除了原子性之外的第二个理由，写进 law。

**如实声明的边界**

- 分类与提交之间不是原子对：并发把目标换成链接时，`rename` 替换的是链接**本身**，永不跟随其
  目标；损坏面限于那个链接，不扩散到它指向的文件。plan 期的 `occupancy` 只是报告，`commit`
  当场重判，不信任调用方传进来的旧观察。
- Windows 不保留 mode：唯一可保留的位是只读，而只读目标被拒绝。这条改的是 F4 的口径，需要
  tuckin 侧同步（`0644` 在 Windows 上是无意义声明）。
- `ApplyEvidence` 不变（`files` / `directories` / `bytes`）：覆盖与否由调用方 plan 期已知，
  证据不重复状态。

**判据（实现时逐条落测试）**

| 情形 | 判据 |
| --- | --- |
| 目标缺失 | 提交成功，字节一致 |
| 常规文件 | 内容换成新字节；POSIX `0600` / `0755` 保持 |
| 目录 | `OccupiedTarget`，目录原样 |
| 指向文件的链接 | `OccupiedTarget`；链接与其目标逐字节不变 |
| reparse（Windows） | `OccupiedTarget` |
| 只读文件（Windows） | `OccupiedTarget` |
| 暂存件不可读 / 中途失败 | 目标旧字节不变（摘要比对） |
| 既有 `publish` 调用 | 行为不变，`AlreadyPublished` 用例不改 |

**不做**：`Admission` 不扩到删除（I4 仍归调用方）；不加 `Merge` 类准入；不动 `LocalObservation`
/ `LocalEntryKind`；不新增依赖、不动 feature 面。

**待裁（实现前需用户点头）**：Windows 只读目标——拒绝（本稿）还是「清只读位 → 覆盖 → 复原」
（三步非原子，中途失败会留下可写文件）。本稿选拒绝，理由是失败面最小且可解释。

**实现与验收（2026-11-11 落地）**

落地形状与上稿的差异只有一条，记入 D21：Windows 只读目标**不做提交前预检**。理由是实现时发现
预检会把 `OccupiedTarget { kind: File }` 这一自相矛盾的报告变成主路径，而 `MoveFileExW` 的
`ERROR_ACCESS_DENIED` 既可能是只读也可能是 ACL，据此命名 `ReadOnlyTarget` 等于猜。改为「直接
尝试替换，OS 拒绝则以 `Io { action: "replace target", path, source }` 如实报出」：四要素齐全，
不新增变体，且没有预检与提交之间的窗口。设计稿里承诺的两条仍然成立——**从不清只读位**、目标
原状不变。

| 门 | 结果 |
| --- | --- |
| `cargo test --all-features -- --test-threads=1` | 10 target / **219 tests，0 failed**（P1 前 213），3m19s |
| 新增用例 | `occupancy_reports_a_regular_file_and_refuses_a_directory`、`commit_replaces_a_regular_file_and_keeps_its_mode`、`commit_creates_the_target_when_the_replacing_admission_finds_it_absent`、`commit_refuses_a_directory_target_without_touching_it`、`commit_refuses_a_symlink_target_without_following_it`、`absent_admission_still_refuses_an_existing_regular_file` |
| 既有 `publish` 面 | 13 处调用点零改动；`AlreadyPublished` 用例不改，`publish` = `commit(Absent)` |
| fmt / clippy `-D warnings` / 严格 doc | 通过 |
| 依赖与 feature | 未增；`inherit_mode` 用 std `PermissionsExt`，`rename_replace` 用 std `fs::rename` |

**实现要点（对照设计）**

| 设计条目 | 落地 |
| --- | --- |
| 覆盖只接受常规文件 | `occupancy()` 用 `symlink_metadata` + `is_reparse_metadata`，`Directory` / `Symlink` / `Reparse` / `Other` 一律 `OccupiedTarget` |
| POSIX 保留 mode | `inherit_mode` 读目标 `mode & 0o7777`，写在暂存件上，再 `rename` |
| 跨目录先重落 | `commit` 在 `StagePlacement` 不匹配时先把树重落到目标父目录，最终 rename 恒为同目录操作 |
| 常规文件不被目录覆盖 | 目标为常规文件而暂存根是目录 → `InvalidPreparation` |
| 不信任旧观察 | `commit` 自己再判一次 occupancy；`occupancy()` 只作报告 |

### 10.5 PS7 报告 — 下游集成验证

| 字段 | 值 |
| --- | --- |
| 目标 | tuckin 的 plain 复制与提交边界在真实消费者上跑通 |
| 状态 | `NOT PLANNED`，仅登记依赖；归属 tuckin `TS3`，不是 pulith 的执行范围 |
| 前置 | tuckin 的 `path` 依赖可用；D24 / D27 能力具备 |

**验收门**：tuckin v0.1 的文件相在双平台部署既有常规文件；含密产物在 Windows 按 `E-P1`
拒绝而不是静默降级。**偏差记录**：空。**结论**：未产生执行结论。
**Reopen 条件**：tuckin 开始 TS3，或 tuckin 决定自行实现暂存与提交。

### 10.6 全局偏差日志

| 序 | Stage | 偏差 | 发现时点 | 影响的门 | 处置 | 状态 |
| --- | --- | --- | --- | --- | --- | --- |
| D1 | PS5 | 旧基线 18/62/172 未说明是否含根包，且只有 Windows 口径 | `s5-1` W1 | W1 可复现性 | 明确 `--format {p}` 含根包；给出去根值并补 Linux 静态图与 default 口径 | 已记录 |
| D2 | PS5 | all-features warm 中位数 1.754 s 与旧基线 4.133 s 相差大 | `s5-1` W1 | W1 编译成本 | 基线改为三次中位数；不跨会话比较、不声称更快 | 已记录 |
| D3 | PS5 | `T/10ms` 精确系数被等价探针证伪（23 次 / 191 次） | `s5-1` W2 | `s5-3` 唤醒预算 | F4 改为「线性成立、系数待内部插桩」；`s5-3` 停在预算确定前 | **影响后续任务** |
| D4 | PS5 | `cargo tree -d` 默认无输出，重复只在 all-features 图（71 组） | `s5-1` 步骤 7 | F3 与 `s5-4` 范围 | F3 修订为「非默认 feature 图属性」；`s5-4` 按 feature 作用域处理 | 已记录 |
| D5 | PS5 | Linux 侧只能静态解析 `cargo tree --target`，本机 `wsl.exe` 被宿主安全策略禁止 | `s5-1` W1 | G6 双平台 | 不声称在 Linux 实跑；双平台结论留给 CI 或用户环境 | **验收限制** |
| D6 | PS5 | 默认环境下 `net::tests::reqwest_inspect_preserves_transport_attempt_evidence` 失败：本机 `HTTP_PROXY` / `HTTPS_PROXY` / `http_proxy` / `https_proxy` = `http://127.0.0.1:62558` 且未设 `no_proxy`，测试期望「连接被拒绝」却被本机代理回 502 | `s5-2` 验证门 | `cargo test --all-features` | 该前提已写在 `tests/common.rs:299-305`；`src/net.rs` 不在本次 diff 内；关闭代理并设 `no_proxy=127.0.0.1,localhost` 后单测与完整套件全绿。属验收环境前提，不是本次改动引入 | **已隔离** |
| D7 | PS5 | 代理关闭后的第一次完整并行套件里 `--test process` 出现 1 次 31/32 的时序性失败 | `s5-2` 验证门 | `cargo test --all-features` | 单独复跑 `--test process` 两次均 32/32 全绿；`--test-threads=1` 全绿；最终完整并行套件 exit=0 全绿。`process.rs` 只改 rustdoc 未改行为。记录为 flaky，不作为通过依据 | **已复现复跑** |
| D8 | PS5 | D5 把「Linux 实跑不可用」判得过宽：此前只探测了 UNC 根 `\\wsl$\` | 本轮只读复核 | G6 双平台 | 实测 `\\wsl$\tumbleweed\`、`\\wsl.localhost\tumbleweed\` 均可达，注册表有 2 个 WSL2 发行版，`home/nostal/.cargo/bin/cargo` 与 `.rustup` 存在；D5 收窄为「缺 Linux 侧执行入口」，历史行不改写 | **限制收窄** |
| D9 | PS5 | 上一轮报告称「`hash` 与 `local` 耦合是设计缺陷」被源码证伪 | 本轮代码复核 | cfg 设计 | `src/hash.rs` 21 处 `cfg(all(local, any(blake3,sha2)))`，`Verify<DigestValue> for LocalMaterial` 与 `open_local_artifact` 明确作用在本地材料上；撤回该批评，保留耦合并在 README 写明语义 | **批评撤回** |
| D10 | PS5 | 默认（带 `HTTP_PROXY`、无 `no_proxy`）环境下 `--test process` 并行出现 11 个 PowerShell fixture 10 s 超时 | 本轮验证门 | `cargo test --all-features` | 网络测试改为注入 `without_proxy()` + 测试自持的 `closed_endpoint()`；进程测试加静态串行锁消除平台 shell 争用。并行 `--test process` 连续两次 32/32 全绿 | **由设计消除** |
| D11 | PS5 | `--test examples` 的 9 个测试并行时卡在子 `cargo build` 编译 `ring` 的 `cl.exe`，输出 12 分钟无进展 | 本轮验证门 | 完整套件 | 停掉挂起任务并清理子进程；属宿主编译环境，不是断言失败。改以单线程 + 超时保护单独验收 | **环境限制** |
| D12 | PS5 | 首次 Linux G6 验收在 502 脏代理下 `--lib` 的 HTTP mock 测试大面积失败 / 挂起（单测复现 `CONNECT proxy failed: proxy server responded 502/502`），§10.3.2 的「脏环境全绿」结论被证伪 | 2026-11-11 WSL 实跑 + 单测复现 | `cargo test --all-features` | 根因不是平台差异：`unwrap_or_default()` 落到隐式读进程代理的 `UreqResources::default()` / `ReqwestResources::default()`，`s5-6` 只覆盖 4 个显式注入点。把两个 `Default` 改为显式直连、代理由 `from_agent` / `from_client` 注入，`without_proxy()` 退化为语义别名；Windows lib 131 / net 20 与 Linux 28 门回归全绿 | **由设计消除** |
| D13 | PS5 | D5 / D8 记录的「Linux 侧执行入口缺失」已解除 | 用户移除 `wsl.exe` 程序黑名单；2026-11-11 WSL 实跑 | G6 双平台 | `wsl.exe -d tumbleweed` 可执行，WSL2 内核与 1.99.0 / 1.88.0 工具链齐备；G6 按实测关闭。边界：验收对象为哈希一致的代码副本而非 git 工作树，CI `hostile-env` job 尚未在远端触发 | **限制解除（附边界）** |
| D14 | PS5 | `hostile-env` 被用户判定为不轻量、不内聚：CI 用 `python3` 起 502 代理、固定端口 62558、`sleep 1`、导出 8 个变量，再重复跑完整套件；开发者本地 `cargo test` 覆盖不到 | 2026-11-11 用户指令 | `cargo test --all-features` | D12 修复后环境代理已不是产品输入，该 job 测的是不存在的输入。删除 job；新增 `tests/ambient_proxy.rs`，在独立测试二进制内自持 502 代理与 200 源站、安装并恢复变量，断言默认直连与显式注入两条路径；临时回退 `Default` 的反向验证复现 `502/502`，证明不是恒绿断言 | **由设计消除** |
| D15 | PS5 | `s5-4` 曾按「轻量」重定价为只做作用域记录、公告清零归 `UD-6`；用户随后要求继续 `s5-4`，重定价被推翻 | 2026-11-11 用户指令「继续 s5-4」「可以升级 zip 主版本」 | `s5-4` 门 | 执行清理门：公告 17→0；zip 8→9 并收敛特征；启用原生 `validate_header_consistency`；升级后 zip 自行拒绝伪造尺寸，原 observed-limit 测试改判 `InvalidPreparation` 并更名 | **已执行** |
| D16 | PS5 | §10.3.6 的方案 B 用「拆 `Container` / `Codec` 两轴」换通用性，被用户判定为仍是包装（「加轴」不改变包装性质） | 2026-11-11 用户指令「不应包装功能，而是接驳功能；只能包装的部分应遗弃」 | `cargo test --all-features`、`cargo clippy -D warnings`、feature 单点 check | 删除格式层（`ArchiveKind` / `sniff` / 4 个 tar 包装 / `is_zip_symlink` / `Plain`），`prepare_tar` 与 `prepare_zip` 直接吃流；`local::materialize` 退回「复制进托管」；`xz2` / `zstd` 离开依赖图；`archive` 只剩 `zip + tar`。内核（限额 / 原子性 / 冲突索引 / 保留名 / 证据）保留，因为两个依赖都不表达这些契约 | **已执行（§10.3.7）** |
| D17 | PS5 | §10.3.7 的验收只跑了受影响组合，没有执行 CI 脚本本体：`ci.yml` 两处 feature 循环仍引用已删除的 `gzip` / `xz` / `zstd`，配对里还有 `zip,gzip`。CI 一旦触发会在 `cargo check --features gzip` 直接报「feature 不存在」；因未推送、本地 `--all-features` 不经过该循环，一直未红 | 2026-11-11 §10.3.8 横向审计 | `lint` 的 feature 边界循环、`feature-tests` job | 改为 `… archive zip tar serde` 与 `zip,tar`，逐条 `cargo check --no-default-features --features` 复核 0 error；同时修掉两处失效文档引用（`src/hash.rs` 的 `ArchiveKind` 影子说明、`examples/vtool/README.md` 的 `gzip` 行） | **已修复** |

| D18 | PS5 | `cargo check --no-default-features --features hash` 在 `-D warnings` 下失败：`ArtifactDescriptor::from_digest` 只在 `local + blake3/sha2` 组合里被调用，`hash` 单点触发 `dead_code`。CI 的 feature 循环带 `RUSTFLAGS=-D warnings`，但从未在远端执行（未推送），与 D17 同族 | 2026-11-11 `s5-7` 验收 | `lint` 的 feature 边界循环 | 给 `from_digest` 补 `#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]`；`hash` 单点复核 0 error | **已修复** |
| D19 | PS5 | `cargo deny` 出现 `rustix 1.1.5` 的 xattr 零长度缓冲区公告（要求 ≥ 1.1.6），来源为 `tempfile` 与 `tar → xattr`；与本轮删除无关，属 advisory 库刷新 | 2026-11-11 `s5-7` 验收 | `cargo deny` 四项 | `cargo update -p rustix` → 1.1.6（`1` 约束内，MSRV 1.88 兼容）；deny 4/4 `ok` | **已修复** |

| D20 | PS5 | `s5-7` 最终套件里 `cancel_stops_descendant_and_returns_cancelled_with_captured_diagnostics` 报 `WorkspaceCleanup`（Windows error 32：被停止的后代仍以工作区为 cwd）；隔离复跑 5/5 绿，说明是满载下的时序，不是删除异步引入 | 2026-11-11 `s5-7` 最终验收 | `cargo test --all-features` | `fail_cleanup` 改用 `TempDir::keep()` + 有界重试（6 次，10→80 ms）；清理仍是 best-effort，主结果不被转成功，`WorkspaceCleanup` 仍可能如实出现 | **已修复（附边界）** |
| D21 | PS6 | §10.4.1 设计稿写「Windows 只读目标 → `OccupiedTarget`」，实现时改为不预检、由 `MoveFileExW` 的 `ERROR_ACCESS_DENIED` 走 `Io`。原方案需要在「只读 = 拒绝」与「ACL 拒绝 = 同一码」之间猜，且引入预检窗口 | 2026-11-11 P1 实现 | §10.4.1 判据表 | 按实际行为记入 §10.4.1「实现与验收」；从不清只读位、目标原状不变两条不变 | **已改写设计** |
记录规则：偏差一旦出现就占一行，即使已修复也保留；不删除、不改写历史行。

### 10.7 状态索引与一致性检查

| Stage | 报告状态（§10） | 计划状态（§3） | 一致 |
| --- | --- | --- | --- |
| PS4 | `ACCEPTANCE OPEN` | `ACCEPTANCE OPEN` | 是 |
| PS5 | `LANDED`（stage 级，§10.3.9；任务 6/6） | `LANDED` | 是 |
| PS6 | `PLANNING ONLY` | `PLANNING ONLY` | 是 |
| PS7 | `NOT PLANNED` | `NOT PLANNED` | 是 |

复核：报告中的实际观测均带命令与输出要点，未复制计划预期值；`s5-1`–`s5-6` 6/6 任务级
`LANDED`，各有逐条验收门结果；PS5 于 2026-11-11 按 §10.3.9 收口为 stage 级 `LANDED`。2026-11-11 后 G6
由 Windows + WSL2 Linux 双平台实测关闭（§10.3.3，D13）；历史报告 §10.3.2 中「G6 OPEN」是
当时的状态记录，不改写，由 §10.3.3 收口。2026-11-11 追加的 §10.3.4 / D14 撤回 `hostile-env`，
当前验证入口收敛为 `cargo test`。§10.3.7 的 archive 接缝与 §10.3.8 的横向审计均已执行/记录，
§10.3.10 把 §10.3.8 混在一列的两条轴拆开（结论：无包装、异步侧是第二份接驳、`same-file` 为死重并已
移除），不修改历史行；§10.3.11 执行其 R1/R2（`s5-7` 单后端化），feature 13 → 11，`reqwest` / `tokio` 离开依赖图。状态索引与 §3、§4.1、`.agents/plan/README.md` 一致。

审计限制：fresh-context 独立复核不可用且未授权，当前为 self-review against fresh repository
evidence；交接时必须说明这一限制。
