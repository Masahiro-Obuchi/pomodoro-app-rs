# Focus・Break自動開始 実装計画

作成日：2026-09-30

状態：完了。仕様は [Product Spec](PRODUCT_SPEC.md) §5、[Domain Model](DOMAIN_MODEL.md) §11、[Persistence Schema](PERSISTENCE_SCHEMA.md) §3・§7で定義する。本書は既存TUIへの実装順序と検証項目を記録する。

## 1. 到達点と適用範囲

初回Focusを手動で開始した後、アプリが起動していて連続した時間を観測できる場合は、Focus自然完了で短休憩（4回目は長休憩）を、Break自然完了で次のFocusを自動開始する。各区間は別IDのSessionとし、前のSessionの完了と新Sessionの開始を一つのドメイン遷移・保存候補にする。新Sessionの開始時刻は完了を認識した観測時刻とし、超過した観測時間を繰り越さない。

Break後のFocusは、そのBreakより前に最後に終了したFocusのCurrent Taskを引き継ぐ。該当するFocusがなければ未入力とする。Quick Start自然完了は従来どおり選択待ちとし、Skip・中止・Resetでは開始待ちに進む。アプリ終了中、再起動時、Observation Gap、保存失敗中には自動で時間を進めない。

## 2. 変更箇所と実装順序

### 2.1 コアの遷移と決定的なテスト

対象：`crates/pomodoro-core/src/domain.rs`、`crates/pomodoro-core/src/domain_tests.rs`。

1. `end_session`で既存Sessionの実行区間を閉じ、Completedと終了時刻を付け、ラウンド進捗と次の種別を決定する。Focus・Breakの自然完了時だけ、終了済みSessionを履歴に置いてから`start_session`で別IDのRunning Sessionを作る。Quick StartとCompleted以外の経路は既存の状態遷移を保つ。
2. Break完了時は履歴を末尾から調べ、当該Breakより前の直近のFocusからCurrent Taskを複製する。BreakにはTaskを設定しない。新しい永続化フィールドや、履歴から状態を再生する仕組みは追加しない。
3. `observe_inner`の現在の`elapsed.min(remaining)`を維持し、新Sessionの開始を`observation.at`に固定する。同じ観測の余剰時間で次Sessionを進めたり、複数Sessionを連続完了させたりしない。観測空白の判定を自然完了より先に行う順序も維持する。
4. 遷移途中でID採番やevent採番が失敗しても、`DomainState::atomic`によりsnapshot・履歴・採番値が一緒に戻ることを確認する。

検証：Focus→短休憩→Focusを操作なしで通し、4回目のFocus後は長休憩、長休憩後は進捗0のFocusとなることを確認する。完了日時・新Session開始日時、ID、経過時間、作業時間、Taskの継承を照合する。Taskがない場合、手動開始したBreakの場合、1回の観測が残り時間を超えた場合、Pause／Resume後の完了も含める。Quick Startの自然完了・Skip・中止・Reset・Observation Gap・復元では次Sessionを自動作成しないことを確認する。既存のReady前提のテスト補助関数と期待値を新仕様に合わせて更新する。

### 2.2 保存・通知・ライフサイクル

対象：`apps/pomodoro-tui/src/controller/`とそのテスト、`crates/pomodoro-platform`のV1往復・実ファイルテスト。

1. コントローラーの`apply_observation`が、前Sessionの履歴追加を「意味のある遷移」と判定し、完了したSessionと新しいActiveを同じ候補として直ちに保存することを確認する。現在の完了種別検出が「1観測で完了は最大1件」という仕様を満たすことも確認する。
2. 保存成功後に前Sessionの完了通知を一度だけ送る。保存前の通知、再試行による二重通知、新Sessionの開始通知としての誤報を防ぐ。通知失敗でも確定済みの新Sessionを巻き戻さない。
3. 保存前失敗・候補保存中失敗・確定不明の再試行では、完了Session、新Session、ID、候補時刻を固定する。保存待ち中の時間は進めず、回復後は自動開始した新SessionをObservation Gapで中断して手動再開を待つ。回復用保存が再度失敗した場合も同じ候補を再試行する。
4. 完了と同じ観測で`shutdown`した場合は、次Sessionを作ってからAppExit中断として保存する。アプリ終了中の時間を計上しない。再起動時に保存済みRunningから復元する場合も、新Sessionを作らず中断にする。
5. V1 JSONは既存の`history.sessions`と`active`で表現できるため、形式・`schema_version`を変えない。既存のReady保存データをそのまま読み込み、新たな自動開始状態を保存・読込してID、Task、ラウンド進捗、実行区間が一致することを実ファイルで確認する。

検証：保存成功、保存失敗からの複数回再試行、通知失敗、完了時刻ちょうどの終了、終了後の復元をコントローラーの決定的な時計・保存スタブで試す。再試行前後の保存候補と通知ログを照合し、V1の実ファイル往復も行う。保存層の変更が不要であることをコードとテストで確認する。

### 2.3 TUI、案内、実行経路

対象：`apps/pomodoro-tui/src/app.rs`、`src/app/actions.rs`、`src/ui.rs`、`src/app/tests/`、`tests/workflow.rs`、`tests/lifecycle.rs`、`tests/terminal.rs`、`crates/pomodoro-platform/src/notification.rs`、`README.md`。

1. 自然完了後に表示される新Sessionの種別・Running状態・残り時間・ラウンド・Taskが保存済み状態と一致するようにする。通常の`Ready`操作は、初回開始とSkip・中止・Reset後に引き続き使えるようにする。
2. TUIの完了メッセージとデスクトップ通知から「次SessionはReady」という表現を除く。保存回復で新Sessionが中断される場合があるため、通知は完了した事実と次の種別を伝え、現在Runningであると断定しない。画面の状態表示でRunning／中断を示す。
3. 完了直前に押されたキーは元のSessionへの操作として扱い、完了後の新Sessionへ流用しない。画面に表示していた操作を時間観測後に別Sessionへ再解釈しない既存の制御を、Focus→BreakとBreak→Focusの双方で確認する。
4. READMEの操作説明、休憩の開始方法、通知文言、手動開始が残るケースを新仕様に合わせる。History表示が完了Sessionを一度だけ集計し、進行中の新Focusも通常どおり計上することを確認する。

検証：短いテスト用設定と疑似端末で、初回Focusの開始から短休憩・次Focusの自動開始とTask表示を確認する。4回目の長休憩、終了・再起動、Historyやヘルプを開いている間の計時・切替は決定的なテストと実ファイル往復で確認する。Quick Start選択待ち、Skip後の手動開始、狭い端末の保存失敗表示も既存の操作経路で確認する。

## 3. 重点的な回帰確認

| 境界 | 期待結果 |
| --- | --- |
| 完了時刻ぴったり／少し超えた観測 | 前Sessionだけ完了し、新Sessionは経過0から始まる |
| 4回目のFocusと長休憩完了 | 長休憩を自動開始し、完了後のFocus開始時に進捗が0になる |
| Task付き／TaskなしのBreak完了 | 直近FocusのTaskを継承、または未入力でFocusを開始する |
| 睡眠・時刻異常・観測空白 | 現在のSessionを中断し、期限超過だけでは自然完了・自動開始しない |
| 保存失敗・確定不明 | 同じ候補とIDで再試行し、回復後の新Sessionは手動再開待ちになる |
| 完了と同時の入力／終了 | 入力を新Sessionへ転用せず、終了では新SessionをAppExit中断にする |
| 旧V1保存データ／新しい自動開始状態 | 旧データはそのまま読み込み、新状態もV1で往復できる |

## 4. 実施単位と完了条件

実装は2.1→2.2→2.3の順に進める。コア変更で既存テストの期待値が変わるため、各段階で関係するテストを更新し、最終的にはコア・コントローラー・TUI・利用文書を一つの動作としてレビューする。旧V1形式の互換性と保存失敗からの復帰を確認する前に完了扱いにしない。

完了条件は、Product Spec §10の自動開始例がコード・保存記録・画面で一致し、上表の境界で二重Session・二重通知・終了中の時間加算が起きないこと。`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`を通し、変更した操作フローを疑似端末で確認する。テストと検証結果は実装のレビュー時に記録する。

## 5. 実施結果

コアの自然完了から次Sessionの作成、保存と通知、TUI表示・利用案内を更新した。既存V1形式を変更せず、自動開始済み状態の実ファイル往復と復元後の中断を確認した。保存失敗の再試行、完了と同時の終了、Task継承、4回目の長休憩、切替境界の入力、History・ヘルプ表示中の切替を決定的なテストで確認し、実行ファイルの疑似端末テストでFocus→短休憩→Focusを確認した。`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`が成功した。
