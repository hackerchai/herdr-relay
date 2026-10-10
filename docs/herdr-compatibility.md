# Herdr 0.9.3 本机适配验证

实验日期：2026-10-09；产品解释更新：2026-10-10。结论：**同一 Herdr server 可以接受多个独立 native client socket 连接，但本产品按最新要求限制同实例同时一个活动 Relay 客户端。** 历史实验记录保留不变；独占要求见 [ADR-0012](adr/0012-single-active-client-per-target.md)。 这里验证的是本机协议能力；Rust Relay、Cloudflare 链路和完整原生界面尚未实现或验收。

## 基线和复现

- 上游版本：Herdr 0.9.3，提交 `7b116c05bfda646af39d2524c54e70c751f57ee8`。
- 实测环境：macOS 27.0、Apple Silicon；使用单独下载的官方 `herdr-macos-aarch64`，未替换本机原有 0.8.2。
- 二进制 SHA-256：`5173a3e0ae42d5d1ab7ebfa5d5e6329f7c3d23f8e1a3677c7ce3231da2884157`，与该版本 GitHub release asset 元数据相符。[官方发行版](https://github.com/herdrdev/herdr/releases/tag/v0.9.3)
- [探针脚本](../scripts/probe-herdr-0.9.3.py)仅使用 Python 标准库；[本次运行结果](validation/herdr-0.9.3-macos-arm64.json)保留了版本、平台、摘要和各项检查结果。

```sh
python3 scripts/probe-herdr-0.9.3.py /absolute/path/to/herdr-0.9.3
```

脚本在 `/tmp` 新建独立目录，隔离 XDG 与 Herdr 环境，启动两个专用 server；只访问自己创建的 socket 和进程，最后关闭并清理。不会访问默认 session、改写现有配置或升级系统中的 Herdr。

客户端是依据上游测试构造的 **native protocol 测试端点**，并非启动了两个官方 TUI。测试使用 JSON API 创建临时 pane、读取结果和观测 PTY 尺寸；输入与 surface 更新走 native client socket。因此它能够验证服务器协议的并发基础，不能代替原生界面、终端恢复或公网端到端测试。[上游测试夹具](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/tests/support/mod.rs)

## 结果及产品含义

| 检查 | 本次结果 | 对 Relay 的要求 |
|---|---|---|
| 两个 native 端点同时完成 bootstrap | 通过，均收到 welcome、snapshot 和 surface | 每个远程 attach 建立一条独立的主机 client socket 连接 |
| 第二个较小端点被动接入 | PTY 保持 40 行、119 列 | 接入本身不应由 Relay 强制抢占终端尺寸 |
| 较小端点取得有效焦点 | PTY 变为 24 行、79 列 | 同一 tab 的 PTY 尺寸受原生活动规则控制 |
| 两个端点分别输入 | 两次命令均执行，两端均收到 surface 更新 | 上游能同时操作；本产品须额外限制一个活动 Relay 连接，此结果不作为多人操作的产品验收 |
| 一个端点断开 | server 存活，另一端仍能输入并收取更新；尺寸回到 40×119 | 清理只影响该客户端连接 |
| 两个独立 Herdr server | 第二个实例的输出未出现在第一个实例 | 根据保存的目标定位 socket，不能回退或跨实例广播 |
| 清理 | 本次创建的两个 server 均退出 | 探针没有留下后台 server |

这里的数字是 pane 的 `stty size`，不等于测试端点声明的完整界面尺寸；119/79 列并非测试失败。没有进行同时竞争写入的冲突策略或高负载测试。

## 已核对的源码约束

1. **连接具有独立身份。** accept 循环为每条连接分配 `client_id`；每个客户端具有自己的 shell location。Relay 应让 Herdr 保持这些区分，不能用单条 socket 的输出广播来模拟多客户端。[accept](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/server/client_accept.rs#L11-L53)、[客户端视图](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/server/headless/client_views.rs#L65-L79)
2. **PTY 几何是共享状态。** 同一 tab 由有效活动决定 geometry controller；不同客户端可有不同的视图选择，但不能承诺各有独立 PTY 尺寸。[几何控制](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/server/headless/client_views.rs#L693-L755)、[上游多客户端测试](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/tests/multi_client.rs#L267-L364)
3. **本地初始握手只有 4 秒。** 应先准备好公网和端点认证，再进入本地 native 握手；不能先打开主机 socket 再等待用户配对。[握手限制](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/server/client_transport.rs#L39-L45)
4. **Herdr 帧与 Relay 帧不能一一对应。** 内部协议版本为 22，endpoint 使用独立协商；常规 frame 上限 2 MiB，图形 frame 为 32 MiB，剪贴板图像 payload 为 16 MiB。Relay 应连续分块搬运字节，不解析或缓存整个 Herdr frame。[wire 定义](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/protocol/wire.rs#L1-L35)
5. **socket 选择有优先级。** 显式 session 优先于环境路径，`HERDR_SOCKET_PATH` 又优先于 `HERDR_CLIENT_SOCKET_PATH`。启动本地 native client 时清除继承的冲突变量，不能将远端 session 当作本地 `--session` 传入。[路径](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/server/socket_paths.rs#L12-L56)、[官方远程 client 启动](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/remote/attach.rs#L3324-L3345)
6. **本地桥接断开需要包装器负责恢复。** 非 federated 的本地 endpoint 断开会返回 `ConnectionLost`；设置 reattach 提示本身不实现 Relay 自动重连。应关闭旧 native 连接，让受监督的 client 退出，再以新连接重新启动；原生终端清理仍需实测。[断线处理](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/client/mod.rs#L2146-L2185)
7. **Relay 自己的有界队列不能证明整个 Herdr 内存有界。** 上游 `send_control` 将控制消息加入 `VecDeque`，此路径未见容量检查；render 路径另有单槽限制。需要专门测量慢端对上游控制队列的影响，必要时向上游修复；不能靠本项目的窗口数值宣告 R12 全面通过。[写队列](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/server/client_transport.rs#L310-L355)

## 尚未验证

| 项目 | 状态 / 下一步 |
|---|---|
| 官方 TUI 输入、resize、Unicode、粘贴、鼠标、图形和退出恢复 | 需要原生 client + Rust 本地桥接实测 |
| 已命名 session 重启后重新发现、沿用共享配置 | 本次只测了两个独立 socket，未测持久 session 重启 |
| Mac Intel、Linux x86_64 / arm64 | 上游有对应发行资产；本次未在这些机器运行 |
| 最低系统版本及 Linux 发行版范围 | 尚未承诺；arm64 macOS 二进制 load command 的 minOS 为 11.0，不等于已验证 11.0 |
| WSS、DO 休眠、端到端加密、配对与撤销 | 未实现，未部署，未进行云端测试 |
| 延迟、吞吐、慢端、资源与费用 | 未压测，不能给出已达标数值 |

跨语言协议草案见 [protocol/README.md](../protocol/README.md)，实施路径与发布门槛见 [implementation-plan.md](implementation-plan.md)。
