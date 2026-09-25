# 自動テスト・CI拡充計画

作成日（日本時間）：2026-09-26

状態：計画策定。実装は未着手。完了済みPhase 0–4とWindows/macOS W5aの進捗は変更しない。W5bの実端末受入は独立して保留する。

基準となる[W5aの4 OS CI実行](https://github.com/Masahiro-Obuchi/pomodoro-app-rs/actions/runs/36166647526)は全job成功。後続PRでは同じ検証の有無と実行時間を比較する。

## 1. 目的と範囲

- 変更がcore・platform・TUI・CI設定のどこに入っても、マージ前に必要な検証結果が得られるようにする。
- 保存値と画面のように層をまたぐ不具合を、実際の操作と保存結果で検出する。既存テストと同じassertionを増やすことは目標にしない。
- Rust 1.86の最低対応を維持し、Linux、Windows x86_64、macOS Apple Silicon/IntelのCI結果を記録する。CIの疑似端末はW5bの実端末確認に数えない。
- リリース・crates.io公開、Webアプリ、UI機能追加はこの計画に含めない。後続タスクはここで整えたCIを使う。

## 2. 現状と優先する抜け

| 現状の証拠 | 残るリスク・扱い |
| --- | --- |
| [現行workflow](../.github/workflows/platform-probe.yml)は`pull_request`と変更パスで起動し、`crates/pomodoro-core/**`を含まない | coreだけを変更したPRでは4 OSのテストが走らない。path filterを除いて全PRを検証する |
| workflowは`push`で起動しない。mainの既存rulesetはPR経由の変更などを要求するが、CI成功は要求していない（計画時点のGitHub設定） | マージ後のコミットを検証する実行と必須チェックがない。先に安定したcheckを作り、その後に既存rulesetへ必須チェックを追加する |
| Rust 1.86でformat、clippy、保存probe、workspace全テスト、保存途中の子プロセス停止、buildを4 OSで実行する | この基準を減らさず、最新stableとの互換性をLinuxの追加jobで確認する。OS固有の実装をLinuxの結果だけで合格にしない |
| coreに8 seed×400操作の決定的な混合操作テスト、platformにV1 codec・失敗注入・別プロセス・復旧テスト、TUIにLinux PTYと4 OSのネイティブPTY/ConPTYテストがある | 既存ケースを複製しない。ネイティブPTYで未検証の未保存終了と、狭い画面の**現在表示中の内容**を優先する |
| `native_terminal.rs`の表示assertionは制御列を除いた出力履歴から文字列を探す | 後続の描画で消えた文字列でも成功し得る。画面の最終状態を検査する方法を導入する |

GitHubのpath filterでworkflowを飛ばすと、必須チェックにした場合はそのcheckがPendingのままマージを妨げ得る。必須CIは変更パスでworkflow自体を省略しない。[GitHub workflow構文](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax)

## 3. 実行順序とレビュー単位

| 順序 | 変更内容 | 完了の証拠 |
| --- | --- | --- |
| CI-1：起動条件と権限 | `platform-probe.yml`のpath filterを外し、全PRとmainへのpush、手動実行で同じ4 OS検証を行う。`GITHUB_TOKEN`を`contents: read`に限定し、check名を固定する。新しいpushで同一PRの古い実行だけをキャンセルし、mainの実行は残す。merge queueを採用する場合のみ`merge_group`を追加する | workflow変更PRで4 OS成功。マージ後のmain push実行が成功 |
| CI-2：coreだけの変更で起動確認 | `domain_tests.rs`の既存の決定的な混合操作テストにseed・操作番号・時刻・commandを示す失敗診断を追加する。変更対象をcoreだけに限定する | coreだけを変更したPRに4 OS checkが作られ、失敗時の操作列を再現できる。既存の8 seed×400操作を単に増やさない |
| CI-3：表示と未保存終了の実行ファイルテスト | `native_terminal.rs`の出力履歴assertionを画面スナップショットで補強する。保存失敗時の24×20の再試行・未保存終了キーと、24×9の拡大案内を実際の最終画面で検査する。`Q`→`y`の未保存終了を4 OSのPTY/ConPTYで通す | 各OSで終了コードが非0、未確定候補が本体へ入らない、ロックが解放され再起動できることを確認。描画が後で消えた場合に失敗するテストになる |
| CI-4：新しいtoolchainと診断 | Rust 1.86の4 OS jobを維持し、Linuxに最新stableでのworkspace test/buildを追加する。各jobでRust/Cargo版を記録する。保存境界・画面サイズが分かるassertionへ必要箇所だけ改める | 1.86とstableの結果を区別してPRに表示できる。障害を再現する条件がログから取れ、テストの自動再試行で失敗を隠さない |
| CI-5：mainのマージ条件 | CI-1～4のcheck名と実行時間が安定した後、GitHubのmainの既存rulesetへ4 OSの必要なcheckを追加する。既存のPR・履歴保護規則を維持し、設定とcheck名を記録する | coreだけ、文書だけ、通常のコード変更のPRに必須checkが現れ、通常のマージで失敗したPRがmainへ入らない。main pushの結果も確認できる |

CI-3は既存のLinux `terminal.rs`で検証済みの入力経路を4 OSへ広げる。表示の検査には端末の現在のセルを追跡できる仕組みを選び、ANSI列を手書きで広く再実装しない。既存のタイムアウトと一時保存先の隔離を保ち、実時間で25分待つテストは追加しない。

CI-2でcoreの混合操作テストを見直す場合は、seed数だけを増やさず、失敗した操作列を再現できる診断を先に整える。codecの全状態roundtrip、保存失敗matrix、TUI workflowの既存assertionを棚卸しし、独立した期待結果のない重複テストは作らない。coverage率は当初の合否基準にせず、未検証の重要な経路を見つけた場合に別PRで測定方法を決める。

## 4. CI-1の起動確認と運用

1. workflow変更PRで4 OSのformat・clippy・probe・全テスト・クラッシュ境界・buildが成功する。
2. CI-1をマージしたmain pushで同じ4 OS jobが成功する。PR上の成功だけでmainの設定完了としない。
3. CI-2のcoreだけを変更するPRで、path filterに頼らず4 OS checkが生じることを確認する。文書だけのPRでもcheckが省略されないことを確認する。
4. check名を確認した後にCI-5を適用する。既存PRのPending状態や、同名checkの取り違えがないことを確認してから必須化する。

PRでは`pull_request`を使い、secretを使う公開・配布処理を同居させない。GitHubはPR用の一時的なmerge commitをcheckoutするため、PRのテストは通常、baseとの結合結果を検証する。[GitHubイベント仕様](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows) 読込専用tokenは[GitHubの権限指針](https://docs.github.com/en/actions/reference/security/secure-use)に従う。

4 OSの全テストを文書だけのPRにも実行するため、実行時間・失敗率を各PRで記録する。PRの古い実行だけを止める設定は[GitHubのconcurrency構文](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#concurrency)を使う。費用や待ち時間が実際の問題になった場合に、常時走る必須checkを残したままjob分割やcacheを別PRで検討する。cacheの導入は速度だけで判断せず、forkからのPRと書込権限の境界を確認する。[GitHub cacheの指針](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching)

## 5. 全体の完了条件

- coreのみのPRを含む全PRとmain pushで必要なCIが起動し、4 OSのRust 1.86 jobが成功する。Linuxのstable jobも成功する。
- 24×20と24×9の現在の画面、未保存終了後の保存値・終了コード・排他解放を、ネイティブPTY/ConPTYテストで確認する。
- 必須checkと実際に起動するjobが一致し、失敗したPRがmainへ入らない。設定済みのcheck名と代表的な実行URLをPRまたは本書に残す。
- W5bの実端末受入は未完了のまま記録し、CIの成功をWindows Terminal・Terminal.appや通知の実表示の証拠として扱わない。

実装で仕様上の振る舞いを変える必要が生じた場合は、対応するProduct Spec・Domain Model・Persistence Schemaを先に更新する。テストとCIの構成だけを変えるPRでは保存契約やユーザー操作を変更しない。
