# 文書一覧

このディレクトリ直下には、現在の仕様と実装全体の計画を置く。完了したPhaseの実行計画は [`archive/`](archive/) に置き、当時の判断を確認するための記録として残す。仕様や現在の進捗は、以下の現行文書を参照する。

## 現行文書

| 文書 | 役割 |
| --- | --- |
| [Product Spec](PRODUCT_SPEC.md) | 製品の目的、MVPの範囲、期待する動作 |
| [Domain Model](DOMAIN_MODEL.md) | 状態、遷移、不変条件 |
| [Persistence Schema](PERSISTENCE_SCHEMA.md) | 保存形式、検証、保存と復旧の契約 |
| [Implementation Plan](IMPLEMENTATION_PLAN.md) | Phaseの順序、完了条件、現在の進捗 |
| [Windows / macOS 対応計画](WINDOWS_MACOS_PLAN.md) | 完了済みPhaseから独立したTUIの対応OS拡張計画 |
| [Windows / macOS W0調査](WINDOWS_MACOS_W0_FINDINGS.md) | 保存APIのネイティブ検証結果、採用候補、残る確認項目 |
| [Windows / macOS W2検証](WINDOWS_MACOS_W2_FINDINGS.md) | macOS保存APIの選択、CI検証、残る実端末確認 |
| [Windows / macOS W3検証](WINDOWS_MACOS_W3_FINDINGS.md) | Windows保存APIの選択、CI検証、残る実端末確認 |
| [Windows / macOS W4検証](WINDOWS_MACOS_W4_FINDINGS.md) | 通知方式と端末復元のCI検証、残る実表示確認 |
| [Windows / macOS W5a自動検証・W5b受入](WINDOWS_MACOS_W5_ACCEPTANCE.md) | 4 OS CIの結果、実端末受入手順と未実施項目 |

## 過去資料

| 文書 | 役割 |
| --- | --- |
| [Phase 2 Plan](archive/PHASE2_PLAN.md) | 完了したPhase 2の実行計画 |
| [Phase 3 Plan](archive/PHASE3_PLAN.md) | 完了したPhase 3の実行計画 |
| [Phase 4 Plan](archive/PHASE4_PLAN.md) | 完了したPhase 4の実行計画 |

過去資料の記述は作成当時の計画であり、現在の仕様や実装状況を示すものではない。
