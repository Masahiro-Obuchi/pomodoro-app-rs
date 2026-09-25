# Windows / macOS W0 調査記録

記録日（日本時間）：2026-09-25

[Windows / macOS 対応計画](WINDOWS_MACOS_PLAN.md)の W0 で、保存 API の候補をネイティブ OS で確認した。W0 は保存 API の調査と契約整理までの部分実施である。実端末の対象バージョンと通知方式はまだ決定していない。現行 TUI は Linux 専用のままであり、この記録は Windows/macOS での保存実装が完成したことを示さない。

## 実測した環境と結果

Rust 1.86.0、GitHub Actions の `ubuntu-24.04`、`windows-2025`、`macos-15`（Apple Silicon）、`macos-15-intel` を使用した。検証コードは [`platform_probe.rs`](../crates/pomodoro-platform/tests/platform_probe.rs)、実行手順は [`platform-probe.yml`](../.github/workflows/platform-probe.yml)にある。[PR #36 の検証結果](https://github.com/Masahiro-Obuchi/pomodoro-app-rs/actions/runs/36025404687)を記録とする。各検証は OS の動作を観測するもので、電源断の再現ではない。

| 項目 | Linux | macOS 15（両アーキテクチャ） | Windows Server 2025 x64 |
| --- | --- | --- | --- |
| 別プロセスの非ブロッキング排他と解放 | 成功 | 成功 | 成功 |
| 保持プロセスを強制終了した後の再取得 | 成功 | 成功 | 成功 |
| 開いた旧本体がある状態での`std::fs::rename`による置換 | 成功 | 成功 | 成功 |
| ファイルハンドルと置換後のパスの同一性を区別 | 成功 | 成功 | 成功 |
| `File::open(directory).sync_all()` | 成功 | 成功 | Access denied |
| 書込用のWindowsディレクトリハンドルと`sync_all()` | 対象外 | 対象外 | 成功 |
| `BaseDirs::state_dir()` | XDG state directory | `None` | `None` |
| `BaseDirs::data_local_dir()` | `~/.local/share` | `~/Library/Application Support` | ユーザーのローカル AppData |
| リンクを追わずに開いたWindowsのシンボリックリンク | 対象外 | 対象外 | リンク自身のmetadataを取得 |

Windowsのディレクトリ同期では `OpenOptionsExt::custom_flags(FILE_FLAG_BACKUP_SEMANTICS)` と書込アクセスを指定した。読込アクセスだけの同じ試行は Access denied だった。この結果は CI の管理者権限を持つ runner で得たもので、Windows 11 の通常ユーザー・異なるファイルシステム・電源断時の保証へそのまま一般化しない。

## W0で残した決定・検証

| 残した事項 | 今回確認した範囲 | 担当レビュー単位 |
| --- | --- | --- |
| 実端末の対象 OS・端末バージョン | CI runner の OS ラベルと CPU アーキテクチャのみ。Windows Terminal、PowerShell、Terminal.app のバージョンは未確認 | W5。実端末受入の開始前に組合せを固定し、結果と README に記録する |
| macOS/Windows の通知方式 | `notify-rust` は候補。Rust 1.86 での adapter ビルド、実表示、権限拒否時の動作は未確認 | W4。方式を選定し、実装と検証結果を記録する |
| 保存 API の残る境界 | 子プロセスへのロック継承、リンク・reparse point を対象にした置換、一時ファイルの安全な削除、通常ユーザーの権限・読込エラーは未確認 | W2/W3でOSごとの保存・復旧実装と失敗注入を確認する。Windows 11通常ユーザーの権限はW5の実端末受入で確認する。TUIのエラー表示はW4で確認する |

## 実装時に採る境界

| 対象 | 方針 | 確認を残すこと |
| --- | --- | --- |
| 保存先 | Linux は従来の XDG state を維持。macOS は Application Support、Windows はローカル AppData の `pomodoro-app-rs` 以下 | 通常ユーザーでの作成・再起動、権限エラー時の停止 |
| 排他 | Linux は現行 `rustix::flock` を維持。macOS/Windows は safe な `fs4::FileExt::try_lock` を候補とする | `state.lock` をリンクとして開かないこと、別プロセスの競合、子プロセスへの継承、異常終了後の再取得 |
| 読込と同一性 | Windows は `OpenOptionsExt` の `FILE_FLAG_OPEN_REPARSE_POINT` でリンク自身を開き、通常ファイルでないものを拒否する。開いた一時ファイルとパスの同一性には `same-file::Handle` を候補とし、内容も別途照合する | 差し替え競合、reparse point の各種類、同一性が確認できない場合の停止 |
| 置換 | 同一ディレクトリ内の一時ファイルと `std::fs::rename` を候補とする。Windowsで置換 API が失敗したときは、変更なしと推定せず本体を再確認する | バックアップ・本体・復旧の各失敗境界、開いた第三者ハンドル、同一ボリューム |
| 同期 | ファイルの同期後、Linux/macOS は親ディレクトリを同期。Windows は `FILE_FLAG_BACKUP_SEMANTICS` 付きの書込用ディレクトリハンドルを同期候補とする | Windows 11 の通常ユーザー権限、保存先と祖先の同期範囲、同期失敗時の固定候補保持 |
| 通知 | 現行 Linux `notify-send` は維持。macOS/Windows は両 OS 対応の `notify-rust` を候補とし、W4 で Rust 1.86 のビルドと実表示を確認する | 通知権限がない場合・表示されない場合も保存成功を取り消さない |

Windowsのファイル置換は、同じボリューム上の操作でもファイルシステム・共有モード・他プロセスのハンドルによって失敗し得る。[Rust `rename` の OS 別説明](https://doc.rust-lang.org/std/fs/fn.rename.html)と[Microsoft `ReplaceFileW` の失敗状態](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)を踏まえ、置換 API のエラー後も「確定不明」を扱う。`same-file` の同一性比較だけには偽陽性の可能性があるため、候補のバイト列照合を省かない。[`same-file` の説明](https://docs.rs/same-file/1.0.6/same_file/struct.Handle.html)

Windowsのディレクトリハンドルを開く方法は[Microsoft のディレクトリハンドル資料](https://learn.microsoft.com/en-us/windows/win32/fileio/obtaining-a-handle-to-a-directory)に、ファイルの同期 API は[`FlushFileBuffers` の資料](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)に従う。ただし、今回の API 成功とプロセス停止テストはハードウェア電源断に対する永続性の証明ではない。必要な同期が失敗した場合は保存成功を返さず、候補を保持して再試行・未保存終了へ進む。

## 次のレビューで必要な検証

- W1：保存先の解決を共通化し、Linux 固有のファイル操作を V1 保存処理から分離する。保存 policy と TUI/controller は W2/W3 まで Linux 限定とし、既存の保存・復旧・失敗注入テストを維持する。
- W2：macOS のロック、`NOFOLLOW` 読込、ファイル・ディレクトリ同期、通常保存・復旧を実ファイルと別プロセスで検証する。
- W3：Windows ServerのCI runnerで、ディレクトリ・祖先の同期、reparse pointの拒否、同一性、置換失敗・確定不明、復旧を実装・検証する。Windows 11の通常ユーザーとローカル保存先での権限・同期の確認はW5の実端末受入へ移し、CI runnerの成功だけでOS全体の対応完了とはしない。
- W4：macOS/Windows の通知 adapter を Rust 1.86 でビルドし、保存成功後だけ通知することと実端末での表示を確認する。
