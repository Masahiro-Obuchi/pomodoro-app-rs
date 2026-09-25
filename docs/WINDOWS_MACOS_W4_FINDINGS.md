# Windows / macOS W4 検証記録

記録日（日本時間）：2026-09-25

[対応計画](WINDOWS_MACOS_PLAN.md)のW4では完了通知をOSごとの手段へ接続し、端末終了時の復元処理を調整した。対象端末のバージョンを固定した実表示・実操作の受入はW5に残す。

## 通知方式

| OS | 方式 | 採用理由と制限 |
| --- | --- | --- |
| Linux | 既存の`notify-send` | 保存先と既存の通知手段を維持する。コマンドがない、通知サービスが拒否する場合は警告にする |
| macOS | `/usr/bin/osascript`の`display notification` | OS付属のコマンドを使う。本文と見出しはAppleScriptソースへ埋め込まず引数で渡す。通知の許可や集中モードによって表示されない場合がある |
| Windows | `tauri-winrt-notification` 0.7.2のWinRT toast | Rust 1.86に適合するcrateを使う。インストーラーを設けない現状では同crateのPowerShell AppUserModelIDを借用するため、通知元はPowerShellとして表示され得る。アプリ固有のIDや通知クリック時の動作は提供しない |

本文は`Focus`、`Quick Start`、`Short Break`、`Long Break`で共通の対応表から作る。controllerは保存確定後にのみ送信を試み、通知失敗を警告として返す。保存済みの完了は戻さず、再試行で同じ完了を再通知しない。これは既存のcontrollerテストで検証する。`show()`やコマンドの成功は実際の表示を保証しない。

macOSの引数渡しと通知構文は[Appleの`osascript`使用例](https://developer.apple.com/documentation/security/customizing-the-xcode-archive-process)に合わせた。macOS CIでは`osacompile`で同じスクリプトをコンパイルし、通知は送らない。Windowsの借用IDは[`tauri-winrt-notification`のAPI資料](https://docs.rs/tauri-winrt-notification/0.7.2/tauri_winrt_notification/struct.Toast.html)を参照した。[Microsoftの非パッケージアプリの通知資料](https://learn.microsoft.com/en-us/windows/win32/shell/enable-desktop-toast-with-appusermodelid)は、アプリ固有の通知元にはStartメニューのショートカットとAppUserModelIDが必要と説明している。W4はそれらのインストール操作を行わない。

## 端末と検証範囲

`TerminalGuard`はbracketed pasteの解除、代替画面からの復帰、raw modeの解除を個別に試みる。途中の操作が失敗しても残りの復元を試す。Linux PTYテストでは通常終了および保存失敗後の終了を含め、子プロセス終了後の端末local modeが起動前に戻ることを確認した。各OS共通の`TestBackend`描画テストは狭い画面の保存再試行・未保存終了キーと拡大案内を確認する。

[W4 CI実行 #30](https://github.com/Masahiro-Obuchi/pomodoro-app-rs/actions/runs/36125428380)ではRust 1.86.0の`ubuntu-24.04`、`windows-2025`、`macos-15`、`macos-15-intel`の4 jobが成功した。各OSでformat、clippy、workspace全テスト・ビルド、保存probe、保存途中の子プロセス強制終了テストを実行した。macOS両runnerでは`osacompile`の構文テストも成功した。LinuxローカルでもRust 1.86.0のworkspaceテスト、clippy、formatとPTYテストが成功した。CIが確認したWindows通知はビルドと共通本文のテストまでで、toastの実表示ではない。

GitHub Actionsのheadless runnerでは、通知の実表示、Windows TerminalとTerminal.appでのキー・Unicode貼付け・リサイズ、通常ユーザーの通知設定、代替画面の見え方は確認できない。当初W4に置いた実端末での暫定確認はこの環境では実施できないため、W5のバージョン固定受入へ移した。W5ではFocus / Quick Startの完了通知、貼付け、保存失敗の復旧画面、正常終了・エラー終了時の画面復帰を実端末で確認する。Windowsでは通知元がPowerShellとなることも表示上の制約として照合する。
