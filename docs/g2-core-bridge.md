# G2：本地 hc 桥接模块

状态：实现及两平台自动/真实 hc 验证已完成，已通过[双 reviewer 验收](acceptance/g2.md)。

## 本阶段交付边界

- 新增原生 Rust 桥接库，以异步输入/输出流连接固定的 hc 可执行文件和固定会话。
- 新增 `herdr-hub-agent local-bridge` 开发命令，供隔离的 `hc --via` 验收使用。
  它将当前机器的一个独立 hc 会话暴露给调用它的父进程，不是远程传输或完整 agent。
- 不监听 TCP/UDP，不做设备注册、身份存储或控制面，不把本机管道当成 P2P。
  后续阶段将已认证的 iroh 流接到同一桥接模块，不另写一套字节转换逻辑。
- bootstrap 只作严格匹配；不执行它。只启动配置好的 `hc connect --session NAME`。
- 无交互、不启动 TUI；stdout 仅为协议，错误和子进程诊断走 stderr。

会话名暂限定 1–64 个 ASCII 字符，首字符为字母、数字或下划线，
其余允许字母、数字、下划线、点和连字符，避免选项或路径解释。
固定 hc 路径必须为绝对路径；不接受网络对端提供可执行文件路径。

## G2 验收标准及证据计划

| 标准 | 预期证据 |
| --- | --- |
| 支持已采集的当前 bootstrap | G1 三例 fixture 驱动的解析测试；默认、命名及显式 default 会话 |
| 拒绝未知命令或会话 | 修改/附加 shell、安装/更新、会话不匹配、非法会话名均拒绝，且不启动子进程 |
| 二进制与双向 EOF | 原生测试子进程读写覆盖所有字节值，输入 EOF 后仍收取最后响应 |
| 背压与取消 | 有界异步复制；暂停消费时不能无限缓存，取消后子进程退出并被回收 |
| 输出结束但输入仍开着 | CLI 不因阻塞 stdin 线程而挂住，保留并排空已产生输出 |
| 子进程失败 | 保留退出状态，诊断不进入协议 stdout |
| 真实 hc 协议 | 在独立 Windows/WSL hc 中分别验证 plain JSON-RPC 与二进制 framed 请求 |
| 会话存续 | 创建专用测试终端，断开并重连桥接后，core 身份/进程和原终端仍存续 |
| 平台边界 | Windows 不调用其不支持的 `machine add --via`；WSL 使用 Linux hc 和 Linux 桥接二进制进行实际接入 |
| 阶段审查与发布 | Herdr App 中 Copilot CLI / Pi 并行只读验收；处理结果后 commit、push |

测试使用独立 HC_SESSION、HOME/XDG 目录和明确创建的测试进程，
不停止真实 core、不修改现有 machine 配置、不影响 devtunnel。
macOS 的实际接入、跨机器网络和完整远程 agent 仍未验收。

## 本地构建与测试

```text
cargo build -p herdr-hub-core-bridge --bin herdr-hub-agent --locked
cargo test -p herdr-hub-core-bridge --locked
cargo clippy -p herdr-hub-core-bridge --all-targets --locked -- -D warnings
```

`herdr-hub-test-hc` 是自动测试用子程序，不应安装或分发。
真实 smoke 需显式提供 hc 和桥接可执行文件路径，脚本自行建立并清理专用测试会话：

```text
python real_hc_smoke.py --hc ABSOLUTE_HC_PATH --bridge ABSOLUTE_AGENT_PATH
```

Linux 使用该平台编译的二进制；Windows 不执行 `machine add --via`，
而是通过真实 hc 的 plain/framed RPC 检查固定会话桥接。
