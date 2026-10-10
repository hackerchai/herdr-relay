# Herdr Relay

[English](README.md) · [简体中文](README.zh-CN.md)

![Herdr Relay — 随时随地，从任意电脑访问你的智能体。](assets/promo/herdr-relay-banner.png)

通过原生终端界面，从另一台电脑访问你的 Herdr 会话。Herdr Relay 支持 macOS 和 Linux，
提供加密配对、多会话共享，以及支持自动重连的后台共享服务。

## 安装

在两台电脑上安装 [Herdr 最新版](https://github.com/herdrdev/herdr/releases/latest)。
Herdr Relay 已在 Herdr 0.9.3 上通过测试。

从[公开的二进制发行版](https://github.com/hackerchai/herdr-relay-releases/releases/latest)
下载 Herdr Relay，或使用 Rust 1.99 及更新版本安装：

```sh
cargo install herdr-relay --locked
```

## 快速开始

在需要被远程访问的电脑上，启动 `herdr`，然后按 **Ctrl-B，松开，再按 Q** 返回 shell，
Herdr 会话会继续运行。接着启动共享：

```sh
herdr-relay serve
```

按 **Space** 勾选会话，按 **Enter** 确认，然后复制配对链接。
后台服务会自动安装并启动。

在另一台电脑上运行：

```sh
herdr-relay pair
```

粘贴链接，然后选择主机和会话。以后连接时，只需运行 `herdr-relay`。
使用默认托管的 Relay 无需注册账号，也无需配置 Cloudflare。

## 使用演示

按名称选择共享的会话，即可连接到其中的智能体工作区。

![选择共享的 Herdr 会话](assets/promo/06-select-session.jpg)

![从 macOS 连接到 Ubuntu 工作区中的 Codex 和 Claude Code](assets/promo/09-connected-workspace.jpg)

*终端画面根据真实演示输出渲染。*

## 使用指南

- [首次连接：中文教程](docs/quickstart.zh-CN.md) · [English](docs/quickstart.md)
- [安装与后台服务](docs/install.md)
- [升级与自动启动](docs/quickstart.zh-CN.md#5-后台服务与升级)
- [自行托管 Relay](docs/self-hosting.md)
- [Relay 开发](relay/README.md) · [协议](protocol/README.md)

## 许可证

[MIT](LICENSE)。二进制发行版和 [crate](https://crates.io/crates/herdr-relay) 公开发布；
源代码仓库同样公开。
