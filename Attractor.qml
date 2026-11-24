import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland

// Headless service with two jobs, both run by the native `attractor` binary built from this repo:
//  - the live wallpaper: kept running on the bottom layer behind your windows (it pauses itself when hidden)
//  - the screensaver: started after the system idle timeout, with Omarchy's screensaver window class so the
//    built-in idle service still locks on time and treats dismissing it as activity
// Settings live in ~/.config/attractor/config.json; changing it restarts the wallpaper.
Item {
  id: root

  // injected by omarchy-shell's service loader: live access to shell.json (idle.screensaver, plugins)
  property var shell: null

  readonly property string home: Quickshell.env("HOME")
  readonly property string pluginDir: Qt.resolvedUrl(".").toString().replace(/^file:\/\//, "").replace(/\/?$/, "/")
  readonly property var sysIdle: (shell && shell.shellConfig && shell.shellConfig.idle) ? shell.shellConfig.idle : null
  readonly property int idleSeconds: {
    var v = sysIdle ? Number(sysIdle.screensaver) : NaN
    return (isFinite(v) && v > 0) ? Math.floor(v) : 150
  }
  readonly property var plugins: (shell && shell.shellConfig && shell.shellConfig.plugins) ? shell.shellConfig.plugins : []
  readonly property bool gridfallEnabled: {
    for (var i = 0; i < plugins.length; i++) if (plugins[i] && plugins[i].id === "io.github.lavart.gridfall") return true
    return false
  }

  // ---- settings
  property var cfg: ({})
  readonly property bool wallpaperOn: cfg.wallpaper !== false
  readonly property string saverSetting: (typeof cfg.saver === "string") ? cfg.saver : "auto"
  // "auto": the screensaver is on unless Gridfall is, so two screensavers never race each other on idle
  readonly property bool saverOn: saverSetting === "on" || (saverSetting === "auto" && !gridfallEnabled)

  function log(m) { console.log("[attractor] " + m) }

  FileView {
    id: cfgFile
    path: root.home + "/.config/attractor/config.json"
    watchChanges: true
    onFileChanged: reload()
    onLoaded: root.applyConfig(text())
    onLoadFailed: root.applyConfig("")
  }

  function applyConfig(raw) {
    var next = {}
    try { next = raw ? JSON.parse(raw) : {} } catch (e) { log("config.json is not valid JSON: " + e) }
    var changed = JSON.stringify(next) !== JSON.stringify(cfg)
    cfg = next
    if (changed) restartWallpaper()
  }

  // ---- live wallpaper
  property bool booted: false
  function startWallpaper() {
    if (!wallpaperOn || wallpaper.running) return
    log("wallpaper start")
    wallpaper.running = true
  }
  function restartWallpaper() {
    if (wallpaper.running) wallpaper.running = false
    wallTimer.restart()
  }
  Process {
    id: wallpaper
    command: ["bash", root.pluginDir + "bin/attractor-run", "--wallpaper"]
    onExited: (code) => { root.log("wallpaper exited (" + code + ")"); wallTimer.interval = (code === 0) ? 1200 : 5000; wallTimer.restart() }
  }
  // starts after load, restarts after an exit or a settings change; never in a tight loop
  Timer { id: wallTimer; interval: 1200; repeat: false; onTriggered: root.startWallpaper() }

  // ---- screensaver
  function launch(reason) {
    if (launcher.running) return
    log("screensaver: " + reason)
    launcher.running = true
  }
  Process { id: launcher; command: ["bash", root.pluginDir + "bin/attractor-launch"] }
  // the stock terminal screensaver would open alongside this one: it is switched off while this saver is on
  Process { id: builtinOff; command: ["bash", root.pluginDir + "bin/attractor-builtin", "off"] }
  Process { id: builtinRestore; command: ["bash", root.pluginDir + "bin/attractor-builtin", "restore"] }
  onSaverOnChanged: { if (!booted) return; if (saverOn) builtinOff.running = true; else builtinRestore.running = true }

  // IdleMonitor does not re-arm when its timeout changes, so a Loader builds a fresh one
  Loader {
    id: idleLoader
    active: false
    sourceComponent: Component {
      IdleMonitor {
        enabled: true
        timeout: root.idleSeconds
        respectInhibitors: true
        onIsIdleChanged: if (isIdle && root.saverOn) root.launch("idle " + root.idleSeconds + "s")
      }
    }
  }
  Timer { id: armTimer; interval: 80; repeat: false; onTriggered: idleLoader.active = true }
  onIdleSecondsChanged: { idleLoader.active = false; armTimer.restart() }

  IpcHandler {
    target: "attractor"
    function preview(): void { root.launch("preview") }
    function restart(): void { root.restartWallpaper() }
    function status(): string {
      return JSON.stringify({ wallpaper: wallpaper.running, wallpaperOn: root.wallpaperOn, saverOn: root.saverOn, saver: root.saverSetting, gridfallEnabled: root.gridfallEnabled, idleSeconds: root.idleSeconds })
    }
  }

  Component.onCompleted: {
    booted = true
    if (saverOn) builtinOff.running = true
    armTimer.restart()
    wallTimer.restart()
  }
  // inline rather than via bin/attractor-builtin: on removal the plugin folder may already be gone
  Component.onDestruction: {
    wallpaper.running = false
    Quickshell.execDetached(["bash", "-c",
      "m=\"${XDG_STATE_HOME:-$HOME/.local/state}/attractor/disabled-builtin-screensaver\"; " +
      "if [ -f \"$m\" ]; then omarchy-toggle screensaver-off off; rm -f \"$m\"; fi"])
  }
}
