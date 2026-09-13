# Domain Model

採用日：2026-09-12

更新日：2026-09-14

この文書は [Product Spec](PRODUCT_SPEC.md) をドメイン上の状態・データ・制約として定義する。今後の実装の設計基準であり、現在のコードがすべて実装済みであることを意味しない。型名・フィールド名は概念上の名称であり、保存JSONの表現やRustの公開APIとは区別する。

## 1. 設計原則と所有関係

- 現在状態をsnapshotとして直接保持・復元する。
- eventは履歴・分析用とし、Event Sourcingにはしない。
- 現在状態をevent replayだけから復元しない。
- snapshotと対応する履歴の変更を一つの保存単位として確定する。
- 同時に実行・中断できるSessionは1つ、開けるInterruptionも1つとする。
- UI、OS、ファイルI/O、通知をコアの状態遷移へ持ち込まない。
- 汎用イベントバス、継承階層、イベント再生エンジンなどの不必要な抽象化を導入しない。

```text
保存データ
├─ snapshot / PomodoroState
│  ├─ Settings
│  ├─ ラウンド進捗
│  └─ 進行状態
│     ├─ Ready
│     ├─ Active：Session + TimerState
│     └─ AwaitingQuickStartDecision
├─ History
│  ├─ 終了済みSession
│  └─ HistoryEvent
└─ 保存管理情報：schema version、保存世代、ID採番情報
```

実行・中断中のSession本体はsnapshotに置き、終了時にHistoryへ移す。終了済みSessionをsnapshotとHistoryの両方で更新し続けない。

開いているInterruptionはTimerStateが所有する。閉じたInterruptionは履歴eventに記録し、Session内に別の中断履歴一覧を重複保持しない。

## 2. 時間モデル

### 2.1 日時と計時時間

日時はUTCを基本とし、出来事がいつ起きたかを表す。計時済み時間は、信頼できる実行区間でタイマーを進めたmsを表す。両者を区別する。

- `Session.elapsed_ms`を計時済み時間の正本とする。
- 残り時間は`planned_duration_ms - elapsed_ms`から求める。
- 記録上の作業時間はFocus・QuickStartの計時済み時間であり、Breakは含めない。
- 実行中の表示用に締切を計算しても、その締切を再起動後の作業実績の根拠にしない。
- プロセス内の経過時間を測る実行時のアンカーを、そのまま永続化しない。

### 2.2 時間の適用順序

1. 観測の連続性、観測空白、時計異常を確認する。
2. 信頼できる経過時間だけを残り時間の範囲内で反映する。
3. 設定時間に達した場合、自然完了のドメイン遷移を生成する。
4. 対象Sessionがまだ操作可能なら、ユーザー操作を適用する。

Observation Gapは自然完了判定より先に処理する。保存された期限が過ぎたことだけを理由に、Focus・QuickStart・Breakを完了させない。

同一プロセス内では最後に確認できた時点、再起動時には保存済みの最終確認時点を計時の境界とする。不明区間を推測して加算しない。

### 2.3 Observation Gapの内部ポリシー

観測空白の判定条件とチェックポイント間隔は内部ポリシーとする。MVPのユーザー設定として公開せず、このドメインモデルでは具体的な数値を固定しない。

プラットフォーム側が時刻・経過・再起動などの観測情報を提供し、コアの計時規則に従って状態を遷移させる。TUIが独自の計時・完了規則を実装しない。

内部ポリシーの閾値にかかわらず、正常終了中は計時せず、Runningのsnapshotを再起動で読み込んだ場合は最終保存以降を不明区間として扱う。

## 3. Session

形式は`struct`。一度開始した作業または休憩の試行を表す。開始待ちでは作成しない。

### データ

| 項目 | 内容 |
| --- | --- |
| `id` | Sessionの識別子 |
| `kind` | SessionKind |
| `current_task` | 任意のCurrent Task |
| `planned_duration_ms` | 開始時に確定した設定時間 |
| `started_at` | 開始日時 |
| `elapsed_ms` | 最後に反映した時点までの計時済み時間 |
| `continued_from_quick_start` | 継続元Quick StartのID。継続以外ではなし |
| `end` | 未終了ならなし。終了後は終了日時とSessionOutcomeをまとめて保持 |

終了日時と終了結果は一つの終了情報にまとめ、片方だけ存在する状態を作らない。

### 関係とinvariant

- 実行・中断中はsnapshotのActiveに所属し、終了情報を持たない。
- 終了後はHistoryに所属し、終了情報を持つ。
- 実行・中断の状態はTimerStateが表す。
- `planned_duration_ms > 0`。
- `0 <= elapsed_ms <= planned_duration_ms`。
- Completedなら`elapsed_ms == planned_duration_ms`。
- 開始後に種別・設定時間・Current Taskを変更しない。
- BreakにはCurrent TaskとQuick Start継続元を持たせない。
- 同じSessionを二度終了させない。

## 4. SessionKindとSessionOutcome

### 4.1 SessionKind

形式は`enum`。Sessionの用途を表す。

| Variant | 作業時間への寄与 | Pomodoro完了回数への寄与 |
| --- | --- | --- |
| Focus | あり | 自然完了時に1回 |
| QuickStart | あり | なし |
| ShortBreak | なし | なし |
| LongBreak | なし | なし |

QuickStartはMVPでは固定120秒とする。実行中・中断中などの状態をSessionKindに混ぜない。

### 4.2 SessionOutcome

形式は`enum`。Sessionの終了情報に属し、試行の終了方法を表す。

| Variant | 意味 |
| --- | --- |
| Completed | 設定時間を計時し終えた |
| Cancelled | 今回の試行を中止した |
| Reset | リセット操作で今回の試行を終了した |
| Skipped | 次の予定へ進む操作で今回の試行を終了した |

- 未終了状態、Pause、Distraction、アプリ終了はSessionOutcomeに含めない。
- Completed以外をFocus完了回数へ加算しない。
- 途中終了でも計時済み時間を消さない。
- Quick Startの終了後の選択は別の事実として保持し、SessionOutcomeを後から書き換えない。

## 5. TimerState

形式は`enum`。開始済み・未終了のSessionが現在どう計時されているかを表す。

| Variant | データ |
| --- | --- |
| Running | 現在の実行区間の開始日時、その開始時点の累積計時時間、最後の確認日時 |
| Interrupted | 開いているInterruption |

### 関係とinvariant

- snapshotのActiveにおいてSessionと組になる。
- 開始待ちは進行状態のReadyで表し、TimerStateにIdleを置かない。
- 終了後はSessionをHistoryへ移し、動作中のTimerStateを持たない。
- RunningとInterruptedは排他的とする。
- Interruptedでは`Session.elapsed_ms`を増やさない。
- Runningに開いたInterruptionを持たせない。
- 残り0のRunningを安定した保存状態として保持せず、終了処理へ進む。
- 実行区間開始時の累積値は、現在の`Session.elapsed_ms`以下とする。

実行区間開始時の累積値は、その後の差分から区間内の計時時間を確定するための境界値である。残り時間の別の正本にはしない。

## 6. Interruption

形式は`struct`。一つの中断の開始と終了を対応付ける。

### データ

| 項目 | 内容 |
| --- | --- |
| ID | 採用中の保存系列内の全中断で一意な識別子。所属Session内の一意性も満たす |
| kind | InterruptionKind |
| 開始境界の日時 | 計時を止めた境界。Distractionでは申告時刻 |
| 記録日時 | アプリが中断を認識・記録した日時 |
| 時刻の不確かさ | 検出した時計異常などの情報 |
| 終了情報 | 終了日時、InterruptionOutcome、算出できる経過時間。不明ならその理由 |

Observation Gapでは開始境界と記録日時が異なり得る。最後の確認が10:00、復元が11:00なら、計時境界は10:00、認識時刻は11:00となる。これは実際の中断開始時刻を推測した記録ではない。

### 所有関係とinvariant

- 開いているInterruptionはTimerStateのInterruptedが所有する。
- 閉じたInterruptionは履歴eventへ移し、TimerStateから取り除く。
- 同時に開いているInterruptionは全体で最大1件。
- 開いている間は終了情報を持たない。
- 一度閉じたInterruptionを再利用しない。
- 開始・終了のeventは同じSession IDと中断IDで対応付く。
- 不明な経過時間を0秒として扱わない。

### 6.1 InterruptionKind

形式は`enum`。計時を止めた理由を表す。

| Variant | 意味 |
| --- | --- |
| Pause | ユーザーの意図的な中断 |
| Distraction | ユーザーによる脱線申告 |
| AppExit | 正常終了操作による計時の停止 |
| ObservationGap | 観測空白・復元時の不明区間による停止 |

- DistractionはFocus・QuickStartでだけ許可する。
- AppExit・ObservationGapをPause回数やDistraction回数に混ぜない。
- すでに中断中なら別の中断を重ねたり、理由を切り替えたりしない。
- Distraction中に終了・再起動しても、KindはDistractionのままとする。
- AppExitは終了操作による停止を表し、OS上でプロセスが消滅した正確な時刻を表すものではない。

### 6.2 InterruptionOutcome

形式は`enum`。中断の終了情報に属し、終了方法を表す。

| Variant | 許可する中断 | 意味 |
| --- | --- | --- |
| Resumed | Pause・AppExit・ObservationGap | 手動再開した |
| Returned | Distraction | Return操作で作業に戻った |
| SessionEnded | すべて | 元のSessionを中止・Reset・Skipした |

- SessionEndedには対応するSessionOutcomeを記録する。
- DistractionをResumedで閉じず、それ以外の中断をReturnedで閉じない。
- SessionEndedを復帰成功として数えない。
- 中断中は計時しないため、自然完了によって中断を閉じない。
- Recovery Timeの完了サンプルはDistractionかつReturnedの場合だけとする。

Return時には、アプリ終了中も含めて申告からの経過時間を扱う。時計異常などで算出できない場合もReturnedという結果は保持し、時間だけを不明とする。

## 7. Current TaskとQuick Startの関連

### 7.1 Current Task

形式は文字列を保持する小さな`struct`とし、任意性を`Option`相当で表す。

- Sessionでは開始時の値を保持する。
- Readyでは編集用の下書きを保持する。
- 空白だけの入力は未入力とする。
- 値がある場合は空でなく、改行を含まない。
- Session開始後は値を変更しない。
- Breakでは保持しない。
- Task ID、完了状態、階層、期限、優先度、プロジェクトは持たせない。

### 7.2 Quick StartからFocusへの関連

独立した関連Entityは作らない。Focusの`continued_from_quick_start`に継続元IDを保持し、選択結果を履歴eventへ記録する。

選択結果はFinish / Continueの小さなenumで表す。Continueのeventには作成したFocus IDを記録する。

#### Invariant

- 継続元を持てるのはFocusだけ。
- 継続元は自然完了済みQuickStartである。
- 一つのQuick Startから作れる継続先Focusは最大1つ。
- Focusには設定された全時間を与え、Quick Startの時間を加算・控除しない。
- Current Taskの値を引き継ぐ。
- Quick StartのCompletedという結果は、継続選択後も変更しない。
- 継続判断、Focus作成、現在状態の切替を一括保存する。
- 継続先FocusをResetした後の新しいFocusに、同じ継続元IDを再利用しない。

## 8. Pomodoroのラウンド進捗とSettings

ラウンド進捗は`completed_focuses_in_round`を保持する`struct`とする。長休憩までの回数NはSettingsを参照する。独立したRound IDやラウンド履歴Entityは作らない。

### 更新規則

- `0 <= completed_focuses_in_round <= N`。
- Focus自然完了で1増やす。Nに達したら長休憩、それ以外は短休憩を用意する。
- Quick Start、Pause、Distraction、Resume、Returnは進捗を変えない。
- 長休憩の完了・中止・SkipでFocus開始待ちへ進むとき、0に戻す。
- 長休憩のResetでは、長休憩をやり直すため進捗を維持する。
- 開始前の長休憩をSkipした場合も0に戻すが、実行していないSessionの履歴は作らない。

Settingsは既存のFocus・短休憩・長休憩の時間と、長休憩までのFocus回数を保持する。各Sessionは開始時に設定時間をコピーし、その後の設定とは独立させる。

設定変更はReadyでだけ許可する。適用時は既存仕様どおりラウンド進捗を0に戻し、Readyの種別を維持する。履歴上のFocus完了数は変更しない。

## 9. 現在状態のsnapshot

### 9.1 PomodoroState

形式は`struct`。現在の作業全体の状態を表し、Settings、ラウンド進捗、進行状態を保持する。この値をsnapshotとして保存する。

### 9.2 進行状態

形式は`enum`とする。

| Variant | データ |
| --- | --- |
| Ready | 次に開始する種別、Current Taskの下書き |
| Active | 未終了のSession、TimerState |
| AwaitingQuickStartDecision | 完了したQuick StartのID、継続先に渡すCurrent Task |

Quick Start本体は時間満了時点でHistoryへ移す。AwaitingQuickStartDecisionには、現在の選択待ちと次のFocus開始に必要な情報を残す。

### Invariant

- 上記3状態のうち一つだけが成立する。
- Readyに開始済みSessionを持たせない。
- Activeに終了済みSessionを持たせない。
- 継続選択待ちの参照先は自然完了済みQuickStartである。
- 継続選択待ちのCurrent Taskは、継続元から引き継ぐ値と一致する。
- 継続選択待ちの間、新たな独立Sessionを開始しない。
- ヘルプ、カーソル、キー割当、画面レイアウトは保持しない。

復元時はsnapshotを直接読む。参照先Sessionとの整合性は検証するが、eventを順に適用して現在状態を作り直さない。

## 10. HistoryとHistoryEvent

### 10.1 History

形式は`struct`。終了済みSessionと履歴eventを保持する。

- Session IDはSession間で、event IDはevent間で重複しない。両者は別の識別子として扱う。
- 各eventの参照先Sessionは、snapshotまたは終了済み一覧に存在する。
- 現在のSessionの集計では、snapshotとそのSessionのeventを参照できる。
- 日別集計や統計はSession・eventから求め、保存上の独立した正本を持たない。
- 現在状態を更新するためのevent replay機能は持たない。

### 10.2 HistoryEventとEventKind

HistoryEventは共通情報を持つ`struct`、EventKindは種類別のデータを持つ`enum`とする。

共通情報は、一意なIDまたは通し番号、対象Session ID、発生・適用日時、記録日時、EventKindとする。時計が戻る場合も順序を保つため、eventの並びを日時だけに依存させない。

| EventKind | 記録する意味 |
| --- | --- |
| RunIntervalRecorded | 実行区間の開始・終了日時、その区間で計時したms、時刻の不確かさ |
| InterruptionStarted | 中断の開始 |
| InterruptionEnded | 中断の終了と、その結果・経過時間 |
| QuickStartDecisionMade | 終了／継続の選択と継続先 |
| AppClosing | 対象Sessionの実行・中断・継続選択待ちで、アプリ終了操作を行った事実 |
| AppRestored | 対象Sessionのsnapshotをアプリ起動時に復元した事実 |
| ObservationGapDetected | 最終確認と再観測の間の不明区間 |

開始前の入力、画面操作、分析に不要な予定変更などはeventにしない。開始待ちでは、必要なsnapshot更新だけを行う。

Sessionの開始日時・終了日時・結果はSession本体を正本とし、SessionStarted／SessionEnded eventには複製しない。AppClosingとAppRestoredは独立したenum variantであり、一つの曖昧なvariantにはまとめない。アプリ起動をまたいだ事実はこれらのeventから確認し、Interruptionにcrossed_app_boundaryを重複保持しない。ObservationGapDetectedは、すでに中断中でも観測空白を記録するために残す。

### 10.3 実行区間の確定

Pause、Distraction、中止、Reset、Skip、自然完了、正常終了、Observation Gapで実行区間を閉じる際にRunIntervalRecordedを作る。

通常のtickではsnapshotの累積計時時間と最終確認情報を更新し、tickごとのeventは作らない。チェックポイントでは、開いている区間を含むsnapshotを保存する。

異常終了後は保存済みsnapshotの情報だけで、最後の確認時点までの区間を閉じられるようにする。過去eventの再生は不要とする。

終了済みSessionの`elapsed_ms`は、そのSessionの確定した実行区間の計時時間の合計と一致する。開いている実行区間の計時時間を、区間確定時に二度加算しない。

対象Sessionの確定済み区間の合計をCとすると、Runningでは`run.elapsed_ms_at_start = C`であり、未確定区間の計時時間は`Session.elapsed_ms - C`。Interruptedおよび終了済みSessionでは`Session.elapsed_ms = C`となる。

## 11. 状態遷移

### 11.1 ユーザー操作と自然完了

| 操作・契機 | 遷移 | snapshot・履歴の変更 |
| --- | --- | --- |
| Focus / Quick Start開始 | 作業開始待ち → 実行中 | 開始日時を持つ新Session |
| Break開始 | 休憩開始待ち → 実行中 | 開始日時を持つ新Session |
| Pause | 実行中 → Pause中 | 区間確定、InterruptionStarted |
| Distraction | Focus / Quick Start実行中 → Distraction中 | 区間確定、InterruptionStarted |
| Resume | Pause / AppExit / ObservationGap中 → 実行中 | Resumedで中断終了、新しい実行区間 |
| Return | Distraction中 → 実行中 | Returnedで中断終了、Recovery Time、新しい実行区間 |
| Focus自然完了 | 実行中 → 休憩開始待ち | 最終区間、Completed、Historyへの移動、ラウンド更新 |
| Quick Start自然完了 | 実行中 → 継続選択待ち | 最終区間、Completed、Historyへの移動。ラウンドは維持 |
| Quick Start終了選択 | 継続選択待ち → Focus開始待ち | Finishの選択event。Sessionを再終了しない |
| Quick Start継続選択 | 継続選択待ち → Focus実行中 | Continueの選択event、開始日時を持つ新Focus |
| Break自然完了 | 実行中 → Focus開始待ち | Completed、Historyへの移動。長休憩ならラウンドを0へ |
| 中止 | 実行中／中断中 → Focus開始待ち | 区間または中断終了、Cancelled。長休憩ならラウンドを0へ |
| Reset | 実行中／中断中 → 同じ種別の開始待ち | 区間または中断終了、Reset、Current Taskを下書きへ |
| FocusのSkip | 実行中／中断中 → 休憩開始待ち | 区間または中断終了、Skipped。完了回数は増やさない |
| Quick StartのSkip | 実行中／中断中 → Focus開始待ち | 区間または中断終了、Skipped。休憩は挟まない |
| BreakのSkip | 実行中／中断中 → Focus開始待ち | 区間または中断終了、Skipped。長休憩ならラウンドを0へ |

中断中にSessionを終了すると、InterruptionEndedのSessionEndedという中断結果を、Session本体の終了結果・日時と対応付ける。SessionEndedという名前の履歴eventは設けない。

開始待ちでのResetは無操作とする。開始待ちのSkipは予定の変更だけで、Session履歴を作らない。休憩開始待ちからQuick Startを始める場合は、まず休憩をSkipして作業開始待ちへ戻る。

### 11.2 終了・再起動・Observation Gap

| 状況 | 処理 |
| --- | --- |
| 実行中に正常終了 | 信頼できる経過まで区間を確定し、AppExit中断を作る |
| Pause中に正常終了 | Pauseを維持し、終了の事実を記録 |
| Distraction中に正常終了 | Distractionを維持し、Return済みにしない |
| 継続選択待ちで正常終了 | 選択待ちを維持し、Finishとみなさない |
| 中断snapshotを復元 | 元のKindを維持し、手動再開を待つ |
| Runningのsnapshotを再起動で復元 | 最終保存済み確認まで区間を確定し、ObservationGap中断へ |
| 実行中にObservation Gapを検出 | 空白を加算せず、最後の確認時点で停止する |
| 中断中にObservation Gapを検出 | 元の中断を維持し、空白の事実を記録する |
| 開始待ち・継続選択待ちで空白 | 計時・Session・Interruptionを新たに開始しない |

これらの停止規則はBreakにも適用する。再起動・空白検出だけでは再開・自然完了しない。

### 11.3 禁止する操作とエラー

| 操作 | エラー条件 |
| --- | --- |
| Start | Activeまたは継続選択待ちである |
| Pause | Runningではない |
| Distraction | Runningではない、またはBreakである |
| Resume | Distraction中、Running、Readyなど、再開対象の中断ではない |
| Return | Distraction中ではない |
| 継続／終了選択 | 対象Quick Startの選択待ちではない、または選択済み |
| Current Task編集 | Session開始後、またはBreakへの設定 |
| 設定変更 | Readyではない |
| Session操作 | 対象が現在のSessionではない、または終了済み |
| 作成・更新・読込 | 不正な時間、ID重複、オーバーフロー、参照・状態の不整合 |

拒否された操作自体は状態・履歴を変更しない。ただし、操作前に独立して適用した時間経過による自然完了は保持する。対象がその時点で終了した場合、同じ入力を次のSessionへ流用しない。

## 12. 保存の整合性

| 情報 | 復元・判断の基準 |
| --- | --- |
| 現在の残り時間 | snapshot内Sessionの設定時間と計時済み時間 |
| 現在の中断理由 | snapshot内Interruption |
| 開始待ち・継続選択待ち | snapshotの進行状態 |
| ラウンド進捗 | snapshotのラウンド進捗 |
| 終了済みSessionの結果 | HistoryのSession記録 |
| 過去の中断・復帰・実行区間 | 履歴event |

完了数・Sessionの終了結果はSession本体から集計する。中断終了eventにSessionの終了理由が含まれていても、Session完了として重複集計しない。

### 12.1 一括保存と再試行

「ドメイン遷移」は、入力された時刻・操作からsnapshot、終了済みSession、event群、ID採番情報の変更候補を生成することを指す。「保存確定」は、保存層がその候補の永続化成功を確認することを指す。前者の成功だけでは、ユーザー向けの操作成功を意味しない。

コアはI/Oを行わず、保存世代・ロック・再試行・通知を管理しない。アプリケーション側が変更候補と最後に保存できた状態を区別し、保存層の成功応答後に候補を採用して副作用を実行する。コアの操作APIは、拒否した操作が元の状態や採番値を部分更新しない境界を提供する。

一つの遷移から生成したsnapshot、終了済みSession、event群、ID採番情報に、保存層が保存世代を付けてまとめて保存する。通常tickの計時更新も純粋な遷移だが、保存タイミングはチェックポイント方針に従う。

- 保存再試行では同じ変更内容とIDを使い、遷移を再実行して別のSession・eventを作らない。
- Quick Start継続は、選択待ちのままか、継続先Focusと関連eventがそろった状態のどちらかで保存する。
- 保存失敗を利用者に示し、保存済みとして終了しない。
- snapshotと履歴の不整合をevent replayで黙って補正せず、読込エラーまたは明示的な復旧対象にする。
- 全体schema version、保存世代、対応形式の判定、ファイルの保護は保存層で扱う。

保存待ちはアプリケーション側の状態であり、TimerStateやInterruptionKindに追加しない。保存失敗中に観測を保留した区間は、保存回復後に通常のObservation Gapとしてコアへ渡し、推測して加算しない。

### 12.2 初期状態

新規データの初期状態は、既定Settings、ラウンド進捗0、Current Task未入力のFocus開始待ち、空のHistoryとする。初期化だけではSession・Interruption・eventを作らない。

新規初期化を許可できる保存先かどうかは保存層が判断する。読込失敗を初期状態へ置き換える処理はドメインにも保存層にも設けない。

## 13. 既存型との対応

| 既存の型・処理 | 扱い | 理由 |
| --- | --- | --- |
| PomodoroTimer | 分割する | Sessionの計時と、休憩・Quick Startを含む進行管理を分ける |
| TimerState | 置き換える | Readyを進行状態へ移し、Runningと理由付き中断を表す |
| SessionKind | 責務を維持してQuickStartを追加 | 種別を表す既存の役割を維持できる |
| TimerConfig | 基本的に維持する | 時間設定と検証を活用し、Quick Startは固定時間とする |
| TimerEvent | 履歴eventへ置き換える | 完了・Skipに加え、区間・中断・復帰・継続を記録する |
| History | 責務を変更する | 日別集計の更新中心から、終了済みSessionとeventの保持へ |
| DailySummary | 保存モデルから外す | 日別集計が必要ならSession・eventから求める表示用の値とする |
| PersistedState | 責務を変更する | snapshot・History・全体versionなどの保存単位へ拡張する |
| TimerStatus | 表示向けの派生値にする | 進行状態と中断理由から求め、独立した保存状態にしない |
| TUIのApp | 接続処理を更新する | 入力・描画・保存・通知を担当し、コアの規則を重複実装しない |

既存の設定検証、境界時刻の検証、時刻を引数で渡すテスト方式は活用する。新たな汎用タイマーエンジンを先に作らず、具体的なSession、TimerState、PomodoroStateに必要な処理を置く。

## 14. Persistence schemaとの境界

保存形式全体の構造、JSONのタグ・フィールド配置、schema versionの番号、I/Oの詳細は [Persistence Schema](PERSISTENCE_SCHEMA.md) に定義する。本書は状態・所有関係・遷移の意味を定義し、保存文書ではそれを再定義しない。

Observation Gapの具体的な閾値・チェックポイント間隔は内部ポリシーで扱い、保存形式のユーザー設定項目にはしない。保存する概念は、計時済み時間、最終確認時点、空白の検出時点、不明区間の事実とする。

旧形式専用のSession・集計・互換フィールドは新しいモデルに設けない。
