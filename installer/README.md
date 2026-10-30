# Installer

Per-user Inno Setup 6 installer for OP36 Helper, plus the portable exe. No admin rights: files go to
`%LOCALAPPDATA%\Programs\OP36 Helper`, shortcuts to the user's Start Menu (and Startup folder when
"Start with Windows" is ticked), and the uninstaller registers under the user's own registry hive.

```
pwsh installer/build.ps1 [-FirmwareDir <dir>] [-SkipCargo]   # installer/Output/op36-helper-setup-<ver>.exe
pwsh installer/build-portable.ps1 [-SkipCargo]                 # installer/Output/op36-helper-portable-<ver>.exe
```

- The version is `helper/Cargo.toml`'s; the exe's version resource must agree (`cargo build` stamps it).
- `-FirmwareDir` must hold `op36_left.uf2`, `op36_right.uf2` and `BUILD_COMMIT.txt`; default
  `firmware/op36-layer-notify`. The release workflow passes the artifact it built.
- Needs Inno Setup 6 (`winget install JRSoftware.InnoSetup`, `scoop install inno-setup`, or
  `choco install innosetup` on CI).
- Silent install, as the self-updater runs it: `op36-helper-setup-<ver>.exe /VERYSILENT /NORESTART /SUPPRESSMSGBOXES`.
- Uninstall keeps `%APPDATA%\op36-helper` (settings, keymap cache, log).
