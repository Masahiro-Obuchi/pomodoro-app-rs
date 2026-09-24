# Windows / macOS W2 検証記録

記録日（日本時間）：2026-09-25

[対応計画](WINDOWS_MACOS_PLAN.md)のW2では、V1保存・読込・再試行・明示復旧をmacOSへ接続した。Linuxと同じ保存policyを共有し、OS依存のファイル操作をUnix adapterに閉じ込めた。Windowsの保存実装はW3で扱う。

## 採用したAPIと保存境界

| 境界 | macOS実装 | 確認方法 |
| --- | --- | --- |
| 排他 | `rustix::flock`、`O_CLOEXEC`と`O_NOFOLLOW`付きの専用`state.lock` | 別プロセスの競合・強制終了後の再取得・exec先への非継承 |
| 読込と一時ファイルの同一性 | `O_NOFOLLOW`で通常ファイルだけを読み、device/inodeで開いたファイルとパスを照合 | リンク、差し替え、未認識ファイルを含む保存・復旧テスト |
| 本体とbackupの置換 | 同一ディレクトリ内の固有一時ファイルから`std::fs::rename` | 実ファイル、各保存段階の失敗注入、プロセス強制終了 |
| ファイルとディレクトリの同期 | `rustix::fs::fcntl_fullfsync`を、一時ファイル、退避ファイル、rename後の親ディレクトリ、初回保存時の祖先ディレクトリに使用 | Apple SiliconとIntelのCI runnerでファイル・ディレクトリAPIを実測し、保存・復旧テストを実行 |

W0で候補とした`fs4`へmacOSの排他を変更せず、Linuxで使っていたsafeな`rustix::flock`を共用した。`rustix`の[`fcntl_fullfsync`](https://docs.rs/rustix/1.1.4/rustix/fs/fn.fcntl_fullfsync.html)はmacOSの`F_FULLFSYNC`を呼ぶ。通常の`fsync`だけではドライブのキャッシュまでの書出しを要求できないため、macOSの同期ではこのAPIを使う。[`rustix::fs::fsync`のmacOSに関する注記](https://docs.rs/rustix/1.1.4/rustix/fs/fn.fsync.html)も参照。

## CIとテストの範囲

Rust 1.86.0のGitHub Actions `macos-15`（Apple Silicon）と`macos-15-intel`を対象とする。platformの保存・復旧・失敗注入・別プロセス排他テストに加え、TUIのcontroller・workflow・起動失敗テスト、全workspaceのビルド、保存途中で子プロセスを強制終了するテストを実行する。Linuxでは同じテストを維持し、WindowsではW1時点のplatformテストを維持する。

[PR #38のCI実行 #17](https://github.com/Masahiro-Obuchi/pomodoro-app-rs/actions/runs/36032406955)では、Apple Silicon・Intel・Linuxの全workspaceテスト、強制終了テスト、ビルドが成功した。Windowsのplatformテストも成功した。Windowsでの保存とTUI全体のビルドはW3の検証対象である。

macOSの一時パスには`/var`と`/private/var`の別表記があるため、パス比較と失敗注入に使うテストディレクトリを正規化した。CIのmacOS保存先では不正なUTF-8バイト列のファイル名の作成が失敗したため、その名前の残存物テストだけをLinuxに限定した。TUI起動テストはmacOSの`~/Library/Application Support`を一時HOME内に作り、実行ファイルがそこを読み込むことを確認する。

## 未確認事項

- CIでの`F_FULLFSYNC`成功とプロセス強制終了テストは、実機の電源断後に必ずデータが残る証明ではない。[Appleの説明](https://developer.apple.com/documentation/xcode/reducing-disk-writes)でもフラッシュの限界がある。必要な同期がエラーを返した場合は保存成功とせず、固定した候補の再試行を求める。
- macOS実行ファイルは現在もLinuxの`notify-send` adapterを呼ぶ。通知方式の選定と実表示はW4で行う。
- macOSのraw mode、代替画面、貼付け、リサイズ、復旧画面は実端末で未検証。W4で暫定確認し、W5で対象OS・端末バージョンを固定して受け入れる。Linux PTYテストはmacOSの実端末検証の代わりにはならない。
