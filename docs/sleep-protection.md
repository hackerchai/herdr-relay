# Keep the sharing computer awake

`herdr-relay serve` enables sleep protection automatically. It belongs to the
running server, including foreground mode, login/boot startup and network
reconnects. It stays active without a connected client. Client commands such as
`pair` and `connect` do not keep the computer awake.

```sh
herdr-relay service status
```

The sharing menu also has **Sleep protection status**. Check each reported
capability: `inhibitor held`, `OS controlled`, or `unavailable`. An unavailable
capability does not stop sharing or conceal the failure. OS requests time out
after five seconds, and a single bounded monitor rechecks locks and retries
failed requests after a 30-second pause. It retains at most three native locks,
without accumulating helper processes or keeping the display on.

`herdr-relay service stop` releases this server's locks. Normal exits and crashes
also release them; restarting reacquires them. There is no persistent change to
system sleep settings. Other apps or other running Relay profiles may still hold
their own inhibitors. Keeping a server awake uses power even when nobody is
connected; stop the service when you want normal automatic sleep to resume.

## macOS and MacBook lid close

The server owns a public IOKit `PreventUserIdleSystemSleep` assertion. This blocks
idle system sleep while allowing the display to dim or turn off. Verify it with:

```sh
pmset -g assertions
```

Look for **Herdr Relay: sharing sessions** under the server process.

**This assertion does not override MacBook lid-close sleep.** It also does not
override explicit Sleep, low battery or thermal protection. A server cannot
promise availability under those conditions using this public API. Relay reports
system suspend and lid close as OS controlled instead of claiming they are blocked.

For supported closed-display operation, connect the MacBook to power, an external
display, and a keyboard and mouse/trackpad, and allow the accessories when macOS
asks. Otherwise, leave the lid open. Relay does not set `pmset disablesleep`,
install a privileged helper, or replace Apple's thermal and power policy.
See [Apple's assertion contract](https://developer.apple.com/documentation/iokit/kiopmassertiontypepreventuseridlesystemsleep),
[MacBook Pro external displays](https://support.apple.com/guide/macbook-pro/connect-an-external-display-apd8cdd74f57/mac),
and [closed-lid accessory requirements](https://support.apple.com/en-us/102282).

## Linux

The server requests separate logind `block` inhibitors for `idle`, `sleep` and
`handle-lid-switch`. Partial permission grants are reported separately and retained
while the other requests retry. The lid lock disables logind's handling of the lid
switch; desktop power managers, privileged overrides and firmware can still apply
their own policies. Verify the locks with:

```sh
systemd-inhibit --list
```

Look for **Herdr Relay**, with the PID of the server. Distribution policy controls
which users may take these locks. Minimal servers may need the `polkitd` package;
services running without a desktop login, including linger services, may need an
administrator to grant these three inhibition permissions. Missing logind, a
missing system bus and permission denial are visible in `service status`.

For a dedicated headless server, an administrator can install polkit and save the
following as `/etc/polkit-1/rules.d/50-herdr-relay-awake.rules`, owned by root, mode
0644. Replace `YOUR_SERVER_USER` with the account running Relay:

```javascript
polkit.addRule(function(action, subject) {
    var allowed = [
        "org.freedesktop.login1.inhibit-block-idle",
        "org.freedesktop.login1.inhibit-block-sleep",
        "org.freedesktop.login1.inhibit-handle-lid-switch"
    ];
    if (subject.user === "YOUR_SERVER_USER" && allowed.indexOf(action.id) !== -1) {
        return polkit.Result.YES;
    }
});
```

This permits that account's processes to acquire those inhibitors; it does not
grant shutdown, reboot or general administration privileges. Relay retries after
the policy becomes available, so a server restart is unnecessary. Remove the rule
to revoke future acquisitions; stop the server to release already-held locks.
See [systemd inhibitor locks](https://systemd.io/INHIBITOR_LOCKS/).

## Virtual machines and availability

An inhibitor in an OrbStack/Linux VM cannot prevent its **Mac host** from sleeping.
Keep the physical host awake too. Power loss, forced sleep, VM suspension, logout
of a macOS user service, network loss and provider outages can still interrupt
remote access. Sleep inhibition complements automatic reconnect and service
restart; it does not provide an unconditional uptime guarantee.

After upgrading an existing installation, ensure the copied service executable is
also upgraded and restarted. A newer CLI reports `unknown` for older daemons that
cannot report their sleep protection; it never assumes they acquired a lock.
