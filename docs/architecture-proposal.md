# Herdr Hub 技术方案与可行性记录

状态：G1 方案与本机可行性已通过双 reviewer 验收，当前采用 iroh 路线；
后续目标环境验证若发现不能满足要求，重新评估，不降低需求。
本文中的控制面与产品功能仍是设计，不是已实现能力。验收证据见 `docs/acceptance/g1.md`。
没有创建 Azure 资源，没有连接外部 P2P/relay 服务，没有改动既有连接方案。

## 已核实的基础

| 项目 | 观察与证据 | 结论边界 |
| --- | --- | --- |
| Rust | 初始未找到工具链；用户授权后已安装 rustc/cargo 1.99.1、rustfmt、clippy，未改 PATH | 当前十一项本机探针及 fmt/clippy 已通过，双 reviewer 复验完成 |
| Windows 构建工具 | `vswhere` 查询到带 x64/x86 C++ 工具的 Visual Studio 安装，已完成 Rust 探针编译链接 | 尚未验证发布构建及其他平台构建 |
| Herdr 自动化 | 已读取 `hc --skill`、`herdr-app --skill` 和 App 命令列表 | 用 hc 启动、等待、读取子 agent；由 App 展示这些会话。不是旧 Herdr TUI |
| reviewer | 两者均在 Herdr 子会话交互运行，限定只读工具，三轮报告均已读取处理 | 首轮未通过，补证据及工具修复后，最终两份报告均认可 G1 通过 |
| hc 连接 | 已安装 hc 的 `connect --help` 提供 `--session`；`machine add --help` 列出 `--via` | 参数帮助不是 Windows 原生发起端可用性的证明；首版仍限定 macOS/WSL2 |
| 既有适配器 | 只读查看其 bootstrap 校验与 `createBridge`：严格匹配连接命令并映射固定 `hc connect`；支持半关闭和背压 | 旧 bootstrap 只证明其对应版本格式；新版本须采集隔离环境的真实输出再支持 |
| WSL bootstrap | 独立 WSL2 普通用户的官方 hc 0.1.19 已实际采集三种会话格式，脚本退出 0 并清理独立 core | [样本及严格接入边界](hc-bootstrap.md)；尚非真实 hc 数据桥接，macOS 未采集 |

Herdr App 当前 workspace 仍保存迁移前的启动目录。
后续 reviewer 显式指定当前仓库目录，不依赖该旧默认路径，不重启 App/core。

## 候选传输比较

| 方案 | 已有能力 | 需要补足的部分 | 初步取舍 |
| --- | --- | --- | --- |
| iroh + 自托管 iroh relay | 端点密钥身份、端到端加密 QUIC 流、NAT 穿透、WebSocket relay、路径观察 | 应用授权、会话约束、撤销、App Service 反向代理兼容性、禁用公共基础设施 | 优先做本地可行性验证，尚未选定 |
| str0m/WebRTC + 独立 WSS 中继 | ICE、DTLS、可靠 DataChannel；成熟的 Sans I/O 实现 | 驱动 I/O/定时器、候选发现、消息到流适配、背压与方向性 EOF；WSS fallback 需独立端到端安全协议 | 可行备选，但流适配和两条安全传输路径的实现面更大 |

候选 iroh 版本暂以官方已发布的 `v1.4.1` 为调查基线，不跟随 main 漂移。
选择库不代表批准使用其公共 relay、地址发现服务或其他外部基础设施。

## iroh 必须先证明的条件

1. **完全私有的配置**：从 empty builder 开始，只配置明确允许的地址与 relay，
   不隐式启用公共地址发现或默认公共 relay。
2. **WebSocket 代理兼容性**：relay 协议支持 WebSocket，但这并不证明穿过
   App Service 的 TLS 终止、升级头转发和超时机制后仍可用。
   先做本机反向代理实验；云端部署验收仍需用户授权。
3. **授权不是库自动完成**：relay 默认 `AllowAll` 不能直接使用。
   `AccessControl::on_connect` 可验证已由握手证明的端点身份，但它是连接级钩子，
   不能据此声称已经实现“设备 A 只能与设备 B 的某个会话通信”的授权。
   必须核实配对限制、连接撤销和到期终止的实现路径。
   若现有库接口不足，须调整设计或选型，不能悄悄放宽 R3。
4. **连接启动与迁移**：用户已确认先经 relay 立即可用，再自动提升为直连。
   由 iroh/QUIC 维护同一连接的顺序、重传与迁移；仍须在 G5 验证
   业务数据连续性和实际路径，不能以“库支持”代替验收。
5. **地址发现基础设施**：当前版本的 relay 可选 QAD/QUIC 地址发现端点。
   App Service 不能承载此类普通 UDP 端点；关闭它后的公网 NAT 穿透能力、
   是否需要额外获批的地址发现设施，必须验证，不能将 QAD 与 STUN 混为一谈。
6. **路径与生命周期**：分别测试强制直连、强制 relay、单向 EOF、
   持续背压、取消及网络断开。库的迁移能力也必须经过适配层验证后才可承诺。

## 候选应用结构

- 一个 Rust hub 服务：设备与权限持久化、控制连接、最小管理页面和私有 relay 接入。
- 一个 Rust agent 程序：支持发起端与远端角色；首版命令行管理，TUI 后续增加；
  单独的无交互 `connect` 模式作为 hc 外部适配器。
- 业务流只连接预配置的 hc 会话，不提供任意目的 TCP 转发或 shell 执行协议。
- 使用成熟协议库处理密钥握手和数据加密；控制面登录与会话授权仍需明确设计。
- 单实例起步，暂不引入 Redis/Kubernetes；单管理员自用、设备手动审批已确认；
  登录采用 GitHub OAuth；拟用 SQLite 保存授权数据，管理网站细节在 G3 实现。

TUI 的状态和配置能力不能混入 `connect` 的 stdout，也不替代服务端管理网站。
用户已确认首版不交付 TUI，后续增加时另行确定交互范围。

## 候选授权与生命周期设计

这些是后续实现约束，当前探针没有实现完整控制面。

1. **管理员**：GitHub OAuth authorization-code 登录，固定回调 origin、
   一次性 state、HttpOnly/Secure 会话 cookie，写操作校验 CSRF。
   将 GitHub API 返回的稳定用户 ID 与预配置 owner ID 比较；
   不按可变用户名授权，不将 token 放进日志或版本控制。
2. **设备**：agent 在本机生成持久设备密钥，注册进入待审批状态；
   配对页面展示公钥指纹。控制连接使用新鲜挑战证明持有设备私钥，
   服务端检查已审批且未撤销。重复挑战、超时证明和未知设备拒绝。
3. **连接授权**：先检查发起端、远端和目标固定 hc 会话的权限，
   再让双方生成本次连接的临时传输密钥。服务端将临时公钥、稳定设备 ID、
   会话、连接 ID、有效期和授权版本绑定到经过认证的授权消息中。
   端点验证预期对端的临时公钥，不仅凭地址或设备名信任对端。
4. **中继配对隔离**：候选方案为每个获批连接分配独立的 `RelayService` /
   客户端注册表，在同一 HTTP 服务下按公开连接 ID 路由。
   实验发现发布的 1.4.1 客户端会将路径覆盖为 `/relay`，不能使用路径前缀分流；
   改用其原生支持的 Authorization 请求头携带连接授权并选择配对实例。
   每个实例只接收该授权的两个临时端点，不能跨实例转发。
   知道路由标识并不足以获准；握手仍必须证明被授权的私钥。
   必须限制同时存活的 relay 实例、连接数、队列及有效期。
5. **一次使用**：授权 ID 原子消费；远端只接受该连接的一条 hc 业务流，
   不执行任意命令，不允许重复打开 hc 子进程。握手完全完成前不启动 hc，
   不用 0-RTT 承载有副作用的授权或 hc 数据。
6. **撤销与过期**：控制面通知双方关闭对应 QUIC 连接并撤销 relay 准入，
   relay 主动断开已注册客户端。只断开 relay 不足以终止已经直连的 QUIC，
   所以远端还必须验证短期连接租约；控制面失联时到期关闭，不影响 core 本身。
7. **数据与恢复**：同一 iroh/QUIC 连接先经 relay 可用，再自动提升为直连。
   同一连接内的可靠性由库负责；真正断开则由 hc 重新连接原 core，
   不重放已发送的应用字节，也不重启原会话。

候选限制：每条连接一个临时端点和独立 relay 注册表有资源开销，
适用于首版小规模自用；具体上限、期限和压力验证在相应阶段确定。
本地探针需验证按请求头分流至多个独立 relay 的可行性，不能仅凭此设计通过 G1。
探针中的 `pair-a` / `pair-b` 仅为非秘密路由标识，不是产品授权票据。

WSL2 的 NAT / mirrored 模式、候选地址、QAD 依赖、代理与验收矩阵见
[WSL2 网络专项分析](wsl-networking.md)。该分析不等于已实测。
用户已授权安装本机专用 WSL2 发行版，使用独立 Linux 用户和 hc 配置/会话采集 bootstrap；
不复用或重启当前 Windows core。

## G1 本地实验计划

| 实验 | 限制 | 通过条件 |
| --- | --- | --- |
| 工具链编译 | 用户级安装，不提权，不改系统 PATH 或现有 hc | 在本机完成 Rust 编译、链接和聚焦检查 |
| 纯本机直接流 | 仅 loopback，无公共发现/relay | 两个独立端点传输已知二进制内容，双向半关闭后完整收尾 |
| 私有本机 relay | 禁用 UDP 数据路径，只允许本机 WebSocket relay | 业务经过 relay 且端点身份认证、加密、流控制正常 |
| relay 准入 | 未授权端点与授权端点分别尝试 | 未授权拒绝；不能只验证合法流 |
| 配对隔离 | 同一 HTTP 端口下通过请求头选择独立 relay | 获批端点可在自身配对传输，使用其他配对标识时被拒绝 |
| 代理及撤销可行性 | 本机 HTTP/WebSocket 代理，不部署 Azure | 升级与握手可用，授权更新对已有连接的处理路径有证据 |
| hc 格式核对 | 独立配置/会话，不改真实 machine 配置 | 采集受支持版本的真实 bootstrap，明确严格匹配规则 |

这些实验仅是可行性证据，不替代 G5 的真实两端 P2P 或 G7 的 macOS/WSL2 验收。
任何关键条件失败或仍无法证明，G1 不通过，先修正方案或记录阻塞。

## 待确认决策

- 官方用户级 Rust 工具链安装已授权，不提权、不修改系统 PATH。
- 已确认优先验证 iroh + 私有 WebSocket relay；失败时再比较 WebRTC 路径。
- 首版单管理员自用、设备手动审批、GitHub OAuth 登录已确认；实际应用注册和部署配置尚未授权执行。
- 先经中继立即可用、可直连时自动切换已确认，不采用等待直连才开始业务的策略。
- 撤销设备资格时关闭其已有连接并阻止重新连接；失联时短期租约到期关闭。
- 首版 CLI、TUI 后续增加已确认；管理网站仍保留。

## 官方依据

- [iroh v1.4.1 release](https://github.com/n0-computer/iroh/releases/tag/v1.4.1)
- [iroh Builder：empty、relay mode、address lookup](https://docs.rs/iroh/1.4.1/iroh/endpoint/struct.Builder.html)
- [iroh relay 协议路径与 WebSocket 协商常量](https://github.com/n0-computer/iroh/blob/v1.4.1/iroh-relay/src/http.rs)
- [relay 的连接级访问控制](https://docs.rs/iroh-relay/1.4.1/iroh_relay/server/trait.AccessControl.html)
- [ClientRequest 的端点身份证明](https://docs.rs/iroh-relay/1.4.1/iroh_relay/server/struct.ClientRequest.html)
- [relay 默认配置与 AllowAll](https://docs.rs/iroh-relay/1.4.1/iroh_relay/server/struct.RelayConfig.html)
- [iroh relay 与 QAD 配置](https://docs.rs/iroh-relay/1.4.1/iroh_relay/server/struct.ServerConfig.html)
- [str0m Sans I/O 与应用驱动职责](https://docs.rs/str0m/latest/str0m/)
