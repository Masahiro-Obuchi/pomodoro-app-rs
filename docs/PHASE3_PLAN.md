# Phase 3 Plan

作成日：2026-09-23
更新日：2026-09-23

本書はPhase 3「TUIでの着手・集中・復帰」を、レビュー可能な変更単位へ分けた実行計画である。Phaseの進捗は[Implementation Plan](IMPLEMENTATION_PLAN.md)だけで管理する。機能の正本は[Product Spec](PRODUCT_SPEC.md)、状態と遷移の正本は[Domain Model](DOMAIN_MODEL.md)、保存契約の正本は[Persistence Schema](PERSISTENCE_SCHEMA.md)とし、本書で別の仕様を定義しない。

## 1. 到達点と境界

Phase 2で通常起動、V1保存、復元、保存失敗の制御と既存キー操作を接続した。Phase 3では、利用者がTUIからCurrent Taskを入力し、Quick Startを始め、Distractionを申告してReturnできるようにする。中止と手動Breakを含む既存の操作、設定、通知、起動・終了の案内も一連の利用経路として揃える。

Current Taskの一覧管理、入力済みSessionの作業名変更、日別集計、詳細履歴、Recovery Timeの可視化は追加しない。振り返りUIの完成とMVP全体の受入検証はPhase 4に残す。V1保存形式を変えず、旧形式の移行も行わない。

TUIはキー入力、編集途中の文字列、表示とヘルプを担当する。`pomodoro-core`の`Command`がSessionの遷移と履歴を決め、既存controllerが保存成功と操作成功の境界を保つ。通常の意味ある操作をTUIから直接`DomainState::apply`して確定させない。

変更の中心は`apps/pomodoro-tui/src/app.rs`の入力分岐、`ui.rs`の状態別表示、必要なら入力draft用の小さなモジュール、`main.rs`のイベント受け渡し、`tests/terminal.rs`の実端末確認とする。`main.rs`は現在、PressのキーコードだけをAppへ渡し、Pasteや修飾キー情報を渡していない。coreにPhase 3専用のUI状態を追加せず、保存形式とplatformの保存・時計APIも維持する。既存controllerで表せない事実が見つかった場合だけ、その契約とテストを同じ変更単位で補う。

## 2. 既存経路と入力の優先順位

| 操作 | 現在の経路 | Phase 3で接続する経路 |
| --- | --- | --- |
| Current Task | 保存済みdraftとSession内の表示、`SetCurrentTask`のcontrollerテスト | Focus・Quick Startの開始待ちでの一行入力、確定・取消・再編集 |
| Quick Start | 種別、2分の計時、完了後の`f`／`c`、復元と保存再試行 | Focus開始待ちからの開始と案内 |
| Distraction / Return | 復元したDistractionからSpaceでReturn | 実行中のFocus・Quick Startからの申告と、その後のReturn |
| Cancelled | coreの終了結果 | Focus・Quick Start・Breakの実行中／中断中からの明示的な中止 |
| Break・既存操作 | 手動開始、Pause／Resume、Reset／Skip、設定、通知 | 新操作と競合しない表示・ヘルプと一連の操作確認 |

新しい通常時キーは`t`（作業名編集）、`2`（2分のQuick Start）、`d`（Distraction）、`x`（中止）を基本割当とする。既存のSpace、`r`、`n`、`s`、`f`、`c`、`q`、`?`と、保存失敗時の`r`／`Q`は維持する。画面の操作案内とREADMEは、各キーが実際に利用できる状態に合わせて更新する。

| 表示中の状態 | 新しいキーの有効範囲 | 維持する主な操作 |
| --- | --- | --- |
| Focus開始待ち | `t`で作業名編集、`2`でQuick Start開始 | SpaceでFocus開始、`s`で設定、`n`で予定をSkip |
| Break開始待ち | `t`・`2`は無効 | SpaceでBreak開始、`n`でFocus開始待ちへ移動 |
| Focus・Quick Start実行中 | `d`でDistraction、`x`で中止 | SpaceでPause、`r`でReset、`n`でSkip |
| Break実行中 | `x`で中止、`d`は無効 | SpaceでPause、`r`でReset、`n`でSkip |
| Distraction中 | `x`で未復帰のまま中止、`d`は無効 | SpaceでReturn、`r`でReset、`n`でSkip |
| Pause・AppExit・ObservationGap中 | `x`で中止、`d`は無効 | SpaceでResume、`r`でReset、`n`でSkip |
| Quick Start継続選択待ち | `t`・`2`・`d`・`x`は無効 | `f`で終了、`c`でFocusへ継続 |

通常の`q`はどの非モーダル状態でも保存して終了する。作業名編集では通常の文字キーを文字として扱い、`q`・`?`・`2`なども入力文字になる。Enterで確定、Escで取消、Backspaceで末尾文字を削除する。Ctrl／Alt付き入力を通常の文字として誤挿入しないよう、必要な修飾キー情報を保持する。設定編集も既存どおり専用モードとする。入力の判定順は、未保存終了の確認 → 保存待ち／終了保存失敗 → 作業名・設定の編集 → 通常操作とする。作業名編集を開いている間もReadyなので計時は始めない。保存失敗へ移ったら編集画面を閉じ、既存の`r`再試行と`Q`未保存終了を表示する。

Bracketed Pasteに対応し`Event::Paste`を送る端末では、貼り付けをまとまった入力として扱う。行区切りを含む`Event::Paste`は編集モード内で一括拒否し、その内容を通常操作へ流さない。編集中以外のPasteはコマンドに変換しない。有効化と解除は端末の初期化・後始末に対で置く。非対応端末などPasteイベントを送らない経路では、貼り付けと通常のキー列を区別できない。最初の改行がEnterとして編集を確定し、後続の`q`や`n`が通常操作になる可能性があるため、複数行貼り付けの一括拒否と操作への流入防止は保証しない。この制約と、単一行の作業名を入力する案内をREADMEに明記する。

入力直前の観測でSessionが自然完了した場合、その入力を次の状態の操作へ流用しない。新しい`d`・`x`も現在のSession IDを持つ`Command`として、既存controllerの完了境界を通す。

## 3. 保存・表示・テストの共通方針

作業名の編集中テキストはTUIだけに置き、Enter前にsnapshotや保存ファイルへ反映しない。Enterでは`CurrentTask::parse`を一度使い、成功した値だけを`SetCurrentTask`としてcontrollerに渡す。長さの独自上限や黙った切り捨ては設けず、画面幅に収まらない文字列も保存値を変えずに表示する。確定済みの同じ作業名を再入力した場合は不要な保存や「変更した」という案内を避ける。

各操作で画面に成功を示すのは保存確定後とし、失敗中は最後に保存済みの状態と未確定候補を区別する。再試行は既存候補だけを保存し直し、Session・Interruption・event・通知を再生成しない。既存の未保存終了は非ゼロ終了とし、通常終了へ読み替えない。

純粋な入力・描画はAppテストで確認し、経過・自然完了・時計異常は注入時計で決定的に確認する。保存失敗と再試行は失敗注入、および必要な箇所で実ファイルを使う。実行ファイルのキー経路と終了状態はPTYテストで確認する。`notify-send`の実起動はテスト成功条件にせず、通知の発火順序を注入した通知先で検証する。各レビュー単位は対応するテストを含む。

## 4. レビュー単位と依存関係

3-0、3-1、3-2を先に進める。3-3は3-0後に独立して進められる。3-4は新しい操作と既存操作の案内を揃え、3-5で全経路を受け入れる。各単位で該当する入力、保存後の表示、失敗時の扱い、テストを揃え、最後までテストだけを先送りしない。レビュー単位が大きくなる場合は、操作経路とその検証を一緒に保ったまま分割する。

### 3-0. Phase 3の実行計画

- **対象**：本書、Implementation Plan、READMEの参照関係。
- **変更**：Phase 2までの利用可能な経路とPhase 3・4の境界を確認し、作業単位と検証を定める。
- **確認・完了条件**：仕様文書との整合、相対リンク、差分の書式を確認する。実装・テストが済んでいない機能を完了扱いしない。Phase 2の完了に依存する。

### 3-1. Current Taskの入力

- **対象**：`app.rs`の編集モード、必要な入力draft、`main.rs`のキー修飾子・Paste受け渡し、`ui.rs`、Appテスト、PTYテスト、README。
- **変更**：Focus開始待ちで`t`を押すと保存済みdraftを初期値にして開く。文字追加・末尾削除、Enterで確定、Escで取消を実装する。`Event::Paste`で受けた行区切りを含む貼り付けは一括拒否し、それ以外の空白・前後空白の正規化は`CurrentTask::parse`に従う。不正な文字列は編集を維持して理由を表示する。作業名付きでFocusを始めたらSessionへ固定し、Resetでは次のdraftへ戻す。Break開始待ち・Active・選択待ちからは開かない。
- **保存境界**：Escや編集中の文字操作は保存しない。Enterだけが`SetCurrentTask`をcontrollerへ渡し、成功後に編集を閉じる。同じ内容ならno-opとして扱う。保存候補が作られた後の失敗では編集を閉じて保存待ちを示し、固定候補を再試行する。時計読取など候補を作れないエラーでは編集内容を維持して原因を表示する。未保存終了後も成功表示はしない。
- **テスト・完了条件**：日本語、空白・前後空白、Backspace、取消・再編集、`q`／`?`を文字として入力する場合、修飾キー、`Event::Paste`で受ける単一行・複数行の貼り付け、同値確定、設定モードとの競合、保存失敗・再試行をAppで確認する。実ファイルとBracketed Pasteを送るPTYで、確定前にファイルが変わらないこと、Pasteの内容が通常操作へ漏れないこと、Focus開始後と再起動後の作業名、未入力開始を確認する。Pasteを送らない通常キー列では改行で編集が確定し得ることを確認し、この制約をREADMEに記す。長い入力と狭い端末でも全内容を保持し、編集操作を見失わない。3-0に依存する。

### 3-2. Quick Startの通常開始

- **対象**：`app.rs`のReady入力分岐、`ui.rs`の開始待ち・選択待ち案内、App／PTYテスト、README。
- **変更**：Focus開始待ちの`2`だけを`Start(QuickStart)`へ接続し、Spaceは通常Focus開始のまま保つ。保存済みCurrent TaskのdraftをQuick Startへ引き継ぐ。Break開始待ちでは`2`を無効とし、`n`でFocus開始待ちへ戻ってから選べる。自然完了後は既存の`f`でFinish、`c`で全時間の新しいFocusへContinueする。
- **保存境界**：開始、自然完了、Finish／Continueの候補は既存controllerで保存確定してから表示・通知する。選択待ちに時間を加算せず、アプリ終了・再起動でも自動選択しない。Continue再試行でFocus・ID・eventを重複作成しない。
- **テスト・完了条件**：Appの注入時計で2分の境界、作業名継承、選択待ち、Finish／Continueの別結果、Focus完了数・ラウンドがQuick Startだけでは増えないことを確認する。PTYで`2`の開始と実ファイル保存を確認し、終了・再起動した選択待ちも検証する。開始・選択の保存失敗は再試行後の一回だけの遷移を確認する。実時間で2分待たない。3-1に依存する。

### 3-3. Distraction申告とReturn

- **対象**：`app.rs`の実行中入力、`ui.rs`の中断理由と操作案内、App／PTYテスト、README。
- **変更**：Focus・Quick StartがRunningのときの`d`を、現在のSession IDを持つ`Distraction`へ接続する。中断中やBreakでは受け付けず、Distraction中のSpaceは既存の`Return`として使う。Return後は残り時間から再開する。
- **保存境界**：申告とReturnはそれぞれ保存後に成功を表示する。失敗時は通常操作と計時を保留し、再試行で同じInterruptionやReturnを重複させない。終了・再起動してもDistractionを維持する。
- **テスト・完了条件**：Focus・Quick Startの申告とReturn、Break・Pause・AppExit・ObservationGap中での無効キー、申告前後の作業時間、中断中の終了と再起動後のReturnを確認する。Recovery Timeに終了中の時間を含める一方、作業時間へ加算しないこと、時計異常時はReturnの事実を残して時間を不明とすることを注入時計で確認する。申告・Return各々の保存失敗と再試行、およびPTYの実キー経路を確認する。3-2が未実施なら、Appテストではcoreの`Start(QuickStart)`で実行中状態を作る。PTYテストでは同じ状態をplatformで有効なV1ファイルに保存して起動し、復元後のObservationGapからSpaceで再開して`d`・Spaceで申告とReturnを確認する。これによりQuick Startの開始キーに依存せず検証できる。3-0に依存し、3-2と独立に進められる。

### 3-4. 明示的な中止と終了結果の案内

- **対象**：`app.rs`のActive入力と保存後メッセージ、`ui.rs`の案内、App／PTYテスト、README。
- **変更**：Running・InterruptedのFocus、Quick Start、Breakで`x`を`End { outcome: Cancelled }`に接続する。保存済みの作業時間は残し、Focus開始待ちへ戻す。Distraction中ならReturnを作らず、InterruptionをSessionEndedで閉じる。Resetは同じ種別・Current Taskの開始待ち、Skipは次の予定へ進む操作として区別して案内する。Readyや選択待ちの`x`は無効とする。
- **保存境界**：中止は保存確定後だけ表示し、失敗中は元のSessionを保存済み状態として表示する。再試行で終了Sessionや中断終了eventを増やさない。
- **テスト・完了条件**：各KindのRunning・Interruptedからの中止、Distraction未復帰の中止、長休憩中止時のラウンド、Reset／Skipとの結果と次の予定の違いを確認する。入力直前の自然完了で`x`を次Sessionへ流用しない。保存失敗・再試行とPTYでのキー経路も確認する。3-3に依存する。

### 3-5. 操作フローの受入と案内

- **対象**：TUIの状態別表示・ヘルプ、README、実ファイル・PTYの統合テスト、Implementation Plan。
- **変更**：Current Task、Quick Start、Focus、Break、Pause、Distraction、Return、中止、Reset／Skip、設定、通知を一つの利用経路として照合する。画面に出すキーと実際の入力分岐を全状態で揃え、重要な案内が狭い端末で欠ける場合は調整する。起動失敗、保存待ち、未保存終了の案内も新操作と整合させる。
- **テスト・完了条件**：実ファイルとPTYで作業名付きQuick Start開始、Distraction後の終了・再起動・Return、手動Break、保存失敗と再試行を確認する。長い待機を要する完了・継続は注入時計と実ファイルの統合テストで補う。保存前に成功通知しないこと、履歴・snapshot・表示が保存後に一致すること、最低Rustバージョンを確認する。Phase 3の完了条件を満たしたときだけImplementation Planを完了へ更新する。3-1～3-4に依存する。

## 5. 検証とPhase 3終了条件

Rustを変更する各レビュー単位で関連テストに加え、次を通す。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

時刻と保存失敗は注入して決定的に検証し、実端末・実ファイルのテストは入力と保存後の結果を確認する。長いsleepに依存しない。Phase 3の終了時には最低Rustバージョンでも確認する。

| Phase 3で通す利用経路 | 主な確認方法 |
| --- | --- |
| 作業名を確定・取消して終了／再起動し、Focus・Quick Startへ引き継ぐ | Appの入力テスト、PTY入力、V1ファイル再読込 |
| Quick Startを開始・自然完了し、FinishまたはContinueを選ぶ | 注入時計のApp／controllerテスト、実ファイル保存、PTYでの開始入力 |
| Distractionを申告して終了／再起動し、同じ中断をReturnで閉じる | 注入時計、V1ファイル再読込、PTYでの申告・Return入力 |
| Focus・Quick Start・Breakを中止し、Reset・Skipとの差を保つ | 状態別入力と保存結果のApp／PTYテスト |
| 新操作の保存が失敗し、再試行または未保存終了を選ぶ | 失敗注入、候補・ID・event・通知回数、実ファイルと終了コード |

実行ファイルだけでは自然完了を待たずに再現しにくい境界は、注入時計と実ファイルのテストを組み合わせる。Phase 4では、ここで動作を確認した経路を使い、Product Specの確認例と最小振り返り表示を一連で照合する。

- Focus・Quick Startの開始前にCurrent Taskを任意入力でき、確定前には保存せず、Sessionと復元後の表示に保持される。
- Quick Startの開始、自然完了後の終了／継続、Pause／ResumeをTUIから操作でき、待機時間を計時しない。
- Focus・Quick StartでのDistraction申告とReturn、各Sessionの中止、手動BreakをTUIから操作でき、Reset・Skipとの結果の違いも記録される。
- 意味ある操作の成功表示と通知は保存成功後に限られ、保存失敗中の保留・再試行・未保存終了が新操作でも機能する。
- README、状態別ヘルプ、実ファイル・端末の確認が実際の操作と一致し、重要な操作と保存失敗の案内が表示できる。Phase 4の振り返りUIとMVP全体の受入を完了扱いしない。
