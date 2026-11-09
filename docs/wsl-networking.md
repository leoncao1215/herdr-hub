# WSL2 发起端的网络风险与验收计划

状态：G1 网络设计分析，网络路径尚未实测。已在独立 WSL2 NAT 环境采集 Linux hc 0.1.19 bootstrap，
但尚未运行 WSL2 的 iroh 数据传输。首版发起端运行 Linux hc 与原生 Linux agent，
不能用原生 Windows hc 或 Windows loopback 探针替代验收。

## 两种网络模式

| 模式 | 地址与可达性 | 对 Herdr Hub 的影响 |
| --- | --- | --- |
| 默认 NAT | WSL 虚拟网卡具有独立私网地址。外部网络通常不能直接路由到该地址；Windows 宿主机及其上游网络可能形成多层 NAT | WSL 的本地候选不能当作公网地址。需要实测双端 NAT 映射与 UDP 穿透；映射不可利用或 UDP 被阻止时保留中继 |
| Mirrored | 在支持的 Windows/WSL 版本上镜像宿主网络接口，改善 IPv6、VPN 和 LAN 访问；仍受宿主与 Hyper-V 防火墙约束 | 可能减少一层 NAT，但不保证公网路由或企业网络允许 UDP。必须区分接口候选、VPN 地址和真正可达的 IPv4/IPv6 路径 |

`127.0.0.1` 的宿主/WSL 互通只是本机互通，不是跨机器 P2P 证据。
网络模式、WSL 版本与 VPN 状态必须随测试证据记录，不能只写“Windows 成功”。
首版不自动修改 `.wslconfig`、添加 portproxy 或放宽 Windows/Hyper-V 防火墙。

## 地址发现与中继

- iroh 的本地接口候选和额外公共地址发现是不同信息来源。
- G1 loopback 探针关闭 QAD，能证明直接/中继流 API 可用，
  **不能证明关闭 QAD 时仍可穿透 WSL 的多层 NAT**。
- App Service 只承载 HTTPS/WebSocket，不部署普通 UDP QAD/STUN。
- 若目标网络中的 P2P 需要额外公共地址发现，须先获得用户对独立 QAD 基础设施
  或其他适配方案的授权。未经批准不使用默认公共 relay/发现服务。
- 无可用公共映射、对称 NAT、VPN 路由或 UDP 策略均可能使直连失败；
  按已确认策略先通过私有 WSS relay 使用，再尝试直连，不能谎报为直连。
- 业务端点仍需校验授权中的临时公钥和租约；候选地址本身不构成身份认证。

## 代理与证书

WSL 的 DNS tunneling、`autoProxy` 和企业代理策略可能影响 hub 与 relay 的 HTTPS 连接。
不能假设 Windows 的代理/信任设置自动适用于 Linux Rust 进程。
实现阶段需显式说明代理配置与证书信任来源，验证 WSS 升级头和授权头能通过代理；
不通过关闭证书校验、绕过代理或修改企业策略“修复”连接。

## G5/G7 授权后的验证矩阵

| 场景 | 必须观察的结果 |
| --- | --- |
| WSL2 NAT，网络允许 UDP | 记录候选类别、是否启用获批 QAD、实际路径和失败原因；只有真正产生直连业务数据才计为 P2P 成功 |
| WSL2 mirrored，网络允许 UDP | 分别识别可达地址族和接口；证明业务路径，而不是只证明 LAN 或 localhost 连通 |
| UDP/直连不可达 | 中继连接可用，状态准确反映 relay，不强迫更改网络策略 |
| HTTPS 需要代理 | 控制面、WSS 升级及请求头选路能工作；认证或代理拒绝时明确失败 |
| 中继已可用后产生直连 | 同一业务流完整、有序、无重复，路径观测确实从 relay 改为 direct |
| 网络切换或 WSL 重启 | 能按策略重连原远端 core，不重启原有终端和 agent；本地进程已退出时不误报在线 |

每种首版发起端都要有自身的真实 hc bootstrap 与端到端证据。
若没有相应设备或授权，该项保持未验收。用户已单独授权本机专用发行版的安装；
这不授权修改网络策略或借用用户现有连接做故障注入。

## 依据

- [Microsoft：WSL 网络、NAT、mirrored、DNS tunneling 和 autoProxy](https://learn.microsoft.com/en-us/windows/wsl/networking)
- [Microsoft：WSL 配置选项](https://learn.microsoft.com/en-us/windows/wsl/wsl-config)
- [Microsoft：Hyper-V 防火墙](https://learn.microsoft.com/en-us/windows/security/operating-system-security/network-security/windows-firewall/hyper-v-firewall)
- [iroh 1.4.1：RelayConfig 的可选 QAD](https://docs.rs/iroh-relay/1.4.1/iroh_relay/struct.RelayConfig.html)
