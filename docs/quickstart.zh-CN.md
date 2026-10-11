# Herdr Relay 首次安装与连接

用 Mac 连接 Ubuntu 上的 Herdr，也可以反过来。普通使用只需要三个入口：
`herdr-relay serve`、`herdr-relay pair` 和 `herdr-relay`。

## 1. 两端安装

两台电脑都安装 [Herdr 0.9.3](https://github.com/herdrdev/herdr/releases/tag/v0.9.3)
和 Herdr Relay。

从 [GitHub Releases](https://github.com/hackerchai/herdr-relay/releases/latest)（国内建议用[下载站](install.md#download-site)）
下载对应系统和架构的压缩包，核对校验和、解压后运行 `./install.sh`。
二进制安装不需要 Rust、Node 或 GitHub 登录；具体步骤见[安装指南](install.md)。

如果已经安装 Rust 1.99 或更新版本，也可以执行：

```sh
cargo install herdr-relay --locked
```

确认命令可用：

```sh
herdr --version
herdr-relay --version
```

## 2. 在 Ubuntu 启动共享

如果使用 OrbStack，可以先在 Mac 运行 `orb -m herdr-relay-demo` 进入演示虚拟机。
在 Ubuntu 启动 Herdr：

```sh
herdr
```

按 **Ctrl-B，松开，再按 Q** 返回 shell，Herdr 会话继续运行。然后启动共享：

```sh
herdr-relay serve
```

用方向键移动，**Space** 勾选要共享的会话，**Enter** 应用。可以多选；
裸命令 `herdr` 启动的会话显示为 `default`。

Relay 自动保存配置、安装并启动后台服务。新安装默认开启登录自启动，
设置界面可以按 `a` 切换。选择复制或打印配对链接，把完整链接带到 Mac。
退出设置界面后，共享继续运行。

链接只能使用一次，10 分钟后过期。配对时 Ubuntu 的 Relay 服务需要在线。

## 3. 在 Mac 配对并连接

```sh
herdr-relay pair
```

粘贴完整链接并按 Enter，随后按主机名和会话名称选择连接。
不用填写设备 ID、target ID 或 `--profile`。

连接后显示的是 Ubuntu 上的原生 Herdr 界面，可以在空闲 shell pane 中运行
`codex` 或 `claude`。这些工具需要另外安装，并完成各自的账号登录。
Mac 只作为连接端时，无需安装 Relay 后台服务。

## 4. 日常使用

以后连接只需：

```sh
herdr-relay
```

这与 `herdr-relay connect` 相同，会打开主机和会话选择菜单。
按 `?` 或 F1 看帮助，`/` 搜索，`r` 刷新，Esc 返回；主机菜单按 `a` 可以改本地别名。

在 Herdr 内按 **Ctrl-B，松开，再按 Q**，返回 Relay 菜单。
远端会话和 agent 继续运行。同一个会话同时只允许一个 Relay 客户端；
显示忙碌时，先断开之前的连接。

## 5. 后台服务与升级

在 Ubuntu 管理共享服务：

```sh
herdr-relay service status
herdr-relay service stop
herdr-relay service start
```

macOS 支持登录自启动。Linux 还可以用 `herdr-relay service autostart boot`
启用开机启动和退出登录后继续运行，系统可能要求授权。
Relay 不会自动创建 Herdr 会话；重启后重新启动原来的 Herdr 会话，共享会恢复。

两端分别升级 Relay：

```sh
herdr-relay update
```

升级无需 GitHub 登录。活动连接存在时，后台服务的新版本会推迟启用，
可以在方便时停止并重新启动服务。详见[安装、服务与升级](install.md)。
