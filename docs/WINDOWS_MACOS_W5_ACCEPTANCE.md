# Windows / macOS W5 受入記録

記録開始日（日本時間）：2026-09-25

状態：Rust 1.86の4 OS CIと疑似端末テストは成功。Windows TerminalとTerminal.appを使用できる実機がないため、バージョンを固定した実端末受入は**未開始**。この記録だけでWindows/macOS対応完了とは判定しない。

[対応計画](WINDOWS_MACOS_PLAN.md)の§5–6と[W4検証記録](WINDOWS_MACOS_W4_FINDINGS.md)を受入基準にする。CI runnerのOS版と実端末のOS版を別々に記録する。

## CIで確認する範囲

[W5 CI実行 #41](https://github.com/Masahiro-Obuchi/pomodoro-app-rs/actions/runs/36161680731)で次の4 jobが成功した。各jobでformat、clippy、workspace全テスト・ビルド、保存probe、保存途中の子プロセス強制終了を実行した。

| 対象 | CI runner | 確認すること | 結果 |
| --- | --- | --- | --- |
| Linux | `ubuntu-24.04` | Rust 1.86の全workspaceテスト・ビルド、Linux PTYとOS共通PTY、保存probe、強制終了境界 | 成功 |
| Windows x86_64 | `windows-2025` | 同上のうちWindows ConPTY、保存・復旧・排他・通知adapterのビルド | 成功 |
| macOS Apple Silicon | `macos-15` | macOS PTY、保存・復旧・排他、AppleScript構文、全workspaceテスト・ビルド | 成功 |
| macOS Intel | `macos-15-intel` | Apple Siliconと同じ | 成功 |

OS共通の疑似端末テストは、`POMODORO_STATE_DIR`で隔離した保存先に対する初回起動、Unicode作業名の貼付け、Focus開始、リサイズ、割り込み・復帰、終了・再起動を通す。保存失敗時には24×20の画面で再試行・未保存終了キーを確認し、同じ候補の再試行後に割り込み記録が残ることも確認する。破損した本体からの復旧では、利用者の同意前に本体を変更しないことを確認する。環境変数を設定しない通常起動はOSの既定保存先を使い、空値は起動前に拒否する。これらはWindows TerminalやTerminal.appでの表示、通知バナー、通常ユーザー権限での保存、GUI端末の復元を代行しない。

Windowsの疑似端末にはConPTYを使う。CIのheadless環境では、PTYライブラリが要求する起動時のカーソル位置応答をテスト側から送って画面の出力待ちを解除する。これはWindows Terminalそのものの挙動の検証ではない。先行CIでLinuxの既存ロックprobeが一度、プロセス終了直後の一時的な`WouldBlock`で失敗したため、漏れたロックは失敗させたまま1秒だけ解放を待つよう調整した。CI実行 #41 ではこのprobeを含め4 jobが成功した。

## 実端末の対象と版

実端末を確保した時点で、**受入を始める前に**次の組合せのOS・端末・ファイルシステムの具体的なバージョンを記入する。対象はWindows x86_64のWindows Terminal上のPowerShell、macOS Apple SiliconとIntelのTerminal.app、Linuxのローカル端末。Windowsは管理者ではない通常ユーザーを使う。WSL、ネットワーク・同期フォルダは対象外。

| 対象 | OSの版・build | 端末の版 | ファイルシステム | 通知設定 | 実施日・結果 |
| --- | --- | --- | --- | --- | --- |
| Windows x86_64 | 未固定 | Windows Terminal：未固定、PowerShell：未固定 | 未確認 | 未確認 | 未実施 |
| macOS Apple Silicon | 未固定 | Terminal.app：未固定 | 未確認 | 未確認 | 未実施 |
| macOS Intel | 未固定 | Terminal.app：未固定 | 未確認 | 未確認 | 未実施 |
| Linux | 未固定 | 未固定 | 未確認 | 未確認 | 未実施 |

版はWindowsの「設定 > システム > バージョン情報」とWindows Terminal / PowerShellの「バージョン情報」、macOSの「このMacについて」とTerminal.appの「バージョン情報」、LinuxのOS情報と端末の「バージョン情報」から記録する。CPUアーキテクチャとRustの版（`rustc --version`）も併記する。

## 実端末での手順と期待結果

既存の作業データを触らないよう、各OSで新しいローカルテストユーザーを用意する。通常実行の保存先はLinuxでは`$XDG_STATE_HOME/pomodoro-app-rs`（未設定なら`~/.local/state/pomodoro-app-rs`）、WindowsではユーザーのローカルAppData配下の`pomodoro-app-rs`、macOSでは`~/Library/Application Support/pomodoro-app-rs`である。新規の保存先ではロック取得時に`state.lock`、初回保存で`state.json`が置かれ、`state.json.bak`は次の保存で既存の本体を置き換えたときに作られる。テスト中も別のユーザーの保存先を編集しない。

1. Rust 1.86以上で`cargo run -p pomodoro-tui`を起動する。既定のFocus 25分・Short Break 5分・Long Break 15分・4 FocusごとのLong Break、Focus開始待ち、ラウンド進捗0、空履歴を画面と初回保存値で照合する。初回保存直後は`state.json.bak`がまだ存在しないことも確認する。
2. `t`で日本語・結合文字・複合絵文字の作業名を入力・貼付けし、保存後に再起動して同じ値が表示されることを確認する。この2回目の保存後に`state.json.bak`が作られ、その内容が初回の`state.json`と一致することを確認する。複数行と制御文字を含む貼付けは拒否理由が見え、保存値に入らないことを確認する。
3. `2`でQuick Startを始め、Pause、Distraction（`d`）、Return（`Space`）、History（`h`）、リサイズを試す。30×25、24×20、24×9の画面で操作キーや拡大案内が見えることを確認する。`r`でResetした後も`t`で作業名を編集できることを確認する。
4. Quick Startを完了させ、OSの通知表示と未確定の`f`/`c`選択を確認する。`c`ならFocusの全25分が新たに始まること、終了・再起動後も選択が待ち状態にあることを確認する。Focus・Short Break・Long Breakの完了通知も短い設定値を使って確認する。通知は保存後にのみ送られ、表示失敗時も保存済みの完了が失われないことを照合する。
5. 保存失敗画面は隔離したテストユーザーでのみ作る。実行中に既存の`state.json.bak`を保存ディレクトリの外へ退避し、同名の空ディレクトリを作って次の保存を試す。`r: Retry save`、`Q: Confirm unsaved exit`、失敗理由が狭い画面でも見えることを確認する。空ディレクトリを取り除いて`r`を押し、同じ候補が保存されることを確認する。別の試行では`Q`→`y`で未保存終了し、終了コードが非0で、最後の確定済みデータが残ることを確認する。
6. バックアップ復旧はアプリを閉じた状態で、テストユーザーの保存先にある検証済み`state.json.bak`を残し、本体を意図的に破損させて試す。表示される保存時刻・損失警告を読み、同意前に本体が変わらず、`y`の後に破損本体が隔離されることを確認する。狭い画面では拡大するまで同意できないことも確認する。
7. 正常終了、保存失敗からの未保存終了、処理可能な起動エラーの後で端末の入力、エコー、画面が戻ることを確認する。別プロセスの二重起動が拒否され、最初のプロセスの保存値は変わらないことを確認する。
8. 睡眠・復帰とシステム時刻の変更後、観測空白とアプリ終了中の時間が作業時間に入らないことを、画面・保存値・履歴で照合する。Windows通常ユーザーでは初回保存とバックアップ更新、ディレクトリ同期の成否も確認する。

各項目で端末の版、操作、画面表示、終了コード、保存前後の`state.json`と`state.json.bak`の差、通知表示・通知元を記録する。Windowsの通知は現状PowerShellのAppUserModelIDを借用するため、通知元がPowerShellと表示される可能性を制約として記録する。CIの成功や通知APIの成功だけでは実表示を成功扱いにしない。

## READMEに反映する案内の候補

実端末受入が成功した組合せだけをREADMEの対応OS・端末として記す。共通の起動コマンドは`cargo run -p pomodoro-tui`、最低Rust版は1.86である。保存先は次の表と実際の端末で照合し、通知の許可と失敗時の扱いも記す。

| OS | 既定の保存先 | 通知と制約 |
| --- | --- | --- |
| Linux | XDG state directoryの`pomodoro-app-rs` | `notify-send`を使い、コマンドやサービスがない場合は保存済みの完了を取り消さない |
| Windows | ローカルAppDataの`pomodoro-app-rs` | WinRT toast。現状はPowerShellの通知元IDを借用し、通知設定によって表示されない場合がある。通常ユーザー権限と表示を実機で確認する |
| macOS | `~/Library/Application Support/pomodoro-app-rs` | OS付属の`osascript`。通知設定や集中モードで表示されない場合がある。Terminal.appからの実表示を確認する |

各保存先の`state.json`は本体、2回目以降の保存で作られる`state.json.bak`は直前の検証済み本体のバックアップ、`state.lock`は排他用である。ネットワーク・同期フォルダ、Windows ARM、GUI、インストーラーは受入対象外。通知失敗は警告になり、保存済みの完了を巻き戻さない。Windowsの既存祖先ディレクトリの同期と電源断耐性は[保存仕様](PERSISTENCE_SCHEMA.md)の制限をそのまま案内する。ここに記した文面は実端末検証前の候補であり、READMEに掲載する対応宣言ではない。

## 判定と利用案内

実端末の組合せと結果がそろい、[計画の§6](WINDOWS_MACOS_PLAN.md)を満たしてからREADMEのLinux専用案内をWindows/macOS対応へ更新する。現時点ではREADMEの対応OS表示を変更しない。保存の電源断耐性とWindowsの既存祖先ディレクトリの同期制限は、[Persistence Schema](PERSISTENCE_SCHEMA.md)と[W3検証記録](WINDOWS_MACOS_W3_FINDINGS.md)の扱いを維持する。
