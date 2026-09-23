# Phase 3 Plan

作成日：2026-09-23

本書はPhase 3「TUIでの着手・集中・復帰」を、レビュー可能な変更単位へ分けた実行計画である。Phaseの進捗は[Implementation Plan](IMPLEMENTATION_PLAN.md)だけで管理する。機能の正本は[Product Spec](PRODUCT_SPEC.md)、状態と遷移の正本は[Domain Model](DOMAIN_MODEL.md)、保存契約の正本は[Persistence Schema](PERSISTENCE_SCHEMA.md)とし、本書で別の仕様を定義しない。

## 1. 到達点と境界

Phase 2で通常起動、V1保存、復元、保存失敗の制御と既存キー操作を接続した。Phase 3では、利用者がTUIからCurrent Taskを入力し、Quick Startを始め、Distractionを申告してReturnできるようにする。中止と手動Breakを含む既存の操作、設定、通知、起動・終了の案内も一連の利用経路として揃える。

Current Taskの一覧管理、入力済みSessionの作業名変更、日別集計、詳細履歴、Recovery Timeの可視化は追加しない。振り返りUIの完成とMVP全体の受入検証はPhase 4に残す。V1保存形式を変えず、旧形式の移行も行わない。

TUIはキー入力、編集途中の文字列、表示とヘルプを担当する。`pomodoro-core`の`Command`がSessionの遷移と履歴を決め、既存controllerが保存成功と操作成功の境界を保つ。通常の意味ある操作をTUIから直接`DomainState::apply`して確定させない。

## 2. 既存経路と操作の割当

| 操作 | 現在の経路 | Phase 3で接続する経路 |
| --- | --- | --- |
| Current Task | 保存済みdraftとSession内の表示、`SetCurrentTask`のcontrollerテスト | Focus・Quick Startの開始待ちでの一行入力、確定・取消・再編集 |
| Quick Start | 種別、2分の計時、完了後の`f`／`c`、復元と保存再試行 | Focus開始待ちからの開始と案内 |
| Distraction / Return | 復元したDistractionからSpaceでReturn | 実行中のFocus・Quick Startからの申告と、その後のReturn |
| Cancelled | coreの終了結果 | 実行中・中断中からの明示的な中止 |
| Break・既存操作 | 手動開始、Pause／Resume、Reset／Skip、設定、通知 | 新操作と競合しない表示・ヘルプと一連の操作確認 |

新しい通常時キーの候補は`t`（作業名編集）、`2`（2分のQuick Start）、`d`（Distraction）、`x`（中止）とする。既存のSpace、`r`、`n`、`s`、`f`、`c`、`q`、`?`、保存失敗時の`r`／`Q`は維持する。各キーは利用できる状態だけで案内し、実装時にREADME・画面ヘルプ・テストを同時に更新する。

## 3. レビュー単位と依存関係

番号順に進める。各単位で該当する入力、保存後の表示、失敗時の扱い、テストを揃える。最後の受入確認までテストだけを先送りしない。

### 3-0. Phase 3の実行計画

- **対象**：本書、Implementation Plan、READMEの参照関係。
- **変更**：Phase 2までの利用可能な経路とPhase 3・4の境界を確認し、作業単位と検証を定める。
- **確認・完了条件**：仕様文書との整合、相対リンク、差分の書式を確認する。実装・テストが済んでいない機能を完了扱いしない。Phase 2の完了に依存する。

### 3-1. Current Taskの入力

- **対象**：TUIのApp、入力draft、描画、ヘルプ、README。
- **変更**：Focus開始待ちで作業名を開き、保存済みdraftを再編集できるようにする。文字入力・削除、Enterでの確定、Escでの取消を扱う。空白だけなら未設定とし、改行などはcoreの`CurrentTask::parse`で拒否する。開始後は編集させず、Breakには入力させない。
- **保存境界**：確定時だけ`SetCurrentTask`をcontrollerへ渡す。保存失敗時は候補を一度だけ作って通常操作を保留し、既存の再試行・未保存終了へ移す。再試行で入力を二重適用しない。
- **テスト・完了条件**：日本語・空白・削除・取消・再編集、保存失敗と再試行、Focus開始後と再起動後の作業名、長い入力と狭い端末の表示を確認する。未入力でも従来どおり開始できる。3-0に依存する。

### 3-2. Quick Startの通常開始

- **対象**：TUIの開始キー、状態別案内、Appと実行ファイルのテスト。
- **変更**：Focus開始待ちから`Start(QuickStart)`を保存確定して開始する。Current Taskのdraftを引き継ぎ、自然完了後は既存の`f`／`c`で終了・継続できるよう案内する。Break開始待ちからは直接始めず、まず既存のSkipでFocus開始待ちへ戻す。
- **保存境界**：開始・終了選択・継続先Focusの生成は既存controllerで保存後に確定する。選択待ちの間は計時せず、自動選択しない。
- **テスト・完了条件**：通常FocusとQuick Startの選び分け、固定2分、作業名継承、終了／継続、保存失敗時の再試行、終了・再起動時の選択待ちを確認する。実時計で2分待つテストにはしない。3-1に依存する。

### 3-3. Distraction申告と中止

- **対象**：TUIの状態別キー、操作案内、Appと実行ファイルのテスト。
- **変更**：実行中のFocus・Quick StartでDistractionを申告し、SpaceでReturnする。中断中に別の中断を重ねない。中止は`End { outcome: Cancelled }`へ接続し、Reset・Skipとの違いを表示する。BreakではDistractionを受け付けない。
- **保存境界**：申告・Return・中止は保存後に成功を表示する。保存失敗中は通常操作と計時を保留し、再試行で同じ中断・終了を重複させない。
- **テスト・完了条件**：申告からReturn、中断中の終了と再起動後のReturn、未復帰の中止、Breakでの拒否、保存失敗・再試行、Recovery Timeに終了中の時間を含めても作業時間へ加算しないことを確認する。3-0に依存し、3-2と独立に進められる。

### 3-4. 操作フローの受入と案内

- **対象**：TUIの状態別表示・ヘルプ、README、実ファイルと端末の統合テスト。
- **変更**：Current Task、Quick Start、Focus、Break、Pause、Distraction、Return、中止、Reset／Skip、設定、通知を一つの利用経路として確認し、キー競合や画面外に隠れる重要な案内を直す。起動失敗、保存待ち、未保存終了の案内も新操作と整合させる。
- **テスト・完了条件**：作業名付きQuick StartからFocusへの継続、Distraction後の終了・再起動・Return、手動Break、保存失敗・再試行を実ファイルと端末で確認する。履歴・snapshot・通知の結果は保存成功後の状態と一致する。3-1～3-3に依存する。

## 4. 検証とPhase 3終了条件

Rustを変更する各レビュー単位で関連テストに加え、次を通す。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

時刻と保存失敗は注入して決定的に検証し、実端末・実ファイルのテストは入力と保存後の結果を確認する。長いsleepに依存しない。Phase 3の終了時には最低Rustバージョンでも確認する。

- Focus・Quick Startの開始前にCurrent Taskを任意入力でき、Sessionと復元後の表示に保持される。
- Quick Startの開始、自然完了後の終了／継続、Pause／ResumeをTUIから操作できる。
- Distractionの申告とReturn、中止、手動BreakをTUIから操作でき、記録と計時が仕様どおりである。
- 意味ある操作の成功表示と通知は保存成功後に限られ、保存失敗中の保留・再試行・未保存終了が新操作でも機能する。
- README、状態別ヘルプ、実ファイル・端末の確認が実際の操作と一致する。Phase 4の振り返りUIとMVP全体の受入を完了扱いしない。
