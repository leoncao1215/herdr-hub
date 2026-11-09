# G1 第二轮复验材料

状态：本机补充验证完成，等待两个 reviewer 独立复验。尚未签发 G1 通过。
本文件只提供当前证据，不包含另一 reviewer 的判断。

## 本轮变化及执行结果

1. 已补充 [WSL2 NAT / mirrored 分析](../wsl-networking.md)，区分设计分析和实际数据路径验证。
2. 用户授权后安装独立 WSL2 Ubuntu 24.04，使用普通 Linux 用户与独立 HOME、
   HC_SESSION、XDG 目录运行官方 hc 0.1.19。[三例真实 bootstrap](../hc-bootstrap.md)
   已采集；连接器只记录参数、故意退出 255，hc 的 `unreachable` 是预期。
   采集脚本退出 0，独立 Linux core 停止，临时目录移除。
   额外检查未发现遗留 Linux hc，原 Windows core/holder 进程未改变。
3. 配对隔离实验现在经过 **WSS 客户端 → TLS 终止 → Hyper HTTP 解析/转发 →
   WebSocket Upgrade 双向转发 → 两个独立 relay 注册表**。
   两组合法二进制流、错误组身份拒绝和跨组连接失败均走此链路。
   这仍不是 Azure 的网关、超时或云端部署验收。
4. 新增本地取消和对端终止用例：先确认握手及应用 ready，
   在背压写入和等待更多数据的读取均处于 pending 时关闭连接，
   断言读写退出且关闭操作在期限内完成。
   另有真实 relay 退出用例：业务 ready 且写入受背压后停止仅承载此测试的 relay，
   不主动关闭任一 QUIC 对端，验证底层路径消失后读写在测试 idle timeout 内失败并清理。
   三秒 idle timeout 仅用于该实验，不是产品默认超时决策。
5. 实验代理不再因单个客户端取消探测/缺少 TLS close_notify 就结束整个监听循环；
   单连接错误记入 stderr，任务 panic 仍传播。测试主结果不再被后续清理错误覆盖。

```text
cargo fmt --all -- --check: exit 0
cargo test --workspace --locked -- --nocapture:
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.03s
cargo clippy --workspace --all-targets --locked -- -D warnings: exit 0
WSL2 capture_hc_bootstrap.py ABSOLUTE_LINUX_HC_PATH: exit 0
Recorded bootstrap JSON matches actual capture: yes
```

Rust/cargo 1.99.1，Windows MSVC；WSL2 网络模式 NAT。
十个 Rust 探针仍在 Windows loopback 上运行，**没有声称已在 Linux 跑过这些传输探针**。

## 当前复验标准

| 标准 | 当前证据 | 不得扩大的结论 |
| --- | --- | --- |
| 二进制与半关闭 | 原始 direct/relay 两例，1 MiB 请求，EOF 后反向完整响应 | 不是 hc 数据桥接 |
| 无 IP transport 的私有 relay | 清除双端 IP transport，只有 loopback relay | 不是公网兜底或 NAT 穿透 |
| 未审批身份与撤销 | 身份拒绝、主动断开、重连拒绝、客户端不能 online | 不是完整审批、租约或直连撤销实现 |
| 背压 | 小窗口下写入阻塞，消费后完整恢复 | 不是产品内存/容量上限 |
| TLS 终止 | 显式测试根证书，不跳过验证 | 不证明企业证书/代理环境 |
| HTTP/WebSocket 代理与配对隔离 | `scoped_relay_routes_do_not_admit_another_pairs_endpoint`、`support/http_proxy.rs` | 只验证本机 Upgrade/授权头保留与路由；跨组失败是有界观察 |
| 本地取消、对端终止及 relay 消失 | `support/lifecycle.rs` 三例；有界等待读写错误和 endpoint 关闭 | 不证明真实网络丢包、所有网络故障或 hc 子进程清理 |
| 成熟协议与绑定方案 | 固定 iroh 版本、端点身份证明、按授权独立注册表的设计 | OAuth、一次使用授权、期限和租约仍待实现 |
| WSL2 与基础设施约束 | `docs/wsl-networking.md` | NAT/mirrored 差异已分析，不代表两种模式已跑数据传输 |
| 当前 hc 输入格式 | `capture_hc_bootstrap.py` 与 hc 0.1.19 的三个真实 fixture | 仅 WSL2/Linux；不把同为 Unix 的 macOS 当作已实测 |

## 新增材料

- `experiments/transport-probe/tests/support/http_proxy.rs`
- `experiments/transport-probe/tests/support/lifecycle.rs`
- `experiments/transport-probe/scripts/capture_hc_bootstrap.py`
- `experiments/transport-probe/fixtures/hc-0.1.19-wsl2-bootstrap.json`
- `docs/wsl-networking.md`
- `docs/hc-bootstrap.md`

其余仍为原工作计划、需求、方案、manifest、lockfile 及 transport/support 源文件。
采集器是实验工具，故意让连接失败，绝不执行捕获的 shell 内容。

## 当前代码 SHA-256

| 文件 | SHA-256 |
| --- | --- |
| `Cargo.lock` | `3BCDA4797D0F30DD29A4BF95F12C894ADB9885A2FE5471F1B2B56D94E705DF62` |
| `experiments/transport-probe/Cargo.toml` | `6E67098575460E78FCA952CA0CA7FABE5A177F38189EB29C048667E0601C9F1A` |
| `experiments/transport-probe/tests/transport.rs` | `31DE7D2DF7DA5E9C1377535389B5F2B13FCACBF46B432B6EFC7389D6443B70EA` |
| `experiments/transport-probe/tests/support/mod.rs` | `80DEBA1C10F7838EED3993184014D6DCDEFA446DD61D3DD7ADFB047789F5A7B7` |
| `experiments/transport-probe/tests/support/http_proxy.rs` | `620A6276FF6F55D0D1CF568146B3C390D763723A654292E976C808629B11707A` |
| `experiments/transport-probe/tests/support/lifecycle.rs` | `28623AAB0A9588B0A04F7D2C87DE1BCB88C20FB66FEEC6EA2BD6703B1ACC1E32` |
| `experiments/transport-probe/scripts/capture_hc_bootstrap.py` | `259F58C1A4763FF26B7C19192D9F53812C5617E8A71854F07E45EF22A8C8C94A` |
| `experiments/transport-probe/fixtures/hc-0.1.19-wsl2-bootstrap.json` | `3DE786F622ED327A0C56BBB4C168B53D86DBDDE94F978BF80A9AF8393CDFDA60` |
