# Windows / macOS W3 検証記録

記録日（日本時間）：2026-09-25

[対応計画](WINDOWS_MACOS_PLAN.md)のW3では、WindowsにV1保存・読込・再試行・明示復旧を接続し、TUI/controllerをビルド・統合テスト対象に加えた。保存候補と復旧のpolicy、V1 JSON検証はLinux/macOSと共有する。

## 採用したAPIと失敗時の扱い

| 境界 | Windows実装 | 検証 |
| --- | --- | --- |
| 排他 | `FILE_FLAG_OPEN_REPARSE_POINT`で開く専用`state.lock`に`fs4::FileExt::try_lock`。共有モードでは保持中の削除・置換を拒否 | 同一・別プロセスの競合、強制終了後の再取得、子プロセスへの非継承 |
| 保存値の読込 | `FILE_FLAG_OPEN_REPARSE_POINT`で開いたハンドルの属性を確認し、reparse pointや通常ファイル以外を拒否 | 実ファイル、リンク、不正JSON、未知version、未認識エントリ |
| 一時・退避ファイルの所有 | 開いた両ファイルの`FILE_ID_INFO`（volume serialと128-bit file ID）と内容を照合してから削除・復旧元確認。IDを取得できない場合は所有扱いしない | 同内容の外部差し替えと再試行のテスト。ReFSで一意とは限らない64-bit IDを所有判定に使わない |
| 置換 | 同一ディレクトリ内の一時ファイルから`std::fs::rename` | 保存・backup・復旧、失敗注入、開いた対象による置換エラーと再試行 |
| 同期 | ファイルの`sync_all`と、`FILE_FLAG_BACKUP_SEMANTICS`付き書込用ディレクトリハンドルの`sync_all` | 保存ディレクトリと今回作成したディレクトリの親、rename後の親、同期失敗時の固定候補保持 |

Windowsでは本体の`rename`がエラーを返しても置換結果を推定しない。保存候補を確定不明として保持し、再試行時に本体を再読込する。本体が同じ候補なら同期を再試行し、旧本体なら同じ候補を書き直し、第三の内容なら競合として停止する。backupの置換エラー時にも旧本体を保持する。[Rustの`rename`説明](https://doc.rust-lang.org/std/fs/fn.rename.html)、[Windowsのディレクトリハンドル資料](https://learn.microsoft.com/en-us/windows/win32/fileio/obtaining-a-handle-to-a-directory)、[`FlushFileBuffers`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)を参照。

Rust 1.86の標準APIは開いたハンドルの128-bit file IDを公開しないため、`pomodoro-winfs`に`GetFileInformationByHandleEx(FileIdInfo)`を呼ぶ小さなFFI境界を設けた。platform本体は`unsafe_code = "forbid"`を維持する。IDを取得できないファイルシステムでは64-bit IDへ降格せず、所有確認をエラーとして停止する。[Microsoftの64-bit IDとReFSの説明](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/ns-fileapi-by_handle_file_information)に基づく。

## 検証範囲と残る事項

[修正後のW3 CI実行 #27](https://github.com/Masahiro-Obuchi/pomodoro-app-rs/actions/runs/36121399616)では、Rust 1.86.0の`windows-2025`、`ubuntu-24.04`、`macos-15`、`macos-15-intel`の4 jobが成功した。各OSで`fmt`、`clippy --workspace --all-targets`、workspace全テスト・ビルド、保存途中の子プロセス強制終了テストを実行した。Windowsでは保存・復旧・失敗注入・別プロセス排他、TUI/controllerのworkflowに加え、既存保存先での同期範囲、同内容の一時ファイル差し替え、読込の共有違反、リンク差し替え、参照先のないリンクを検証した。

- Windows Server 2025のCI runnerは、Windows 11の通常ユーザーや対象のローカルファイルシステムでの権限を代表しない。保存ディレクトリと今回作成したディレクトリの親の同期権限、保存先のACL、リンク/reparse pointの種類はW5の実端末受入で再確認する。必要な同期ができない場合は保存成功を返さず、候補を保持する。
- Windowsでは既存の祖先ディレクトリを書込用に開かない。今回作成した保存ディレクトリの親だけを同期するため、以前のプロセスが作成した祖先が未同期だった場合、その名前の電源断後の存続は未確認である。W5で通常ユーザーの保存とこの制限の扱いを確認する。
- CIでの同期API成功とプロセス強制終了は、ハードウェア電源断後の永続性を証明しない。Windowsの電源断時の保証は未確認として扱う。
- Windows実行ファイルは現在もLinuxの`notify-send` adapterを呼ぶ。通知方式と実端末でのraw mode、代替画面、貼付け、リサイズ、復旧画面はW4/W5で検証する。Windowsの既知フォルダ解決はテスト用環境変数だけで変更できないため、実行ファイルの起動・再起動と実際の保存先はW5で対象端末を使って確認する。
