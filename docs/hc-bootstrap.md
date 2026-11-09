# hc bootstrap 接入约定

## 已采集的证据

在用户授权的独立 WSL2 Ubuntu 24.04 测试发行版内，以普通 Linux 用户运行官方
hc 0.1.19，实际调用 `hc machine add --via`。采集器记录 hc 传给连接器的最后一个参数，
不求值、不执行该命令、不连接真实对端。

- [采集脚本](../experiments/transport-probe/scripts/capture_hc_bootstrap.py)
- [三个真实样本](../experiments/transport-probe/fixtures/hc-0.1.19-wsl2-bootstrap.json)
- 场景：默认远端会话、普通命名会话、带点与连字符的命名会话。
- 连接器故意返回 255；hc 返回 `unreachable` 是预期的采集结果，不是传输成功。
- 环境只保留明确设置的 Linux PATH、HOME、HC_SESSION 和 XDG 目录；
  各场景的输出路径通过连接器固定参数传递，不依赖已启动 core 的环境刷新。
- 采集后只停止该独立 Linux core，删除该次临时目录。检查未发现遗留 Linux hc 进程，
  Windows core 的进程与 holder 均未改变。

WSL 当前网络模式为 NAT；此次采集不使用网络，也不构成 NAT 穿透验收。
macOS 尚未实际采集，G7 仍须验证该平台；不能将 WSL 样本标为 macOS 实测。

## G2 应实现的严格边界

样本中的外层 `sh -c`、可执行文件发现、缺失提示和最终 `exec ... connect` 都作为
固定格式进行识别，而不是当成 shell 脚本执行。

允许的变化只有已采集格式中的可选 `HC_SESSION=NAME`：

- 未提供时映射 `default`。
- 提供时只接受明确限定的会话名字符集合，并与 agent 预配置的目标会话精确比较。
- 匹配成功后执行固定的本地 hc 可执行文件和固定 argv：`connect --session NAME`。
- 拒绝额外语句、换行、命令替换、重定向、安装/更新探测、未知格式及会话不匹配。
- 捕获程序版本不等于连接器收到的文本带版本号；该文本没有版本字段。
  因此应维护已验证的格式/版本矩阵，未知形状 fail closed，不能假装从文本自动识别版本。
- 接口 stdout 仅传 hc 二进制协议；所有诊断在 stderr。

真实 Windows 主机的 `hc connect --session`、子进程清理、断线后原会话存续及
两个发起平台的实际端到端接入，仍属于 G2/G7，不是此次采集已经完成的能力。
