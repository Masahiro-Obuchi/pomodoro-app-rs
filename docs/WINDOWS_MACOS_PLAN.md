# Windows / macOS 対応計画

作成日（日本時間）：2026-09-25

状態：W0のネイティブ検証は[PR #36](https://github.com/Masahiro-Obuchi/pomodoro-app-rs/pull/36)でレビュー中。W1の共通境界は[PR #37](https://github.com/Masahiro-Obuchi/pomodoro-app-rs/pull/37)で検証中。macOS/Windowsの保存実装は未着手

この計画は完了済みの Phase 0–4 とは独立した、既存 TUI の対応 OS 拡張を扱う。進捗は本書で管理し、Phase の完了状態は変更しない。実装を始める際は、各レビュー単位を独立した commit / PR にまとめ、依存順に確認する。

W0の実測と判断候補は[調査記録](WINDOWS_MACOS_W0_FINDINGS.md)に置く。OSごとの保存実装と実端末受入は引き続き必要である。

W1では保存先の解決を共通化し、Linux固有のファイル操作をV1保存 policy から分離する。TUI/controller の Linux 限定 `cfg` は、W2/W3で対象OSの保存 adapter が利用できるようになったときに外す。

## 1. 目標と対象

- Windows と macOS のネイティブ環境で、既存 TUI を起動し、操作、保存、再起動、復旧、終了できるようにする。WSL は Windows 版の検証に数えない。
- 初期対象は Windows x86_64、macOS Apple Silicon と Intel。Linux は既存の動作と保存先を維持する。対象 OS の具体的なバージョンと端末は最初の検証 PR で固定し、README と CI に記す。
- `pomodoro-core` の遷移、V1 JSON のフィールド・検証規則、保存候補の再試行、保存成功後の通知、キー操作と画面上の意味を維持する。既存の Linux 保存データを自動移動しない。ファイルを手動で移す場合も、V1 として厳密に検証してから読む。
- 対象は TUI とその実行基盤。GUI、インストーラー、自動更新、クラウド同期、ネットワーク共有上の保存、Windows ARM は今回の受入範囲に含めない。

## 2. 現状と設計上の判断点

| 領域 | 現状 | 対応時に確かめること |
| --- | --- | --- |
| ドメイン・時計 | core は OS 非依存。`ObservationClock` は `SystemTime` と `Instant` を使う | スリープ、時刻変更、再起動後に停止時間を加算しない |
| TUI | Crossterm / Ratatui を使用。ただし `apps/pomodoro-tui/src/lib.rs` の公開モジュールは Linux 限定 | Windows/macOS で raw mode、代替画面、キー、貼付け、リサイズ、通常終了・処理可能なエラー時の端末復元が動く |
| 保存先 | `StorageLocation::discover()` は Linux の XDG state directory と、それがない OS での `data_local_dir()` を使用 | Linux のパスを変えず、Windows のローカルデータ領域と macOS の Application Support に保存する。実際のパスを表示・文書化する |
| 排他 | Linux `rustix::flock` と `NOFOLLOW` 付きの専用 `state.lock` | 各 OS で読込前から最終保存まで非ブロッキング排他し、別プロセスと異常終了を検証する |
| 保存と復旧 | `rustix`、Unix の file ID、`fs::rename`、ファイル・親ディレクトリ・祖先ディレクトリの同期に依存 | 置換の原子性、再試行、同名ファイルの差し替え、リンク、クラッシュ時の判定、同期の成否を OS ごとに確認する |
| 通知 | `notify-send` 固定 | 各 OS の通知手段を選び、保存成功後だけ試みる。通知失敗は保存済みの完了を取り消さない |
| テスト | 保存・TUI の統合テストの多くが Linux 限定。端末テストは Linux PTY を使用 | OS 共通のケースを再利用し、OS 固有のケースと実端末確認を追加する |

`directories` 6.0 の `BaseDirs::state_dir()` は Linux で値を返し、Windows/macOS では値を返さない。現在の `data_local_dir()` フォールバックはそれぞれローカル AppData と `~/Library/Application Support` を指す。パスの決定と表示は検証するが、Windows のパスを XDG と呼ばない。

## 3. 守る契約と着手時の調査

保存の意味は [Persistence Schema §6–9](PERSISTENCE_SCHEMA.md)を基準にする。ロック取得前の読込、破損ファイルからの自動初期化、バックアップや不明なファイルの自動採用、保存失敗後の候補作り直しは行わない。V1 codec と `save_generation` の意味は OS 間で同一とする。

最初に、Windows と macOS の**ローカルファイルシステム**で次を小さな実験として確認する。実験結果、使用 API / crate、最低 Rust 1.86 と workspace の `unsafe_code = "forbid"` との適合、未保証事項を PR に記録する。

1. 専用ロックファイルの非ブロッキング排他、プロセス終了時の解放、子プロセスへの意図しない継承の防止。
2. 既存の `state.json` / `.bak` への同一ディレクトリ内での置換、対象が開かれている場合とリンク・reparse point の場合の動作。Windows の置換は Unix と同じだと仮定しない。
3. ファイルの同期と、置換後のディレクトリエントリおよび新規作成した保存ディレクトリの永続化手段。`sync_all` や rename の成功だけで、Linux と同じ電源断耐性があると宣言しない。
4. 一時・退避ファイルの同一性確認と安全な削除。Unix の device/inode 比較を Windows へそのまま移さない。
5. 保存先を発見できない、権限がない、既存ファイルを開けない場合の表示と終了。OneDrive などの同期フォルダやネットワーク保存先は受入対象に入れない。

これらを満たす safe な実装手段が見つかるまでは、その OS の保存層を「対応済み」としない。特に電源断耐性を既存仕様と同じ水準で示せない場合は、実装・README を先行して対応済みにせず、保証の差と代替案を Persistence Schema に明記する変更を別途レビューする。プロセスを強制終了するテストは電源断の証明とは扱わない。

## 4. 実行順序とレビュー単位

| 順序 / PR | 作業 | 完了の証拠 |
| --- | --- | --- |
| W0：調査と保存契約 | §3 の実験を各 OS で行い、保存 API、対象 OS・端末、通知の実装方法を決定。Persistence Schema §7–9 の Linux 固有記述を共通契約と OS 別の実現方法に更新する | 2 OS の実験コードまたは再現手順と結果、採用 API の理由、未保証事項がレビューできる |
| W1：共通境界と保存先 | Linux 固有 `cfg` を必要最小限の実装モジュールへ移し、TUI / controller / V1 codec / 保存 policy の共通部分を分離する。OS ごとの保存先を明文化 | Linux の保存パスと既存テストが維持される。Windows/macOS で保存先解決と共通部分を検証できる。実行ファイル全体の各 OS ビルドは W2 / W3 で通す |
| W2：macOS 保存 | Unix 系の API を再利用できる部分と macOS 固有の同期・ロックを分け、保存・読込・再試行・復旧を接続 | macOS の実ファイル・別プロセステストと保存境界の失敗注入が通る |
| W3：Windows 保存 | Windows 向けのロック、リンクを追わない読込、同一性確認、置換・同期を保存層内に実装。Linux/macOS の保存 policy は共有する | Windows の実ファイル・別プロセステストと保存境界の失敗注入が通る |
| W4：通知と端末 | 通知本文と送信時機を共通化し、各 OS の通知 adapter を追加。TUI の起動、入力、画面復元、エラー表示を調整 | 保存成功前に通知しない、通知失敗でデータを失わない。実端末で主要キー・貼付け・リサイズ・復旧画面を確認 |
| W5：受入、CI、利用案内 | OS 別の統合テストと CI matrix を整備。README に導入・実行、保存先、通知権限と失敗、対応端末、制約を記す | Linux / Windows / macOS の CI と各 OS の実端末確認を記録し、§6 の受入条件を満たす |

W2 と W3 は別 PR にし、それぞれの対象 OS で保存の性質を確認してから W4 に進む。共通 policy を切り出す際は Linux の既存の失敗注入テストを先に維持する。後続 PR が未着手でも、先行 PR は対応済みと誤認させる表示や文書を追加しない。

## 5. テスト計画

| 検証群 | Linux | macOS | Windows |
| --- | --- | --- | --- |
| `fmt`、`clippy --workspace --all-targets`、`test --workspace`、`build --workspace`（Rust 1.86） | 必須 | 必須 | 必須 |
| V1 fixture の読込・再保存、未知 version / 不正 JSON / 未知エントリを上書きしない | 必須 | 必須 | 必須 |
| 同一保存先の二重起動、異常終了後のロック再取得、別保存先の並行起動 | 必須 | 必須 | 必須 |
| バックアップ、本体置換、保存失敗と同一候補の再試行、確定不明、明示復旧 | 必須 | 必須 | 必須 |
| シンボリックリンク / reparse point、通常ファイル以外、権限拒否、外部差し替え | 必須 | 必須 | 必須 |
| 既存 Linux PTY の端末テスト | 必須 | 対象外 | 対象外 |
| ネイティブ端末での初回起動、操作、終了、再起動、復旧と画面復元 | 必須 | 必須 | 必須 |

GitHub Actions には `ubuntu-latest`、`windows-latest`、`macos-latest` と `macos-15-intel` の job を設ける。Linux PTY テストだけを Linux に限定し、保存・controller・描画の共通テストは各 OS で動かす。CI の headless 環境では通知の実表示や端末の完全な動作は確認できないため、通知は送信 adapter のテストと実機確認を併用する。Windows は Windows Terminal 上の PowerShell、macOS は Terminal.app を最低限の実端末確認先とし、具体的なバージョンを W0 で固定する。

実端末では Focus と Quick Start、Current Task の Unicode 入力と貼付け、Pause / Distraction / Return、History、狭い画面での保存再試行と未保存終了、バックアップ確認、正常終了と処理可能なエラー時の raw mode と代替画面の復元を確認する。スリープや時刻変更後の観測空白も各 OS で確認し、終了中の時間が作業時間に加わらないことを保存内容と表示で照合する。

## 6. 対応完了の判定

- W0 の保存契約を両 OS で満たし、W1–W5 の PR がレビュー・マージ済みである。保存の保証に変更がある場合は、仕様と利用案内を先に更新し、その変更を含む受入結果がある。
- Linux の既存テストに退行がなく、Windows/macOS の実ファイル・別プロセス・失敗注入テストが通る。V1 保存値、ID、履歴、ラウンド進捗が再起動後も一致する。
- 初回起動から一連の操作、保存失敗からの回復、明示的バックアップ復旧、正常・異常終了後の再起動をネイティブ実行ファイルで確認する。
- 対応 OS、端末、保存先、通知の条件と残る制限が README と実際の動作で一致する。これらを満たした時点で初めて Linux 専用の案内を更新する。

## 参考資料

- [Crossterm 0.29](https://docs.rs/crossterm/0.29.0/crossterm/)：TUI 入出力の対応範囲。
- [`directories` 6.0 `BaseDirs`](https://docs.rs/directories/6.0.0/directories/struct.BaseDirs.html)：OS ごとの state / local data directory。
- [Rust `std::fs::rename`](https://doc.rust-lang.org/std/fs/fn.rename.html)：Windows と Unix の置換動作の差。
- [Microsoft `ReplaceFileW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew) と [FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)：Windows の置換・同期の候補 API。
- [GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)：CI の OS 別 runner。
