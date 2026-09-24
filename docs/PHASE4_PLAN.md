# Phase 4 Plan

作成日：2026-09-24

本書はPhase 4「履歴の確認とMVPの受入検証」の実行計画である。Phaseの進捗は[Implementation Plan](IMPLEMENTATION_PLAN.md)で管理する。振る舞いの正本は[Product Spec](PRODUCT_SPEC.md)、状態と集計の正本は[Domain Model](DOMAIN_MODEL.md)、保存契約の正本は[Persistence Schema](PERSISTENCE_SCHEMA.md)とする。

## 1. 到達点と既存経路

Phase 3までに、Current Task、Quick Start、Distraction／Return、中止、手動Breakと、保存・復元・再試行をTUIから操作できるようになった。coreの`ReflectionSummary`は記録上の作業時間、Focus自然完了数、Distraction申告数、Return数を終了済みSession・eventと現在のSessionから計算する。TUIのHistory欄にも累計値があるが、狭い端末では操作欄を優先して非表示になり、作業時間は分単位に切り捨てている。

Phase 4では、Product Specの4指標を利用者がいつでも確認できる表示を整え、表示値が保存済みSession・eventと一致することを、操作・保存・再起動をまたぐテストで確認する。保存失敗中は未確定候補の値を確定済みの集計として見せない。日別集計、詳細履歴一覧、Recovery Timeの可視化、統計の拡充はMVPの必須範囲に含めない。

## 2. レビュー単位

### 4-0. 実行計画と現状の確認

- **対象**：本書、Implementation Plan、文書索引、READMEの参照関係。
- **変更**：Phase 3のマージと完了条件を確認し、Phase 4の表示・検証の単位を定める。
- **完了条件**：Product Specの4指標と確認例、既存の集計・TUI・保存経路との対応を明確にし、Phase 3の過去計画を変更しない。

### 4-1. 最小振り返り表示

- **対象**：TUIの入力・表示・ヘルプ、Appの描画テスト、README。
- **変更**：通常操作から4指標を開いて確認できる表示を追加する。現在のSessionの記録済み時間と申告を含め、作業時間は短いSessionも判別できる精度で表示する。狭い端末でも表示への入口と閉じ方を確認できるようにする。
- **境界**：表示の開閉はドメイン操作や保存を起こさない。保存待ちと未保存終了の入力を優先し、未確定候補を確定済み履歴と混ぜない。集計規則はcoreに置いたまま利用する。
- **完了条件**：空の履歴、進行中、Pause・Distraction、終了済み、再起動後、保存失敗中の表示を、coreの集計値と照合する。表示幅と高さを変え、操作・復旧の案内が隠れないことを確認する。

### 4-2. 保存記録とMVP確認例の受入

- **対象**：注入時計を使うApp／controllerテスト、実ファイル・PTYの統合テスト、必要な不整合修正。
- **変更**：Product Spec第10節の確認例を操作から保存・再起動・集計・表示までつなげて検証する。Quick Start完了とFocus完了の合計、途中中止、再起動をまたぐDistraction／Return、Observation Gap、選択待ちの復元、新規起動と不正ファイル停止を扱う。
- **境界**：2分・25分の実時間待ちは使わず、注入時計で計時を確定させる。PTYはキー経路と画面表示、実ファイルは保存結果と再読込を確認する。既存テストで十分に証明済みの境界は重複テストを増やさず、抜けた接続だけ補う。
- **完了条件**：表示値が保存されたSession・eventから再計算した値と一致し、再試行で重複しない。全確認例の検証先をPRに記録する。

### 4-3. 利用案内と終了判定

- **対象**：README、文書索引、Implementation Plan、検証結果。
- **変更**：振り返りの開き方、各指標の意味、保存失敗・復旧・再起動の通常手順を実装に合わせる。Phase 4の完了条件を確認してから進捗を完了へ更新し、本書を過去資料へ移す。
- **完了条件**：Product Specの最小振り返りと確認例を満たし、READMEと画面の案内が一致する。全workspaceの整形、Clippy、テスト、Rust 1.86でのビルドとテストを通す。

## 3. 検証と終了条件

各Rust変更で`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`を通す。保存境界の検証には実ファイルを使い、実行ファイルの表示と操作にはPTYを使う。時間の経過と失敗は注入して決定的に確認する。

Phase 4を完了とする条件は、4指標をTUIから確認できること、保存記録と表示の一致、Product Spec第10節の確認例の受入、利用案内の整合である。完了前にPhase 3までの既存操作・保存契約を変更する必要が出た場合は、その理由と回帰テストを該当PRに含める。
