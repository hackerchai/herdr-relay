# Herdr Relay 首次安装与连接

这份教程使用当前公开版本支持的原生命令：Ubuntu 是被连接的电脑，Mac 是操作端。全程不需要 `--profile`、`--origin` 或 `--herdr`，也不需要 GitHub 权限、Cloudflare 账户或演示快捷脚本。

不指定 `--profile` 时，Relay 自动使用默认目录 `~/.local/state/herdr-relay`；设置了 `XDG_STATE_HOME` 时使用 `$XDG_STATE_HOME/herdr-relay`。设备身份和配对会保存在这里，后续不必重新配对。

## 1 两端安装软件

已经有 Herdr 0.9.3 和 Herdr Relay 0.1.1 或更新版本，可以跳到第 2 步。当前 Mac 和 OrbStack 演示 VM 都已经装好软件；此前演示的配对使用了单独目录，不会自动成为这里的默认配对。

全新 Ubuntu 先安装编译工具：

```sh
sudo apt update
sudo apt install -y curl build-essential
```

全新 Mac 如果没有 Command Line Tools，执行下面的命令并完成系统安装窗口：

```sh
xcode-select --install
```

两端如果没有 Rust 1.99 或更新版本，按 [Rust 官方安装方式](https://rust-lang.org/tools/install/) 安装：

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
```

然后在两端分别执行：

```sh
curl -fsSL https://herdr.dev/install.sh | sh
cargo install herdr-relay --locked
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
herdr --version
herdr-relay --version
```

Herdr 使用[官方安装器](https://herdr.dev/docs/install/)。目前验证的官方版本是 0.9.3。Relay 的公开安装渠道是 [crates.io](https://crates.io/crates/herdr-relay)，`cargo install` 会在本机编译，首次安装需要一些时间。没有公开可用的 Relay 一键二进制安装脚本。

若新终端找不到命令，把上面的 `export PATH=...` 一行放进 Mac 的 `~/.zshrc` 或 Ubuntu 的 `~/.bashrc`。

## 2 在 Ubuntu 启动并共享 Herdr

如果使用这次创建的 OrbStack VM，先从 Mac 进入它：

```sh
orb -m herdr-relay-demo
```

在 Ubuntu 中直接启动 Herdr，使用默认会话：

```sh
herdr
```

完成 Herdr 首次设置后，按 **Ctrl+B，松开，再按 Q** 返回 Ubuntu shell。Herdr 会话继续运行。

接着执行：

```sh
herdr-relay identity init
herdr-relay share add --session default --name Ubuntu
herdr-relay service install
herdr-relay service start
herdr-relay service status
```

`herdr` 本身不需要 `--session`；Relay 的共享命令目前需要显式写 `--session default` 来选择默认会话。等状态显示 `Relay online` 后生成配对串：

```sh
herdr-relay pair create
```

复制输出中完整的一次性配对串。它在 10 分钟内有效；Ubuntu 的 Relay 服务需要保持在线。

## 3 在 Mac 配对并连接

在 Mac 的另一个终端执行：

```sh
herdr-relay identity init
herdr-relay pair accept
```

出现提示后粘贴配对串并按 Enter。粘贴时不显示字符是正常的。成功消息中的 `this device may access ...` 后面是 Ubuntu 的设备 ID。

把下面的 `设备ID` 替换为那个值，查看共享目标：

```sh
herdr-relay targets --host 设备ID
```

输出的第一列是目标 ID。把两个占位词都替换为真实值：

```sh
herdr-relay connect --host 设备ID --target 目标ID
```

现在看到的是 Ubuntu 的原生 Herdr 界面。在里面执行 `uname -a` 可以确认正在操作 Linux；也可以运行已经安装的 `codex` 或 `claude`。AI 工具首次登录由各自的账号流程完成，与 Relay 配对独立。

Mac 仅作为连接端时，不需要安装或启动 Relay 后台服务。

## 4 日常使用

Ubuntu 的 Herdr 和 Relay 服务保持运行时，Mac 以后只需要重复最后一条 `herdr-relay connect` 命令，安装、初始化、共享和配对都不用重做。

退出连接仍是 **Ctrl+B，然后 Q**，Ubuntu 的会话和 agent 会继续运行。一个共享目标同时只允许一个 Relay 客户端；遇到 `TARGET_BUSY` 时先退出之前的连接。

Ubuntu 重启或用户会话结束后，按需重新打开 Herdr 并启动 Relay：

```sh
herdr
# Ctrl+B，然后 Q
herdr-relay service start
```

当前 Relay 没有交互菜单，连接命令仍要求 `--host` 和 `--target`，尚无交互选设备或连接别名。Herdr 自身的终端界面与 Relay 的 CLI 是两个入口。`--profile` 是可选的高级隔离参数，不属于普通用户的必填步骤。
