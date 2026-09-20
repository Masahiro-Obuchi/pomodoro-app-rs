# Persistence Schema

更新日：2026-09-20

本書は [Product Spec](PRODUCT_SPEC.md) と [Domain Model](DOMAIN_MODEL.md) を単一JSONの保存形式へ具体化する。今後の実装の設計基準であり、実装済みであることを意味しない。開発順序と進捗は [Implementation Plan](IMPLEMENTATION_PLAN.md) に置く。

## 1. 範囲と正本

MVPは新規データで開始し、対応する保存形式だけを読み込む。旧形式の設定・集計・タイマー状態の引継ぎ、専用DTO、互換フィールドは設けない。

現在状態はsnapshotから直接復元する。eventは分析・整合性検証に使い、event replayによる復元や自動修正は行わない。

| 情報 | 正本 |
| --- | --- |
| 現在のSession・中断・次の予定・継続待ち | snapshot |
| Settings・ラウンド進捗 | snapshot |
| 終了済みSessionの時間・結果・Current Task | history.sessions |
| 確定済み実行区間・過去の中断・継続判断 | history.events |

Session本体は実行・中断中にはsnapshotに置き、終了時に履歴へ移す。日別集計、累計、平均、割合は保存せず、Session・eventから求める。MVPでは履歴の自動削除・圧縮を行わない。

## 2. 保存単位と共通表現

以下は保存契約を示す擬似定義であり、Rustの公開APIを固定するものではない。

```rust
struct PersistedStateV1 {
    schema_version: u32, // 必ず1
    save_generation: u64,
    saved_at: UtcTimestamp,
    id_allocators: IdAllocators,
    snapshot: PomodoroSnapshot,
    history: HistoryV1,
}

struct IdAllocators {
    next_session_id: u64,
    next_interruption_id: u64,
    next_event_sequence: u64,
}

struct HistoryV1 {
    sessions: Vec<Session>, // 終了済みのみ
    events: Vec<HistoryEvent>,
}
```

- schema_versionはファイル全体に一つだけ置く。snapshotやhistoryに独立したversionを持たせない。
- V1の読込にはトップレベルのschema_version: 1が必須。versionなし・null・未対応のversionは読込エラーとする。
- save_generationは保存単位の世代。最初の保存を1とし、次の保存成功で1増やす。event数や完了数とは独立する。
- saved_atは保存候補を作成したUTC日時で、復旧候補の日時表示に使う。同じ候補の再試行では変更しない。作業実績や世代の順序を求める根拠にしない。
- 日時はUTCのRFC 3339文字列、精度はms。時間量は整数のmsとする。Settingsは既存TimerConfigの秒単位と検証範囲を維持する。
- Session ID・Interruption IDは型で区別し、それぞれ保存単位全体で1から採番する。
- eventは1からの通し番号sequenceをIDとして兼用する。別のUUIDは設けない。
- ID・event番号・保存世代の一意性と単調増加は、現在採用中の保存系列内で保証する。その系列内ではID・event番号を再利用せず、採番情報を状態・履歴と一括保存する。数値のオーバーフローはエラーにする。
- バックアップ復旧で失われた枝まで含む永久一意性や枝同士の統合は保証しない。UUID、系列ID、別ファイルの採番台帳は設けず、復旧元の採番情報をそのまま採用する。
- フィールド名とenum値はsnake_case。必須キーは省略せず、任意値はnullにする。
- 欠落、未知フィールド、未知variant、重複キー、範囲外の値を拒否する。読めない情報を捨てて再保存しない。

### 2.1 codecの境界

保存表現とcoreの型は別の境界とする。V1の保存DTOから値を検証してドメインへ渡し、coreへファイルI/Oや保存専用の属性を持ち込まない。

- 日時の出力・受理形式は`YYYY-MM-DDTHH:MM:SS.sssZ`とし、UTC・小数3桁に固定する。受理範囲は`1970-01-01T00:00:00.000Z`から`9999-12-31T23:59:59.999Z`まで。coreのUnix epochからの整数msと相互変換する。
- 存在しない日付、うるう秒表現、範囲外、異なるオフセット表現、異なる小数桁数は拒否する。ms未満の丸めや範囲への切詰めは行わない。書込時にも同じ制約を検証する。
- 任意値でもキーは必須とする。明示的な`null`だけが値なしを意味し、キー欠落を`None`へ読み替えない。
- Current Taskの保存文字列は、前後の空白除去などの入力正規化を済ませた値に限る。空文字・空白だけ・前後に空白が残る文字列・行区切りを拒否し、読込時にはtrimや未入力化をしない。未入力は`null`とする。入力時の規則は[Domain Model §7.1](DOMAIN_MODEL.md)を参照する。
- Settingsを含むすべてのオブジェクトで未知フィールドと重複キーを拒否する。既存型のSerde変換を使う場合も、この契約を弱めない。
- version判別で重複キーを上書きする中間表現へ先に変換しない。判別後の全体decodeでも、元JSONの情報を失わず検証する。

日時の文字列表現を限定するのは新規V1のcodecを小さく保つためであり、旧形式の日時や外部ツールの出力を互換変換する機能は設けない。

### 2.2 初回保存例

初回保存のJSON例を示す。§6の新規判定で、保存先にstate.lock以外のエントリがないことを確認してから、この状態を作る。

```json
{
  "schema_version": 1,
  "save_generation": 1,
  "saved_at": "2026-09-12T00:00:00.000Z",
  "id_allocators": {
    "next_session_id": 1,
    "next_interruption_id": 1,
    "next_event_sequence": 1
  },
  "snapshot": {
    "settings": {
      "focus_seconds": 1500,
      "short_break_seconds": 300,
      "long_break_seconds": 900,
      "focuses_before_long_break": 4
    },
    "round_progress": {
      "completed_focuses_in_round": 0
    },
    "state": {
      "status": "ready",
      "next_kind": "focus",
      "current_task_draft": null
    }
  },
  "history": {
    "sessions": [],
    "events": []
  }
}
```

## 3. Snapshotの保存表現

PomodoroSnapshotはDomain ModelのPomodoroStateの保存表現であり、別のドメインモデルではない。settings、round_progress、stateを持つ。Settingsのフィールドは上記JSON、ラウンド進捗はcompleted_focuses_in_roundとする。

stateはstatusをタグにする。

| status | フィールド |
| --- | --- |
| ready | next_kind、current_task_draft |
| active | session、timer |
| awaiting_quick_start_decision | quick_start_session_id、current_task |

Sessionのフィールド名はDomain Modelに合わせ、id、kind、current_task、planned_duration_ms、started_at、elapsed_ms、continued_from_quick_start、endとする。endはnullまたはended_atとoutcomeを持つオブジェクト。

SessionKindの値はfocus、quick_start、short_break、long_break。SessionOutcomeの値はcompleted、cancelled、reset、skipped。Current Taskは文字列またはnullとし、独立したIDを持たせない。

timerもstatusをタグにする。

| status | フィールド |
| --- | --- |
| running | run |
| interrupted | interruption |

runは次の境界情報を持つ。

| フィールド | 意味 |
| --- | --- |
| started_at | 現在の実行区間の開始UTC日時 |
| elapsed_ms_at_start | 区間開始時点のSession累積計時時間 |
| last_confirmed_at | 最後に計時を反映したUTC日時 |
| time_uncertainty | 時計異常などの情報。なければnull |

Sessionのelapsed_msが計時済み時間の正本であり、区間の未確定時間はelapsed_ms - elapsed_ms_at_startから求める。再起動時にも、この境界情報だけで保存済みの範囲を確定できる。

interruptionはid、kind、started_at、recorded_at、time_uncertainty、endを持つ。開いている中断なのでendは必ずnull。

- kindはpause、distraction、app_exit、observation_gap。
- started_atは計時を止めた境界、recorded_atは認識した日時。Observation Gapでは異なり得る。
- アプリ終了・復元はapp_closing、app_restoredとして記録する。crossed_app_boundaryは保存しない。中断のKindは変更しない。
- time_uncertaintyはnull、clock_moved_backward、clock_discontinuity、insufficient_clock_evidenceのいずれか。単なるアプリ終了や長い中断を時計異常とはみなさない。

### 復元時の処理

まず保存内容をdecode・検証し、その後「今回起動した」という事実をDomain Modelの遷移として適用する。

| 保存状態 | 処理 |
| --- | --- |
| Ready | 種別・下書き・進捗を維持 |
| Active / Running | 保存済みの最終確認まで区間を確定し、ObservationGap中断へ |
| Active / Interrupted | 同じ中断を維持し、起動をまたいだ事実を記録 |
| AwaitingQuickStartDecision | 選択待ちを維持し、自動選択しない |

Runningの復元では、空白の検出、中断開始、AppRestoredなどの対応eventとsnapshotを一括保存してから通常操作を許可する。中断中では中断を追加せず、AppRestoredを記録する。継続待ちのAppRestoredは完了済みQuick Startを対象にする。Readyには対象Sessionがないので、起動だけでSessionのeventを作らない。

この停止規則はBreakにも適用する。復元や保存済み期限の超過だけでは計時・再開・自然完了しない。

### 永続化しない情報

Instantなどの単調時計アンカー、表示用deadline・残り秒数、tickや保存の実行予定、ロックハンドル、保存再試行の制御状態、UIのカーソルや画面状態は保存しない。Observation Gapの閾値とチェックポイント間隔もユーザー設定として保存せず、内部policyで扱う。

## 4. 履歴eventの保存表現

```rust
struct HistoryEvent {
    sequence: u64,
    session_id: SessionId,
    effective_at: UtcTimestamp,
    recorded_at: UtcTimestamp,
    payload: EventKind,
}
```

effective_atは発生・適用日時、recorded_atは認識・記録日時。時計が戻っても順序を保つため、配列はsequence順に保存する。

payloadはtypeをタグにする。

| type | 固有フィールド |
| --- | --- |
| run_interval_recorded | started_at、ended_at、credited_ms、time_uncertainty |
| interruption_started | interruption_id、interruption_kind |
| interruption_ended | interruption_id、end |
| quick_start_decision_made | decision |
| app_closing | なし |
| app_restored | なし |
| observation_gap_detected | last_confirmed_at、detected_at、reason |

observation_gap_detectedのreasonはrestart、observation_discontinuity、clock_anomaly。検出の内部閾値は記録しない。

### 中断履歴

interruption_startedの共通日時が、中断の開始境界と記録日時を表す。interruption_endedのendはended_at、outcome、durationを持ち、ended_atはeventのeffective_atと一致させる。

- outcomeは`{ "type": "resumed" }`、`{ "type": "returned" }`、または`{ "type": "session_ended", "session_outcome": "cancelled" }`の形。session_outcomeにはcancelled・reset・skippedだけを許可する。
- durationは`{ "status": "known", "elapsed_ms": 60000 }`、または`{ "status": "unknown", "reason": "clock_discontinuity" }`の形。reasonはtime_uncertaintyと同じ列挙値から選ぶ。
- 開いた中断の正本はsnapshot。開始eventのID・Kind・開始境界・記録日時との一致を検証する。
- 閉じた中断は開始・終了eventの組で表す。別の中断一覧やSession内の中断履歴配列は作らない。
- Recovery Timeの完了サンプルはDistractionかつReturnedだけ。アプリ終了中も含む日時差を扱い、時計異常などがあれば結果はReturnedのまま時間をUnknownにする。

履歴表示で開始・終了eventを対応付けることは、現在状態のevent replayではない。

### 実行区間と二重集計の防止

区間確定の契機はDomain Modelに従う。tickはsnapshotだけを更新し、tickごとのeventを生成しない。

区間合計とsnapshotの累積時間・開区間境界が満たす式は、意味の正本である[Domain Model §10.3](DOMAIN_MODEL.md)に従い、読込直後・保存直前に検証する。

区間確定では加算済みの時間をeventへ記録するだけで、Sessionに再加算しない。credited_msは0以上とし、不明な時間の補完には使わない。

Sessionの開始・終了・結果・計時時間はSession本体を参照し、session_started／session_ended eventは保存しない。閉じた中断の時刻異常はend.durationのUnknown理由に集約し、同じ理由のtime_uncertaintyフィールドをeventに重複保存しない。

### Quick Startの選択

decisionは`{ "type": "finish" }`または`{ "type": "continue", "focus_session_id": 2 }`の形。eventのsession_idは継続元Quick Startを指す。

Continueの選択event、新しいFocus、Current Taskと継続元の関連、Activeへの切替、採番情報を一括保存する。Focusのcontinued_from_quick_startと選択eventの参照が一致することを検証する。Quick Start本体に別の継続先フィールドは持たせない。

## 5. 保存単位の検証

decode直後と保存直前に、Domain Modelのinvariantに加えて次を検証する。不整合の自動修正はしない。

- snapshotのActiveと終了済み一覧に同一Sessionを重複所有しない。
- Activeに終了情報や残り0のSessionを置かず、終了済みSessionには終了情報を必須とする。
- Activeの設定時間は現在のSettingsの該当種別の時間と一致させる。Quick Startは固定120秒とする。
- IDは各種類の中で一意。next値は既存最大値より大きい。event番号は1から連続し、next_event_sequenceはその次とする。
- 全eventの対象Session、継続元・継続先の参照が存在する。
- 終了eventのない中断は全体で最大一つで、snapshotの開いた中断と一致する。閉じた中断の開始・終了は同じSessionを参照する。
- 中断結果とKindが対応し、Session終了によって閉じた中断はSession本体の終了結果と一致する。
- 実行区間とSession累積時間はDomain Model §10.3の式に一致する。event順序上、中断をまたぐ区間や二重確定を許可しない。
- ライフサイクル・観測空白・継続判断eventが、そのevent位置で許可される状態に属することをDomain Model §10.2に従って検証する。
- 中断時間のKnown/Unknownが、当該中断について保存された時計異常の証拠と矛盾しないことをDomain Model §6.2に従って検証する。
- CompletedのQuick Startは継続待ちの対象であるか、一つの選択eventを持つ。それ以外のQuick Startに選択eventを持たせない。
- Continueの参照・Current Taskが一致し、一つのQuick Startに継続先Focusは最大一つ。
- Quick Startの設定時間、BreakのTask禁止などの種別制約を満たす。
- ラウンド進捗は0以上N以下。履歴から再計算せず、長休憩なら必ずNという条件も置かない。
- 時間不明を0へ変換しない。UTC日時の全体的な昇順は要求せず、区間の日時は記録された不確かさも含めて検証する。

現在のSettingsを使って過去Sessionの設定時間を検証し直さない。日別集計を表示する際のタイムゾーンや、時計異常を含む区間の日付配分は表示・集計側の方針とし、不明な時間帯への配分を推測しない。

## 6. 読込と書込許可

読込と初期化可否の判定を、通常の書込許可より先に行う。排他ロックと有効な保存状態、または新規保存先の確認が揃った場合だけ、書込可能な保存ハンドルを渡す。

| 保存先の状態 | 動作 |
| --- | --- |
| 保存先にstate.lock以外のエントリなし | 新規初期化を許可 |
| 有効なV1本体あり | snapshotを検証して復元 |
| 本体なし、バックアップや一時ファイル等の残存物あり | 新規とみなさず、有効な.bakだけを復旧候補として案内。候補がなければ停止 |
| JSON不正・schema不正・参照不整合 | 起動・通常保存を止め、元ファイルを維持 |
| versionなし・未対応version | 対応形式でないことを示して停止。別形式として読み直さない |
| 権限エラー・読込I/O失敗 | ファイルなしと区別して停止 |

本体・バックアップがない場合の新規判定では、未認識の名前のファイルやディレクトリも残存物として扱う。名前の確認だけを行い、それらの内容を解析して復旧元に選ぶことはしない。有効な本体がある場合は、本体を優先する。

読込エラーから既定状態を返す分岐を作らない。開発用の既存ファイルが残っている場合も、アプリは自動削除・初期化しない。新規データで開始する際は、アプリ停止中に開発者が保存先を明示的に整理する。

## 7. Atomic saveと保存再試行

snapshot、終了済みSession、event、採番情報、保存世代を一つの未確定の保存候補として固定する。本節は通常保存の手順であり、破損本体を置換する復旧保存にはそのまま適用しない。初回以外は、最後の確定世代をgとして以下を行う。

1. ロックを保持し、本体が最後に読み込んだ／保存した内容と一致することを確認する。世代だけでなく内容も比較する。
2. 保存候補を一度だけ作り、世代g + 1とsaved_atを設定して全体を検証する。
3. 検証済み本体をバックアップ用の固有一時ファイルにコピーし、ファイルをfsyncする。state.json.bakへrenameし、親ディレクトリをfsyncする。初回保存では省略する。
4. 本体と同じディレクトリに、固有名の一時ファイルを排他的新規作成する。候補のJSON全体を書き込み、ファイルをfsyncする。
5. 一時ファイルをstate.jsonへrenameする。
6. 親ディレクトリをfsyncする。
7. メモリ上の確定世代を更新し、保存成功として通知する。

意味のある操作・自然完了は7の後にアプリケーションが成功扱いにし、UI確定や通知を行う。通常tickのチェックポイントでは毎tickの確定通知は不要だが、書込時の成功条件・失敗時の保留は同じとする。

保存ディレクトリを新規作成した場合は、その作成を永続化する親側の同期も行う。

renameによる置換と、クラッシュ後にも残ることは区別する。ファイル同期だけではディレクトリエントリの永続化まで保証しないため、rename後の親ディレクトリ同期が必要となる。[rename(2)](https://man7.org/linux/man-pages/man2/rename.2.html)、[fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html)

### 失敗位置と再試行

| 失敗位置 | 扱い |
| --- | --- |
| 本体rename前 | 旧本体を維持し、同じ候補を再試行 |
| 本体rename後、ディレクトリ同期未完了 | 保存の確定状態が不明。本体の再確認を先に行う |
| ディレクトリ同期成功後 | 保存成功 |

確定状態が不明な場合、本体が候補と一致すれば必要な同期を再実行し、同じ世代を確定する。旧本体と一致すれば同じ候補を書き直す。どちらとも異なれば競合・破損として停止する。この判定前にバックアップを更新しない。

再試行ではドメイン遷移を実行し直さず、候補の内容・ID・世代・saved_atを変えない。

保存失敗中は候補をメモリに保ち、通常操作と計時反映を保留して未保存状態を表示する。保存回復後、Runningだった候補は観測を中断した境界からObservation Gapとして処理し、その遷移も保存してから操作を再開する。中断中・継続待ちでは元の状態を維持する。保存待ち専用のドメイン中断Kindは追加しない。

終了時に保存できなければ、再試行または未保存での終了を明示的に選べるようにする。保存成功として正常終了しない。

## 8. バックアップと復旧

| ファイル | 役割 |
| --- | --- |
| state.json | 現在の保存単位 |
| state.json.bak | 直前の検証済み保存単位 |
| 固有名の一時ファイル | 書込途中。通常ロードでは採用しない |
| 固有名の退避ファイル | 明示的な復旧操作前の元ファイル |
| state.lock | 排他制御専用 |

本体が有効なら本体を使う。世代の大きい一時ファイルがあっても自動採用しない。本体が破損していてバックアップが有効なら、復旧候補として案内し、明示操作で復旧する。両方破損していればすべて維持して停止する。

MVPの復旧元は直前のstate.json.bakに限定する。一時ファイルや退避ファイルの探索・自動選別・枝の統合は実装しない。

復旧の確認は通常のTUI操作を開始する前に行う。復旧候補の日時と、以降の情報を失う可能性を示し、明示確認を受けた場合だけ復旧保存へ進む。拒否・キャンセル時は保存データを変更せず終了する（読込前のロック取得は行う）。通常画面に復旧専用の管理UIは設けない。

復旧保存は通常保存と明示的に分岐する。

1. ロックを保持し、バックアップ全体を検証して、復旧元以降の情報を失う可能性・復旧元日時を表示し、明示的な復旧操作を受ける。
2. 置換対象の本体がある場合、バイト列を固有名の退避ファイルへコピーしてファイルと親ディレクトリを同期する。本体が読めない・退避できない場合は置換しない。退避した内容から本体が変化していないことも確認する。
3. 復旧元snapshotへ復元時のドメイン遷移を適用し、候補を作る。世代は復旧元の世代 + 1、採番は復旧元のnext値から続ける。破損ファイルの値や失われた採番値を推測しない。
4. 通常保存の「本体を検証済み基準と比較」「本体を.bakへ更新」は行わない。有効な.bakを維持したまま、7節の一時ファイル書込・fsync・本体rename・親ディレクトリfsyncを行う。
5. 成功後に候補を採用中の系列・保存基準とし、その後の保存から通常保存へ戻る。失敗・確定不明では、退避済み本体／同じ候補との比較で7節の再試行規則を適用し、復元遷移を再実行しない。

保存世代・IDが失われた枝の値と重複することは許容する。保証は復旧元から採用した系列内に限り、外部の永久識別子として使わない。

未対応の新しいversionを、古いバックアップへの自動復旧の理由にしない。保存途中のクラッシュでも、検証可能な本体を読み、必要なら明示的に復旧する。

## 9. Single-process protection

Linuxのローカル保存先を対象に、専用state.lockへ非ブロッキング排他ロックを取得する。ロック取得は本体の読込・初期化判定・復旧より前とし、終了時の最終保存までハンドルを保持する。二つ目の起動は使用中の保存先を示して終了する。

renameするstate.jsonをロック対象にせず、state.lock自体も終了時に削除しない。PIDファイルの存在や期限ではなく、カーネルのロック状態で判定する。子プロセスにロックハンドルを引き継がせず、異常終了後も再取得できるようにする。[flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html)

実装はワークスペースの最低Rustバージョンで使えるsafeなAPIを保存層に閉じ込める。世代・内容の確認は補助であり、排他ロックの代わりにはしない。同じ規約を使わない旧アプリや外部ツールとの同時書込はサポートしない。

## 10. 検証方針

本節は必要な性質を示す。テスト名、ケースごとの実装状況、実行結果の台帳は置かず、具体的な証拠はテストとPRに残す。

| 対象 | 検証する性質 | 種別 |
| --- | --- | --- |
| 保存表現 | 全snapshot状態、終了結果、任意値、Known/Unknownがround-tripする | Unit |
| schema検証 | 欠落、未知値、重複キー、オーバーフロー、未対応versionを拒否する | Unit |
| 全体検証 | 所有・参照・採番・中断対応・継続関係・区間合計の不整合を拒否する | Unit |
| 復元 | 全Session種別で不明区間を加算せず、手動再開を待つ | Unit |
| Recovery Time | 終了・再起動をまたぐReturnと、時計異常による時間不明を区別する | Unit |
| 初回起動 | 本体も復旧候補もない場合だけ初期化する | Integration |
| 読込失敗 | 起動・終了を試しても元ファイルを初期状態で上書きしない | Integration |
| 保存の一括性 | Quick Start継続などでsnapshot・履歴・IDが部分保存されない | Unit / Integration |
| 再試行 | 候補・世代・IDを維持し、eventを重複生成しない | Unit / Integration |
| I/O失敗 | 書込・ファイル同期・各rename・ディレクトリ同期の失敗位置を区別する | Integration |
| クラッシュ | 保存の各境界で停止しても、旧／新本体の検証か明示復旧へ進める | Integration |
| 復旧 | 退避・バックアップ保護を行い、両方破損や未対応versionで自動初期化しない | Integration |
| 排他 | 二重起動を拒否し、異常終了後に再取得できる | Integration |
| 外部変更 | 世代違い・同世代の内容変更を検出して保存を止める | Integration |
| 終了時失敗 | 保存済みと扱って正常終了しない | Integration |

時計はテストから与え、実時間のsleepへ依存しない。ファイル操作の失敗注入と子プロセス停止を組み合わせる。プロセス停止だけで電源断時の永続性まで検証できたとは扱わない。
